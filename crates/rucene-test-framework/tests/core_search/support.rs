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
#[derive(Debug)]
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

    /// `await(timeout, unit)`: whether the count reached zero before the
    /// timeout elapsed.
    pub fn await_timeout(&self, timeout: std::time::Duration) -> bool {
        let count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        let (count, _) = self
            .zero
            .wait_timeout_while(count, timeout, |count| *count > 0)
            .unwrap_or_else(|e| e.into_inner());
        *count == 0
    }

    /// `await()`.
    pub fn await_(&self) {
        let mut count = self.count.lock().unwrap_or_else(|e| e.into_inner());
        while *count > 0 {
            count = self.zero.wait(count).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// The `Field` named `name` that the Java test keeps a reference to after
/// `doc.add(field)` and changes in place, reached through the document.
///
/// # Panics
///
/// `ClassCastException` when the document has no `Field` of that name.
pub fn field_mut<'a>(
    doc: &'a mut rucene::document::Document,
    name: &str,
) -> &'a rucene::document::Field {
    field_mut_as::<rucene::document::Field>(doc, name)
}

/// The field of class `T` named `name` that the Java test keeps a reference
/// to after `doc.add(field)` and changes in place, reached through the
/// document ([`rucene::document::Document::get_field_mut_as`]).
///
/// # Panics
///
/// `ClassCastException` when the document has no field of that name and
/// class.
pub fn field_mut_as<'a, T: rucene::index::IndexableField + 'static>(
    doc: &'a mut rucene::document::Document,
    name: &str,
) -> &'a T {
    doc.get_field_mut_as::<T>(name).unwrap_or_else(|| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.ClassCastException",
            format!("no {} named {name}", std::any::type_name::<T>()),
        )
        .throw()
    })
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

/// `assertNotEquals(unexpected, actual)` over `Query.equals`.
pub fn assert_query_not_equals(unexpected: &Arc<dyn Query>, actual: &Arc<dyn Query>) {
    if unexpected.query_eq(&**actual) {
        randomizedtesting::junit::Assert::fail(&format!(
            "Values should be different. Actual: {}",
            rucene::search::query_to_string(&**actual)
        ));
    }
}

/// `assertEquals(expected, actual)` over `Query.equals`, with JUnit's
/// `expected:<…> but was:<…>` message.
pub fn assert_query_equals(expected: &dyn Query, actual: &dyn Query) {
    if !expected.query_eq(actual) {
        randomizedtesting::junit::Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            rucene::search::query_to_string(expected),
            rucene::search::query_to_string(actual)
        ));
    }
}
