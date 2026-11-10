//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTermdocPerf.java`.
//!
//! `RepeatingTokenizer`, a package-private top-level class of the Java file,
//! is ported here with it.

use std::sync::Arc;
use std::time::Instant;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::{
    Analyzer, AnalyzerState, CharReader, GlobalReuseStrategy, TokenStreamComponents, Tokenizer,
    TokenizerLogic,
};
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::OpenMode;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_NONE};
use rucene::index::IndexReader;
use rucene::search::NO_MORE_DOCS;
use rucene::store::Directory;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{char_term_id, Att};
use rucene_test_framework::analysis::tokenizer_components;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test, verbose};
use crate::support::npe;

/// `TestTermdocPerf`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTermdocPerf",
        vec![test("testTermDocPerf", test_term_doc_perf)],
    )
}

/// `class RepeatingTokenizer extends Tokenizer`.
struct RepeatingTokenizer {
    random: Arc<dyn Random>,
    percent_docs: f32,
    max_tf: i32,
    num: i32,
    term_att: Att,
    value: String,
}

impl std::fmt::Debug for RepeatingTokenizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepeatingTokenizer")
            .field("num", &self.num)
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RepeatingAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepeatingAnalyzer").finish_non_exhaustive()
    }
}

impl TokenizerLogic for RepeatingTokenizer {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _input: &mut dyn CharReader,
    ) -> Result<bool> {
        self.num -= 1;
        if self.num >= 0 {
            source.clear_attributes();
            self.term_att
                .char_term_mut()
                .append_string(Some(&self.value));
            return Ok(true);
        }
        Ok(false)
    }

    fn reset(&mut self, _source: &mut AttributeSource, _reader: &mut dyn CharReader) -> Result<()> {
        if self.random.next_float() < self.percent_docs {
            self.num = self.random.next_int_bound(self.max_tf) + 1;
        } else {
            self.num = 0;
        }
        Ok(())
    }
}

/// The anonymous `Analyzer` of `addDocs`.
struct RepeatingAnalyzer {
    state: AnalyzerState,
    val: String,
    random: Arc<dyn Random>,
    percent_docs: f32,
    max_tf: i32,
}

impl Analyzer for RepeatingAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestTermdocPerf$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        // `new RepeatingTokenizer(val, random, percentDocs, maxTF)`.
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        let t = Tokenizer::with_source(
            source,
            RepeatingTokenizer {
                random: Arc::clone(&self.random),
                percent_docs: self.percent_docs,
                max_tf: self.max_tf,
                num: 0,
                term_att,
                value: self.val.clone(),
            },
        );
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(t))))
    }
}

/// `void addDocs(Random random, Directory dir, int ndocs, String field, String val, int maxTF,
/// float percentDocs)`.
#[allow(dead_code)]
fn add_docs(
    random: Arc<dyn Random>,
    dir: &Arc<dyn Directory>,
    ndocs: i32,
    field: &str,
    val: &str,
    max_tf: i32,
    percent_docs: f32,
) -> Result<()> {
    let analyzer: Arc<dyn Analyzer> = Arc::new(RepeatingAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        val: val.to_string(),
        random,
        percent_docs,
        max_tf,
    });

    let mut doc = Document::new();

    doc.add(Box::new(LuceneTestCase::new_string_field(
        field,
        val,
        Store::NO,
    )));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(analyzer));
    iwc.set_open_mode(OpenMode::CREATE)?;
    iwc.set_max_buffered_docs(100)?;
    iwc.set_merge_policy(Some(Arc::new(
        LuceneTestCase::new_log_merge_policy_merge_factor(100)?,
    )))?;
    let writer = IndexWriter::new(Arc::clone(dir), iwc)?;

    for _ in 0..ndocs {
        writer.add_document(&doc)?;
    }

    writer.force_merge(1)?;
    writer.close()
}

/// `public int doTest(int iter, int ndocs, int maxTF, float percentDocs)`.
#[allow(dead_code)]
fn do_test(iter: i32, ndocs: i32, max_tf: i32, percent_docs: f32) -> Result<i32> {
    let mut dir = LuceneTestCase::new_directory();

    let start = Instant::now();
    add_docs(
        random(),
        &dir.as_directory(),
        ndocs,
        "foo",
        "val",
        max_tf,
        percent_docs,
    )?;
    if verbose() {
        println!(
            "milliseconds for creation of {ndocs} docs = {}",
            start.elapsed().as_millis()
        );
    }

    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;

    let mut tenum = MultiTerms::get_terms(&reader, "foo")?
        .unwrap_or_else(|| npe("MultiTerms.getTerms"))
        .iterator()?;

    let start = Instant::now();

    let mut ret = 0;
    let mut tdocs: Option<Box<dyn PostingsEnum>> = None;
    let random = JdkRandom::new(random().next_long());
    for _ in 0..iter {
        tenum.seek_ceil(&BytesRef::from_char_sequence("val"))?;
        let mut docs = TestUtil::docs_of(&random, &mut *tenum, tdocs.take(), POSTINGS_ENUM_NONE)?;
        while docs.next_doc()? != NO_MORE_DOCS {
            ret += docs.doc_id();
        }
        tdocs = Some(docs);
    }

    if verbose() {
        println!(
            "milliseconds for {iter} TermDocs iteration: {}",
            start.elapsed().as_millis()
        );
    }

    // Java leaves the reader and the directory open.
    let _ = (reader, &mut dir);
    Ok(ret)
}

/// Port of `TestTermdocPerf.testTermDocPerf`.
fn test_term_doc_perf() -> Result<()> {
    // performance test for 10% of documents containing a term
    // doTest(100000, 10000,3,.1f);
    Ok(())
}
