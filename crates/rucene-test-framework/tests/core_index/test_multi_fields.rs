//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMultiFields.java`.
//!
//! * The anonymous `FilterMergePolicy(NoMergePolicy.INSTANCE)` whose
//!   `keepFullyDeletedSegment` returns `true` is
//!   `KeepFullyDeletedSegmentsMergePolicy` over `NoMergePolicy`.
//! * The fields Java changes before each `addDocument` are changed through
//!   the document (`field_mut`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::multi_bits::MultiBits;
use rucene::index::multi_fields::MultiTerms;
use rucene::index::terms::Term;
use rucene::index::{IndexReader, POSTINGS_ENUM_FREQS, POSTINGS_ENUM_NONE};
use rucene::search::NO_MORE_DOCS;
use rucene::util::unicode_util::UnicodeUtil;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_null, class, random, test, verbose};
use crate::support::{field_mut, npe, KeepFullyDeletedSegmentsMergePolicy};

/// `TestMultiFields`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestMultiFields",
        vec![
            test("testRandom", test_random),
            test("testSeparateEnums", test_separate_enums),
            test("testTermDocsEnum", test_term_docs_enum),
        ],
    )
}

/// `newIndexWriterConfig(new MockAnalyzer(random()))`.
fn mock_iwc() -> IndexWriterConfig {
    LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))))
}

/// Port of `TestMultiFields.testRandom`.
fn test_random() -> Result<()> {
    let num = LuceneTestCase::at_least(2);
    for iter in 0..num {
        if verbose() {
            println!("TEST: iter={iter}");
        }

        let dir = LuceneTestCase::new_directory();

        let iwc = mock_iwc();
        iwc.set_merge_policy(Some(Arc::new(KeepFullyDeletedSegmentsMergePolicy::new(
            NoMergePolicy::instance(),
        ))))?;
        let w = IndexWriter::new(dir.as_directory(), iwc)?;
        let mut docs: HashMap<BytesRef, Vec<i32>> = HashMap::new();
        let mut deleted: HashSet<i32> = HashSet::new();
        let mut terms: Vec<BytesRef> = Vec::new();

        let num_docs = TestUtil::next_int(&*random(), 1, 100 * *RANDOM_MULTIPLIER);
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            "",
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            "",
            Store::NO,
        )));

        let only_unique_terms = random().next_boolean();
        if verbose() {
            println!("TEST: onlyUniqueTerms={only_unique_terms} numDocs={num_docs}");
        }
        let mut unique_terms: HashSet<BytesRef> = HashSet::new();
        for i in 0..num_docs {
            if !only_unique_terms && random().next_boolean() && !terms.is_empty() {
                let term = terms[random().next_int_bound(terms.len() as i32) as usize].clone();
                docs.get_mut(&term)
                    .unwrap_or_else(|| npe("docs.get(term)"))
                    .push(i);
                field_mut(&mut doc, "field").set_string_value(term.utf8_to_string()?)?;
            } else {
                let s = TestUtil::random_unicode_string_max(&*random(), 10);
                let term = BytesRef::from_char_sequence(&s);
                docs.entry(term.clone()).or_default().push(i);
                terms.push(term.clone());
                unique_terms.insert(term);
                field_mut(&mut doc, "field").set_string_value(s)?;
            }
            field_mut(&mut doc, "id").set_string_value(i.to_string())?;
            w.add_document(&doc)?;
            if random().next_int_bound(4) == 1 {
                w.commit()?;
            }
            if i > 0 && random().next_int_bound(20) == 1 {
                let del_id = random().next_int_bound(i);
                deleted.insert(del_id);
                w.delete_documents_by_terms(vec![Term::from_text("id", &del_id.to_string())])?;
                if verbose() {
                    println!("TEST: delete {del_id}");
                }
            }
        }

        if verbose() {
            let mut terms_list: Vec<BytesRef> = unique_terms.iter().cloned().collect();
            terms_list.sort();
            println!("TEST: terms in UTF-8 order:");
            for b in &terms_list {
                println!("  {} {b}", UnicodeUtil::to_hex_string(&b.utf8_to_string()?));
                for doc_id in &docs[b] {
                    if deleted.contains(doc_id) {
                        println!("    {doc_id} (deleted)");
                    } else {
                        println!("    {doc_id}");
                    }
                }
            }
        }

        let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
        w.close()?;
        drop(w);
        if verbose() {
            println!("TEST: reader={reader}");
        }

        let live_docs = MultiBits::get_live_docs(&reader)?;
        for del_doc in &deleted {
            Assert::assert_false(
                live_docs
                    .as_ref()
                    .unwrap_or_else(|| npe("liveDocs"))
                    .get(*del_doc as usize),
            );
        }

        for _ in 0..100 {
            let term = &terms[random().next_int_bound(terms.len() as i32) as usize];
            if verbose() {
                println!(
                    "TEST: seek term={} {term}",
                    UnicodeUtil::to_hex_string(&term.utf8_to_string()?)
                );
            }

            let postings_enum =
                TestUtil::docs(&*random(), &reader, "field", term, None, POSTINGS_ENUM_NONE)?;
            assert_not_null(&postings_enum);
            let mut postings_enum = postings_enum.unwrap_or_else(|| npe("postingsEnum"));

            for doc_id in &docs[term] {
                assert_equals(doc_id, &postings_enum.next_doc()?);
            }
            assert_equals(&NO_MORE_DOCS, &postings_enum.next_doc()?);
        }

        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

/// The index `testSeparateEnums` and `testTermDocsEnum` build: one document
/// `f:j` in each of two segments.
fn two_segment_index() -> Result<(
    rucene_test_framework::util::lucene_test_case::TestDirectory,
    Arc<dyn IndexReader>,
)> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(dir.as_directory(), mock_iwc())?;
    let mut d = Document::new();
    d.add(Box::new(LuceneTestCase::new_string_field(
        "f",
        "j",
        Store::NO,
    )));
    w.add_document(&d)?;
    w.commit()?;
    w.add_document(&d)?;
    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    Ok((dir, r))
}

/// Port of `TestMultiFields.testSeparateEnums`.
fn test_separate_enums() -> Result<()> {
    let (dir, r) = two_segment_index()?;
    let j = BytesRef::from_char_sequence("j");
    let mut d1 = TestUtil::docs(&*random(), &r, "f", &j, None, POSTINGS_ENUM_NONE)?
        .unwrap_or_else(|| npe("d1"));
    let mut d2 = TestUtil::docs(&*random(), &r, "f", &j, None, POSTINGS_ENUM_NONE)?
        .unwrap_or_else(|| npe("d2"));
    assert_equals(&0, &d1.next_doc()?);
    assert_equals(&0, &d2.next_doc()?);
    r.close()?;
    dir.close()
}

/// Port of `TestMultiFields.testTermDocsEnum`.
fn test_term_docs_enum() -> Result<()> {
    let (dir, r) = two_segment_index()?;
    let mut de = MultiTerms::get_term_postings_enum(
        &r,
        "f",
        &BytesRef::from_char_sequence("j"),
        POSTINGS_ENUM_FREQS,
    )?
    .unwrap_or_else(|| npe("de"));
    assert_equals(&0, &de.next_doc()?);
    assert_equals(&1, &de.next_doc()?);
    assert_equals(&NO_MORE_DOCS, &de.next_doc()?);
    r.close()?;
    dir.close()
}
