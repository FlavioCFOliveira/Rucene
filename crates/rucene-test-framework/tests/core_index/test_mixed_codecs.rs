//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMixedCodecs.java`.

use std::collections::HashSet;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::codecs::lookup;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::terms::Term;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test, verbose};
use crate::support::npe;

/// `TestMixedCodecs`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestMixedCodecs",
        vec![test("test", test_)],
    )
}

/// Port of `TestMixedCodecs.test`.
fn test_() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(1000);
    let mut dir = LuceneTestCase::new_directory();
    let mut w: Option<RandomIndexWriter> = None;
    let mut docs_left_in_this_segment = 0;

    let mut doc_upto = 0;
    while doc_upto < num_docs {
        if verbose() {
            println!("TEST: {doc_upto} of {num_docs}");
        }
        if docs_left_in_this_segment == 0 {
            let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
                MockAnalyzer::new(&*random()),
            )));
            if random().next_boolean() {
                // Make sure we aggressively mix in SimpleText
                // since it has different impls for all codec
                // formats...
                iwc.set_codec(lookup("SimpleText")?)?;
            }
            if let Some(w) = w.take() {
                w.close()?;
            }
            w = Some(RandomIndexWriter::new_with_config(
                &*random(),
                dir.as_directory(),
                iwc,
            )?);
            docs_left_in_this_segment = TestUtil::next_int(&*random(), 10, 100);
        }
        let mut doc = Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "id",
            &doc_upto.to_string(),
            Store::YES,
        )));
        w.as_ref().unwrap_or_else(|| npe("w")).add_document(&doc)?;
        doc_upto += 1;
        docs_left_in_this_segment -= 1;
    }

    if verbose() {
        println!("\nTEST: now delete...");
    }

    let w = w.unwrap_or_else(|| npe("w"));
    // Random delete half the docs:
    let mut deleted: HashSet<i32> = HashSet::new();
    while (deleted.len() as i32) < num_docs / 2 {
        let to_delete = random().next_int_bound(num_docs);
        if !deleted.contains(&to_delete) {
            deleted.insert(to_delete);
            w.delete_documents(Term::from_text("id", &to_delete.to_string()))?;
            if random().next_int_bound(17) == 6 {
                let r = w.get_reader()?;
                assert_equals(&(num_docs - deleted.len() as i32), &r.num_docs());
                r.close()?;
            }
        }
    }

    w.close()?;
    drop(w);
    dir.close()
}
