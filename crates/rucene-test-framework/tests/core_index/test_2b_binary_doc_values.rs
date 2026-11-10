//! Port of `lucene/core/src/test/org/apache/lucene/index/Test2BBinaryDocValues.java`.
//!
//! Java mutates the `byte[]` the field's `BytesRef` wraps between
//! `addDocument` calls; Rucene's `Document` owns its fields, so the document
//! is rebuilt per add with the current bytes (no random draw involved).

use std::sync::Arc;

use randomizedtesting::annotations::TimeoutSuite;
use randomizedtesting::jdk::reflect::Class;
use rucene::document::{BinaryDocValuesField, Document};
use rucene::error::Result;
use rucene::index::documents_writer::MAX_DOCS;
use rucene::index::index_writer::IndexWriter;
use rucene::store::{ByteArrayDataInput, ByteArrayDataOutput, DataInput, DataOutput};
use rucene::util::BytesRef;
use rucene_test_framework::util::lucene_test_case::{
    Monster, SuppressCodecs, SuppressSysoutChecks,
};
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, test};
use crate::support::{never_throttle, open_reader, println_flush, two_b_config};

/// `Test2BBinaryDocValues`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.Test2BBinaryDocValues",
        vec![
            test("testFixedBinary", test_fixed_binary),
            test("testVariableBinary", test_variable_binary),
        ],
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
                value: "takes ~ 6 hours if the heap is 5gb".to_string(),
            })
            .annotate(SuppressSysoutChecks {
                bug_url: "Stuff gets printed.".to_string(),
            })
        },
    )
}

/// Indexes `IndexWriter.MAX_DOCS` docs with a fixed binary field.
fn test_fixed_binary() -> Result<()> {
    let mut dir =
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("2BFixedBinary"));
    never_throttle(&dir);

    let w = IndexWriter::new(dir.as_directory(), two_b_config()?)?;

    let mut bytes = [0u8; 4];

    for i in 0..MAX_DOCS {
        bytes[0] = (i >> 24) as u8;
        bytes[1] = (i >> 16) as u8;
        bytes[2] = (i >> 8) as u8;
        bytes[3] = i as u8;
        let mut doc = Document::new();
        doc.add(Box::new(BinaryDocValuesField::new(
            "dv",
            BytesRef::new(bytes.to_vec()),
        )));
        w.add_document(&doc)?;
        if i % 100000 == 0 {
            println_flush(&format!("indexed: {i}"));
        }
    }

    w.force_merge(1)?;
    w.close()?;

    println_flush("verifying...");

    let r = open_reader(&dir)?;
    let mut expected_value: i32 = 0;
    for context in Arc::clone(&r).leaves()? {
        let reader = context.leaf_reader();
        let mut dv = reader.get_binary_doc_values("dv")?.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.NullPointerException",
                "dv",
            )
            .throw()
        });
        for i in 0..reader.max_doc() {
            bytes[0] = (expected_value >> 24) as u8;
            bytes[1] = (expected_value >> 16) as u8;
            bytes[2] = (expected_value >> 8) as u8;
            bytes[3] = expected_value as u8;
            assert_equals(&i, &dv.next_doc()?);
            let term = dv.binary_value()?;
            assert_equals(&BytesRef::new(bytes.to_vec()), &term);
            expected_value += 1;
        }
    }

    r.close()?;
    dir.close()
}

/// Indexes `IndexWriter.MAX_DOCS` docs with a variable binary field.
fn test_variable_binary() -> Result<()> {
    let mut dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "2BVariableBinary",
    ));
    never_throttle(&dir);

    let w = IndexWriter::new(dir.as_directory(), two_b_config()?)?;

    let mut bytes = [0u8; 4];

    for i in 0..MAX_DOCS {
        let length = {
            let mut encoder = ByteArrayDataOutput::new(&mut bytes);
            encoder.write_v_int(i % 65535)?; // 1, 2, or 3 bytes
            encoder.position()
        };
        let mut data = BytesRef::new(bytes.to_vec());
        data.length = length;
        let mut doc = Document::new();
        doc.add(Box::new(BinaryDocValuesField::new("dv", data)));
        w.add_document(&doc)?;
        if i % 100000 == 0 {
            println_flush(&format!("indexed: {i}"));
        }
    }

    w.force_merge(1)?;
    w.close()?;

    println_flush("verifying...");

    let r = open_reader(&dir)?;
    let mut expected_value: i32 = 0;
    for context in Arc::clone(&r).leaves()? {
        let reader = context.leaf_reader();
        let mut dv = reader.get_binary_doc_values("dv")?.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.NullPointerException",
                "dv",
            )
            .throw()
        });
        for i in 0..reader.max_doc() {
            assert_equals(&i, &dv.next_doc()?);
            let term = dv.binary_value()?;
            let mut input = ByteArrayDataInput::with_range(&term.bytes, term.offset, term.length);
            assert_equals(&(expected_value % 65535), &input.read_v_int()?);
            randomizedtesting::junit::Assert::assert_true(input.eof());
            expected_value += 1;
        }
    }

    r.close()?;
    dir.close()
}
