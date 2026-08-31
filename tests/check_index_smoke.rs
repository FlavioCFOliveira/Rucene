use std::sync::Arc;

use rucene::document::{Document, Store, StringField, TextField};
use rucene::index::check_index::{CheckIndex, Level};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::store::{Directory, RamDirectory};

fn build(dir: Arc<dyn Directory>, segments: usize) {
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for s in 0..segments {
        for d in 0..5 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", format!("{s}-{d}"), Store::YES).unwrap(),
            ));
            doc.add(Box::new(
                TextField::new("body", format!("hello world {s} {d}"), Store::YES).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
    }
    writer.close().unwrap();
}

#[test]
fn sequential_and_concurrent_checks_agree() {
    for threads in [1i32, 4] {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        build(Arc::clone(&dir), 3);

        let mut checker = CheckIndex::with_write_lock(Arc::clone(&dir), None);
        checker.set_level(Level::MAX_VALUE).unwrap();
        checker.set_thread_count(threads).unwrap();
        let status = checker.check_index().unwrap();
        checker.close().unwrap();

        assert!(status.clean, "threads={threads} status={status:?}");
        assert_eq!(status.num_segments, 3, "threads={threads}");
        assert_eq!(status.segment_infos.len(), 3, "threads={threads}");
        assert_eq!(status.num_bad_segments, 0, "threads={threads}");
        assert!(status.valid_counter, "threads={threads}");
        for seg in &status.segment_infos {
            assert!(seg.open_reader_passed);
            assert_eq!(seg.max_doc, 5);
            assert!(seg.error.is_none());
        }
    }
}

#[test]
fn only_segments_marks_the_status_partial() {
    let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    build(Arc::clone(&dir), 2);

    let mut checker = CheckIndex::with_write_lock(Arc::clone(&dir), None);
    checker.set_level(Level::MAX_VALUE).unwrap();
    let all = checker.check_index().unwrap();
    let first = all.segment_infos[0].name.clone();
    let status = checker
        .check_index_segments(Some(&[first.clone()]))
        .unwrap();
    checker.close().unwrap();

    assert!(status.partial);
    assert_eq!(status.segments_checked, vec![first]);
    assert_eq!(status.segment_infos.len(), 1);
}

#[test]
fn do_main_reports_a_clean_index() {
    let tmp = std::env::temp_dir().join(format!("rucene-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let dir: Arc<dyn Directory> = Arc::new(rucene::store::FSDirectory::open(&tmp).unwrap());
    build(Arc::clone(&dir), 2);
    drop(dir);

    let code = rucene::index::check_index::do_main(&[
        tmp.to_string_lossy().to_string(),
        "-level".to_string(),
        "3".to_string(),
    ])
    .unwrap();
    assert_eq!(code, 0);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn upgrader_parse_args_rejects_a_missing_directory() {
    let empty: [String; 0] = [];
    assert!(rucene::index::check_index::IndexUpgrader::parse_args(&empty).is_err());
    assert!(rucene::index::check_index::IndexUpgrader::parse_args(&[
        "-dir-impl".to_string()
    ])
    .is_err());
}
