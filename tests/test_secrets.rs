//! `TestSecrets` hands out the accessors the five owning types register.
//!
//! Pins rmp #211 against Apache Lucene Core 10.5.0: each getter of
//! `org.apache.lucene.internal.tests.TestSecrets` runs the static initializer
//! of the owning class when its accessor is not registered yet
//! (`TestSecrets.java:52-110`), and that initializer registers an
//! implementation — `IndexWriter` the `IndexWriterAccess` and the
//! `IndexPackageAccess`, `SegmentReader`, `ConcurrentMergeScheduler` and
//! `FilterIndexInput` their own. The port had the traits and the registry but
//! no implementation, so every getter failed.
//!
//! The getters only answer callers under `tests/`, the stand-in for Lucene's
//! test-framework package, which is why this is an integration test.

use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Store, StringField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::FieldInfo;
use rucene::internal::tests_hooks::TestSecrets;
use rucene::store::{Directory, RamDirectory};

fn writer_with_one_segment() -> (Arc<dyn Directory>, IndexWriter) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", "a".to_string(), Store::YES).unwrap(),
    ));
    writer.add_document(&doc).unwrap();
    writer.commit().unwrap();
    (dir, writer)
}

#[test]
fn every_getter_returns_the_accessor_its_owner_registers() {
    // IndexWriterAccess, registered by IndexWriter's static initializer.
    let writer_access =
        TestSecrets::get_index_writer_access().expect("IndexWriter registers its accessor");
    let (dir, writer) = writer_with_one_segment();
    assert_eq!(writer_access.get_segment_count(&writer).unwrap(), 1);
    assert!(!writer_access.is_closed(&writer));
    assert!(!writer_access.is_deleter_closed(&writer).unwrap());
    assert_eq!(
        writer_access
            .newest_segment(&writer)
            .unwrap()
            .map(|info| info.info.name.clone()),
        Some("_0".to_string())
    );
    assert!(writer_access.seg_string(&writer).unwrap().contains("_0"));
    assert!(writer_access.get_doc_writer_thread_pool_size(&writer) >= 0);

    // IndexPackageAccess, piggybacked on IndexWriter's static initializer.
    let package_access =
        TestSecrets::get_index_package_access().expect("IndexWriter registers the package access");
    let mut builder = package_access
        .new_field_infos_builder(None, Some("parent"))
        .unwrap();
    builder.add(&FieldInfo::new("f", 0)).unwrap();
    let infos = builder.finish().unwrap();
    assert_eq!(infos.size(), 1);
    // No field named `parent` was added, so the finished infos name none.
    assert_eq!(infos.get_parent_field(), None);
    assert!(
        package_access
            .new_field_infos_builder(Some("same"), Some("same"))
            .is_err(),
        "FieldNumbers refuses one field as both soft-deletes and parent field"
    );

    // SegmentReaderAccess, registered by SegmentReader.
    let segment_reader_access =
        TestSecrets::get_segment_reader_access().expect("SegmentReader registers its accessor");
    writer.close().unwrap();
    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    let leaves = Arc::clone(&reader).leaves();
    let leaf = leaves[0].leaf_reader();
    let segment_reader = leaf
        .as_segment_reader()
        .expect("a committed leaf is a segment reader");
    let core = segment_reader_access.get_core(segment_reader);
    assert!(
        Arc::ptr_eq(&core, &segment_reader_access.get_core(segment_reader)),
        "the accessor hands out the reader's one core"
    );

    // ConcurrentMergeSchedulerAccess, registered by ConcurrentMergeScheduler.
    let cms_access = TestSecrets::get_concurrent_merge_scheduler_access()
        .expect("ConcurrentMergeScheduler registers its accessor");
    let mut cms = ConcurrentMergeScheduler::new();
    cms_access.set_suppress_exceptions(&mut cms);

    // FilterIndexInputAccess, registered by FilterIndexInput.
    let filter_access = TestSecrets::get_filter_input_index_access()
        .expect("FilterIndexInput registers its accessor");
    struct TestOnlyInput;
    filter_access.add_test_filter_type(std::any::TypeId::of::<TestOnlyInput>());
    assert!(rucene::store::is_test_filter_input(std::any::TypeId::of::<
        TestOnlyInput,
    >()));

    // A second call hands out the very same accessors: each static
    // initializer ran once.
    assert!(Arc::ptr_eq(
        &writer_access,
        &TestSecrets::get_index_writer_access().unwrap()
    ));
    assert!(Arc::ptr_eq(
        &package_access,
        &TestSecrets::get_index_package_access().unwrap()
    ));
}
