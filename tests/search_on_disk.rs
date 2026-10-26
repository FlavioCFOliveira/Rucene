//! Searching an on-disk index must actually find documents.
//!
//! Regression guard for rmp task #180. Before it, `SegmentTermsEnum` could
//! position on a term but never decoded the term's metadata, because the
//! metadata decode needs the postings reader and the reader was owned behind
//! `&mut self` by `Lucene103BlockTreeTermsReader` while the enum held only a
//! shared handle. The visible effect was silence rather than an error:
//! `seek_exact` answered `true`, `doc_freq` answered `0`, and
//! `IndexSearcher::count` over a `TermQuery` answered `Ok(0)`.
//!
//! These tests fail if any of the three regresses.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, StringField, TextField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::Term;
use rucene::search::{IndexSearcher, Query, TermQuery};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

fn doc(d: i32, colour: &str) -> Document {
    let mut document = Document::new();
    document.add(Box::new(
        StringField::new("id", format!("doc-{d}"), Store::YES).expect("id"),
    ));
    document.add(Box::new(
        TextField::new("body", format!("{colour} thing number {d}"), Store::YES).expect("body"),
    ));
    document
}

#[test]
fn term_query_over_on_disk_index_finds_documents() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let tmp = std::env::temp_dir().join("rucene-search-on-disk");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(&tmp).unwrap());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for d in 0..6 {
        let colour = if d % 2 == 0 { "red" } else { "blue" };
        writer.add_document(&doc(d, colour)).unwrap();
    }
    writer.commit().unwrap();

    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let searcher = IndexSearcher::new(reader as Arc<dyn rucene::index::IndexReader>).unwrap();

    let q: Arc<dyn Query> = Arc::new(TermQuery::new(Term::new(
        "body",
        BytesRef::new(b"red".to_vec()),
    )));
    let count = searcher.count(Arc::clone(&q));
    println!("count(TermQuery(body:red)) = {count:?}");
    assert_eq!(
        count.unwrap(),
        3,
        "TermQuery over an on-disk index must find its documents"
    );
}

#[test]
fn a_positioned_term_reports_its_statistics_and_postings() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let tmp = std::env::temp_dir().join("rucene-search-on-disk-stats");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(&tmp).unwrap());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Arc::new(NoMergePolicy::new()))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for d in 0..6 {
        let colour = if d % 2 == 0 { "red" } else { "blue" };
        writer.add_document(&doc(d, colour)).unwrap();
    }
    writer.commit().unwrap();

    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let leaves = reader.leaves();
    println!("leaves = {}", leaves.len());
    let leaf = leaves[0].leaf_reader();
    let terms = leaf.terms("body").unwrap().expect("body terms");
    println!(
        "terms.size={} sumDocFreq={} docCount={}",
        terms.size().unwrap(),
        terms.sum_doc_freq().unwrap(),
        terms.doc_count().unwrap()
    );
    let mut te = terms.iterator().unwrap();
    println!(
        "seek_exact(red) = {:?}",
        te.seek_exact(&BytesRef::new(b"red".to_vec()))
    );
    assert_eq!(
        te.doc_freq().unwrap(),
        3,
        "doc_freq must decode the frame's metadata"
    );
    assert_eq!(
        te.total_term_freq().unwrap(),
        3,
        "total_term_freq must decode too"
    );
    println!("term_state       = {:?}", te.term_state().map(|_| "ok"));
    println!(
        "postings         = {:?}",
        te.postings(None, 0).map(|_| "ok")
    );
    println!("impacts          = {:?}", te.impacts(0).map(|_| "ok"));
    // Does the postings enum actually yield the documents?
    let mut pe = te.postings(None, 0).expect("postings");
    let mut docs = Vec::new();
    loop {
        let d = pe.next_doc().expect("next_doc");
        if d == rucene::search::NO_MORE_DOCS {
            break;
        }
        docs.push(d);
        if docs.len() > 20 {
            break;
        }
    }
    assert_eq!(
        docs,
        vec![0, 2, 4],
        "postings must yield the documents holding the term"
    );
}
