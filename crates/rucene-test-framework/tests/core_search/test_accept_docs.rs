//! Port of `lucene/core/src/test/org/apache/lucene/search/TestAcceptDocs.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::search::{
    all, from_iterator_supplier, from_live_docs, AcceptDocs, DocIdSetIterator,
    DocIdSetIteratorSupplier, NO_MORE_DOCS,
};
use rucene::util::{FixedBitSet, MatchNoBits};
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, class, test, IAE};

const NPE: &str = "java.lang.NullPointerException";

/// `TestAcceptDocs`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestAcceptDocs",
        vec![
            test("testValidation", test_validation),
            test(
                "testIteratorIgnoresDeletedDocs",
                test_iterator_ignores_deleted_docs,
            ),
            test("testIteratorIsNew", test_iterator_is_new),
        ],
    )
}

/// `() -> null`: a supplier producing `null` iterators.
struct NullSupplier;

impl DocIdSetIteratorSupplier for NullSupplier {
    fn get(&mut self) -> Result<Option<Box<dyn DocIdSetIterator>>> {
        Ok(None)
    }
}

/// `() -> DocIdSetIterator.all(maxDoc)`.
fn all_supplier(max_doc: i32) -> impl FnMut() -> Result<Box<dyn DocIdSetIterator>> {
    move || Ok(Box::new(all(max_doc)?) as Box<dyn DocIdSetIterator>)
}

fn test_validation() -> Result<()> {
    // iterator supplier must be non-null
    expect_throws(NPE, || {
        from_iterator_supplier::<NullSupplier>(None, None, 1)
    });
    // iterator supplier may not produce null iterators
    expect_throws(NPE, || {
        from_iterator_supplier(Some(NullSupplier), None, 1)?.iterator()
    });
    // Bits length != maxDoc
    expect_throws(IAE, || {
        from_live_docs(Some(Box::new(MatchNoBits::new(3))), 4)
    });
    Ok(())
}

fn test_iterator_ignores_deleted_docs() -> Result<()> {
    let max_doc = 5;
    let deleted_doc = 3;
    let mut live_docs = FixedBitSet::new(max_doc as usize);
    let len = live_docs.length();
    live_docs.set_range(0, len);
    live_docs.clear(deleted_doc as usize);
    // `liveDocs.asReadOnlyBits()`, one read-only view per `AcceptDocs`
    // (Java shares the view; both read the same bit set).
    let bits_accept_docs = from_live_docs(Some(Box::new(live_docs.as_read_only_bits())), max_doc)?;
    let iterator_accept_docs = from_iterator_supplier(
        Some(all_supplier(max_doc)),
        Some(Box::new(live_docs.as_read_only_bits())),
        max_doc,
    )?;
    let accept_docs_list: Vec<Box<dyn AcceptDocs>> =
        vec![Box::new(bits_accept_docs), Box::new(iterator_accept_docs)];
    for mut accept_docs in accept_docs_list {
        let accept_bits = accept_docs.bits()?.expect("deleted docs");
        assert_equals(&(max_doc as usize), &accept_bits.length());
        for i in 0..max_doc {
            assert_equals(&(i != deleted_doc), &accept_bits.get(i as usize));
        }

        let mut iterator = accept_docs.iterator()?;
        for i in 0..max_doc {
            if i != deleted_doc {
                assert_equals(&i, &iterator.next_doc()?);
            }
        }
        assert_equals(&NO_MORE_DOCS, &iterator.next_doc()?);
    }
    Ok(())
}

fn test_iterator_is_new() -> Result<()> {
    let max_doc = 5;
    let bits_accept_docs = from_live_docs(None, max_doc)?;
    let iterator_accept_docs = from_iterator_supplier(Some(all_supplier(max_doc)), None, max_doc)?;
    let accept_docs_list: Vec<Box<dyn AcceptDocs>> =
        vec![Box::new(bits_accept_docs), Box::new(iterator_accept_docs)];
    for mut accept_docs in accept_docs_list {
        let mut iterator = accept_docs.iterator()?;
        assert_equals(&-1, &iterator.doc_id());
        iterator.next_doc()?;
        iterator = accept_docs.iterator()?;
        assert_equals(&-1, &iterator.doc_id());
        // Triggers lazy loading of matches into a bit set when created from an iterator
        accept_docs.bits()?;
        iterator = accept_docs.iterator()?;
        assert_equals(&-1, &iterator.doc_id());
        iterator.next_doc()?;
        iterator = accept_docs.iterator()?;
        assert_equals(&-1, &iterator.doc_id());
    }
    Ok(())
}
