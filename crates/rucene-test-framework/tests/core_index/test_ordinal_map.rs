//! Port of `lucene/core/src/test/org/apache/lucene/index/TestOrdinalMap.java`.
//!
//! **Divergence.** `testRamBytesUsed` measures the `OrdinalMap` with
//! `RamUsageTester.ramUsed(map, ORDINAL_MAP_ACCUMULATOR)`, a reflective walk
//! of its object graph that skips the `LongValues.ZEROES`/`IDENTITY`
//! singletons, the shared `NullReader` and the `owner` field. The port's
//! `RamUsageTester` measures only types with a `RamObject` model, and
//! `OrdinalMap` (with its `PackedLongValues`) has none yet, so the
//! measurement fails with `crate::core_support::ram_used_unmodelled`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, SortedDocValuesField, SortedSetDocValuesField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::doc_values::OrdinalMap;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::multi_doc_values::{
    MultiDocValues, MultiSortedDocValues, MultiSortedSetDocValues,
};
use rucene::index::IndexReader;
use rucene::util::extra::ZeroesLongValues;
use rucene::util::{Accountable, BytesRef};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_equals, assert_not_null, class, ram_used_unmodelled, random, test,
};

/// `TestOrdinalMap`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestOrdinalMap",
        vec![
            test("testRamBytesUsed", test_ram_bytes_used),
            test(
                "testOneSegmentWithAllValues",
                test_one_segment_with_all_values,
            ),
        ],
    )
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))
/// .setCodec(TestUtil.alwaysDocValuesFormat(TestUtil.getDefaultDocValuesFormat()))`.
fn config() -> Result<IndexWriterConfig> {
    let cfg = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    cfg.set_codec(TestUtil::always_doc_values_format(
        TestUtil::get_default_doc_values_format(),
    ))?;
    Ok(cfg)
}

/// `RamUsageTester.ramUsed(map, ORDINAL_MAP_ACCUMULATOR)`: see the module
/// divergence.
fn ordinal_map_ram_used(_map: &OrdinalMap) -> i64 {
    ram_used_unmodelled("org.apache.lucene.index.OrdinalMap")
}

/// Port of `TestOrdinalMap.testRamBytesUsed`.
fn test_ram_bytes_used() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let cfg = config()?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), cfg)?;
    let max_doc = TestUtil::next_int(&*random(), 10, 1000);
    let max_term_length = TestUtil::next_int(&*random(), 1, 4);
    for _ in 0..max_doc {
        let mut d = Document::new();
        if random().next_boolean() {
            d.add(Box::new(SortedDocValuesField::new(
                "sdv",
                BytesRef::from_char_sequence(&TestUtil::random_simple_string_max(
                    &*random(),
                    max_term_length,
                )),
            )));
        }
        let num_sorted_set = random().next_int_bound(3);
        for _ in 0..num_sorted_set {
            d.add(Box::new(SortedSetDocValuesField::new(
                "ssdv",
                BytesRef::from_char_sequence(&TestUtil::random_simple_string_max(
                    &*random(),
                    max_term_length,
                )),
            )));
        }
        iw.add_document(&d)?;
        if LuceneTestCase::rarely() {
            iw.get_reader()?.close()?;
        }
    }
    iw.commit()?;
    let r = iw.get_reader()?;
    let ir: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
    let sdv = MultiDocValues::get_sorted_values(&ir, "sdv")?;
    if let Some(multi) = sdv
        .as_ref()
        .and_then(|v| v.disi_as_any())
        .and_then(|a| a.downcast_ref::<MultiSortedDocValues>())
    {
        let map = multi.mapping();
        assert_equals(&ordinal_map_ram_used(map), &map.ram_bytes_used());
    }
    let ssdv = MultiDocValues::get_sorted_set_values(&ir, "ssdv")?;
    if let Some(multi) = ssdv
        .as_ref()
        .and_then(|v| v.disi_as_any())
        .and_then(|a| a.downcast_ref::<MultiSortedSetDocValues>())
    {
        let map = multi.mapping();
        assert_equals(&ordinal_map_ram_used(map), &map.ram_bytes_used());
    }
    iw.close()?;
    r.close()?;
    dir.close()
}

/// Port of `TestOrdinalMap.testOneSegmentWithAllValues`: tests the case
/// where one segment contains all of the global ords. In this case, we apply
/// a small optimization and hardcode the first segment indices and global ord
/// deltas as all zeroes.
fn test_one_segment_with_all_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let cfg = config()?;
    cfg.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    let iw = IndexWriter::new(dir.as_directory(), cfg)?;

    let num_terms = 1000;
    for i in 0..num_terms {
        let mut d = Document::new();
        let term = i.to_string();
        d.add(Box::new(SortedDocValuesField::new(
            "sdv",
            BytesRef::from_char_sequence(&term),
        )));
        iw.add_document(&d)?;
    }
    iw.force_merge(1)?;

    for _ in 0..10 {
        let mut d = Document::new();
        let term = random().next_int_bound(num_terms).to_string();
        d.add(Box::new(SortedDocValuesField::new(
            "sdv",
            BytesRef::from_char_sequence(&term),
        )));
        iw.add_document(&d)?;
    }
    iw.commit()?;

    let r = directory_reader::open_with_writer(&*iw)?;
    let ir: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;
    let sdv = MultiDocValues::get_sorted_values(&ir, "sdv")?;
    assert_not_null(&sdv);
    let multi = sdv
        .as_ref()
        .and_then(|v| v.disi_as_any())
        .and_then(|a| a.downcast_ref::<MultiSortedDocValues>());
    Assert::assert_true(multi.is_some());
    let multi = multi.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            "not a MultiDocValues.MultiSortedDocValues",
        )
        .throw()
    });

    // Check that the optimization kicks in.
    let map = multi.mapping();
    // `assertEquals(LongValues.ZEROES, ...)`: the singleton is the
    // `ZeroesLongValues` type
    Assert::assert_true(
        map.first_segments()
            .as_any()
            .is_some_and(|a| a.is::<ZeroesLongValues>()),
    );
    Assert::assert_true(
        map.global_ord_deltas()
            .as_any()
            .is_some_and(|a| a.is::<ZeroesLongValues>()),
    );

    // Check the map's basic behavior.
    assert_equals(&num_terms, &(map.get_value_count() as i32));
    for i in 0..num_terms {
        assert_equals(&0, &map.get_first_segment_number(i as i64));
        assert_equals(&(i as i64), &map.get_first_segment_ord(i as i64));
    }

    iw.close()?;
    r.close()?;
    dir.close()
}
