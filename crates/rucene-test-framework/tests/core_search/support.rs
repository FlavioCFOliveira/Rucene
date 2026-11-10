//! Plumbing shared by the classes of the `core_search` target: Java test
//! classes whose instance fields live in a fixture struct the runner
//! constructs, and the query shorthands the ported bodies use.

#![allow(dead_code)]

use std::any::Any;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, ClassBuilder, Constructor, Method};
use rucene::error::Result;
use rucene::index::Term;
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::LuceneTestCase;

/// A public `test*` method of a class whose instance is the fixture `F`.
pub fn ftest<F: Any + Send>(name: &str, body: fn(&mut F) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |f: &mut F| {
            if let Err(e) = body(f) {
                rethrow_java(e)
            }
        })
}

/// `public void setUp() throws Exception { super.setUp(); body }`.
pub fn set_up<F: Any + Send>(body: fn(&mut F) -> Result<()>) -> Method {
    Method::new("setUp")
        .throws(&["java.lang.Exception"])
        .body(move |f: &mut F| {
            LuceneTestCase::super_set_up();
            if let Err(e) = body(f) {
                rethrow_java(e)
            }
        })
}

/// `public void tearDown() throws Exception { body; super.tearDown(); }`.
pub fn tear_down<F: Any + Send>(body: fn(&mut F) -> Result<()>) -> Method {
    Method::new("tearDown")
        .throws(&["java.lang.Exception"])
        .body(move |f: &mut F| {
            if let Err(e) = body(f) {
                rethrow_java(e)
            }
            LuceneTestCase::super_tear_down();
        })
}

/// `public class name extends LuceneTestCase` whose instance is `F`
/// (`F::default()`, Java's field initialisers), with `methods`.
pub fn fixture_class<F: Any + Send + Default>(name: &str, methods: Vec<Method>) -> Arc<Class> {
    fixture_class_annotated::<F>(name, methods, |b| b)
}

/// [`fixture_class`] with class-level annotations.
pub fn fixture_class_annotated<F: Any + Send + Default>(
    name: &str,
    methods: Vec<Method>,
    annotate: impl FnOnce(ClassBuilder) -> ClassBuilder,
) -> Arc<Class> {
    let mut builder = annotate(
        Class::builder(name)
            .extends(&LuceneTestCase::class())
            .constructor(Constructor::no_args(F::default)),
    );
    for m in methods {
        builder = builder.method(m);
    }
    builder.build()
}

/// `new Term(field, text)`.
pub fn term(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

/// `new TermQuery(new Term(field, text))`.
pub fn tq(field: &str, text: &str) -> Arc<dyn Query> {
    Arc::new(TermQuery::new(term(field, text)))
}

/// `Object.toString()` of a class that does not override it:
/// `getClass().getName() + "@" + Integer.toHexString(hashCode())`, the
/// identity hash standing for the address.
pub fn object_to_string<T: ?Sized>(class_name: &str, object: &T) -> String {
    format!(
        "{class_name}@{:x}",
        (object as *const T as *const () as usize) as u32
    )
}

/// `java.util.concurrent.CountDownLatch`.
pub struct CountDownLatch {
    count: std::sync::Mutex<i64>,
    zero: std::sync::Condvar,
}

impl CountDownLatch {
    /// `new CountDownLatch(count)`.
    pub fn new(count: i64) -> CountDownLatch {
        CountDownLatch {
            count: std::sync::Mutex::new(count),
            zero: std::sync::Condvar::new(),
        }
    }

    /// `countDown()`.
    pub fn count_down(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        if *count > 0 {
            *count -= 1;
            if *count == 0 {
                self.zero.notify_all();
            }
        }
    }

    /// `await()`.
    pub fn await_(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count > 0 {
            count = self.zero.wait(count).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// The same `Field` object added to a document again: a field with the same
/// name, current string value and type.
///
/// **Divergence from Lucene 10.5.0.** Java tests keep a reference to a field
/// they added to a `Document`, call `setStringValue` on it and add the
/// document again; a Rucene `Document` owns its fields, so the test keeps the
/// field and rebuilds the document from copies of it, in the same order and
/// without drawing again.
pub fn copy_field(field: &rucene::document::Field) -> Result<rucene::document::Field> {
    rucene::document::Field::new(
        field.name(),
        field.string_value().unwrap_or_default(),
        rucene::document::FieldType::new_from(field.field_type()),
    )
}

/// `new Document()` holding copies of `fields`, in order (see
/// [`copy_field`]).
pub fn doc_of(fields: &[&rucene::document::Field]) -> Result<rucene::document::Document> {
    let mut doc = rucene::document::Document::new();
    for f in fields {
        doc.add(Box::new(copy_field(f)?));
    }
    Ok(doc)
}

/// A `public static void name()` method annotated `annotation`
/// (`@BeforeClass`, `@AfterClass`).
pub fn static_hook(
    name: &str,
    annotation: impl randomizedtesting::jdk::reflect::JavaAnnotation,
    body: fn() -> Result<()>,
) -> Method {
    use randomizedtesting::jdk::reflect::modifier;
    Method::new(name)
        .modifiers(modifier::PUBLIC | modifier::STATIC)
        .throws(&["java.lang.Exception"])
        .annotate(annotation)
        .static_body(move || {
            if let Err(e) = body() {
                rethrow_java(e)
            }
        })
}

/// `IOUtils.close`-style release of a reader, closing it.
pub fn close_reader(
    reader: Option<Arc<dyn rucene::index::directory_reader::DirectoryReader>>,
) -> Result<()> {
    if let Some(reader) = reader {
        reader.close()?;
    }
    Ok(())
}

/// Closes a test directory, if set.
pub fn close_dir(
    dir: Option<rucene_test_framework::util::lucene_test_case::TestDirectory>,
) -> Result<()> {
    if let Some(mut dir) = dir {
        dir.close()?;
    }
    Ok(())
}

/// A `FieldDoc[]` seen as the `ScoreDoc[]` Java's `TopFieldDocs.scoreDocs`
/// is (`FieldDoc extends ScoreDoc`).
pub fn score_docs_of(
    field_docs: &[rucene::search::field_doc::FieldDoc],
) -> Vec<rucene::search::ScoreDoc> {
    field_docs.iter().map(|d| d.score_doc.clone()).collect()
}

/// A `LeafReaderContext` standing for the `null` context Java tests pass to
/// `Collector.getLeafCollector(null)`: the one leaf of a fixed, non-random
/// single-document index, which the caller keeps alive with the returned
/// reader.
///
/// **Divergence from Lucene 10.5.0.** Rucene's
/// `Collector::get_leaf_collector` takes a context by reference, so a real
/// context stands for `null`; the collectors these tests exercise only
/// record and replay it.
pub fn null_leaf_context() -> Result<(
    Arc<dyn rucene::index::directory_reader::DirectoryReader>,
    Vec<Arc<rucene::index::reader_context::LeafReaderContext>>,
)> {
    use rucene::index::IndexReader;
    let dir: Arc<dyn rucene::store::Directory> =
        Arc::new(rucene::store::ByteBuffersDirectory::default());
    let writer = rucene::index::index_writer::IndexWriter::new(
        Arc::clone(&dir),
        rucene::index::index_writer_config::IndexWriterConfig::new(),
    )?;
    writer.add_document(&rucene::document::Document::new())?;
    writer.commit()?;
    writer.close()?;
    let reader = rucene::index::directory_reader::open(dir)?;
    let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
    Ok((reader, leaves))
}
