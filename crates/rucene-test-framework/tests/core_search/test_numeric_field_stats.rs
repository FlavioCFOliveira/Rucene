//! Port of `lucene/core/src/test/org/apache/lucene/search/TestNumericFieldStats.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::InetAddressPoint;
use rucene::document::{
    Document, Field, FieldType, IntField, LongField, SortedNumericDocValuesField, Store,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::IndexReader;
use rucene::search::numeric_field_stats::{NumericFieldStats, Stats};
use rucene::util::BytesRef;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{
    assert_equals, assert_equals_msg, assert_not_null, assert_null, class, test,
};

/// `TestNumericFieldStats`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestNumericFieldStats",
        vec![
            test(
                "testGetStatsWithAllByteWidths",
                test_get_stats_with_all_byte_widths,
            ),
            test("testGetStatsWithLongField", test_get_stats_with_long_field),
            test("testGetStatsWithIntField", test_get_stats_with_int_field),
            test(
                "testGetStatsFromDocValuesSkipper",
                test_get_stats_from_doc_values_skipper,
            ),
            test("testGetStatsEmptyIndex", test_get_stats_empty_index),
            test(
                "testGetStatsNonexistentField",
                test_get_stats_nonexistent_field,
            ),
            test(
                "testGetStatsDocValuesWithoutSkipIndex",
                test_get_stats_doc_values_without_skip_index,
            ),
            test("testDocCountWithLongField", test_doc_count_with_long_field),
            test(
                "testDocCountFromDocValuesSkipper",
                test_doc_count_from_doc_values_skipper,
            ),
            test(
                "testGetStatsWithExtremeLongValues",
                test_get_stats_with_extreme_long_values,
            ),
            test(
                "testGetStatsWithExtremeIntValues",
                test_get_stats_with_extreme_int_values,
            ),
            test(
                "testGetStatsMultiValuedField",
                test_get_stats_multi_valued_field,
            ),
            test(
                "testGetStatsMultipleSegments",
                test_get_stats_multiple_segments,
            ),
            test(
                "testGetStatsWithSegmentsWithAndWithoutField",
                test_get_stats_with_segments_with_and_without_field,
            ),
            test(
                "testGetStatsReturnsNullForWidePointValues",
                test_get_stats_returns_null_for_wide_point_values,
            ),
        ],
    )
}

fn new_custom_width_point(name: &str, num_bytes: i32, mut value: i64) -> Result<Field> {
    let r#type = FieldType::new();
    r#type.set_dimensions(1, num_bytes)?;
    r#type.freeze();
    let mut packed = vec![0u8; num_bytes as usize];
    for i in (0..num_bytes as usize).rev() {
        packed[i] = (value & 0xFF) as u8;
        value >>= 8;
    }
    packed[0] ^= 0x80;
    Field::new_with_bytes(name, BytesRef::new(packed), r#type)
}

/// `newIndexWriterConfig()`.
fn default_config() -> Result<IndexWriterConfig> {
    Ok(LuceneTestCase::new_index_writer_config())
}

fn min_value_for_width(num_bytes: i32) -> i64 {
    // `-(1L << 63)` wraps to `Long.MIN_VALUE` in Java.
    (1i64 << (num_bytes * 8 - 1)).wrapping_neg()
}

fn max_value_for_width(num_bytes: i32) -> i64 {
    // `(1L << 63) - 1` wraps to `Long.MAX_VALUE` in Java.
    (1i64 << (num_bytes * 8 - 1)).wrapping_sub(1)
}

/// `NumericFieldStats.getStats(reader, field)`.
fn get_stats(
    reader: &Arc<dyn rucene::index::DirectoryReader>,
    field: &str,
) -> Result<Option<Stats>> {
    NumericFieldStats::get_stats(Arc::clone(reader) as Arc<dyn IndexReader>, field)
}

/// `try (Directory dir = newDirectory(); IndexWriter w = new IndexWriter(dir,
/// config))`: runs `body`, then closes the writer and the directory.
///
/// `config` builds the configuration after the directory is drawn, in
/// Java's order.
fn with_writer(
    config: impl FnOnce() -> Result<IndexWriterConfig>,
    body: impl FnOnce(&IndexWriter) -> Result<()>,
) -> Result<()> {
    let dir: TestDirectory = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), config()?)?;
    let outcome = body(&w);
    w.close()?;
    dir.close()?;
    outcome
}

/// `try (IndexReader reader = DirectoryReader.open(w))`.
fn with_reader(
    w: &IndexWriter,
    body: impl FnOnce(&Arc<dyn rucene::index::DirectoryReader>) -> Result<()>,
) -> Result<()> {
    let reader = directory_reader::open_with_writer(w)?;
    let outcome = body(&reader);
    reader.close()?;
    outcome
}

fn doc_with(field: impl rucene::index::IndexableField + 'static) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(field));
    doc
}

/// Port of `TestNumericFieldStats.testGetStatsWithAllByteWidths`.
fn test_get_stats_with_all_byte_widths() -> Result<()> {
    for num_bytes in 1..=8 {
        let min = min_value_for_width(num_bytes);
        let max = max_value_for_width(num_bytes);
        with_writer(default_config, |w| {
            for value in [min, 0, max] {
                w.add_document(&doc_with(new_custom_width_point(
                    "field", num_bytes, value,
                )?))?;
            }
            w.commit()?;
            with_reader(w, |reader| {
                let stats = get_stats(reader, "field")?;
                let msg = format!("numBytes={num_bytes}");
                if stats.is_none() {
                    Assert::fail(&msg);
                }
                let stats = stats.expect("assertNotNull");
                assert_equals_msg(Some(&msg), &min, &stats.min);
                assert_equals_msg(Some(&msg), &max, &stats.max);
                assert_equals_msg(Some(&msg), &3, &stats.doc_count);
                Ok(())
            })
        })?;
    }
    Ok(())
}

/// The single-segment statistics check: `assertNotNull(stats)`, then
/// `min` and `max`.
fn assert_min_max(stats: Option<Stats>, min: i64, max: i64) {
    assert_not_null(&stats);
    let stats = stats.expect("assertNotNull(stats)");
    assert_equals(&min, &stats.min);
    assert_equals(&max, &stats.max);
}

/// Port of `TestNumericFieldStats.testGetStatsWithLongField`.
fn test_get_stats_with_long_field() -> Result<()> {
    with_writer(default_config, |w| {
        for value in [10i64, 20, 30] {
            w.add_document(&doc_with(LongField::new("field", value, Store::NO)))?;
        }
        w.commit()?;
        with_reader(w, |reader| {
            assert_min_max(get_stats(reader, "field")?, 10, 30);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsWithIntField`.
fn test_get_stats_with_int_field() -> Result<()> {
    with_writer(default_config, |w| {
        for value in [-5, 0, 42] {
            w.add_document(&doc_with(IntField::new("field", value, Store::NO)))?;
        }
        w.commit()?;
        with_reader(w, |reader| {
            assert_min_max(get_stats(reader, "field")?, -5, 42);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsFromDocValuesSkipper`.
fn test_get_stats_from_doc_values_skipper() -> Result<()> {
    with_writer(default_config, |w| {
        for value in [100i64, 200, 300] {
            w.add_document(&doc_with(SortedNumericDocValuesField::indexed_field(
                "field", value,
            )))?;
        }
        w.commit()?;
        with_reader(w, |reader| {
            assert_min_max(get_stats(reader, "field")?, 100, 300);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsEmptyIndex`.
fn test_get_stats_empty_index() -> Result<()> {
    with_writer(default_config, |w| {
        w.commit()?;
        with_reader(w, |reader| {
            assert_null(&get_stats(reader, "field")?);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsNonexistentField`.
fn test_get_stats_nonexistent_field() -> Result<()> {
    with_writer(default_config, |w| {
        w.add_document(&doc_with(LongField::new("other_field", 42, Store::NO)))?;
        w.commit()?;
        with_reader(w, |reader| {
            assert_null(&get_stats(reader, "field")?);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsDocValuesWithoutSkipIndex`.
fn test_get_stats_doc_values_without_skip_index() -> Result<()> {
    with_writer(default_config, |w| {
        for value in [10i64, 20, 30] {
            w.add_document(&doc_with(SortedNumericDocValuesField::new("field", value)))?;
        }
        w.commit()?;
        with_reader(w, |reader| {
            let stats = get_stats(reader, "field")?;
            assert_null(&stats);
            Ok(())
        })
    })
}

/// `NumericFieldStats.getStats(reader, "field").docCount()`.
fn doc_count(reader: &Arc<dyn rucene::index::DirectoryReader>) -> Result<i32> {
    Ok(get_stats(reader, "field")?
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new("java.lang.NullPointerException", "")
                .throw()
        })
        .doc_count)
}

/// Port of `TestNumericFieldStats.testDocCountWithLongField`.
fn test_doc_count_with_long_field() -> Result<()> {
    with_writer(default_config, |w| {
        for value in [10i64, 20, 30] {
            w.add_document(&doc_with(LongField::new("field", value, Store::NO)))?;
        }
        w.add_document(&Document::new())?;
        w.commit()?;
        with_reader(w, |reader| {
            assert_equals(&3, &doc_count(reader)?);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testDocCountFromDocValuesSkipper`.
fn test_doc_count_from_doc_values_skipper() -> Result<()> {
    with_writer(default_config, |w| {
        for value in [10i64, 20, 30] {
            w.add_document(&doc_with(SortedNumericDocValuesField::indexed_field(
                "field", value,
            )))?;
        }
        w.add_document(&Document::new())?;
        w.commit()?;
        with_reader(w, |reader| {
            assert_equals(&3, &doc_count(reader)?);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsWithExtremeLongValues`.
fn test_get_stats_with_extreme_long_values() -> Result<()> {
    with_writer(default_config, |w| {
        w.add_document(&doc_with(LongField::new("field", i64::MIN, Store::NO)))?;
        w.add_document(&doc_with(LongField::new("field", i64::MAX, Store::NO)))?;
        w.commit()?;
        with_reader(w, |reader| {
            assert_min_max(get_stats(reader, "field")?, i64::MIN, i64::MAX);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsWithExtremeIntValues`.
fn test_get_stats_with_extreme_int_values() -> Result<()> {
    with_writer(default_config, |w| {
        w.add_document(&doc_with(IntField::new("field", i32::MIN, Store::NO)))?;
        w.add_document(&doc_with(IntField::new("field", i32::MAX, Store::NO)))?;
        w.commit()?;
        with_reader(w, |reader| {
            assert_min_max(
                get_stats(reader, "field")?,
                i32::MIN as i64,
                i32::MAX as i64,
            );
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsMultiValuedField`.
fn test_get_stats_multi_valued_field() -> Result<()> {
    with_writer(default_config, |w| {
        let mut doc1 = Document::new();
        doc1.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "field", 5,
        )));
        doc1.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "field", 50,
        )));
        w.add_document(&doc1)?;
        w.add_document(&doc_with(SortedNumericDocValuesField::indexed_field(
            "field", 25,
        )))?;
        w.commit()?;
        with_reader(w, |reader| {
            assert_min_max(get_stats(reader, "field")?, 5, 50);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsMultipleSegments`.
fn test_get_stats_multiple_segments() -> Result<()> {
    let config = || -> Result<IndexWriterConfig> {
        let config = LuceneTestCase::new_index_writer_config();
        config.set_merge_policy(Some(NoMergePolicy::instance()))?;
        Ok(config)
    };
    with_writer(config, |w| {
        for value in [100i64, 50, 200] {
            w.add_document(&doc_with(LongField::new("field", value, Store::NO)))?;
            w.commit()?;
        }
        with_reader(w, |reader| {
            Assert::assert_true((Arc::clone(reader) as Arc<dyn IndexReader>).leaves()?.len() > 1);
            let stats = get_stats(reader, "field")?;
            assert_min_max(stats, 50, 200);
            assert_equals(&3, &stats.expect("assertNotNull").doc_count);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsWithSegmentsWithAndWithoutField`.
fn test_get_stats_with_segments_with_and_without_field() -> Result<()> {
    with_writer(default_config, |w| {
        w.add_document(&doc_with(LongField::new("field", 100, Store::NO)))?;
        w.commit()?;
        w.add_document(&Document::new())?;
        w.commit()?;
        with_reader(w, |reader| {
            let stats = get_stats(reader, "field")?;
            assert_min_max(stats, 100, 100);
            assert_equals(&1, &stats.expect("assertNotNull").doc_count);
            Ok(())
        })
    })
}

/// Port of `TestNumericFieldStats.testGetStatsReturnsNullForWidePointValues`.
fn test_get_stats_returns_null_for_wide_point_values() -> Result<()> {
    with_writer(default_config, |w| {
        // `InetAddress.getByName("192.168.0.1")`
        w.add_document(&doc_with(InetAddressPoint::new(
            "field",
            std::net::IpAddr::from([192, 168, 0, 1]),
        )))?;
        w.commit()?;
        with_reader(w, |reader| {
            assert_null(&get_stats(reader, "field")?);
            Ok(())
        })
    })
}
