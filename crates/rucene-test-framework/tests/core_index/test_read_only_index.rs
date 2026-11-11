//! Port of `lucene/core/src/test/org/apache/lucene/index/TestReadOnlyIndex.java`.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::{AfterClass, BeforeClass};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::terms::Term;
use rucene::search::phrase_query::PhraseQuery;
use rucene::search::term_query::TermQuery;
use rucene::search::Query;
use rucene::store::java_nio_file::{IntoNioPath, Path};
use rucene::store::{Directory, FSDirectory};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};
use crate::support::static_hook;

/// `TestReadOnlyIndex`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestReadOnlyIndex",
        vec![
            static_hook("buildIndex", BeforeClass, build_index),
            static_hook("afterClass", AfterClass, after_class),
            test("testReadOnlyIndex", test_read_only_index),
        ],
    )
}

/// `private static final String longTerm`.
const LONG_TERM: &str = "longtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongtermlongterm";

/// `private static final String text = "This is the text to be indexed. " + longTerm`.
fn text() -> String {
    format!("This is the text to be indexed. {LONG_TERM}")
}

/// `private static Path indexPath`.
static INDEX_PATH: Mutex<Option<Path>> = Mutex::new(None);

/// `indexPath`.
fn index_path() -> Path {
    INDEX_PATH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| crate::support::npe("indexPath"))
}

/// `Files.createTempDirectory(prefix)`: a new directory in the default
/// temporary-file directory, named `prefix` followed by a random suffix.
fn create_temp_directory(prefix: &str) -> Result<Path> {
    let base = std::env::temp_dir();
    loop {
        let candidate = base.join(format!("{prefix}{}", rand_suffix()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate.into_nio_path()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
}

/// The random part of a `Files.createTempDirectory` name. It does not draw
/// from the test's random source, as Java's `TempFileHelper` uses its own
/// `SecureRandom`.
fn rand_suffix() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.finish()
}

/// `@BeforeClass public static void buildIndex()`.
fn build_index() -> Result<()> {
    let path = create_temp_directory("readonlyindex")?.to_absolute_path();
    *INDEX_PATH.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.clone());

    // borrows from TestDemo, but not important to keep in sync with demo
    let analyzer = Arc::new(MockAnalyzer::new(&*random()));
    let directory = LuceneTestCase::new_fs_directory(&path);
    let iwriter = RandomIndexWriter::new(&*random(), directory.as_directory(), analyzer.clone())?;
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "fieldname",
        &text(),
        Store::YES,
    )));
    iwriter.add_document(&doc)?;
    iwriter.close()?;
    directory.close()?;
    rucene::analysis::Analyzer::close(&*analyzer);
    Ok(())
}

/// `@AfterClass public static void afterClass()`.
fn after_class() -> Result<()> {
    *INDEX_PATH.lock().unwrap_or_else(|e| e.into_inner()) = None;
    Ok(())
}

/// Port of `TestReadOnlyIndex.testReadOnlyIndex`.
///
/// Java runs the body with only read permissions on the index directory
/// (`RuntimePermission("*")`, `PropertyPermission("*", "read")`, and
/// `FilePermission` "read" on `indexPath` and `indexPath/-`); see
/// [`LuceneTestCase::run_with_restricted_permissions`] for the port.
fn test_read_only_index() -> Result<()> {
    LuceneTestCase::run_with_restricted_permissions(do_test_read_only_index)
}

/// `private Void doTestReadOnlyIndex()`.
fn do_test_read_only_index() -> Result<()> {
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(index_path())?);
    let ireader = directory_reader::open(Arc::clone(&dir))?;
    let isearcher = LuceneTestCase::new_searcher(Arc::clone(&ireader));

    // borrows from TestDemo, but not important to keep in sync with demo

    assert_equals(
        &1,
        &isearcher.count(Arc::new(TermQuery::new(Term::from_text(
            "fieldname",
            LONG_TERM,
        ))))?,
    );
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("fieldname", "text")));
    let hits = isearcher.search(query, 1)?;
    assert_equals(&1i64, &hits.total_hits.value());
    // Iterate through the results:
    let stored_fields = isearcher.stored_fields()?;
    for i in 0..hits.score_docs.len() {
        let hit_doc = stored_fields.document(hits.score_docs[i].doc)?;
        assert_equals(&Some(text()), &hit_doc.get("fieldname"));
    }

    // Test simple phrase query
    let phrase_query = PhraseQuery::from_strings("fieldname", &["to", "be"])?;
    assert_equals(&1, &isearcher.count(Arc::new(phrase_query))?);

    ireader.close()?;
    // Java never closes `dir`.
    drop(dir);
    Ok(())
}
