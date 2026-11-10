//! Every reader answers the same top-level context from `getContext()`.
//!
//! `LeafReader.getContext()` answers the `LeafReaderContext` the reader built
//! for itself (`LeafReader.java:51, 59-62`) and `CompositeReader.getContext()`
//! builds its `CompositeReaderContext` once and caches it
//! (`CompositeReader.java:49, 93-102`). `IndexReader.leaves()` is
//! `getContext().leaves()` (`IndexReader.java:417-419`) and
//! `new IndexSearcher(reader)` takes `reader.getContext()`
//! (`IndexSearcher.java:207-209`), so `searcher.getIndexReader().leaves()` are
//! the searcher's own leaf contexts — which `QueryUtils` and `CheckHits` rely
//! on when they hand them to a weight the searcher built.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, StringField};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, ExitableDirectoryReaderWrapper,
};
use rucene::index::filter_reader::FilterLeafReader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::parallel_reader::{ParallelCompositeReader, ParallelLeafReader};
use rucene::index::{
    directory_reader, CompositeReader, DirectoryReader, IndexReader, IndexReaderContext,
    LeafReader, LeafReaderContext, MultiReader, QueryTimeout,
};
use rucene::search::IndexSearcher;
use rucene::store::{ByteBuffersDirectory, Directory};

#[derive(Debug)]
struct NeverTimesOut;

impl QueryTimeout for NeverTimesOut {
    fn should_exit(&self) -> bool {
        false
    }
}

/// Two segments of three documents each.
fn directory_reader() -> Arc<dyn DirectoryReader> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
    let mut config = IndexWriterConfig::new();
    config
        .set_merge_policy(Some(Arc::new(NoMergePolicy::new())))
        .expect("no merges");
    let writer = IndexWriter::new(Arc::clone(&dir), config).expect("index writer");
    for d in 0..6 {
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", format!("d{d}"), Store::YES).expect("id"),
        ));
        writer.add_document(&doc).expect("add document");
        if d == 2 {
            writer.commit().expect("commit");
        }
    }
    writer.commit().expect("commit");
    writer.close().expect("close");
    directory_reader::open(dir).expect("open a directory reader")
}

fn ids(leaves: &[Arc<LeafReaderContext>]) -> Vec<usize> {
    leaves.iter().map(|leaf| leaf.id()).collect()
}

fn assert_cached(name: &str, reader: Arc<dyn IndexReader>, leaves: usize) {
    let searcher = IndexSearcher::new(Arc::clone(&reader)).expect("searcher");
    let expected = ids(searcher.get_leaf_contexts());
    assert_eq!(expected.len(), leaves, "{name}: leaves");
    assert_eq!(
        ids(&Arc::clone(searcher.get_index_reader()).leaves().unwrap()),
        expected,
        "{name}: searcher.getIndexReader().leaves()"
    );
    assert_eq!(
        ids(&Arc::clone(&reader).leaves().unwrap()),
        expected,
        "{name}: reader.leaves()"
    );
    assert!(
        Arc::ptr_eq(
            &Arc::clone(&reader).get_context().unwrap(),
            searcher.get_top_reader_context()
        ),
        "{name}: reader.getContext()"
    );
}

#[test]
fn every_reader_type_answers_its_cached_context() {
    let directory = directory_reader();
    let segment: Arc<dyn LeafReader> = Arc::clone(&directory).leaves().unwrap()[0].leaf_reader();

    assert_cached(
        "directory",
        Arc::clone(&directory) as Arc<dyn IndexReader>,
        2,
    );
    let exitable = ExitableDirectoryReaderWrapper::new(Arc::new(NeverTimesOut))
        .wrap(Arc::clone(&directory))
        .expect("wrap");
    assert_cached("filter directory", exitable as Arc<dyn IndexReader>, 2);
    let multi = MultiReader::new(vec![Arc::clone(&directory) as Arc<dyn IndexReader>], false)
        .expect("multi");
    assert_cached("multi", Arc::new(multi), 2);
    let parallel = ParallelCompositeReader::with_close_flag(
        false,
        vec![Arc::clone(&directory) as Arc<dyn CompositeReader>],
    )
    .expect("parallel composite");
    assert_cached("parallel composite", Arc::new(parallel), 2);

    assert_cached("segment", Arc::clone(&segment).as_index_reader(), 1);
    let filter: Arc<dyn LeafReader> =
        Arc::new(FilterLeafReader::new(Arc::clone(&segment)).expect("filter leaf"));
    assert_cached("filter leaf", filter.as_index_reader(), 1);
    let parallel_leaf: Arc<dyn LeafReader> = Arc::new(
        ParallelLeafReader::with_close_flag(false, vec![Arc::clone(&segment)])
            .expect("parallel leaf"),
    );
    assert_cached("parallel leaf", parallel_leaf.as_index_reader(), 1);
}
