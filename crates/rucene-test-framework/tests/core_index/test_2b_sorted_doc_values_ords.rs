//! Port of `lucene/core/src/test/org/apache/lucene/index/Test2BSortedDocValuesOrds.java`.
//!
//! Java mutates the `byte[]` the field's `BytesRef` wraps between
//! `addDocument` calls. A Rust `BytesRef` owns its bytes (a `byte[]` aliased
//! by the test and the field cannot be mutated through one alias in safe
//! Rust), so the test hands the current bytes to the document's own field
//! with `setBytesValue` before each add.

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, SortedDocValuesField};
use rucene::error::Result;
use rucene::index::doc_values::DocValues;
use rucene::index::documents_writer::MAX_DOCS;
use rucene::index::index_writer::IndexWriter;
use rucene::util::BytesRef;
use rucene_test_framework::util::lucene_test_case::{
    Monster, SuppressCodecs, SuppressSysoutChecks,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, test};
use crate::support::{never_throttle, open_reader, println_flush, two_b_config};

/// `Test2BSortedDocValuesOrds`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test2BSortedDocValuesOrds",
        vec![test("test2BOrds", test_2b_ords)],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string(), "Direct".to_string()],
            })
            .annotate(TimeoutSuite {
                millis: 80 * 3_600_000, // effectively no limit
            })
            // The six hour time was achieved on a Linux 3.13 system with these specs:
            // 3-core AMD at 2.5Ghz, 12 GB RAM, 5GB test heap, 2 test JVMs, 2TB SATA.
            .annotate(Monster {
                value: "Takes ~ 6 hours if the heap is 5gb".to_string(),
            })
            .annotate(SuppressSysoutChecks {
                bug_url: "Stuff gets printed".to_string(),
            })
        },
    )
}

/// Indexes `Integer.MAX_VALUE` docs with a fixed binary field.
fn test_2b_ords() -> Result<()> {
    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("2BOrds"));
    never_throttle(&dir);

    let w = IndexWriter::new(dir.as_directory(), two_b_config()?)?;

    let mut doc = Document::new();
    let mut bytes = [0u8; 4];
    let dv_field = SortedDocValuesField::new("dv", BytesRef::new(bytes.to_vec()));
    doc.add(Box::new(dv_field));

    for i in 0..MAX_DOCS {
        bytes[0] = (i >> 24) as u8;
        bytes[1] = (i >> 16) as u8;
        bytes[2] = (i >> 8) as u8;
        bytes[3] = (i) as u8;
        doc.get_field_mut_as::<SortedDocValuesField>("dv")
            .expect("dvField")
            .set_bytes_value(BytesRef::new(bytes.to_vec()))?;
        w.add_document(&doc)?;
        if i % 100000 == 0 {
            println_flush(&format!("indexed: {i}"));
        }
    }

    w.force_merge(1)?;
    w.close()?;

    println_flush("verifying...");

    let r = open_reader(&dir)?;
    let mut counter: i32 = 0;
    for context in Arc::clone(&r).leaves()? {
        let reader = context.leaf_reader();
        let mut dv = DocValues::get_sorted(&*reader, "dv")?;
        for i in 0..reader.max_doc() {
            assert_equals(&i, &dv.next_doc()?);
            bytes[0] = (counter >> 24) as u8;
            bytes[1] = (counter >> 16) as u8;
            bytes[2] = (counter >> 8) as u8;
            bytes[3] = (counter) as u8;
            counter += 1;
            let term = dv.lookup_ord(dv.ord_value()?)?;
            assert_equals(&BytesRef::new(bytes.to_vec()), &term);
        }
    }

    r.close()?;
    dir.close()
}
