//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDefaultCodecParallelizesIO.java`.
//!
//! Java's static fields `dir` and `reader` are the suite's [`STATICS`].

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::BytesRef;
use rucene_test_framework::store::SerialIOCountingDirectory;
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::hamcrest::{assert_greater_than, assert_less_than};
use crate::core_support::{class, random, test};
use crate::support::static_hook;

/// `TestDefaultCodecParallelizesIO`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDefaultCodecParallelizesIO",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            static_hook("afterClass", AfterClass, after_class),
            test("testTermsSeekExact", test_terms_seek_exact),
            test("testStoredFields", test_stored_fields),
        ],
    )
}

/// `private static SerialIOCountingDirectory dir; private static IndexReader reader;`
///
/// `dir` wraps `bb_dir` through a shared handle, whose `close` is a no-op;
/// closing `dir` also closes `bb_dir`, as `FilterDirectory.close()` closes
/// `in`.
struct Statics {
    dir: Arc<SerialIOCountingDirectory>,
    bb_dir: Arc<ByteBuffersDirectory>,
    reader: Arc<dyn DirectoryReader>,
}

static STATICS: Mutex<Option<Statics>> = Mutex::new(None);

/// The suite's `dir` and `reader`.
fn statics() -> (Arc<SerialIOCountingDirectory>, Arc<dyn DirectoryReader>) {
    let guard = STATICS.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard
        .as_ref()
        .unwrap_or_else(|| crate::support::npe("TestDefaultCodecParallelizesIO.dir"));
    (Arc::clone(&s.dir), Arc::clone(&s.reader))
}

/// `@BeforeClass public static void beforeClass()`.
fn before_class() -> Result<()> {
    let bb_dir = Arc::new(ByteBuffersDirectory::new());
    {
        let docs = LineFileDocs::with_default_file(&*random())?;
        let iwc = IndexWriterConfig::new();
        iwc.set_use_compound_file(false)?;
        iwc.set_merge_policy(Some(LuceneTestCase::new_log_merge_policy_use_cfs(false)))?;
        iwc.set_codec(TestUtil::get_default_codec())?;
        let w = IndexWriter::new(Arc::clone(&bb_dir) as Arc<dyn Directory>, iwc)?;
        let body = || -> Result<()> {
            let num_docs = LuceneTestCase::at_least(10_000);
            for _ in 0..num_docs {
                let doc = docs.next_doc()?;
                w.add_document(&*doc.borrow())?;
            }
            w.force_merge(1)
        };
        let outcome = body();
        // try-with-resources closes `w`, then `docs`.
        let closed = w.close();
        docs.close();
        outcome?;
        closed?;
    }
    let dir = Arc::new(SerialIOCountingDirectory::new(Box::new(
        rucene::index::documents_writer::SharedDirectory(Arc::clone(&bb_dir) as Arc<dyn Directory>),
    )));
    let reader = directory_reader::open(Arc::clone(&dir) as Arc<dyn Directory>)?;
    *STATICS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Statics {
        dir,
        bb_dir,
        reader,
    });
    Ok(())
}

/// `@AfterClass public static void afterClass()`: `IOUtils.close(reader, dir)`.
fn after_class() -> Result<()> {
    let statics = STATICS.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(Statics {
        dir,
        bb_dir,
        reader,
    }) = statics
    {
        let closed_reader = reader.close();
        drop(reader);
        let still_referenced = || LuceneError::illegal_state("the directory is still referenced");
        let closed_dir = match Arc::try_unwrap(dir) {
            Ok(mut dir) => dir.close(),
            Err(_) => Err(still_referenced()),
        };
        let closed_bb_dir = match Arc::try_unwrap(bb_dir) {
            Ok(mut bb_dir) => bb_dir.close(),
            Err(_) => Err(still_referenced()),
        };
        closed_reader?;
        closed_dir?;
        closed_bb_dir?;
    }
    Ok(())
}

/// Simulate term lookup in a BooleanQuery.
///
/// Port of `TestDefaultCodecParallelizesIO.testTermsSeekExact`.
fn test_terms_seek_exact() -> Result<()> {
    let (dir, reader) = statics();
    let prev_count = dir.count();

    let leaf = LuceneTestCase::get_only_leaf_reader(reader as Arc<dyn IndexReader>)?;
    let terms = leaf
        .terms("body")?
        .unwrap_or_else(|| crate::support::npe("terms(\"body\")"));
    let term_values = ["a", "which", "the", "for", "he"];
    let mut enums = Vec::with_capacity(term_values.len());
    for _ in 0..term_values.len() {
        enums.push(terms.iterator()?);
    }
    let mut suppliers = Vec::with_capacity(term_values.len());
    for (te, value) in enums.iter_mut().zip(term_values) {
        suppliers.push(te.prepare_seek_exact(&BytesRef::from_char_sequence(value))?);
    }
    let mut non_null_io_suppliers = 0i64;
    for supplier in suppliers.iter_mut().flatten() {
        non_null_io_suppliers += 1;
        supplier.get()?;
    }

    assert_greater_than(non_null_io_suppliers, 0);
    let new_count = dir.count();
    assert_greater_than(new_count, prev_count);
    assert_less_than(new_count, prev_count + non_null_io_suppliers);
    Ok(())
}

/// Simulate stored fields retrieval.
///
/// Port of `TestDefaultCodecParallelizesIO.testStoredFields`.
fn test_stored_fields() -> Result<()> {
    let (dir, reader) = statics();
    let prev_count = dir.count();

    let leaf_reader = LuceneTestCase::get_only_leaf_reader(reader as Arc<dyn IndexReader>)?;
    let mut stored_fields = leaf_reader.stored_fields()?;
    let mut docs = [0i32; 20];
    for d in docs.iter_mut() {
        *d = random().next_int_bound(leaf_reader.max_doc());
        stored_fields.prefetch(*d)?;
    }
    for &doc in &docs {
        stored_fields.document(doc)?;
    }

    let new_count = dir.count();
    assert_greater_than(new_count, prev_count);
    assert_less_than(new_count, prev_count + docs.len() as i64);
    Ok(())
}
