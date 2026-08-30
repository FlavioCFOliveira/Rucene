//! Port of `org.apache.lucene.util.fst.NodeHash`.

use std::sync::atomic::AtomicI64;
use std::sync::Arc as StdArc;

use crate::error::{LuceneError, Result};
use crate::util::byte_block_pool::ByteBlockPool;
use crate::util::packed::PackedInts;

use super::byte_block_pool_reverse_bytes_reader::{
    pool_append, pool_append_from_pool, pool_position, pool_read_bytes,
    ByteBlockPoolReverseBytesReader,
};
use super::fst::{
    Arc, BitTable, BytesReader, ARCS_FOR_BINARY_SEARCH, ARCS_FOR_CONTINUOUS,
    ARCS_FOR_DIRECT_ADDRESSING, FINAL_END_NODE, FST, NON_FINAL_END_NODE,
};
use super::fst_compiler::{FSTCompiler, UnCompiledNode};
use super::outputs::Outputs;

/// Multiplier of the rolling node hash.
///
/// Equivalent to the local constant `PRIME` in `NodeHash`.
const PRIME: i64 = 31;

/// De-duplicates states, that is, looks up already frozen states.
///
/// Equivalent to the package-private `org.apache.lucene.util.fst.NodeHash<T>`.
///
/// Nodes are added to the primary table until it reaches half of the requested
/// RAM limit; the primary table then becomes the read-only fallback table and a
/// fresh primary table is started. Finding a node in the fallback table
/// promotes it back to the primary one, which gives a cheap approximation of
/// LRU behaviour.
///
/// # Java to Rust adaptations
///
/// * Lucene's `NodeHash` holds a reference back to the `FSTCompiler` that owns
///   it. Here the compiler is passed to [`NodeHash::add`] instead.
/// * The two address tables are `PagedGrowableWriter`s in Lucene, that is,
///   bit-packed paged arrays. `org.apache.lucene.util.packed.PagedGrowableWriter`
///   is not ported yet, so this port stores them in `Vec<i64>`. This costs more
///   memory per hash slot but cannot change a single FST byte: the tables only
///   decide hash-slot placement, and the RAM accounting below is computed from
///   [`PackedInts::bits_required`] rather than from the tables' real footprint,
///   exactly as in Lucene.
/// * The hash values themselves never reach the index either. Two equal nodes
///   always hash alike, so the set of de-duplicated nodes -- and therefore the
///   bytes written -- is the same whatever the hash function is. Lucene relies
///   on the same property, since `BytesRef.hashCode()` is seeded from a
///   per-JVM random value.
pub struct NodeHash<O: Outputs> {
    /// Primary table: nodes are added here until it reaches half of
    /// [`NodeHash::ram_limit_bytes`], at which point it becomes the fallback.
    primary_table: PagedGrowableHash,

    /// How many bytes the primary and fallback tables together are allowed to
    /// use.
    ram_limit_bytes: i64,

    /// Read-only fallback table; finding a frozen node here promotes it to the
    /// primary table.
    fallback_table: Option<PagedGrowableHash>,

    scratch_arc: Arc<O::Output>,

    /// Length of the last node found by [`NodeHash::get_fallback`].
    last_fallback_node_length: i32,

    /// Hash slot of the last node found by [`NodeHash::get_fallback`].
    last_fallback_hash_slot: i64,
}

impl<O: Outputs> NodeHash<O> {
    /// Creates a suffix cache bounded by `ram_limit_mb` megabytes.
    ///
    /// Equivalent to `new NodeHash(FSTCompiler, double)`. When the limit is
    /// hit, the least recently used suffixes are discarded and the FST is no
    /// longer minimal; a larger limit brings it closer to minimal.
    ///
    /// # Panics
    ///
    /// The caller must pass a strictly positive limit; `FSTCompiler` only
    /// builds a `NodeHash` when `suffixRAMLimitMB > 0`, which is the same guard
    /// Lucene's constructor enforces with an `IllegalArgumentException`.
    pub fn new(ram_limit_mb: f64) -> Self {
        debug_assert!(ram_limit_mb > 0.0, "ramLimitMB must be > 0");
        let as_bytes = ram_limit_mb * 1024.0 * 1024.0;
        let ram_limit_bytes = if as_bytes >= i64::MAX as f64 {
            // Quietly truncate to i64::MAX in bytes too.
            i64::MAX
        } else {
            as_bytes as i64
        };

        Self {
            primary_table: PagedGrowableHash::new(),
            ram_limit_bytes,
            fallback_table: None,
            scratch_arc: Arc::default(),
            last_fallback_node_length: -1,
            last_fallback_hash_slot: -1,
        }
    }

    /// Looks the node up in the fallback table, returning its address or `0`.
    ///
    /// Equivalent to the private `NodeHash.getFallback`.
    fn get_fallback(
        &mut self,
        fst: &FST<O>,
        node_in: &UnCompiledNode<O::Output>,
        hash: i64,
    ) -> Result<i64> {
        let NodeHash {
            fallback_table,
            scratch_arc,
            last_fallback_node_length,
            last_fallback_hash_slot,
            ..
        } = self;
        *last_fallback_node_length = -1;
        *last_fallback_hash_slot = -1;
        let Some(fallback_table) = fallback_table.as_ref() else {
            // No fallback yet: the primary table is not large enough to swap.
            return Ok(0);
        };
        let mut hash_slot = hash & fallback_table.mask;
        let mut c = 0i64;
        loop {
            let node_address = fallback_table.node_address(hash_slot);
            if node_address == 0 {
                // Not found.
                return Ok(0);
            }
            let length =
                fallback_table.nodes_equal(fst, scratch_arc, node_in, node_address, hash_slot)?;
            if length != -1 {
                // Store the node length for further use.
                *last_fallback_node_length = length;
                *last_fallback_hash_slot = hash_slot;
                // A frozen version of this node is already here.
                return Ok(node_address);
            }

            // Quadratic probe.
            c += 1;
            hash_slot = (hash_slot + c) & fallback_table.mask;
        }
    }

    /// Returns the address of the frozen form of `node_in`, freezing and adding
    /// it when it is not already known.
    ///
    /// Equivalent to `NodeHash.add`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while freezing the node or reading a frozen
    /// one back.
    pub fn add(
        &mut self,
        compiler: &mut FSTCompiler<'_, O>,
        node_in: &UnCompiledNode<O::Output>,
    ) -> Result<i64> {
        let hash = hash_uncompiled_node(compiler.fst().outputs(), node_in);

        let mut hash_slot = hash & self.primary_table.mask;
        let mut c = 0i64;

        loop {
            let existing = self.primary_table.node_address(hash_slot);
            if existing == 0 {
                // The node is not in the primary table; is it in the fallback?
                let mut node_address = self.get_fallback(compiler.fst(), node_in, hash)?;
                if node_address != 0 {
                    debug_assert!(
                        self.last_fallback_hash_slot != -1 && self.last_fallback_node_length != -1
                    );

                    // It was already in the fallback: promote it to primary.
                    self.primary_table.set_node_address(hash_slot, node_address);
                    let fallback_hash_slot = self.last_fallback_hash_slot;
                    let node_length = self.last_fallback_node_length as usize;
                    let fallback_table = self
                        .fallback_table
                        .as_ref()
                        .expect("INVARIANT: get_fallback only succeeds with a fallback table");
                    self.primary_table.copy_fallback_node_bytes(
                        hash_slot,
                        fallback_table,
                        fallback_hash_slot,
                        node_length,
                    )?;
                } else {
                    // Not in the fallback either: freeze and add the incoming
                    // node.
                    node_address = compiler.add_node(node_in)?;

                    // 0 is the empty marker of the hash table, so it had better
                    // be impossible to get a frozen node at 0.
                    debug_assert!(
                        node_address != FINAL_END_NODE && node_address != NON_FINAL_END_NODE
                    );

                    self.primary_table.set_node_address(hash_slot, node_address);
                    let scratch_len = compiler.scratch_bytes.position();
                    self.primary_table.copy_node_bytes(
                        hash_slot,
                        compiler.scratch_bytes.bytes(),
                        scratch_len,
                    )?;
                }

                // How many bytes would be used with "perfect" hashing:
                //  - x2 for the FST node address,
                //  - x2 for the copied node address,
                //  - plus the bytes copied out of the FST into copied_nodes.
                // Each address accounts for the approximate hash table overhead,
                // halfway between 33.3% and 66.6%. Note that some of the copied
                // nodes are shared between the fallback and primary tables, so
                // this computation is pessimistic.
                //
                // The more precise RAM figure is deliberately not used: it leads
                // to unpredictable quantized behaviour, because the 2x rehashing
                // leaves the FST size unchanged over large ranges of the limit
                // and then drops suddenly at a secret threshold. Measuring
                // "perfect" hash storage and approximating the overhead makes the
                // behaviour strictly monotonic instead.
                let copied_bytes = pool_position(&self.primary_table.copied_nodes);
                let ram_bytes_used = self.primary_table.count
                    * 2
                    * i64::from(PackedInts::bits_required(node_address)?)
                    / 8
                    + self.primary_table.count
                        * 2
                        * i64::from(PackedInts::bits_required(copied_bytes)?)
                        / 8
                    + copied_bytes;

                // Divide the limit by 2 because the fallback gets half the RAM
                // and the primary gets the other half.
                if ram_bytes_used >= self.ram_limit_bytes / 2 {
                    // Time to fall back: the fallback table is now used
                    // read-only, to promote a node (suffix) to primary if it is
                    // encountered again.
                    let new_size = 16.max(self.primary_table.fst_node_address.len());
                    let old_primary = std::mem::replace(
                        &mut self.primary_table,
                        PagedGrowableHash::with_size(new_size),
                    );
                    self.fallback_table = Some(old_primary);
                } else if (self.primary_table.count as f32)
                    > self.primary_table.fst_node_address.len() as f32 * (2.0 / 3.0)
                {
                    // Rehash at 2/3 occupancy.
                    let NodeHash {
                        primary_table,
                        scratch_arc,
                        ..
                    } = self;
                    primary_table.rehash(compiler.fst(), scratch_arc, node_address)?;
                }

                return Ok(node_address);
            }

            let NodeHash {
                primary_table,
                scratch_arc,
                ..
            } = self;
            if primary_table.nodes_equal(
                compiler.fst(),
                scratch_arc,
                node_in,
                existing,
                hash_slot,
            )? != -1
            {
                // The same node, in frozen form, is already in the primary table.
                return Ok(existing);
            }

            // Quadratic probe.
            c += 1;
            hash_slot = (hash_slot + c) & self.primary_table.mask;
        }
    }
}

/// Hash code of an unfrozen node.
///
/// Equivalent to the private `NodeHash.hash(UnCompiledNode)`. This must be
/// identical to [`PagedGrowableHash::hash`], the frozen case.
fn hash_uncompiled_node<O: Outputs>(outputs: &O, node: &UnCompiledNode<O::Output>) -> i64 {
    let mut h: i64 = 0;
    for arc_idx in 0..node.num_arcs {
        let arc = &node.arcs[arc_idx];
        h = PRIME.wrapping_mul(h).wrapping_add(i64::from(arc.label));
        let n = arc.target;
        h = PRIME
            .wrapping_mul(h)
            .wrapping_add(i64::from((n ^ (n >> 32)) as i32));
        h = PRIME
            .wrapping_mul(h)
            .wrapping_add(outputs.output_hash(&arc.output));
        h = PRIME
            .wrapping_mul(h)
            .wrapping_add(outputs.output_hash(&arc.next_final_output));
        if arc.is_final {
            h = h.wrapping_add(17);
        }
    }
    h
}

/// Block size of the copied-node arena.
///
/// Equivalent to `NodeHash.PagedGrowableHash.BLOCK_SIZE_BYTES`; this port lets
/// [`ByteBlockPool`] pick its own 32 KB blocks, so the constant only documents
/// the original value.
const _BLOCK_SIZE_BYTES: usize = 1 << 18;

/// The open-addressing hash table backing one generation of the suffix cache.
///
/// Equivalent to the inner class `NodeHash.PagedGrowableHash`.
struct PagedGrowableHash {
    /// FST node address, at the slot given by the masked hash of the node arcs.
    fst_node_address: Vec<i64>,
    /// Address inside [`PagedGrowableHash::copied_nodes`] of the node stored at
    /// the same slot as in [`PagedGrowableHash::fst_node_address`].
    copied_node_address: Vec<i64>,
    count: i64,
    mask: i64,
    /// Byte slices copied out of the FST for the nodes added to the hash, so
    /// that they do not have to be read back from the FST itself and the FST
    /// bytes can stream straight to disk as append-only writes.
    copied_nodes: ByteBlockPool,
}

impl PagedGrowableHash {
    /// Creates a table with 16 slots.
    ///
    /// Equivalent to `new PagedGrowableHash()`.
    fn new() -> Self {
        Self::with_size(16)
    }

    /// Creates a table with `size` slots, which must be a power of two.
    ///
    /// Equivalent to `new PagedGrowableHash(long, long)`; the `lastNodeAddress`
    /// argument only sizes the bit-packed writer Lucene uses and has no
    /// counterpart here.
    fn with_size(size: usize) -> Self {
        debug_assert!(size.is_power_of_two(), "size must be a power of two");
        Self {
            fst_node_address: vec![0; size],
            copied_node_address: vec![0; size],
            count: 0,
            mask: size as i64 - 1,
            copied_nodes: ByteBlockPool::new(StdArc::new(AtomicI64::new(0))),
        }
    }

    /// Returns the copied bytes at the provided hash slot.
    ///
    /// Equivalent to `PagedGrowableHash.getBytes`.
    ///
    /// # Errors
    ///
    /// Propagates the errors of the underlying arena.
    #[allow(dead_code)]
    fn bytes(&self, hash_slot: i64, length: usize) -> Result<Vec<u8>> {
        let address = self.copied_node_address[hash_slot as usize];
        debug_assert!(address - length as i64 + 1 >= 0);
        let mut buf = vec![0u8; length];
        pool_read_bytes(
            &self.copied_nodes,
            address - length as i64 + 1,
            &mut buf,
            0,
            length,
        )?;
        Ok(buf)
    }

    /// Returns the node address stored at the provided hash slot.
    ///
    /// Equivalent to `PagedGrowableHash.getNodeAddress`.
    fn node_address(&self, hash_slot: i64) -> i64 {
        self.fst_node_address[hash_slot as usize]
    }

    /// Stores a node address at the provided hash slot.
    ///
    /// Equivalent to `PagedGrowableHash.setNodeAddress`.
    fn set_node_address(&mut self, hash_slot: i64, node_address: i64) {
        debug_assert_eq!(self.fst_node_address[hash_slot as usize], 0);
        self.fst_node_address[hash_slot as usize] = node_address;
        self.count += 1;
    }

    /// Copies the node bytes out of the FST.
    ///
    /// Equivalent to `PagedGrowableHash.copyNodeBytes`.
    fn copy_node_bytes(&mut self, hash_slot: i64, bytes: &[u8], length: usize) -> Result<()> {
        debug_assert_eq!(self.copied_node_address[hash_slot as usize], 0);
        pool_append(&mut self.copied_nodes, bytes, 0, length)?;
        // Write the offset, which points to the last byte of the node just
        // copied, since the node is later read in reverse.
        self.copied_node_address[hash_slot as usize] = pool_position(&self.copied_nodes) - 1;
        Ok(())
    }

    /// Promotes the node bytes from the fallback table.
    ///
    /// Equivalent to `PagedGrowableHash.copyFallbackNodeBytes`.
    fn copy_fallback_node_bytes(
        &mut self,
        hash_slot: i64,
        fallback_table: &PagedGrowableHash,
        fallback_hash_slot: i64,
        node_length: usize,
    ) -> Result<()> {
        debug_assert_eq!(self.copied_node_address[hash_slot as usize], 0);
        let fallback_address = fallback_table.copied_node_address[fallback_hash_slot as usize];
        // fallback_address is the last offset of the node, but the bytes have
        // to be copied from the start address.
        let fallback_start_address = fallback_address - node_length as i64 + 1;
        debug_assert!(fallback_start_address >= 0);
        pool_append_from_pool(
            &mut self.copied_nodes,
            &fallback_table.copied_nodes,
            fallback_start_address,
            node_length,
        )?;
        // Write the offset, which points to the last byte of the node just
        // copied, since the node is later read in reverse.
        self.copied_node_address[hash_slot as usize] = pool_position(&self.copied_nodes) - 1;
        Ok(())
    }

    /// Doubles the table size and re-inserts every entry.
    ///
    /// Equivalent to the private `PagedGrowableHash.rehash`.
    fn rehash<O: Outputs>(
        &mut self,
        fst: &FST<O>,
        scratch_arc: &mut Arc<O::Output>,
        _last_node_address: i64,
    ) -> Result<()> {
        // Double the hash table size on each rehash.
        let new_size = 2 * self.fst_node_address.len();
        let mut new_copied_node_address = vec![0i64; new_size];
        let mut new_fst_node_address = vec![0i64; new_size];
        let new_mask = new_size as i64 - 1;
        for idx in 0..self.fst_node_address.len() {
            let address = self.fst_node_address[idx];
            if address != 0 {
                let mut hash_slot = self.hash(fst, scratch_arc, address, idx as i64)? & new_mask;
                let mut c = 0i64;
                loop {
                    if new_fst_node_address[hash_slot as usize] == 0 {
                        new_fst_node_address[hash_slot as usize] = address;
                        new_copied_node_address[hash_slot as usize] = self.copied_node_address[idx];
                        break;
                    }

                    // Quadratic probe.
                    c += 1;
                    hash_slot = (hash_slot + c) & new_mask;
                }
            }
        }

        self.mask = new_mask;
        self.fst_node_address = new_fst_node_address;
        self.copied_node_address = new_copied_node_address;
        Ok(())
    }

    /// Hash code of a frozen node.
    ///
    /// Equivalent to the private `PagedGrowableHash.hash(long, long)`. This must
    /// match [`hash_uncompiled_node`] precisely.
    fn hash<O: Outputs>(
        &self,
        fst: &FST<O>,
        scratch_arc: &mut Arc<O::Output>,
        node_address: i64,
        hash_slot: i64,
    ) -> Result<i64> {
        let mut input = self.bytes_reader(node_address, hash_slot);

        let mut h: i64 = 0;
        fst.read_first_real_target_arc(node_address, scratch_arc, &mut input)?;
        loop {
            h = PRIME
                .wrapping_mul(h)
                .wrapping_add(i64::from(scratch_arc.label()));
            let target = scratch_arc.target();
            h = PRIME
                .wrapping_mul(h)
                .wrapping_add(i64::from((target ^ (target >> 32)) as i32));
            h = PRIME
                .wrapping_mul(h)
                .wrapping_add(fst.outputs().output_hash(scratch_arc.output()));
            h = PRIME
                .wrapping_mul(h)
                .wrapping_add(fst.outputs().output_hash(scratch_arc.next_final_output()));
            if scratch_arc.is_final() {
                h = h.wrapping_add(17);
            }
            if scratch_arc.is_last() {
                break;
            }
            fst.read_next_real_arc(scratch_arc, &mut input)?;
        }

        Ok(h)
    }

    /// Compares an unfrozen node with the frozen node at byte location
    /// `address`, returning the node length when they are equal and `-1`
    /// otherwise.
    ///
    /// Equivalent to the private `PagedGrowableHash.nodesEqual`. The node length
    /// is used to promote the node from the fallback table to the primary one.
    fn nodes_equal<O: Outputs>(
        &self,
        fst: &FST<O>,
        scratch_arc: &mut Arc<O::Output>,
        node: &UnCompiledNode<O::Output>,
        address: i64,
        hash_slot: i64,
    ) -> Result<i32> {
        let mut input = self.bytes_reader(address, hash_slot);
        fst.read_first_real_target_arc(address, scratch_arc, &mut input)?;

        // Fail fast for a node with fixed length arcs.
        if scratch_arc.bytes_per_arc() != 0 {
            debug_assert!(node.num_arcs > 0);
            // The frozen node uses fixed-width arc encoding, the same number of
            // bytes per arc, but may be sparse or dense.
            match scratch_arc.node_flags() {
                ARCS_FOR_BINARY_SEARCH => {
                    // Sparse.
                    if node.num_arcs as i32 != scratch_arc.num_arcs() {
                        return Ok(-1);
                    }
                }
                ARCS_FOR_DIRECT_ADDRESSING => {
                    // Dense: compare both the number of labels allocated in the
                    // array, some of which may not actually be arcs, and the
                    // number of arcs.
                    let label_range = node.arcs[node.num_arcs - 1].label - node.arcs[0].label + 1;
                    if label_range != scratch_arc.num_arcs()
                        || node.num_arcs as i32 != BitTable::count_bits(scratch_arc, &mut input)?
                    {
                        return Ok(-1);
                    }
                }
                ARCS_FOR_CONTINUOUS => {
                    let label_range = node.arcs[node.num_arcs - 1].label - node.arcs[0].label + 1;
                    if label_range != scratch_arc.num_arcs() {
                        return Ok(-1);
                    }
                }
                other => {
                    return Err(LuceneError::IllegalState(format!(
                        "unhandled scratchArc.nodeFlags() {other}"
                    )));
                }
            }
        }

        // Compare arc by arc to see whether there is a difference.
        for arc_upto in 0..node.num_arcs {
            let arc = &node.arcs[arc_upto];
            if arc.label != scratch_arc.label()
                || !fst.outputs().equals(&arc.output, scratch_arc.output())
                || arc.target != scratch_arc.target()
                || !fst
                    .outputs()
                    .equals(&arc.next_final_output, scratch_arc.next_final_output())
                || arc.is_final != scratch_arc.is_final()
            {
                return Ok(-1);
            }

            if scratch_arc.is_last() {
                return if arc_upto == node.num_arcs - 1 {
                    // The position is one index past the starting address, as
                    // the node is read backwards.
                    i32::try_from(address - input.position()).map_err(|_| {
                        LuceneError::IllegalState(format!(
                            "node length {} does not fit in an i32",
                            address - input.position()
                        ))
                    })
                } else {
                    Ok(-1)
                };
            }

            fst.read_next_real_arc(scratch_arc, &mut input)?;
        }

        // The unfrozen node has fewer arcs than the frozen node.
        Ok(-1)
    }

    /// Returns a reader positioned so that FST addresses can be used directly.
    ///
    /// Equivalent to the private `PagedGrowableHash.getBytesReader`.
    fn bytes_reader(
        &self,
        node_address: i64,
        hash_slot: i64,
    ) -> ByteBlockPoolReverseBytesReader<'_> {
        // Make sure the node address and the hash slot are consistent.
        debug_assert_eq!(self.fst_node_address[hash_slot as usize], node_address);
        let local_address = self.copied_node_address[hash_slot as usize];
        let mut reader = ByteBlockPoolReverseBytesReader::new(&self.copied_nodes);
        reader.set_pos_delta(node_address - local_address);
        reader
    }
}
