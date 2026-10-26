//! `IndexSearcher` installs Java's default query cache and caching policy.
//!
//! Regression guard for the divergence #192 C-4 removed: Java seeds
//! `IndexSearcher.DEFAULT_QUERY_CACHE` with
//! `new LRUQueryCache(1000, min(1 << 25, maxMemory / 20))` and
//! `DEFAULT_CACHING_POLICY` with a `UsageTrackingQueryCachingPolicy`, and every
//! new searcher starts with both (`IndexSearcher.java:83-100, 131-132`). The
//! port used to start with no cache and no policy, so these assertions fail
//! against it.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::search::IndexSearcher;
use rucene::store::{Directory, RamDirectory};

#[test]
fn a_new_searcher_starts_with_the_default_cache_and_policy() {
    let default_cache =
        IndexSearcher::get_default_query_cache().expect("Java installs a default query cache");
    let rendered = format!("{default_cache:?}");
    assert!(
        rendered.contains("LRUQueryCache")
            && rendered.contains("maxSize: 1000")
            && rendered.contains("maxRamBytesUsed: 33554432"),
        "the default cache is an LRUQueryCache(1000, 1 << 25), got {rendered}"
    );
    let default_policy = IndexSearcher::get_default_query_caching_policy()
        .expect("Java installs a default caching policy");
    assert!(
        format!("{default_policy:?}").contains("UsageTrackingQueryCachingPolicy"),
        "got {default_policy:?}"
    );

    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).expect("writer");
    writer.commit().expect("commit");
    writer.close().expect("close");
    let reader = directory_reader::open(dir).expect("reader");
    let searcher = IndexSearcher::new(reader as Arc<dyn IndexReader>).expect("searcher");
    let cache = searcher
        .get_query_cache()
        .expect("the searcher starts with the cache");
    assert!(
        Arc::ptr_eq(cache, &default_cache),
        "it is the shared default"
    );
    let policy = searcher
        .get_query_caching_policy()
        .expect("the searcher starts with the policy");
    assert!(
        Arc::ptr_eq(policy, &default_policy),
        "it is the shared default"
    );
}
