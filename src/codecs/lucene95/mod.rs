//! `lucene95` codec support ported from `org.apache.lucene.codecs.lucene95`.
//!
//! Holds the doc-id encoding shared by every vector format since Lucene 9.5,
//! and the slice accessor a reader exposes so a caller can prefetch its data.

use crate::codecs::hnsw::flat_vectors::DocsWithFieldSet;
use crate::codecs::lucene90::indexed_disi::{write_bit_set, IndexedDISI, DEFAULT_DENSE_RANK_POWER};
use crate::error::{LuceneError, Result};
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::DocIdSetIterator;
use crate::store::{DataInput, DataOutput, IndexInput, IndexOutput};

/// Block shift the vector formats use for their monotonic ord-to-doc mapping.
///
/// Equivalent to `Lucene99FlatVectorsFormat.DIRECT_MONOTONIC_BLOCK_SHIFT`.
pub const DIRECT_MONOTONIC_BLOCK_SHIFT: i32 = 16;
use crate::util::packed::{DirectMonotonicMeta, DirectMonotonicReader, DirectMonotonicWriter};

/// A reader that can hand out the input slice holding its data.
///
/// Equivalent to `org.apache.lucene.codecs.lucene95.HasIndexSlice`, which the
/// vector readers implement so a caller can prefetch or memory-map their bytes.
pub trait HasIndexSlice {
    /// Returns the slice this reader reads from, when it has one.
    ///
    /// Equivalent to `HasIndexSlice.getSlice()`.
    fn get_slice(&self) -> Option<&dyn IndexInput>;
}

// -----------------------------------------------------------------------------
// OrdToDoc configuration
// -----------------------------------------------------------------------------

pub struct OrdToDocDISIReaderConfiguration {
    /// See `OrdToDocDISIReaderConfiguration`.
    pub size: i32,
    /// See `OrdToDocDISIReaderConfiguration`.
    pub jump_table_entry_count: i16,
    /// See `OrdToDocDISIReaderConfiguration`.
    pub addresses_offset: i64,
    /// See `OrdToDocDISIReaderConfiguration`.
    pub addresses_length: i64,
    /// See `OrdToDocDISIReaderConfiguration`.
    pub docs_with_field_offset: i64,
    /// See `OrdToDocDISIReaderConfiguration`.
    pub docs_with_field_length: i64,
    /// See `OrdToDocDISIReaderConfiguration`.
    pub dense_rank_power: i8,
    /// See `OrdToDocDISIReaderConfiguration`.
    pub meta: DirectMonotonicMeta,
}

impl std::fmt::Debug for OrdToDocDISIReaderConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrdToDocDISIReaderConfiguration")
            .field("size", &self.size)
            .field("docs_with_field_offset", &self.docs_with_field_offset)
            .finish_non_exhaustive()
    }
}

impl Clone for OrdToDocDISIReaderConfiguration {
    fn clone(&self) -> Self {
        Self {
            size: self.size,
            jump_table_entry_count: self.jump_table_entry_count,
            addresses_offset: self.addresses_offset,
            addresses_length: self.addresses_length,
            docs_with_field_offset: self.docs_with_field_offset,
            docs_with_field_length: self.docs_with_field_length,
            dense_rank_power: self.dense_rank_power,
            meta: DirectMonotonicMeta {
                block_shift: self.meta.block_shift,
                num_blocks: self.meta.num_blocks,
                mins: self.meta.mins.clone(),
                avgs: self.meta.avgs.clone(),
                offsets: self.meta.offsets.clone(),
                bpvs: self.meta.bpvs.clone(),
            },
        }
    }
}

impl OrdToDocDISIReaderConfiguration {
    /// Returns whether the field has no vectors at all.
    ///
    /// Equivalent to `OrdToDocDISIReaderConfiguration.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.docs_with_field_offset == -2
    }

    /// Returns whether every document has a vector.
    ///
    /// Equivalent to `OrdToDocDISIReaderConfiguration.isDense()`.
    pub fn is_dense(&self) -> bool {
        self.docs_with_field_offset == -1
    }

    /// Writes the doc-id encoding metadata for a vector field.
    ///
    /// Equivalent to `OrdToDocDISIReaderConfiguration.writeStoredMeta`.
    pub fn write_stored_meta(
        meta_out: &mut dyn IndexOutput,
        vector_data: &mut dyn IndexOutput,
        count: i32,
        max_doc: i32,
        docs_with_field: &DocsWithFieldSet,
    ) -> Result<()> {
        if count == 0 {
            meta_out.write_long(-2)?;
            meta_out.write_long(0)?;
            meta_out.write_short(-1)?;
            meta_out.write_byte((-1i8) as u8)?;
        } else if count == max_doc {
            meta_out.write_long(-1)?;
            meta_out.write_long(0)?;
            meta_out.write_short(-1)?;
            meta_out.write_byte((-1i8) as u8)?;
        } else {
            let offset = vector_data.file_pointer();
            meta_out.write_long(offset)?;
            let jump_table_entry_count = {
                let mut iter = docs_with_field.iterator()?;
                write_bit_set(iter.as_mut(), vector_data, DEFAULT_DENSE_RANK_POWER)?
            };
            meta_out.write_long(vector_data.file_pointer() - offset)?;
            meta_out.write_short(jump_table_entry_count)?;
            meta_out.write_byte(DEFAULT_DENSE_RANK_POWER as u8)?;

            let start = vector_data.file_pointer();
            meta_out.write_long(start)?;
            meta_out.write_v_int(DIRECT_MONOTONIC_BLOCK_SHIFT)?;
            let mut writer = DirectMonotonicWriter::new(
                meta_out,
                vector_data,
                count as i64,
                DIRECT_MONOTONIC_BLOCK_SHIFT,
            )?;
            let mut iter = docs_with_field.iterator()?;
            while iter.next_doc()? != NO_MORE_DOCS {
                writer.add(iter.doc_id() as i64)?;
            }
            writer.finish()?;
            meta_out.write_long(vector_data.file_pointer() - start)?;
        }
        Ok(())
    }

    /// Reads back what [`write_stored_meta`](Self::write_stored_meta) wrote.
    ///
    /// Equivalent to `OrdToDocDISIReaderConfiguration.fromStoredMeta`.
    pub fn read_stored_meta(input: &mut dyn DataInput, size: i32) -> Result<Self> {
        let docs_with_field_offset = input.read_long()?;
        let docs_with_field_length = input.read_long()?;
        let jump_table_entry_count = input.read_short()?;
        let dense_rank_power = input.read_byte()? as i8;

        let mut addresses_offset = 0i64;
        let mut addresses_length = 0i64;
        let mut meta = DirectMonotonicMeta {
            block_shift: 0,
            num_blocks: 0,
            mins: Vec::new(),
            avgs: Vec::new(),
            offsets: Vec::new(),
            bpvs: Vec::new(),
        };

        if docs_with_field_offset > -1 {
            addresses_offset = input.read_long()?;
            let block_shift = input.read_v_int()?;
            meta = DirectMonotonicMeta::load(input, size as i64, block_shift)?;
            addresses_length = input.read_long()?;
        }

        Ok(Self {
            size,
            jump_table_entry_count,
            addresses_offset,
            addresses_length,
            docs_with_field_offset,
            docs_with_field_length,
            dense_rank_power,
            meta,
        })
    }
}
