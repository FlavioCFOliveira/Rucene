//! Lucene 9.0 live-docs format.
//!
//! Ported from `org.apache.lucene.codecs.lucene90.Lucene90LiveDocsFormat`.

#![deny(unsafe_code)]

use crate::codecs::codec_util;
use crate::codecs::live_docs::LiveDocsFormat;
use crate::codecs::stub::SegmentCommitInfo;
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::{file_name_from_generation, LIVE_DOCS_EXTENSION};
use crate::store::{Directory, IOContext, IndexInput, IndexOutput};
use crate::util::bit_sets::{DenseLiveDocs, SparseFixedBitSet, SparseLiveDocs};
use crate::util::{Bits, FixedBitSet};

const CODEC_NAME: &str = "Lucene90LiveDocs";
const VERSION_START: i32 = 0;
const VERSION_CURRENT: i32 = VERSION_START;

/// Deletion-rate threshold for choosing sparse vs dense representation.
const SPARSE_DENSE_THRESHOLD: f64 = 0.01;

/// Lucene 9.0 live-docs format.
///
/// Equivalent to `org.apache.lucene.codecs.lucene90.Lucene90LiveDocsFormat`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lucene90LiveDocsFormat;

impl Lucene90LiveDocsFormat {
    /// Creates a new live-docs format instance.
    pub fn new() -> Self {
        Self
    }
}

impl LiveDocsFormat for Lucene90LiveDocsFormat {
    fn name(&self) -> &str {
        "Lucene90LiveDocs"
    }

    fn read_live_docs(
        &self,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn Bits>> {
        let gen = info.get_del_gen();
        let name = file_name_from_generation(&info.info.name, LIVE_DOCS_EXTENSION, gen)
            // `fileNameFromGeneration` returns `null` for generation -1, and
            // the directory dereferences it: the JVM's `NullPointerException`.
            .ok_or_else(LuceneError::null_pointer)?;
        let max_doc = info.info.max_doc()?;
        let del_count = info.get_del_count();
        let deletion_rate = del_count as f64 / max_doc as f64;

        let mut input = dir.open_checksum_input(&name)?;
        let result = (|| {
            codec_util::check_index_header(
                input.as_mut(),
                CODEC_NAME,
                VERSION_START,
                VERSION_CURRENT,
                &info.info.id(),
                &radix36(gen as u64),
            )?;
            read_live_docs(
                input.as_mut(),
                max_doc as usize,
                deletion_rate,
                del_count as usize,
            )
        })();
        // `finally { CodecUtil.checkFooter(input, priorE); }` inside the
        // try-with-resources (`Lucene90LiveDocsFormat.java:88-105`).
        let result = codec_util::check_footer_after(input.as_mut(), result);
        crate::store::close_after(input.as_mut(), result)
    }

    fn write_live_docs(
        &self,
        bits: &dyn Bits,
        dir: &dyn Directory,
        info: &SegmentCommitInfo,
        new_del_count: i32,
        context: &dyn IOContext,
    ) -> Result<()> {
        let gen = info.get_next_del_gen();
        let name = file_name_from_generation(&info.info.name, LIVE_DOCS_EXTENSION, gen)
            // `fileNameFromGeneration` returns `null` for generation -1, and
            // the directory dereferences it: the JVM's `NullPointerException`.
            .ok_or_else(LuceneError::null_pointer)?;
        // `try (IndexOutput output = dir.createOutput(name, context))`
        // (`Lucene90LiveDocsFormat.java:171-183`): the output is closed
        // whether or not the body throws; a failure of the body wins, and the
        // failure of the close it triggers is suppressed.
        let mut output = dir.create_output(&name, context)?;
        let body = (|| -> Result<usize> {
            codec_util::write_index_header(
                output.as_mut(),
                CODEC_NAME,
                VERSION_CURRENT,
                &info.info.id(),
                &radix36(gen as u64),
            )?;
            let del_count = write_bits(output.as_mut(), bits)?;
            codec_util::write_footer(output.as_mut())?;
            Ok(del_count)
        })();
        let closed = output.close();
        let del_count = body?;
        closed?;

        let expected = info.get_del_count() + new_del_count;
        if del_count != expected as usize {
            // `Lucene90LiveDocsFormat.java:184-192`: `CorruptIndexException(String,
            // String)` with the file name as the resource description.
            return Err(LuceneError::corrupt_index(
                format!(
                    "bits.deleted={del_count} info.delcount={} newdelcount={new_del_count}",
                    info.get_del_count()
                ),
                &name,
            ));
        }
        Ok(())
    }

    fn files(&self, info: &SegmentCommitInfo, files: &mut Vec<String>) -> Result<()> {
        if info.has_deletions() {
            if let Some(name) =
                file_name_from_generation(&info.info.name, LIVE_DOCS_EXTENSION, info.get_del_gen())
            {
                files.push(name);
            }
        }
        Ok(())
    }
}

fn read_live_docs(
    input: &mut dyn IndexInput,
    max_doc: usize,
    deletion_rate: f64,
    expected_del_count: usize,
) -> Result<Box<dyn Bits>> {
    let (live_docs, actual_del_count): (Box<dyn Bits>, usize) =
        if deletion_rate <= SPARSE_DENSE_THRESHOLD {
            let sparse = read_sparse_fixed_bit_set(input, max_doc)?;
            let actual_del_count = sparse.cardinality();
            let live_docs = SparseLiveDocs::builder(sparse, max_doc)
                .with_deleted_count(actual_del_count)
                .build();
            (Box::new(live_docs), actual_del_count)
        } else {
            let dense = read_fixed_bit_set(input, max_doc)?;
            let actual_del_count = max_doc - dense.cardinality();
            let live_docs = DenseLiveDocs::builder(dense, max_doc)
                .with_deleted_count(actual_del_count)
                .build();
            (Box::new(live_docs), actual_del_count)
        };

    if actual_del_count != expected_del_count {
        // `Lucene90LiveDocsFormat.java:123-126`.
        return Err(LuceneError::corrupt_index(
            format!("bits.deleted={actual_del_count} info.delcount={expected_del_count}"),
            input.to_java_string(),
        ));
    }
    Ok(live_docs)
}

fn read_fixed_bit_set(input: &mut dyn IndexInput, length: usize) -> Result<FixedBitSet> {
    let num_words = FixedBitSet::bits2words(length);
    let mut data = vec![0u64; num_words];
    for slot in data.iter_mut().take(num_words) {
        *slot = input.read_long()? as u64;
    }
    FixedBitSet::from_bits(data, length)
}

fn read_sparse_fixed_bit_set(
    input: &mut dyn IndexInput,
    length: usize,
) -> Result<SparseFixedBitSet> {
    let num_words = FixedBitSet::bits2words(length);
    let mut data = vec![0u64; num_words];
    for slot in data.iter_mut().take(num_words) {
        *slot = input.read_long()? as u64;
    }

    let mut sparse = SparseFixedBitSet::new(length)?;
    for (word_index, word) in data.iter().enumerate() {
        // Disk format stores live docs (1 = live, 0 = deleted). SparseLiveDocs
        // stores deleted docs (1 = deleted), so we invert unset bits.
        if *word == !0u64 {
            continue;
        }
        let base_doc_id = word_index << 6;
        let max_doc_in_word = (base_doc_id + 64).min(length);
        for doc_id in base_doc_id..max_doc_in_word {
            let bit_index = doc_id & 63;
            if (word & (1u64 << bit_index)) == 0 {
                sparse.set(doc_id);
            }
        }
    }
    Ok(sparse)
}

/// Equivalent to the private `Lucene90LiveDocsFormat.writeBits(IndexOutput, Bits)`
/// (`Lucene90LiveDocsFormat.java:195-214`), which copies 1024 bits at a time
/// through [`Bits::apply_mask`].
fn write_bits(output: &mut dyn IndexOutput, bits: &dyn Bits) -> Result<usize> {
    let mut del_count = bits.length();
    // Copy bits in batches of 1024 bits at once using Bits#applyMask, which is faster than checking
    // bits one by one.
    let mut copy = FixedBitSet::new(1024);
    let mut offset = 0;
    while offset < bits.length() {
        let num_bits_to_copy = (bits.length() - offset).min(copy.length());
        copy.set_range(0, copy.length());
        if num_bits_to_copy < copy.length() {
            // Clear ghost bits
            copy.clear_range(num_bits_to_copy, copy.length());
        }
        bits.apply_mask(&mut copy, offset)?;
        del_count -= copy.cardinality();
        let long_count = FixedBitSet::bits2words(num_bits_to_copy);
        let words = copy.get_bits();
        for word in words.iter().take(long_count) {
            output.write_long(word.load(std::sync::atomic::Ordering::Relaxed) as i64)?;
        }
        offset += copy.length();
    }
    Ok(del_count)
}

/// Formats `value` in base-36 (lowercase), matching Java's
/// `Long.toString(gen, Character.MAX_RADIX)`.
fn radix36(value: u64) -> String {
    if value == 0 {
        "0".to_string()
    } else {
        let mut digits = Vec::new();
        let mut v = value;
        while v > 0 {
            let digit = (v % 36) as u8;
            let c = if digit < 10 {
                b'0' + digit
            } else {
                b'a' + (digit - 10)
            };
            digits.push(c);
            v /= 36;
        }
        digits.reverse();
        String::from_utf8(digits).unwrap()
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ByteBuffersDirectory;

    fn make_info(name: &str, max_doc: i32, del_count: i32, del_gen: i64) -> SegmentCommitInfo {
        let info = crate::codecs::tests::test_segment_info(name, max_doc);
        SegmentCommitInfo::new(info, del_count, 0, del_gen, -1, -1, [0u8; 16]).unwrap()
    }

    /// `writeBits` writes the live docs as `bits2words(length)` longs, one
    /// 1024-bit window at a time, and returns `length` minus the live bits
    /// (`Lucene90LiveDocsFormat.java:195-214`): the words are the live bits,
    /// with nothing past `length`, and the count is the deleted documents —
    /// for live docs that only answer `get` (the default `applyMask`) and for
    /// a `FixedBitSet` (its own) alike. Here every third of 1500 documents is
    /// deleted: 500 deletions, 24 words.
    #[test]
    fn write_bits_writes_the_live_docs_and_counts_the_deletions() {
        #[derive(Debug)]
        struct EveryThirdDeleted;
        impl Bits for EveryThirdDeleted {
            fn get(&self, index: usize) -> bool {
                !index.is_multiple_of(3)
            }
            fn length(&self) -> usize {
                1500
            }
        }
        let mut expected = vec![0u64; FixedBitSet::bits2words(1500)];
        let mut fixed = FixedBitSet::new(1500);
        for index in (0..1500).filter(|index| index % 3 != 0) {
            expected[index >> 6] |= 1u64 << (index & 63);
            fixed.set(index);
        }
        let expected: Vec<u8> = expected
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        for bits in [&EveryThirdDeleted as &dyn Bits, &fixed] {
            let mut out = crate::store::MockIndexOutput::new("liv", "_0_1.liv");
            assert_eq!(write_bits(&mut out, bits).unwrap(), 500);
            assert_eq!(out.into_inner(), expected);
        }
    }

    /// Regression (rmp #351): `writeLiveDocs` writes inside a
    /// try-with-resources over the output (`Lucene90LiveDocsFormat.java:171-183`),
    /// so a write that fails still closes the `.liv` output, and the write's
    /// failure is the one raised.
    #[test]
    fn a_failed_write_closes_the_output() {
        let (dir, _tracking, log) = crate::codecs::tests::tracking_directory();
        log.fail_write("_0_1.liv");
        let live = FixedBitSet::new(10);
        let error = Lucene90LiveDocsFormat::new()
            .write_live_docs(
                &live,
                dir.as_ref(),
                &make_info("_0", 10, 0, -1),
                10,
                &*crate::store::DEFAULT_IO_CONTEXT,
            )
            .unwrap_err();
        assert!(error.to_string().contains("write failed"), "{error:?}");
        assert_eq!(log.close_count("_0_1.liv"), 1);
    }

    fn reading_info(name: &str, max_doc: i32, del_count: i32) -> SegmentCommitInfo {
        // After a first live-docs write the generation is 1 (the file is
        // `_0_1.liv`). The del_count reflects the five deleted documents.
        let info = crate::codecs::tests::test_segment_info(name, max_doc);
        SegmentCommitInfo::new(info, del_count, 0, 1, -1, -1, [0u8; 16]).unwrap()
    }

    #[test]
    fn round_trip_dense_live_docs() {
        let dir = ByteBuffersDirectory::default();
        let format = Lucene90LiveDocsFormat::new();
        let info = make_info("_0", 100, 0, -1);

        // Build a live-docs bit set with 5 deleted documents (5% > 1% sparse threshold).
        let mut live = FixedBitSet::new(100);
        for i in 0..100 {
            if i % 20 != 0 {
                live.set(i);
            }
        }

        format
            .write_live_docs(&live, &dir, &info, 5, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();

        let read_back = format
            .read_live_docs(
                &dir,
                &reading_info("_0", 100, 5),
                &*crate::store::DEFAULT_IO_CONTEXT,
            )
            .unwrap();

        assert_eq!(read_back.length(), 100);
        for i in 0..100 {
            assert_eq!(read_back.get(i), live.get(i), "doc {i}");
        }
    }

    #[test]
    fn round_trip_sparse_live_docs() {
        let dir = ByteBuffersDirectory::default();
        let format = Lucene90LiveDocsFormat::new();
        let info = make_info("_0", 1000, 0, -1);

        // Only 5 deletions out of 1000 = 0.5% -> sparse representation.
        let mut live = FixedBitSet::new(1000);
        for i in 0..1000 {
            if i != 7 && i != 42 && i != 123 && i != 456 && i != 789 {
                live.set(i);
            }
        }

        format
            .write_live_docs(&live, &dir, &info, 5, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();

        let read_back = format
            .read_live_docs(
                &dir,
                &reading_info("_0", 1000, 5),
                &*crate::store::DEFAULT_IO_CONTEXT,
            )
            .unwrap();

        assert_eq!(read_back.length(), 1000);
        for i in 0..1000 {
            assert_eq!(read_back.get(i), live.get(i), "doc {i}");
        }
    }

    #[test]
    fn files_lists_live_docs_when_deletions_exist() {
        let format = Lucene90LiveDocsFormat::new();
        let info = make_info("_0", 10, 1, 0);
        let mut files = Vec::new();
        format.files(&info, &mut files).unwrap();
        assert_eq!(files, vec!["_0.liv"]);
    }

    #[test]
    fn files_empty_when_no_deletions() {
        let format = Lucene90LiveDocsFormat::new();
        let info = make_info("_0", 10, 0, -1);
        let mut files = Vec::new();
        format.files(&info, &mut files).unwrap();
        assert!(files.is_empty());
    }
}
