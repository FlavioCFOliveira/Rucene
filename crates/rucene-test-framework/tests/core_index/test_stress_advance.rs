//! Port of `lucene/core/src/test/org/apache/lucene/index/TestStressAdvance.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_NONE};
use rucene::index::terms::SeekStatus;
use rucene::index::IndexReader;
use rucene::search::NO_MORE_DOCS;
use rucene::util::BytesRef;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TEST_NIGHTLY;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test, verbose};
use crate::support::{field_mut, npe};

/// `TestStressAdvance`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestStressAdvance",
        vec![test("testStressAdvance", test_stress_advance)],
    )
}

/// Port of `TestStressAdvance.testStressAdvance`.
fn test_stress_advance() -> Result<()> {
    let num_iters = if *TEST_NIGHTLY { 3 } else { 1 };
    for iter in 0..num_iters {
        if verbose() {
            println!("\nTEST: iter={iter}");
        }
        let dir = LuceneTestCase::new_directory();
        let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let mut a_docs: HashSet<i32> = HashSet::new();
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            "",
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            "",
            Store::YES,
        )));
        let num = LuceneTestCase::at_least(4097);
        if verbose() {
            println!("\nTEST: numDocs={num}");
        }
        for id in 0..num {
            if random().next_int_bound(4) == 3 {
                field_mut(&mut doc, "field").set_string_value("a".to_string())?;
                a_docs.insert(id);
            } else {
                field_mut(&mut doc, "field").set_string_value("b".to_string())?;
            }
            field_mut(&mut doc, "id").set_string_value(id.to_string())?;
            w.add_document(&doc)?;
            if verbose() {
                println!("\nTEST: doc upto {id}");
            }
        }

        w.force_merge(1)?;

        let mut a_doc_ids: Vec<i32> = Vec::new();
        let mut b_doc_ids: Vec<i32> = Vec::new();

        let r = w.get_reader()?;
        let stored_fields = r.stored_fields()?;
        let id_to_doc_id_len = r.max_doc();
        for doc_id in 0..id_to_doc_id_len {
            let id: i32 = stored_fields
                .document(doc_id)?
                .get("id")
                .unwrap_or_else(|| npe("id"))
                .parse()
                .expect("Integer.parseInt");
            if a_docs.contains(&id) {
                a_doc_ids.push(doc_id);
            } else {
                b_doc_ids.push(doc_id);
            }
        }
        let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
        let mut te = leaf
            .terms("field")?
            .unwrap_or_else(|| npe("terms(\"field\")"))
            .iterator()?;

        let mut de: Option<Box<dyn PostingsEnum>> = None;
        for iter2 in 0..10 {
            if verbose() {
                println!("\nTEST: iter={iter} iter2={iter2}");
            }
            assert_equals(
                &SeekStatus::FOUND,
                &te.seek_ceil(&BytesRef::from_char_sequence("a"))?,
            );
            let mut docs = TestUtil::docs_of(&*random(), &mut *te, de.take(), POSTINGS_ENUM_NONE)?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?;
            test_one(&mut *docs, &a_doc_ids)?;
            de = Some(docs);

            assert_equals(
                &SeekStatus::FOUND,
                &te.seek_ceil(&BytesRef::from_char_sequence("b"))?,
            );
            let mut docs = TestUtil::docs_of(&*random(), &mut *te, de.take(), POSTINGS_ENUM_NONE)?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?;
            test_one(&mut *docs, &b_doc_ids)?;
            de = Some(docs);
        }

        w.close()?;
        r.close()?;
        dir.close()?;
    }
    Ok(())
}

/// `private void testOne(PostingsEnum docs, List<Integer> expected)`.
fn test_one(docs: &mut dyn PostingsEnum, expected: &[i32]) -> Result<()> {
    if verbose() {
        println!("test");
    }
    let mut upto: i32 = -1;
    let len = expected.len() as i32;
    while upto < len {
        if verbose() {
            println!("  cycle upto={upto} of {len}");
        }
        let doc_id;
        if random().next_int_bound(4) == 1 || upto == len - 1 {
            // test nextDoc()
            if verbose() {
                println!("    do nextDoc");
            }
            upto += 1;
            doc_id = docs.next_doc()?;
        } else {
            // test advance()
            let inc = TestUtil::next_int(&*random(), 1, len - 1 - upto);
            if verbose() {
                println!("    do advance inc={inc}");
            }
            upto += inc;
            doc_id = docs.advance(expected[upto as usize])?;
        }
        if upto == len {
            if verbose() {
                println!("  expect docID={NO_MORE_DOCS} actual={doc_id}");
            }
            assert_equals(&NO_MORE_DOCS, &doc_id);
        } else {
            if verbose() {
                println!("  expect docID={} actual={doc_id}", expected[upto as usize]);
            }
            Assert::assert_true(doc_id != NO_MORE_DOCS);
            assert_equals(&expected[upto as usize], &doc_id);
        }
    }
    Ok(())
}
