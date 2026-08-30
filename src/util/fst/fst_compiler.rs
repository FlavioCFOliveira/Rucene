//! Port of `org.apache.lucene.util.fst.FSTCompiler`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`FSTCompiler`] | `FSTCompiler<T>` |
//! | [`Builder`] | `FSTCompiler.Builder<T>` |
//! | [`CompilerArc`] | `FSTCompiler.Arc<T>` |
//! | [`CompiledNode`] | `FSTCompiler.CompiledNode` |
//! | [`UnCompiledNode`] | `FSTCompiler.UnCompiledNode<T>` |
//! | [`FixedLengthArcsBuffer`] | `FSTCompiler.FixedLengthArcsBuffer` |
//! | [`FstOutputSink`] | the `DataOutput` field of `FSTCompiler` |
//! | [`NullFSTReader`] | `FSTCompiler.NullFSTReader` |

use crate::error::{LuceneError, Result};
use crate::store::DataOutput;
use crate::util::{Accountable, ArrayUtil, IntsRef, IntsRefBuilder};

use super::fst::{
    get_num_presence_bytes, InputType, ARCS_FOR_BINARY_SEARCH, ARCS_FOR_CONTINUOUS,
    ARCS_FOR_DIRECT_ADDRESSING, BIT_ARC_HAS_FINAL_OUTPUT, BIT_ARC_HAS_OUTPUT, BIT_FINAL_ARC,
    BIT_LAST_ARC, BIT_STOP_NODE, BIT_TARGET_NEXT, FINAL_END_NODE, FST, NON_FINAL_END_NODE,
    VERSION_90, VERSION_CONTINUOUS_ARCS, VERSION_CURRENT,
};
use super::fst::{BytesReader, FSTMetadata};
use super::fst_reader::FSTReader;
use super::growable_byte_array_data_output::GrowableByteArrayDataOutput;
use super::node_hash::NodeHash;
use super::outputs::Outputs;
use super::read_write_data_output::ReadWriteDataOutput;

/// Default maximum oversizing of a fixed arc array allowed to enable direct
/// addressing instead of binary search.
///
/// Equivalent to `FSTCompiler.DIRECT_ADDRESSING_MAX_OVERSIZING_FACTOR`.
pub const DIRECT_ADDRESSING_MAX_OVERSIZING_FACTOR: f32 = 1.0;

/// Depth up to which a node is expanded with fixed length arcs when it has at
/// least [`FIXED_LENGTH_ARC_SHALLOW_NUM_ARCS`] arcs; `0` means the root only.
///
/// Equivalent to `FSTCompiler.FIXED_LENGTH_ARC_SHALLOW_DEPTH`.
pub const FIXED_LENGTH_ARC_SHALLOW_DEPTH: usize = 3;

/// Number of arcs from which a shallow node is expanded with fixed length arcs.
///
/// Equivalent to `FSTCompiler.FIXED_LENGTH_ARC_SHALLOW_NUM_ARCS`.
pub const FIXED_LENGTH_ARC_SHALLOW_NUM_ARCS: usize = 5;

/// Number of arcs from which any node is expanded with fixed length arcs.
///
/// Equivalent to `FSTCompiler.FIXED_LENGTH_ARC_DEEP_NUM_ARCS`.
pub const FIXED_LENGTH_ARC_DEEP_NUM_ARCS: usize = 10;

/// Maximum oversizing factor allowed for direct addressing compared to binary
/// search when expansion credits allow the oversizing.
///
/// Equivalent to `FSTCompiler.DIRECT_ADDRESSING_MAX_OVERSIZE_WITH_CREDIT_FACTOR`.
const DIRECT_ADDRESSING_MAX_OVERSIZE_WITH_CREDIT_FACTOR: f32 = 1.66;

/// Target address of an arc whose target node has not been compiled yet.
///
/// Lucene models this with the `FSTCompiler.Node` interface, whose two
/// implementations are `UnCompiledNode` and `CompiledNode`; only the compiled
/// address is ever read. This port stores the address directly and uses this
/// sentinel, which is the same `-2` that Lucene's commented-out assertion in
/// `UnCompiledNode.replaceLast` refers to, until the target is compiled.
const NOT_COMPILED: i64 = -2;

/// Returns an on-heap [`ReadWriteDataOutput`] that allows the FST to be read
/// immediately after writing, and optionally saved to an external
/// [`DataOutput`].
///
/// Equivalent to the static `FSTCompiler.getOnHeapReaderWriter(int)`, which is
/// declared to return `DataOutput` and is cast to `ReadWriteDataOutput` at
/// every call site; this port returns the concrete type.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `block_bits` is outside
/// `1 ..= 31`.
pub fn get_on_heap_reader_writer(block_bits: i32) -> Result<ReadWriteDataOutput> {
    ReadWriteDataOutput::new(block_bits)
}

/// The sink the FST bytes are streamed to.
///
/// Equivalent to the `DataOutput dataOutput` field of `FSTCompiler`, together
/// with the `instanceof` checks Lucene performs on it: `ReadWriteDataOutput`
/// can be frozen, read back and accounted for, while any other `DataOutput`
/// can only be written to. Modelling the two cases as an enum replaces those
/// runtime type tests with an exhaustive match.
pub enum FstOutputSink<'a> {
    /// An on-heap output that doubles as an [`FSTReader`], as returned by
    /// [`get_on_heap_reader_writer`].
    OnHeap(ReadWriteDataOutput),
    /// Any other [`DataOutput`], for example an
    /// [`crate::store::IndexOutput`]; the FST is streamed to it and can only
    /// be used after being read back.
    Streaming(&'a mut dyn DataOutput),
}

impl FstOutputSink<'_> {
    /// Freezes the sink when it is the on-heap one.
    ///
    /// Equivalent to the `dataOutput instanceof ReadWriteDataOutput` check in
    /// `FSTCompiler.finish`.
    fn freeze(&mut self) {
        if let FstOutputSink::OnHeap(output) = self {
            output.freeze();
        }
    }

    /// Returns the memory used by the sink, or `0` when it is not accountable.
    ///
    /// Equivalent to the `dataOutput instanceof Accountable` check in
    /// `FSTCompiler.fstRamBytesUsed`.
    fn ram_bytes_used(&self) -> i64 {
        match self {
            FstOutputSink::OnHeap(output) => output.ram_bytes_used(),
            FstOutputSink::Streaming(_) => 0,
        }
    }
}

impl<'a> From<ReadWriteDataOutput> for FstOutputSink<'a> {
    fn from(output: ReadWriteDataOutput) -> Self {
        FstOutputSink::OnHeap(output)
    }
}

impl DataOutput for FstOutputSink<'_> {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        match self {
            FstOutputSink::OnHeap(output) => output.write_byte(b),
            FstOutputSink::Streaming(output) => output.write_byte(b),
        }
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        match self {
            FstOutputSink::OnHeap(output) => output.write_bytes(b, offset, len),
            FstOutputSink::Streaming(output) => output.write_bytes(b, offset, len),
        }
    }
}

/// The [`FSTReader`] of an FST that was not built with a readable
/// [`DataOutput`].
///
/// Equivalent to the private `FSTCompiler.NullFSTReader`: it refuses both
/// reading and writing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullFSTReader;

impl Accountable for NullFSTReader {
    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

impl FSTReader for NullFSTReader {
    fn get_reverse_bytes_reader(&self) -> Result<Box<dyn BytesReader + '_>> {
        Err(LuceneError::UnsupportedOperation(
            "FST was not constructed with get_on_heap_reader_writer()".to_string(),
        ))
    }

    fn write_to(&self, _out: &mut dyn DataOutput) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "FST was not constructed with get_on_heap_reader_writer()".to_string(),
        ))
    }
}

/// A pending, seen but not yet serialized, arc.
///
/// Equivalent to the package-private `FSTCompiler.Arc<T>`.
#[derive(Debug, Clone, Default)]
pub struct CompilerArc<T> {
    /// The arc label; really an unsigned byte for `BYTE1` inputs.
    pub label: i32,
    /// Address of the compiled target node, or [`NOT_COMPILED`] until
    /// [`UnCompiledNode::replace_last`] fills it in.
    pub target: i64,
    /// Whether the arc accepts the input read so far.
    pub is_final: bool,
    /// The arc output.
    pub output: T,
    /// The final output of the target node.
    pub next_final_output: T,
}

/// A node that has already been serialized, identified by its address.
///
/// Equivalent to `FSTCompiler.CompiledNode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompiledNode {
    /// Address of the node in the FST byte stream.
    pub node: i64,
}

/// A pending, seen but not yet serialized, node.
///
/// Equivalent to `FSTCompiler.UnCompiledNode<T>`.
///
/// # Java to Rust adaptations
///
/// * Lucene's node holds an `owner` back-reference to the compiler, used only
///   to reach `NO_OUTPUT` and the `Outputs` instance. A back-reference of that
///   kind cannot be expressed in safe Rust without reference counting, so the
///   two values are passed in as arguments instead.
#[derive(Debug, Clone, Default)]
pub struct UnCompiledNode<T> {
    /// Number of arcs actually used in [`UnCompiledNode::arcs`].
    pub num_arcs: usize,
    /// The arcs of this node; the vector is oversized and only the first
    /// [`UnCompiledNode::num_arcs`] entries are meaningful.
    pub arcs: Vec<CompilerArc<T>>,
    /// The output produced when this node is final.
    pub output: T,
    /// Whether this node accepts the input read so far.
    pub is_final: bool,
    /// This node's depth, starting from the automaton root.
    pub depth: usize,
}

impl<T: Clone + Default> UnCompiledNode<T> {
    /// Creates an empty node at the given depth.
    ///
    /// Equivalent to `new UnCompiledNode(FSTCompiler, int)`.
    pub fn new(no_output: &T, depth: usize) -> Self {
        Self {
            num_arcs: 0,
            arcs: vec![CompilerArc::default()],
            output: no_output.clone(),
            is_final: false,
            depth,
        }
    }

    /// Resets this node for reuse, keeping its depth.
    ///
    /// Equivalent to `UnCompiledNode.clear`.
    pub fn clear(&mut self, no_output: &T) {
        self.num_arcs = 0;
        self.is_final = false;
        self.output = no_output.clone();
        // The depth never changes for nodes on the frontier, even when reused.
    }

    /// Returns the output of the last arc, which must carry `label_to_match`.
    ///
    /// Equivalent to `UnCompiledNode.getLastOutput`.
    pub fn get_last_output(&self, label_to_match: i32) -> &T {
        debug_assert!(self.num_arcs > 0);
        debug_assert_eq!(self.arcs[self.num_arcs - 1].label, label_to_match);
        &self.arcs[self.num_arcs - 1].output
    }

    /// Appends an arc with the given label, whose target is not compiled yet.
    ///
    /// Equivalent to `UnCompiledNode.addArc`.
    pub fn add_arc(&mut self, label: i32, no_output: &T) {
        debug_assert!(label >= 0);
        debug_assert!(self.num_arcs == 0 || label > self.arcs[self.num_arcs - 1].label);
        if self.num_arcs == self.arcs.len() {
            let new_len = ArrayUtil::oversize(self.num_arcs + 1, 8).max(self.num_arcs + 1);
            self.arcs.resize(new_len, CompilerArc::default());
        }
        let arc = &mut self.arcs[self.num_arcs];
        self.num_arcs += 1;
        arc.label = label;
        arc.target = NOT_COMPILED;
        arc.output = no_output.clone();
        arc.next_final_output = no_output.clone();
        arc.is_final = false;
    }

    /// Points the last arc at a compiled node.
    ///
    /// Equivalent to `UnCompiledNode.replaceLast`.
    pub fn replace_last(
        &mut self,
        label_to_match: i32,
        target: i64,
        next_final_output: T,
        is_final: bool,
    ) {
        debug_assert!(self.num_arcs > 0);
        let arc = &mut self.arcs[self.num_arcs - 1];
        debug_assert_eq!(arc.label, label_to_match);
        arc.target = target;
        arc.next_final_output = next_final_output;
        arc.is_final = is_final;
    }

    /// Sets the output of the last arc.
    ///
    /// Equivalent to `UnCompiledNode.setLastOutput`.
    pub fn set_last_output(&mut self, label_to_match: i32, new_output: T) {
        debug_assert!(self.num_arcs > 0);
        let arc = &mut self.arcs[self.num_arcs - 1];
        debug_assert_eq!(arc.label, label_to_match);
        arc.output = new_output;
    }

    /// Pushes an output prefix forward onto every arc.
    ///
    /// Equivalent to `UnCompiledNode.prependOutput`.
    pub fn prepend_output<O: Outputs<Output = T>>(&mut self, outputs: &O, output_prefix: &T) {
        for arc_idx in 0..self.num_arcs {
            self.arcs[arc_idx].output = outputs.add(output_prefix, &self.arcs[arc_idx].output);
        }

        if self.is_final {
            self.output = outputs.add(output_prefix, &self.output);
        }
    }
}

/// Reusable buffer for building nodes with fixed length arcs, that is, with
/// binary search or direct addressing.
///
/// Equivalent to `FSTCompiler.FixedLengthArcsBuffer`, which wraps a
/// `ByteArrayDataOutput`; this port writes into the array directly and gets the
/// `writeVInt` encoding from the [`DataOutput`] trait.
#[derive(Debug)]
pub struct FixedLengthArcsBuffer {
    bytes: Vec<u8>,
    position: usize,
}

impl FixedLengthArcsBuffer {
    /// Creates a buffer with the maximum length a fixed-length-arc node header
    /// needs: one flags byte plus two `VInt`s.
    ///
    /// Equivalent to the field initialiser `new byte[11]`.
    pub fn new() -> Self {
        Self {
            bytes: vec![0u8; 11],
            position: 0,
        }
    }

    /// Ensures the internal array can hold `capacity` bytes, enlarging and
    /// clearing it when it cannot.
    ///
    /// Equivalent to `FixedLengthArcsBuffer.ensureCapacity`, which also
    /// discards the previous content.
    pub fn ensure_capacity(&mut self, capacity: usize) -> &mut Self {
        if self.bytes.len() < capacity {
            self.bytes = vec![0u8; ArrayUtil::oversize(capacity, 1).max(capacity)];
        }
        self
    }

    /// Rewinds the write position to `0`.
    ///
    /// Equivalent to `FixedLengthArcsBuffer.resetPosition`.
    pub fn reset_position(&mut self) -> &mut Self {
        self.position = 0;
        self
    }

    /// Returns the current write position.
    ///
    /// Equivalent to `FixedLengthArcsBuffer.getPosition`.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Returns the internal array.
    ///
    /// Equivalent to `FixedLengthArcsBuffer.getBytes`.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the internal array for mutation.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

impl Default for FixedLengthArcsBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl DataOutput for FixedLengthArcsBuffer {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        if self.position >= self.bytes.len() {
            return Err(LuceneError::IllegalState(
                "FixedLengthArcsBuffer overflow".to_string(),
            ));
        }
        self.bytes[self.position] = b;
        self.position += 1;
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        if self.position + len > self.bytes.len() {
            return Err(LuceneError::IllegalState(
                "FixedLengthArcsBuffer overflow".to_string(),
            ));
        }
        self.bytes[self.position..self.position + len].copy_from_slice(&b[offset..offset + len]);
        self.position += len;
        Ok(())
    }
}

/// Fluent constructor for an [`FSTCompiler`].
///
/// Equivalent to `FSTCompiler.Builder<T>`.
pub struct Builder<'a, O: Outputs> {
    input_type: InputType,
    outputs: O,
    suffix_ram_limit_mb: f64,
    allow_fixed_length_arcs: bool,
    data_output: Option<FstOutputSink<'a>>,
    direct_addressing_max_oversizing_factor: f32,
    version: i32,
}

impl<'a, O: Outputs> Builder<'a, O> {
    /// Creates a builder for the given input type and outputs.
    ///
    /// Equivalent to `new FSTCompiler.Builder(FST.INPUT_TYPE, Outputs)`.
    /// Strings are represented as [`InputType::Byte4`], that is, full Unicode
    /// code points; use [`super::no_outputs::NoOutputs`] to build an FSA.
    pub fn new(input_type: InputType, outputs: O) -> Self {
        Self {
            input_type,
            outputs,
            suffix_ram_limit_mb: 32.0,
            allow_fixed_length_arcs: true,
            data_output: None,
            direct_addressing_max_oversizing_factor: DIRECT_ADDRESSING_MAX_OVERSIZING_FACTOR,
            version: VERSION_CURRENT,
        }
    }

    /// Sets the approximate maximum amount of RAM, in megabytes, used to hold
    /// the suffix cache that lets the FST share common suffixes.
    ///
    /// Equivalent to `Builder.suffixRAMLimitMB`. Pass [`f64::INFINITY`] to keep
    /// every suffix and build an exactly minimal FST, or `0` to disable suffix
    /// sharing entirely. This is not a precise limit: the implementation
    /// approximates the overhead of its hash tables.
    ///
    /// Default: `32.0` MB.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `mb` is negative.
    pub fn suffix_ram_limit_mb(mut self, mb: f64) -> Result<Self> {
        if mb < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "suffixRAMLimitMB must be >= 0; got: {mb}"
            )));
        }
        self.suffix_ram_limit_mb = mb;
        Ok(self)
    }

    /// Enables or disables the fixed length arc optimisation, that is, binary
    /// search and direct addressing.
    ///
    /// Equivalent to `Builder.allowFixedLengthArcs`. Default: `true`.
    pub fn allow_fixed_length_arcs(mut self, allow_fixed_length_arcs: bool) -> Self {
        self.allow_fixed_length_arcs = allow_fixed_length_arcs;
        self
    }

    /// Sets the sink used for low-level writing of the FST.
    ///
    /// Equivalent to `Builder.dataOutput`. Use
    /// [`FstOutputSink::OnHeap`] with [`get_on_heap_reader_writer`] to make the
    /// FST immediately readable; otherwise the FST has to be read back from the
    /// corresponding input.
    pub fn data_output(mut self, data_output: FstOutputSink<'a>) -> Self {
        self.data_output = Some(data_output);
        self
    }

    /// Overrides the maximum oversizing of a fixed arc array allowed to enable
    /// direct addressing of arcs instead of binary search.
    ///
    /// Equivalent to `Builder.directAddressingMaxOversizingFactor`. A negative
    /// value effectively disables direct addressing. This factor does not
    /// decide whether a node uses variable or fixed length arcs, only how a
    /// fixed-length node is encoded. Default: `1`.
    pub fn direct_addressing_max_oversizing_factor(mut self, factor: f32) -> Self {
        self.direct_addressing_max_oversizing_factor = factor;
        self
    }

    /// Expert: sets the codec version to write.
    ///
    /// Equivalent to `Builder.setVersion`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `version` is outside
    /// `VERSION_90 ..= VERSION_CURRENT`.
    pub fn set_version(mut self, version: i32) -> Result<Self> {
        if !(VERSION_90..=VERSION_CURRENT).contains(&version) {
            return Err(LuceneError::IllegalArgument(format!(
                "Expected version in range [{VERSION_90}, {VERSION_CURRENT}], got {version}"
            )));
        }
        self.version = version;
        Ok(self)
    }

    /// Creates the [`FSTCompiler`].
    ///
    /// Equivalent to `Builder.build`.
    ///
    /// # Errors
    ///
    /// Propagates the failure to allocate the default on-heap sink.
    pub fn build(self) -> Result<FSTCompiler<'a, O>> {
        let data_output = match self.data_output {
            Some(data_output) => data_output,
            // Create a default DataOutput if none was specified.
            None => FstOutputSink::OnHeap(get_on_heap_reader_writer(15)?),
        };
        Ok(FSTCompiler::new(
            self.input_type,
            self.suffix_ram_limit_mb,
            self.outputs,
            self.allow_fixed_length_arcs,
            data_output,
            self.direct_addressing_max_oversizing_factor,
            self.version,
        ))
    }
}

/// Builds a minimal FST, mapping an [`IntsRef`] term to an arbitrary output,
/// from pre-sorted terms with outputs.
///
/// Equivalent to `org.apache.lucene.util.fst.FSTCompiler<T>`. The FST becomes
/// an FSA when [`super::no_outputs::NoOutputs`] is used. It is written
/// on the fly into the compact serialized format, which can be saved to a
/// directory or traversed directly, and it is always finite: there are no
/// cycles.
///
/// The algorithm is the one described in
/// "Direct construction of minimal acyclic subsequential transducers"
/// (<http://citeseerx.ist.psu.edu/viewdoc/summary?doi=10.1.1.24.3698>).
///
/// # Java to Rust adaptations
///
/// * Lucene's `NodeHash` holds a reference back to the compiler. Here the
///   suffix cache is owned by the compiler and temporarily moved out while it
///   runs, so that it can be handed the compiler it needs.
/// * The `DataOutput` field is the [`FstOutputSink`] enum; see its
///   documentation.
pub struct FSTCompiler<'a, O: Outputs> {
    dedup_hash: Option<NodeHash<O>>,
    /// A temporary FST used while building, for the [`NodeHash`] cache.
    pub(crate) fst: FST<O>,
    no_output: O::Output,

    last_input: IntsRefBuilder,

    /// Whether the padding byte still has to be written.
    padding_byte_pending: bool,

    /// The current frontier.
    frontier: Vec<UnCompiledNode<O::Output>>,

    /// Used for the `BIT_TARGET_NEXT` optimisation: instead of storing the
    /// address of an arc's target node, a single bit notes that the next node
    /// in the byte stream is the target.
    pub(crate) last_frozen_node: i64,

    // Reused temporarily while building the FST.
    num_bytes_per_arc: Vec<i32>,
    num_label_bytes_per_arc: Vec<i32>,
    fixed_length_arcs_buffer: FixedLengthArcsBuffer,

    arc_count: i64,
    node_count: i64,
    binary_search_node_count: i64,
    direct_addressing_node_count: i64,
    continuous_node_count: i64,

    allow_fixed_length_arcs: bool,
    direct_addressing_max_oversizing_factor: f32,
    version: i32,
    direct_addressing_expansion_credit: i64,

    /// The sink the FST bytes are streamed to.
    data_output: FstOutputSink<'a>,

    /// Buffer holding the bytes of the one node currently being written.
    pub(crate) scratch_bytes: GrowableByteArrayDataOutput,

    num_bytes_written: i64,
}

impl<'a, O: Outputs> FSTCompiler<'a, O> {
    /// Equivalent to the private `FSTCompiler` constructor.
    fn new(
        input_type: InputType,
        suffix_ram_limit_mb: f64,
        outputs: O,
        allow_fixed_length_arcs: bool,
        data_output: FstOutputSink<'a>,
        direct_addressing_max_oversizing_factor: f32,
        version: i32,
    ) -> Self {
        let no_output = outputs.no_output();
        let fst = FST::from_reader(
            FSTMetadata::new(input_type, outputs, None, -1, version, 0),
            Box::new(NullFSTReader),
        );
        let dedup_hash = if suffix_ram_limit_mb > 0.0 {
            Some(NodeHash::new(suffix_ram_limit_mb))
        } else {
            None
        };
        let frontier = (0..10)
            .map(|idx| UnCompiledNode::new(&no_output, idx))
            .collect();
        Self {
            dedup_hash,
            fst,
            no_output,
            last_input: IntsRefBuilder::new(),
            // Pad: ensure no node gets address 0, which is reserved to mean the
            // stop state with no arcs. The actual byte is written lazily.
            padding_byte_pending: true,
            frontier,
            last_frozen_node: 0,
            num_bytes_per_arc: vec![0; 4],
            num_label_bytes_per_arc: vec![0; 4],
            fixed_length_arcs_buffer: FixedLengthArcsBuffer::new(),
            arc_count: 0,
            node_count: 0,
            binary_search_node_count: 0,
            direct_addressing_node_count: 0,
            continuous_node_count: 0,
            allow_fixed_length_arcs,
            direct_addressing_max_oversizing_factor,
            version,
            direct_addressing_expansion_credit: 0,
            data_output,
            scratch_bytes: GrowableByteArrayDataOutput::new(),
            num_bytes_written: 1,
        }
    }

    /// Returns the temporary FST used while building.
    ///
    /// Equivalent to the package-private field `FSTCompiler.fst`.
    pub(crate) fn fst(&self) -> &FST<O> {
        &self.fst
    }

    /// Returns the [`FSTReader`] backing this compiler's sink.
    ///
    /// Equivalent to `FSTCompiler.getFSTReader`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the sink is not the on-heap
    /// one and therefore cannot be read back.
    pub fn fst_reader(&self) -> Result<&dyn FSTReader> {
        match &self.data_output {
            FstOutputSink::OnHeap(output) => Ok(output),
            FstOutputSink::Streaming(_) => Err(LuceneError::IllegalState(
                "The DataOutput must implement FSTReader".to_string(),
            )),
        }
    }

    /// Consumes this compiler and returns the [`FSTReader`] backing its sink,
    /// ready to be handed to [`FST::from_fst_reader`].
    ///
    /// This is the owning counterpart of [`FSTCompiler::fst_reader`], needed
    /// because an [`FST`] owns its reader while Lucene's `getFSTReader()` only
    /// hands out a reference to a garbage-collected object.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the sink is not the on-heap
    /// one.
    pub fn into_fst_reader(self) -> Result<Box<dyn FSTReader>> {
        match self.data_output {
            FstOutputSink::OnHeap(output) => Ok(Box::new(output)),
            FstOutputSink::Streaming(_) => Err(LuceneError::IllegalState(
                "The DataOutput must implement FSTReader".to_string(),
            )),
        }
    }

    /// Returns the maximum oversizing of a fixed arc array allowed to enable
    /// direct addressing.
    ///
    /// Equivalent to `FSTCompiler.getDirectAddressingMaxOversizingFactor`.
    pub fn direct_addressing_max_oversizing_factor(&self) -> f32 {
        self.direct_addressing_max_oversizing_factor
    }

    /// Returns the number of nodes, including the implicit final node.
    ///
    /// Equivalent to `FSTCompiler.getNodeCount`.
    pub fn node_count(&self) -> i64 {
        // 1+ in order to count the -1 implicit final node.
        1 + self.node_count
    }

    /// Returns the number of arcs.
    ///
    /// Equivalent to `FSTCompiler.getArcCount`.
    pub fn arc_count(&self) -> i64 {
        self.arc_count
    }

    /// Returns the number of nodes written with binary-search arcs.
    ///
    /// Equivalent to the package-private field
    /// `FSTCompiler.binarySearchNodeCount`.
    pub fn binary_search_node_count(&self) -> i64 {
        self.binary_search_node_count
    }

    /// Returns the number of nodes written with direct-addressing arcs.
    ///
    /// Equivalent to the package-private field
    /// `FSTCompiler.directAddressingNodeCount`.
    pub fn direct_addressing_node_count(&self) -> i64 {
        self.direct_addressing_node_count
    }

    /// Returns the number of nodes written with continuous arcs.
    ///
    /// Equivalent to the package-private field
    /// `FSTCompiler.continuousNodeCount`.
    pub fn continuous_node_count(&self) -> i64 {
        self.continuous_node_count
    }

    /// Returns the memory used while building the FST.
    ///
    /// Equivalent to `FSTCompiler.fstRamBytesUsed`.
    pub fn fst_ram_bytes_used(&self) -> i64 {
        self.scratch_bytes.ram_bytes_used() + self.data_output.ram_bytes_used()
    }

    /// Returns the number of FST bytes written so far.
    ///
    /// Equivalent to `FSTCompiler.fstSizeInBytes`.
    pub fn fst_size_in_bytes(&self) -> i64 {
        self.num_bytes_written
    }

    /// Compiles `node_in`, de-duplicating it against the suffix cache when one
    /// is configured.
    ///
    /// Equivalent to the private `FSTCompiler.compileNode`.
    fn compile_node(&mut self, node_in: &mut UnCompiledNode<O::Output>) -> Result<CompiledNode> {
        let bytes_pos_start = self.num_bytes_written;
        let node = if self.dedup_hash.is_some() {
            if node_in.num_arcs == 0 {
                let node = self.add_node(node_in)?;
                self.last_frozen_node = node;
                node
            } else {
                // The suffix cache needs the compiler, which owns it: move it
                // out for the duration of the call and put it back afterwards,
                // error or not.
                let mut dedup_hash = self
                    .dedup_hash
                    .take()
                    .expect("INVARIANT: checked by is_some above");
                let result = dedup_hash.add(self, node_in);
                self.dedup_hash = Some(dedup_hash);
                result?
            }
        } else {
            self.add_node(node_in)?
        };

        let bytes_pos_end = self.num_bytes_written;
        if bytes_pos_end != bytes_pos_start {
            // The FST added a new node.
            debug_assert!(bytes_pos_end > bytes_pos_start);
            self.last_frozen_node = node;
        }

        node_in.clear(&self.no_output);

        Ok(CompiledNode { node })
    }

    /// Serializes a new node by appending its bytes to the end of the stream.
    ///
    /// Equivalent to the package-private `FSTCompiler.addNode`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while writing.
    pub(crate) fn add_node(&mut self, node_in: &UnCompiledNode<O::Output>) -> Result<i64> {
        if node_in.num_arcs == 0 {
            return Ok(if node_in.is_final {
                FINAL_END_NODE
            } else {
                NON_FINAL_END_NODE
            });
        }
        // Reset the scratch writer to prepare for a new write.
        self.scratch_bytes.set_position(0);

        let outputs = self.fst.metadata.outputs.clone();
        let input_type = self.fst.metadata.input_type;
        let no_output = self.no_output.clone();

        let do_fixed_length_arcs = self.should_expand_node_with_fixed_length_arcs(node_in);
        if do_fixed_length_arcs && self.num_bytes_per_arc.len() < node_in.num_arcs {
            let new_len = ArrayUtil::oversize(node_in.num_arcs, 4).max(node_in.num_arcs);
            self.num_bytes_per_arc = vec![0; new_len];
            self.num_label_bytes_per_arc = vec![0; new_len];
        }

        self.arc_count += node_in.num_arcs as i64;

        let last_arc = node_in.num_arcs - 1;

        let mut last_arc_start = 0usize;
        let mut max_bytes_per_arc = 0i32;
        let mut max_bytes_per_arc_without_label = 0i32;
        for arc_idx in 0..node_in.num_arcs {
            let arc = &node_in.arcs[arc_idx];
            let target_node = arc.target;
            let mut flags = 0i32;

            if arc_idx == last_arc {
                flags += BIT_LAST_ARC;
            }

            if self.last_frozen_node == target_node && !do_fixed_length_arcs {
                // TODO(Lucene): for better performance, at the cost of more RAM,
                // this could be avoided except when the arc is near the last arc.
                flags += BIT_TARGET_NEXT;
            }

            if arc.is_final {
                flags += BIT_FINAL_ARC;
                if !outputs.equals(&arc.next_final_output, &no_output) {
                    flags += BIT_ARC_HAS_FINAL_OUTPUT;
                }
            } else {
                debug_assert!(outputs.equals(&arc.next_final_output, &no_output));
            }

            let target_has_arcs = target_node > 0;

            if !target_has_arcs {
                flags += BIT_STOP_NODE;
            }

            if !outputs.equals(&arc.output, &no_output) {
                flags += BIT_ARC_HAS_OUTPUT;
            }

            self.scratch_bytes.write_byte(flags as u8)?;
            let label_start = self.scratch_bytes.position();
            write_label(input_type, &mut self.scratch_bytes, arc.label)?;
            let num_label_bytes = self.scratch_bytes.position() - label_start;

            if !outputs.equals(&arc.output, &no_output) {
                outputs.write(&arc.output, &mut self.scratch_bytes)?;
            }

            if !outputs.equals(&arc.next_final_output, &no_output) {
                outputs.write_final_output(&arc.next_final_output, &mut self.scratch_bytes)?;
            }

            if target_has_arcs && (flags & BIT_TARGET_NEXT) == 0 {
                debug_assert!(target_node > 0);
                self.scratch_bytes.write_v_long(target_node)?;
            }

            // Just write the arcs "like normal" on the first pass, but record
            // how many bytes each one took and the maximum size.
            if do_fixed_length_arcs {
                let num_arc_bytes = self.scratch_bytes.position() - last_arc_start;
                self.num_bytes_per_arc[arc_idx] = num_arc_bytes as i32;
                self.num_label_bytes_per_arc[arc_idx] = num_label_bytes as i32;
                last_arc_start = self.scratch_bytes.position();
                max_bytes_per_arc = max_bytes_per_arc.max(num_arc_bytes as i32);
                max_bytes_per_arc_without_label =
                    max_bytes_per_arc_without_label.max((num_arc_bytes - num_label_bytes) as i32);
            }
        }

        if do_fixed_length_arcs {
            debug_assert!(max_bytes_per_arc > 0);
            // Second pass: "expand" all arcs to take up a fixed byte size.

            let label_range = node_in.arcs[node_in.num_arcs - 1].label - node_in.arcs[0].label + 1;
            debug_assert!(label_range > 0);
            let continuous_label = label_range == node_in.num_arcs as i32;
            if continuous_label && self.version >= VERSION_CONTINUOUS_ARCS {
                self.write_node_for_direct_addressing_or_continuous(
                    node_in,
                    max_bytes_per_arc_without_label,
                    label_range,
                    true,
                )?;
                self.continuous_node_count += 1;
            } else if self.should_expand_node_with_direct_addressing(
                node_in,
                max_bytes_per_arc,
                max_bytes_per_arc_without_label,
                label_range,
            ) {
                self.write_node_for_direct_addressing_or_continuous(
                    node_in,
                    max_bytes_per_arc_without_label,
                    label_range,
                    false,
                )?;
                self.direct_addressing_node_count += 1;
            } else {
                self.write_node_for_binary_search(node_in, max_bytes_per_arc)?;
                self.binary_search_node_count += 1;
            }
        }

        self.scratch_bytes.reverse_written_bytes();
        // Write the padding byte if needed.
        if self.padding_byte_pending {
            self.write_padding_byte()?;
        }
        self.scratch_bytes.write_to(&mut self.data_output)?;
        self.num_bytes_written += self.scratch_bytes.position() as i64;

        self.node_count += 1;
        Ok(self.num_bytes_written - 1)
    }

    /// Writes the padding byte, ensuring no node gets address `0`, which is
    /// reserved to mean the stop state with no arcs.
    ///
    /// Equivalent to the private `FSTCompiler.writePaddingByte`.
    fn write_padding_byte(&mut self) -> Result<()> {
        debug_assert!(self.padding_byte_pending);
        self.data_output.write_byte(0)?;
        self.padding_byte_pending = false;
        Ok(())
    }

    /// Returns whether the given node should be expanded with fixed length
    /// arcs, based on its depth and its number of arcs.
    ///
    /// Equivalent to the private
    /// `FSTCompiler.shouldExpandNodeWithFixedLengthArcs`. Fixed length arcs use
    /// more space but allow binary search or direct addressing instead of a
    /// linear scan.
    fn should_expand_node_with_fixed_length_arcs(&self, node: &UnCompiledNode<O::Output>) -> bool {
        self.allow_fixed_length_arcs
            && ((node.depth <= FIXED_LENGTH_ARC_SHALLOW_DEPTH
                && node.num_arcs >= FIXED_LENGTH_ARC_SHALLOW_NUM_ARCS)
                || node.num_arcs >= FIXED_LENGTH_ARC_DEEP_NUM_ARCS)
    }

    /// Returns whether the given node should be expanded with direct
    /// addressing instead of binary search.
    ///
    /// Equivalent to the private
    /// `FSTCompiler.shouldExpandNodeWithDirectAddressing`.
    fn should_expand_node_with_direct_addressing(
        &mut self,
        node_in: &UnCompiledNode<O::Output>,
        num_bytes_per_arc: i32,
        max_bytes_per_arc_without_label: i32,
        label_range: i32,
    ) -> bool {
        // Anticipate precisely the size of the encodings.
        let size_for_binary_search = num_bytes_per_arc * node_in.num_arcs as i32;
        let size_for_direct_addressing = get_num_presence_bytes(label_range)
            + self.num_label_bytes_per_arc[0]
            + max_bytes_per_arc_without_label * node_in.num_arcs as i32;

        // Determine the allowed oversize compared to binary search. This is
        // defined by a parameter of the FST builder, 1 by default: no oversize.
        let allowed_oversize =
            (size_for_binary_search as f32 * self.direct_addressing_max_oversizing_factor) as i32;
        let expansion_cost = size_for_direct_addressing - allowed_oversize;

        // Select direct addressing if either:
        // - direct addressing is smaller than binary search, in which case the
        //   credit is incremented by the reduced size, to be used later;
        // - direct addressing is larger, but the positive credit allows the
        //   oversizing, in which case the credit is decremented by the oversize.
        // In addition, never oversize to a clearly too large node size.
        if expansion_cost <= 0
            || (self.direct_addressing_expansion_credit >= i64::from(expansion_cost)
                && size_for_direct_addressing as f32
                    <= allowed_oversize as f32 * DIRECT_ADDRESSING_MAX_OVERSIZE_WITH_CREDIT_FACTOR)
        {
            self.direct_addressing_expansion_credit -= i64::from(expansion_cost);
            true
        } else {
            false
        }
    }

    /// Rewrites the arcs of `node_in` as a fixed-length array preceded by a
    /// binary-search node header.
    ///
    /// Equivalent to the private `FSTCompiler.writeNodeForBinarySearch`.
    fn write_node_for_binary_search(
        &mut self,
        node_in: &UnCompiledNode<O::Output>,
        max_bytes_per_arc: i32,
    ) -> Result<()> {
        // Build the header in a buffer. It is a false, special arc which is in
        // fact a node header: node flags followed by node metadata.
        self.fixed_length_arcs_buffer.reset_position();
        self.fixed_length_arcs_buffer
            .write_byte(ARCS_FOR_BINARY_SEARCH)?;
        self.fixed_length_arcs_buffer
            .write_v_int(node_in.num_arcs as i32)?;
        self.fixed_length_arcs_buffer
            .write_v_int(max_bytes_per_arc)?;
        let header_len = self.fixed_length_arcs_buffer.position();

        // Expand the arcs in place, backwards.
        let mut src_pos = self.scratch_bytes.position();
        let mut dest_pos = header_len + node_in.num_arcs * max_bytes_per_arc as usize;
        debug_assert!(dest_pos >= src_pos);
        if dest_pos > src_pos {
            self.scratch_bytes.set_position(dest_pos);
            for arc_idx in (0..node_in.num_arcs).rev() {
                dest_pos -= max_bytes_per_arc as usize;
                let arc_len = self.num_bytes_per_arc[arc_idx] as usize;
                src_pos -= arc_len;
                if src_pos != dest_pos {
                    debug_assert!(dest_pos > src_pos);
                    // Copy the bytes from src_pos to dest_pos, expanding the arc
                    // from variable length to fixed length.
                    self.scratch_bytes.copy_within(src_pos, dest_pos, arc_len);
                }
            }
        }

        // Finally write the header.
        debug_assert!(header_len <= self.scratch_bytes.position());
        self.scratch_bytes.bytes_mut()[..header_len]
            .copy_from_slice(&self.fixed_length_arcs_buffer.bytes()[..header_len]);
        Ok(())
    }

    /// Rewrites the arcs of `node_in` as a fixed-length array preceded by a
    /// direct-addressing or continuous node header.
    ///
    /// Equivalent to the private
    /// `FSTCompiler.writeNodeForDirectAddressingOrContinuous`.
    fn write_node_for_direct_addressing_or_continuous(
        &mut self,
        node_in: &UnCompiledNode<O::Output>,
        max_bytes_per_arc_without_label: i32,
        label_range: i32,
        continuous: bool,
    ) -> Result<()> {
        // Expand the arcs backwards in a buffer because the labels are removed,
        // so the resulting arcs may occupy less space. Drop the label bytes,
        // since the label can be inferred from the arc index, the presence bits
        // and the first label. Keep the first label.
        let header_max_len = 11usize;
        let num_presence_bytes = if continuous {
            0
        } else {
            get_num_presence_bytes(label_range) as usize
        };
        let mut src_pos = self.scratch_bytes.position();
        let total_arc_bytes = self.num_label_bytes_per_arc[0] as usize
            + node_in.num_arcs * max_bytes_per_arc_without_label as usize;
        let mut buffer_offset = header_max_len + num_presence_bytes + total_arc_bytes;
        self.fixed_length_arcs_buffer.ensure_capacity(buffer_offset);
        // Copy the arcs to the buffer, dropping all labels except the first.
        for arc_idx in (0..node_in.num_arcs).rev() {
            buffer_offset -= max_bytes_per_arc_without_label as usize;
            let src_arc_len = self.num_bytes_per_arc[arc_idx] as usize;
            src_pos -= src_arc_len;
            let label_len = self.num_label_bytes_per_arc[arc_idx] as usize;
            // Copy the flags.
            self.scratch_bytes.copy_to_slice(
                src_pos,
                self.fixed_length_arcs_buffer.bytes_mut(),
                buffer_offset,
                1,
            );
            // Skip the label, copy the remainder.
            let remaining_arc_len = src_arc_len - 1 - label_len;
            if remaining_arc_len != 0 {
                self.scratch_bytes.copy_to_slice(
                    src_pos + 1 + label_len,
                    self.fixed_length_arcs_buffer.bytes_mut(),
                    buffer_offset + 1,
                    remaining_arc_len,
                );
            }
            if arc_idx == 0 {
                // Copy the label of the first arc only.
                buffer_offset -= label_len;
                self.scratch_bytes.copy_to_slice(
                    src_pos + 1,
                    self.fixed_length_arcs_buffer.bytes_mut(),
                    buffer_offset,
                    label_len,
                );
            }
        }
        debug_assert_eq!(buffer_offset, header_max_len + num_presence_bytes);

        // Build the header in the buffer. It is a false, special arc which is in
        // fact a node header: node flags followed by node metadata.
        self.fixed_length_arcs_buffer.reset_position();
        self.fixed_length_arcs_buffer.write_byte(if continuous {
            ARCS_FOR_CONTINUOUS
        } else {
            ARCS_FOR_DIRECT_ADDRESSING
        })?;
        // labelRange instead of numArcs.
        self.fixed_length_arcs_buffer.write_v_int(label_range)?;
        // maxBytesPerArcWithoutLabel instead of maxBytesPerArc.
        self.fixed_length_arcs_buffer
            .write_v_int(max_bytes_per_arc_without_label)?;
        let header_len = self.fixed_length_arcs_buffer.position();

        // Write the header.
        self.scratch_bytes.set_position(0);
        self.scratch_bytes
            .write_bytes(self.fixed_length_arcs_buffer.bytes(), 0, header_len)?;

        // Write the presence bits.
        if !continuous {
            self.write_presence_bits(node_in)?;
            debug_assert_eq!(
                self.scratch_bytes.position(),
                header_len + num_presence_bytes
            );
        }

        // Write the first label and the arcs.
        self.scratch_bytes.write_bytes(
            self.fixed_length_arcs_buffer.bytes(),
            buffer_offset,
            total_arc_bytes,
        )?;
        debug_assert_eq!(
            self.scratch_bytes.position(),
            header_len + num_presence_bytes + total_arc_bytes
        );
        Ok(())
    }

    /// Writes the one-bit-per-label presence table of a direct-addressing node.
    ///
    /// Equivalent to the private `FSTCompiler.writePresenceBits`.
    fn write_presence_bits(&mut self, node_in: &UnCompiledNode<O::Output>) -> Result<()> {
        let mut presence_bits: u8 = 1; // The first arc is always present.
        let mut presence_index = 0i32;
        let mut previous_label = node_in.arcs[0].label;
        for arc_idx in 1..node_in.num_arcs {
            let label = node_in.arcs[arc_idx].label;
            debug_assert!(label > previous_label);
            presence_index += label - previous_label;
            while presence_index >= 8 {
                self.scratch_bytes.write_byte(presence_bits)?;
                presence_bits = 0;
                presence_index -= 8;
            }
            // Set the bit at presence_index to flag that the corresponding arc
            // is present.
            presence_bits |= 1 << presence_index;
            previous_label = label;
        }
        debug_assert_eq!(
            presence_index,
            (node_in.arcs[node_in.num_arcs - 1].label - node_in.arcs[0].label) % 8
        );
        // The last byte is not 0 and the last arc is always present.
        debug_assert_ne!(presence_bits, 0);
        debug_assert_ne!(presence_bits & (1 << presence_index), 0);
        self.scratch_bytes.write_byte(presence_bits)
    }

    /// Compiles the states of the previous input's orphaned suffix.
    ///
    /// Equivalent to the private `FSTCompiler.freezeTail`.
    fn freeze_tail(&mut self, prefix_len_plus_1: usize) -> Result<()> {
        let down_to = prefix_len_plus_1.max(1);

        let mut idx = self.last_input.length();
        while idx >= down_to {
            // Take the node out of the frontier so that it and its parent can
            // be handled at the same time; the very same node, cleared by
            // `compile_node`, is put back, so its depth is preserved.
            let mut node = std::mem::take(&mut self.frontier[idx]);
            let prev_idx = idx - 1;
            // These have to be read before `compile_node`, which clears the
            // node's state, in order to call `replace_last` afterwards.
            let next_final_output = node.output.clone();
            debug_assert!(node.num_arcs != 0 || node.is_final);
            let is_final = node.is_final;

            // This node makes it and is now compiled; first, compile any
            // targets that were previously undecided.
            let compiled = self.compile_node(&mut node);
            self.frontier[idx] = node;
            let compiled = compiled?;

            let label = self.last_input.int_at(prev_idx);
            self.frontier[prev_idx].replace_last(label, compiled.node, next_final_output, is_final);

            idx -= 1;
        }
        Ok(())
    }

    /// Adds the next input/output pair.
    ///
    /// Equivalent to `FSTCompiler.add`. The input must sort after the previous
    /// one; adding the same input twice in a row with different outputs is also
    /// allowed, as long as the [`Outputs`] implementation supports
    /// [`Outputs::merge`]. The input is fully consumed, so the caller is free
    /// to reuse it, but the output is not.
    ///
    /// # Errors
    ///
    /// Propagates write errors and the [`LuceneError::UnsupportedOperation`] of
    /// [`Outputs::merge`] when the same input is added twice.
    pub fn add(&mut self, input: &IntsRef, output: O::Output) -> Result<()> {
        let outputs = self.fst.metadata.outputs.clone();
        let no_output = self.no_output.clone();
        let mut output = output;

        debug_assert!(
            self.last_input.length() == 0 || ints_ref_compare(input, &self.last_input.get()) >= 0,
            "inputs are added out of order"
        );

        if input.length == 0 {
            // Empty input: only allowed as the first input. This is special
            // cased because the packed FST format cannot represent the empty
            // input, since finalness is stored on the incoming arc, not on the
            // node.
            self.frontier[0].is_final = true;
            return self.set_empty_output(output);
        }

        // Compare the shared prefix length.
        let mut pos1 = 0usize;
        let mut pos2 = input.offset;
        let pos1_stop = self.last_input.length().min(input.length);
        while pos1 < pos1_stop && self.last_input.int_at(pos1) == input.ints[pos2] {
            pos1 += 1;
            pos2 += 1;
        }
        let prefix_len_plus_1 = pos1 + 1;

        if self.frontier.len() < input.length + 1 {
            let new_len = ArrayUtil::oversize(input.length + 1, 8).max(input.length + 1);
            for idx in self.frontier.len()..new_len {
                self.frontier.push(UnCompiledNode::new(&no_output, idx));
            }
        }

        // Minimize and compile states from the previous input's orphaned suffix.
        self.freeze_tail(prefix_len_plus_1)?;

        // Init tail states for the current input.
        for idx in prefix_len_plus_1..=input.length {
            let label = input.ints[input.offset + idx - 1];
            self.frontier[idx - 1].add_arc(label, &no_output);
        }

        let last_node_idx = input.length;
        if self.last_input.length() != input.length || prefix_len_plus_1 != input.length + 1 {
            self.frontier[last_node_idx].is_final = true;
            self.frontier[last_node_idx].output = no_output.clone();
        }

        // Push conflicting outputs forward, only as far as needed.
        for idx in 1..prefix_len_plus_1 {
            let label = input.ints[input.offset + idx - 1];
            let last_output = self.frontier[idx - 1].get_last_output(label).clone();

            let common_output_prefix;

            if !outputs.equals(&last_output, &no_output) {
                common_output_prefix = outputs.common(&output, &last_output);
                let word_suffix = outputs.subtract(&last_output, &common_output_prefix);
                self.frontier[idx - 1].set_last_output(label, common_output_prefix.clone());
                self.frontier[idx].prepend_output(&outputs, &word_suffix);
            } else {
                common_output_prefix = no_output.clone();
            }

            output = outputs.subtract(&output, &common_output_prefix);
        }

        if self.last_input.length() == input.length && prefix_len_plus_1 == 1 + input.length {
            // The same input appeared more than once in a row, mapping to
            // multiple outputs.
            let merged = outputs.merge(&self.frontier[last_node_idx].output, &output)?;
            self.frontier[last_node_idx].output = merged;
        } else {
            // This new arc is private to this new input; set its arc output to
            // the leftover output.
            let label = input.ints[input.offset + prefix_len_plus_1 - 1];
            self.frontier[prefix_len_plus_1 - 1].set_last_output(label, output);
        }

        // Save the last input.
        self.last_input.copy_ints_ref(input);
        Ok(())
    }

    /// Records the output produced for the empty string.
    ///
    /// Equivalent to the package-private `FSTCompiler.setEmptyOutput`.
    ///
    /// # Errors
    ///
    /// Propagates the [`LuceneError::UnsupportedOperation`] of
    /// [`Outputs::merge`] when the empty string is added twice.
    pub(crate) fn set_empty_output(&mut self, v: O::Output) -> Result<()> {
        let merged = match &self.fst.metadata.empty_output {
            Some(existing) => self.fst.metadata.outputs.merge(existing, &v)?,
            None => v,
        };
        self.fst.metadata.empty_output = Some(merged);
        Ok(())
    }

    /// Records the start node and the total size.
    ///
    /// Equivalent to the package-private `FSTCompiler.finish`.
    fn finish(&mut self, new_start_node: i64) -> Result<()> {
        debug_assert!(new_start_node <= self.num_bytes_written);
        if self.fst.metadata.start_node != -1 {
            return Err(LuceneError::IllegalState("already finished".to_string()));
        }
        let new_start_node =
            if new_start_node == FINAL_END_NODE && self.fst.metadata.empty_output.is_some() {
                0
            } else {
                new_start_node
            };
        self.fst.metadata.start_node = new_start_node;
        self.fst.metadata.num_bytes = self.num_bytes_written;
        // Freeze the sink if applicable.
        self.data_output.freeze();
        Ok(())
    }

    /// Returns the metadata of the final FST, or `None` when nothing at all is
    /// accepted by the FST.
    ///
    /// Equivalent to `FSTCompiler.compile`.
    ///
    /// To obtain the FST when the on-heap sink was used:
    ///
    /// ```text
    /// let metadata = compiler.compile()?;
    /// let fst = FST::from_fst_reader(metadata, compiler.into_fst_reader()?);
    /// ```
    ///
    /// When a streaming sink was used, read the bytes back and pass the
    /// resulting input to [`FST::new`] or to an
    /// [`super::off_heap_fst_store::OffHeapFSTStore`].
    ///
    /// # Errors
    ///
    /// Propagates any error raised while writing the remaining nodes.
    pub fn compile(&mut self) -> Result<Option<FSTMetadata<O>>> {
        // Minimize the nodes in the last word's suffix.
        self.freeze_tail(0)?;

        let mut root = std::mem::take(&mut self.frontier[0]);
        if root.num_arcs == 0 {
            if self.fst.metadata.empty_output.is_none() {
                // Return None for a completely empty FST, which accepts nothing.
                self.frontier[0] = root;
                return Ok(None);
            }
            // The padding byte has not been written so far, but the FST is
            // still valid.
            self.write_padding_byte()?;
        }

        let compiled = self.compile_node(&mut root);
        self.frontier[0] = root;
        let compiled = compiled?;

        self.finish(compiled.node)?;

        Ok(Some(self.fst.metadata.clone()))
    }
}

/// Writes one `BYTE1`/`BYTE2`/`BYTE4` label.
///
/// Equivalent to the private `FSTCompiler.writeLabel`.
fn write_label(input_type: InputType, out: &mut dyn DataOutput, v: i32) -> Result<()> {
    debug_assert!(v >= 0, "v={v}");
    match input_type {
        InputType::Byte1 => {
            debug_assert!(v <= 255, "v={v}");
            out.write_byte(v as u8)
        }
        InputType::Byte2 => {
            debug_assert!(v <= 65535, "v={v}");
            out.write_short(v as i16)
        }
        InputType::Byte4 => out.write_v_int(v),
    }
}

/// Compares two [`IntsRef`] values the way `IntsRef.compareTo` does.
///
/// Returns a negative value when `a` sorts first, `0` when they are equal, and
/// a positive value otherwise. Element comparison is signed, as in Java.
pub(crate) fn ints_ref_compare(a: &IntsRef, b: &IntsRef) -> i64 {
    let a_slice = a.slice();
    let b_slice = b.slice();
    let n = a_slice.len().min(b_slice.len());
    for i in 0..n {
        if a_slice[i] > b_slice[i] {
            return 1;
        } else if a_slice[i] < b_slice[i] {
            return -1;
        }
    }
    a_slice.len() as i64 - b_slice.len() as i64
}
