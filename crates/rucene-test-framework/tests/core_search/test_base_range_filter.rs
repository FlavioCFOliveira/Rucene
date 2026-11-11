//! Port of `lucene/core/src/test/org/apache/lucene/search/TestBaseRangeFilter.java`.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, Assert, BeforeClass};
use rucene::document::{
    Document, DoublePoint, FloatPoint, IntPoint, LongPoint, NumericDocValuesField,
    SortedDocValuesField, Store,
};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::index_writer_config::OpenMode;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::{close_dir, close_reader, field_mut, field_mut_as, static_hook};
use crate::core_support::{assert_equals_msg, class, random, test};

pub const F: bool = false;
pub const T: bool = true;

/// Collation interacts badly with hyphens -- collation produces different
/// ordering than Unicode code-point ordering -- so two indexes are created:
/// one which can't have negative random integers, for testing collated
/// ranges, and the other which can have negative random integers, for all
/// other tests.
pub struct TestIndex {
    pub max_r: i32,
    pub min_r: i32,
    pub allow_negative_random_ints: bool,
    pub index: TestDirectory,
}

impl TestIndex {
    fn new(random: &dyn Random, min_r: i32, max_r: i32, allow_negative_random_ints: bool) -> Self {
        TestIndex {
            min_r,
            max_r,
            allow_negative_random_ints,
            index: LuceneTestCase::new_directory_with(random),
        }
    }
}

/// The static fields of `TestBaseRangeFilter`.
pub struct Statics {
    pub signed_index_reader: Arc<dyn DirectoryReader>,
    pub unsigned_index_reader: Arc<dyn DirectoryReader>,
    pub signed_index_dir: TestIndex,
    pub unsigned_index_dir: TestIndex,
    pub max_id: i32,
}

pub static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

pub const MIN_ID: i32 = 0;

/// `Integer.toString(Integer.MAX_VALUE).length()`.
const INT_LENGTH: usize = 10;

/// a simple padding function that should work with any int
pub fn pad(mut n: i32) -> String {
    let mut b = String::with_capacity(40);
    let mut p = "0";
    if n < 0 {
        p = "-";
        n = i32::MAX + n + 1;
    }
    b.push_str(p);
    let s = n.to_string();
    for _ in s.len()..=INT_LENGTH {
        b.push('0');
    }
    b.push_str(&s);
    b
}

/// `TestBaseRangeFilter`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestBaseRangeFilter",
        vec![
            static_hook(
                "beforeClassBaseTestRangeFilter",
                BeforeClass,
                before_class_base_test_range_filter,
            ),
            static_hook(
                "afterClassBaseTestRangeFilter",
                AfterClass,
                after_class_base_test_range_filter,
            ),
            test("testPad", test_pad),
        ],
    )
}

/// `TestBaseRangeFilter` as the superclass of `TestMultiTermConstantScore`:
/// its static hooks, which run before the subclass's; the subclass binds the
/// inherited `testPad` itself (see `J2R_GUIDE.md` section 4).
pub fn base_class() -> Arc<Class> {
    randomizedtesting::jdk::reflect::Class::builder("org.apache.lucene.search.TestBaseRangeFilter")
        .extends(&LuceneTestCase::class())
        .method(static_hook(
            "beforeClassBaseTestRangeFilter",
            BeforeClass,
            before_class_base_test_range_filter,
        ))
        .method(static_hook(
            "afterClassBaseTestRangeFilter",
            AfterClass,
            after_class_base_test_range_filter,
        ))
        .build()
}

pub fn before_class_base_test_range_filter() -> Result<()> {
    let max_id = LuceneTestCase::at_least(500);
    let mut signed_index_dir = TestIndex::new(&*random(), i32::MAX, i32::MIN, true);
    let mut unsigned_index_dir = TestIndex::new(&*random(), i32::MAX, 0, false);
    let signed_index_reader = build(&*random(), &mut signed_index_dir, max_id)?;
    let unsigned_index_reader = build(&*random(), &mut unsigned_index_dir, max_id)?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        signed_index_reader,
        unsigned_index_reader,
        signed_index_dir,
        unsigned_index_dir,
        max_id,
    });
    Ok(())
}

pub fn after_class_base_test_range_filter() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(s) = statics {
        close_reader(Some(s.signed_index_reader))?;
        close_reader(Some(s.unsigned_index_reader))?;
        close_dir(Some(s.signed_index_dir.index))?;
        close_dir(Some(s.unsigned_index_dir.index))?;
    }
    Ok(())
}

fn build(
    random: &dyn Random,
    index: &mut TestIndex,
    max_id: i32,
) -> Result<Arc<dyn DirectoryReader>> {
    /* build an index */

    let mut doc = Document::new();
    let id_field = LuceneTestCase::new_string_field_with(random, "id", "", Store::YES);
    let id_dv_field = SortedDocValuesField::new("id", BytesRef::new(Vec::new()));
    let int_id_field = IntPoint::new("id_int", &[0])?;
    let int_dv_field = NumericDocValuesField::new("id_int", 0);
    let float_id_field = FloatPoint::new("id_float", &[0.0])?;
    let float_dv_field = NumericDocValuesField::new("id_float", 0);
    let long_id_field = LongPoint::new("id_long", &[0])?;
    let long_dv_field = NumericDocValuesField::new("id_long", 0);
    let double_id_field = DoublePoint::new("id_double", &[0.0])?;
    let double_dv_field = NumericDocValuesField::new("id_double", 0);
    let rand_field = LuceneTestCase::new_string_field_with(random, "rand", "", Store::YES);
    let rand_dv_field = SortedDocValuesField::new("rand", BytesRef::new(Vec::new()));
    let body_field = LuceneTestCase::new_string_field_with(random, "body", "", Store::NO);
    let body_dv_field = SortedDocValuesField::new("body", BytesRef::new(Vec::new()));
    doc.add(Box::new(id_field));
    doc.add(Box::new(id_dv_field));
    doc.add(Box::new(int_id_field));
    doc.add(Box::new(int_dv_field));
    doc.add(Box::new(float_id_field));
    doc.add(Box::new(float_dv_field));
    doc.add(Box::new(long_id_field));
    doc.add(Box::new(long_dv_field));
    doc.add(Box::new(double_id_field));
    doc.add(Box::new(double_dv_field));
    doc.add(Box::new(rand_field));
    doc.add(Box::new(rand_dv_field));
    doc.add(Box::new(body_field));
    doc.add(Box::new(body_dv_field));

    let config = LuceneTestCase::new_index_writer_config_with(
        random,
        Some(Arc::new(MockAnalyzer::new(random))),
    );
    config.set_open_mode(OpenMode::CREATE)?;
    config.set_max_buffered_docs(TestUtil::next_int(random, 50, 1000))?;
    config.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(random, index.index.as_directory(), config)?;
    TestUtil::reduce_open_files(writer.w())?;

    loop {
        let mut min_count = 0;
        let mut max_count = 0;

        for d in MIN_ID..=max_id {
            field_mut(&mut doc, "id").set_string_value(pad(d))?;
            field_mut_as::<SortedDocValuesField>(&mut doc, "id")
                .set_bytes_value(BytesRef::from_char_sequence(&pad(d)))?;
            field_mut_as::<IntPoint>(&mut doc, "id_int").set_int_value(d)?;
            field_mut_as::<NumericDocValuesField>(&mut doc, "id_int").set_value(i64::from(d));
            field_mut_as::<FloatPoint>(&mut doc, "id_float").set_float_value(d as f32)?;
            field_mut_as::<NumericDocValuesField>(&mut doc, "id_float")
                .set_value(i64::from((d as f32).to_bits() as i32));
            field_mut_as::<LongPoint>(&mut doc, "id_long").set_long_value(i64::from(d))?;
            field_mut_as::<NumericDocValuesField>(&mut doc, "id_long").set_value(i64::from(d));
            field_mut_as::<DoublePoint>(&mut doc, "id_double").set_double_value(f64::from(d))?;
            field_mut_as::<NumericDocValuesField>(&mut doc, "id_double")
                .set_value(f64::from(d).to_bits() as i64);
            let r = if index.allow_negative_random_ints {
                random.next_int()
            } else {
                random.next_int_bound(i32::MAX)
            };
            if index.max_r < r {
                index.max_r = r;
                max_count = 1;
            } else if index.max_r == r {
                max_count += 1;
            }

            if r < index.min_r {
                index.min_r = r;
                min_count = 1;
            } else if r == index.min_r {
                min_count += 1;
            }
            field_mut(&mut doc, "rand").set_string_value(pad(r))?;
            field_mut_as::<SortedDocValuesField>(&mut doc, "rand")
                .set_bytes_value(BytesRef::from_char_sequence(&pad(r)))?;
            field_mut(&mut doc, "body").set_string_value("body".to_string())?;
            field_mut_as::<SortedDocValuesField>(&mut doc, "body")
                .set_bytes_value(BytesRef::from_char_sequence("body"))?;
            writer.add_document(&doc)?;
        }

        if min_count == 1 && max_count == 1 {
            let ir = writer.get_reader()?;
            writer.close()?;
            return Ok(ir);
        }

        writer.delete_all()?;
    }
}

pub fn test_pad() -> Result<()> {
    let tests = [-9999999, -99560, -100, -3, -1, 0, 3, 9, 10, 1000, 999999999];
    for i in 0..tests.len() - 1 {
        let a = tests[i];
        let b = tests[i + 1];
        let aa = pad(a);
        let bb = pad(b);
        let label = format!("{a}:{aa} vs {b}:{bb}");
        assert_equals_msg(
            Some(&format!("length of {label}")),
            &aa.encode_utf16().count(),
            &bb.encode_utf16().count(),
        );
        Assert::assert_true_msg(&format!("compare less than {label}"), aa < bb);
    }
    Ok(())
}
