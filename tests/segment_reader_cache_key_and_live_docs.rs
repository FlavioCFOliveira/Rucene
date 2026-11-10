//! Regression tests of two `SegmentReader` members the test framework's
//! reader wrappers (rmp #344) rely on, both measured against Lucene 10.5.0 by
//! `IndexWrappersOracle` (`crates/rucene-test-framework/tests/fixtures/iw_wrappers.tsv`,
//! rows `plain leafN.cache` and `plain leafN.live`):
//!
//! * `getReaderCacheHelper()` answers one helper with one key for the life of
//!   the reader (`SegmentReader.java:330-349`): the key of two calls is the
//!   same key (`reader=same+stable`). The port handed out a clone holding a
//!   zero-sized key by value, whose address — the port's key identity —
//!   changed with every call.
//! * `getLiveDocs()` returns the `SparseLiveDocs` or `DenseLiveDocs`
//!   `Lucene90LiveDocsFormat` reads (`Lucene90LiveDocsFormat.java:108-128`),
//!   which `instanceof LiveDocs` accepts (`deleted=1`). The port's shared
//!   handle answered `None` to `Bits::into_live_docs`, so
//!   `getLiveDocsWithDeletedIterator()` was `null` and `AssertingLeafReader`
//!   took its plain-`Bits` branch.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, StringField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{CacheKey, IndexReader, LeafReader, Term};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::{Bits, BytesRef};

/// A segment of 4 documents, `id-1` deleted.
fn leaf_with_one_deletion() -> (Arc<dyn IndexReader>, Arc<dyn LeafReader>) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    // no merge may rewrite the segment without its deleted document
    let config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))
        .unwrap();
    let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..4 {
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", format!("id-{i}"), Store::YES).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
    }
    writer.commit().unwrap();
    writer
        .delete_documents_by_terms(vec![Term::new("id", BytesRef::new(b"id-1".to_vec()))])
        .unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let leaves = Arc::clone(&reader).leaves().unwrap();
    assert_eq!(leaves.len(), 1);
    let leaf = leaves[0].leaf_reader();
    (reader, leaf)
}

fn key_address(key: &CacheKey) -> *const CacheKey {
    key as *const CacheKey
}

#[test]
fn reader_cache_helper_key_is_the_same_on_every_call() {
    let (reader, leaf) = leaf_with_one_deletion();
    let first = leaf.get_reader_cache_helper().unwrap();
    let second = leaf.get_reader_cache_helper().unwrap();
    assert_eq!(
        key_address(first.get_key()),
        key_address(second.get_key()),
        "getReaderCacheHelper().getKey() must be one key"
    );
    // and it is not the core key
    let core = leaf.get_core_cache_helper().unwrap();
    assert_ne!(key_address(first.get_key()), key_address(core.get_key()));
    reader.close().unwrap();
}

#[test]
fn live_docs_of_a_segment_with_deletions_are_live_docs() {
    let (reader, leaf) = leaf_with_one_deletion();
    let live = leaf.get_live_docs().unwrap();
    assert_eq!(live.length(), 4);
    assert!(!live.get(1));
    let live_docs = leaf
        .get_live_docs()
        .and_then(Bits::into_live_docs)
        .expect("SparseLiveDocs or DenseLiveDocs: instanceof LiveDocs");
    assert_eq!(live_docs.deleted_count(), 1);
    let mut deleted = live_docs.deleted_docs_iterator().unwrap();
    assert_eq!(deleted.next_doc().unwrap(), 1);
    assert_eq!(deleted.next_doc().unwrap(), NO_MORE_DOCS);
    let with_iterator = leaf
        .get_live_docs_with_deleted_iterator()
        .expect("getLiveDocsWithDeletedIterator() is the LiveDocs");
    assert_eq!(with_iterator.deleted_count(), 1);
    // the narrowed handle still reads the shared bits
    assert_eq!(with_iterator.identity(), live.identity());
    reader.close().unwrap();
}
