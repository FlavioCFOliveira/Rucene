//! Port of `lucene/core/src/test/org/apache/lucene/index/TestManyFields.java`.
//!
//! Test that creates way, way, way too many fields

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::term_query::TermQuery;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::lucene_test_case::SuppressCodecs;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class_annotated, random, test, verbose};

/// `private static final FieldType storedTextType = new FieldType(TextField.TYPE_NOT_STORED)`.
static STORED_TEXT_TYPE: LazyLock<FieldType> =
    LazyLock::new(|| FieldType::new_from(TextField::type_not_stored()));

/// `TestManyFields` (`@SuppressCodecs("SimpleText")`).
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.index.TestManyFields",
        vec![
            test("testManyFields", test_many_fields),
            test("testDiverseDocs", test_diverse_docs),
            test("testRotatingFieldNames", test_rotating_field_names),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// `newField(name, value, storedTextType)`.
fn stored_text(name: &str, value: &str) -> Box<Field> {
    Box::new(LuceneTestCase::new_field(name, value, &STORED_TEXT_TYPE))
}

/// Port of `TestManyFields.testManyFields`.
fn test_many_fields() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_max_buffered_docs(10)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    for j in 0..100 {
        let mut doc = Document::new();
        doc.add(stored_text(&format!("a{j}"), &format!("aaa{j}")));
        doc.add(stored_text(&format!("b{j}"), &format!("aaa{j}")));
        doc.add(stored_text(&format!("c{j}"), &format!("aaa{j}")));
        doc.add(stored_text(&format!("d{j}"), "aaa"));
        doc.add(stored_text(&format!("e{j}"), "aaa"));
        doc.add(stored_text(&format!("f{j}"), "aaa"));
        writer.add_document(&doc)?;
    }
    writer.close()?;
    drop(writer);

    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;
    assert_equals(&100, &reader.max_doc());
    assert_equals(&100, &reader.num_docs());
    for j in 0..100 {
        let doc_freq =
            |field: String, text: String| reader.doc_freq(&Term::from_text(&field, &text));
        assert_equals(&1, &doc_freq(format!("a{j}"), format!("aaa{j}"))?);
        assert_equals(&1, &doc_freq(format!("b{j}"), format!("aaa{j}"))?);
        assert_equals(&1, &doc_freq(format!("c{j}"), format!("aaa{j}"))?);
        assert_equals(&1, &doc_freq(format!("d{j}"), "aaa".to_string())?);
        assert_equals(&1, &doc_freq(format!("e{j}"), "aaa".to_string())?);
        assert_equals(&1, &doc_freq(format!("f{j}"), "aaa".to_string())?);
    }
    reader.close()?;
    dir.close()
}

/// Port of `TestManyFields.testDiverseDocs`.
fn test_diverse_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = mock_iwc();
    iwc.set_ram_buffer_size_mb(0.5)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let n = LuceneTestCase::at_least(1);
    for _ in 0..n {
        // First, docs where every term is unique (heavy on
        // Posting instances)
        for _ in 0..100 {
            let mut doc = Document::new();
            for _ in 0..100 {
                doc.add(stored_text("field", &random().next_int().to_string()));
            }
            writer.add_document(&doc)?;
        }

        // Next, many single term docs where only one term
        // occurs (heavy on byte blocks)
        for _ in 0..100 {
            let mut doc = Document::new();
            doc.add(stored_text(
                "field",
                "aaa aaa aaa aaa aaa aaa aaa aaa aaa aaa",
            ));
            writer.add_document(&doc)?;
        }

        // Next, many single term docs where only one term
        // occurs but the terms are very long (heavy on
        // char[] arrays)
        for j in 0..100 {
            let mut b = String::new();
            let x = format!("{j}.");
            for _ in 0..1000 {
                b.push_str(&x);
            }
            let long_term = b;

            let mut doc = Document::new();
            doc.add(stored_text("field", &long_term));
            writer.add_document(&doc)?;
        }
    }
    writer.close()?;
    drop(writer);

    let reader = directory_reader::open(dir.as_directory())?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let total_hits = searcher.count(Arc::new(TermQuery::new(Term::from_text("field", "aaa"))))?;
    assert_equals(&(i64::from(n) * 100), &i64::from(total_hits));
    reader.close()?;

    dir.close()
}

/// LUCENE-4398
///
/// Port of `TestManyFields.testRotatingFieldNames`.
fn test_rotating_field_names() -> Result<()> {
    let dir = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
        "TestIndexWriter.testChangingFields",
    ));
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_ram_buffer_size_mb(0.2)?;
    iwc.set_max_buffered_docs(-1)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut upto = 0;

    let ft = FieldType::new_from(TextField::type_not_stored());
    ft.set_omit_norms(true)?;

    let mut first_doc_count = -1;
    for iter in 0..10 {
        let start_flush_count = w.get_flush_count();
        let mut doc_count = 0;
        while w.get_flush_count() == start_flush_count {
            let mut doc = Document::new();
            for _ in 0..10 {
                doc.add(Box::new(Field::new(
                    &format!("field{upto}"),
                    "content".to_string(),
                    ft.clone(),
                )?));
                upto += 1;
            }
            w.add_document(&doc)?;
            doc_count += 1;
        }

        if verbose() {
            println!("TEST: iter={iter} flushed after docCount={doc_count}");
        }

        if iter == 0 {
            first_doc_count = doc_count;
        }

        Assert::assert_true_msg(
            &format!(
                "flushed after too few docs: first segment flushed at docCount={first_doc_count}, but current segment flushed after docCount={doc_count}; iter={iter}"
            ),
            (doc_count as f32) / first_doc_count as f32 > 0.9,
        );

        if upto > 5000 {
            // Start re-using field names after a while
            // ... important because otherwise we can OOME due
            // to too many FieldInfo instances.
            upto = 0;
        }
    }
    w.close()?;
    drop(w);
    dir.close()
}
