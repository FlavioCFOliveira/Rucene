//! Port of `lucene/core/src/test/org/apache/lucene/index/Test2BNumericDocValues.java`.
//!
//! Java calls `dvField.setLongValue(i)` on the field already added to the
//! document; Rucene's `Document` owns its fields, so the document is
//! rebuilt per add with the current value.

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::documents_writer::MAX_DOCS;
use rucene::index::index_writer::IndexWriter;
use rucene_test_framework::util::lucene_test_case::{
    Monster, SuppressCodecs, SuppressSysoutChecks,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, test};
use crate::support::{never_throttle, open_reader, println_flush, two_b_config};

/// `Test2BNumericDocValues`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test2BNumericDocValues",
        vec![test("testNumerics", test_numerics)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
            .annotate(TimeoutSuite {
                millis: 8 * 3_600_000,
            })
            .annotate(Monster {
                value: "takes ~ 2 hours if the heap is 5gb".to_string(),
            })
            .annotate(SuppressSysoutChecks {
                bug_url: "Stuff gets printed".to_string(),
            })
        },
    )
}

/// Indexes `IndexWriter.MAX_DOCS` docs with an increasing dv field.
fn test_numerics() -> Result<()> {
    let mut dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("2BNumerics"));
    never_throttle(&dir);

    let w = IndexWriter::new(dir.as_directory(), two_b_config()?)?;

    for i in 0..MAX_DOCS {
        // `dvField.setLongValue(i)`
        let mut doc = Document::new();
        doc.add(Box::new(NumericDocValuesField::new("dv", i as i64)));
        w.add_document(&doc)?;
        if i % 100000 == 0 {
            println_flush(&format!("indexed: {i}"));
        }
    }

    w.force_merge(1)?;
    w.close()?;

    println_flush("verifying...");

    let r = open_reader(&dir)?;
    let mut expected_value: i64 = 0;
    for context in Arc::clone(&r).leaves()? {
        let reader = context.leaf_reader();
        let mut dv = reader.get_numeric_doc_values("dv")?.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.NullPointerException",
                "dv",
            )
            .throw()
        });
        for i in 0..reader.max_doc() {
            assert_equals(&i, &dv.next_doc()?);
            assert_equals(&expected_value, &dv.long_value()?);
            expected_value += 1;
        }
    }

    r.close()?;
    dir.close()
}
