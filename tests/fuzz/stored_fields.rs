//! Defensive fuzz-style tests for the stored-fields reader.
//!
//! Everything the reader consumes comes straight off disk and is therefore
//! untrusted. Lucene's own decoders answer corruption with an exception —
//! `CorruptIndexException`, `ArrayIndexOutOfBoundsException`,
//! `ArithmeticException` — or, where Java's arithmetic wraps, with a
//! nonsensical value; none of them abort the JVM. A Rust port must match that:
//! an `Err`, or a wrapped value where Java wraps, but never a panic.
//!
//! These tests take a valid segment, corrupt it systematically, and assert
//! exactly that. They are the regression net for a family of defects — an
//! unsigned subtraction in the LZ4 match decoder, an unsigned subtraction in
//! the small-integer float form, an overflowing multiply in the `TLong`
//! scaling step, an overflowing add in the chunk-header bounds check and a
//! negative length from non-monotonic chunk offsets — each of which panicked in
//! a debug build on bytes Lucene itself reads without incident.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::index::{
    FieldInfo, FieldInfos, SegmentInfo, StoredFieldVisitor, StoredFieldVisitorStatus,
};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::StringHelper;
use rucene::util::Version;

/// The three files a stored-fields segment is made of.
const EXTENSIONS: [&str; 3] = ["fdt", "fdx", "fdm"];

fn codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

fn field_infos() -> FieldInfos {
    FieldInfos::new(vec![
        FieldInfo::new("text", 0),
        FieldInfo::new("number", 1),
        FieldInfo::new("blob", 2),
    ])
    .expect("field infos")
}

fn segment_info(directory: &Arc<dyn Directory>, max_doc: i32) -> SegmentInfo {
    SegmentInfo::new(
        Arc::clone(directory),
        Version::LATEST,
        Some(Version::LATEST),
        "_0".to_string(),
        max_doc,
        false,
        false,
        codec(),
        HashMap::new(),
        StringHelper::random_id(),
        HashMap::new(),
        Default::default(),
    )
    .expect("segment info")
}

/// A visitor that accepts everything and keeps nothing.
#[derive(Debug, Default)]
struct DrainingVisitor;

impl StoredFieldVisitor for DrainingVisitor {
    fn needs_field(
        &mut self,
        _info: &FieldInfo,
    ) -> rucene::error::Result<StoredFieldVisitorStatus> {
        Ok(StoredFieldVisitorStatus::Yes)
    }
}

/// Writes a segment whose documents exercise every stored type, including the
/// `TLong` scaling branches and a compressible binary payload.
fn write_segment(directory: &Arc<dyn Directory>, info: &SegmentInfo, docs: i32) {
    let infos = field_infos();
    let text = infos.field_info("text").unwrap();
    let number = infos.field_info("number").unwrap();
    let blob = infos.field_info("blob").unwrap();
    let mut writer = codec()
        .stored_fields_format()
        .fields_writer(directory.as_ref(), info, &*DEFAULT_IO_CONTEXT)
        .expect("fields writer");
    for doc in 0..docs {
        writer.start_document().unwrap();
        writer
            .write_field_string(
                text,
                &format!("document number {doc} with some repeated text"),
            )
            .unwrap();
        // Day, hour and second precision plus a plain value, so every `TLong`
        // branch appears in the stream.
        writer
            .write_field_i64(number, i64::from(doc) * 86_400_000)
            .unwrap();
        writer
            .write_field_i64(number, i64::from(doc) * 3_600_000 + 1)
            .unwrap();
        writer.write_field_f32(number, -1.0).unwrap();
        writer.write_field_f64(number, -1.0).unwrap();
        writer
            .write_field_bytes(blob, &(0..64u8).map(|b| b ^ doc as u8).collect::<Vec<_>>())
            .unwrap();
        writer.finish_document().unwrap();
    }
    writer.finish(docs).unwrap();
    writer.close().unwrap();
}

fn read_file(directory: &Arc<dyn Directory>, name: &str) -> Vec<u8> {
    let mut input = directory
        .open_input(name, &*DEFAULT_IO_CONTEXT)
        .expect("open");
    let length = input.length() as usize;
    let mut bytes = vec![0u8; length];
    input.read_bytes(&mut bytes, 0, length).expect("read");
    bytes
}

fn write_file(directory: &Arc<dyn Directory>, name: &str, bytes: &[u8]) {
    let mut output = directory
        .create_output(name, &*DEFAULT_IO_CONTEXT)
        .expect("create");
    output.write_bytes(bytes, 0, bytes.len()).expect("write");
    output.close().expect("close");
}

/// Reads every document of a segment, reporting only whether it panicked.
///
/// The reader is expected to answer corruption with an `Err`, or with whatever
/// value Java's wrapping arithmetic would produce; the one outcome this asserts
/// against is an abort.
fn visit_all(directory: &Arc<dyn Directory>, info: &SegmentInfo, docs: i32) {
    let reader = match codec().stored_fields_format().fields_reader(
        directory.as_ref(),
        info,
        &field_infos(),
        &*DEFAULT_IO_CONTEXT,
    ) {
        Ok(reader) => reader,
        // Refusing to open a corrupt segment is a perfectly good answer.
        Err(_) => return,
    };
    let _ = reader.check_integrity();
    for doc in 0..docs {
        let mut visitor = DrainingVisitor;
        let _ = reader.document(doc, &mut visitor);
        // Reading the same document twice must behave the same way: a block
        // condemned by a failed reset must not be served from cached state.
        let mut again = DrainingVisitor;
        let _ = reader.document(doc, &mut again);
    }
}

/// Builds a fresh directory holding a corrupted copy of a valid segment.
fn corrupt_copy(
    original: &Arc<dyn Directory>,
    file: &str,
    at: usize,
    value: u8,
) -> Arc<dyn Directory> {
    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    for extension in EXTENSIONS {
        let name = format!("_0.{extension}");
        let mut bytes = read_file(original, &name);
        if name == file {
            bytes[at] = value;
        }
        write_file(&corrupt, &name, &bytes);
    }
    corrupt
}

#[test]
fn every_single_byte_corruption_of_the_data_file_is_survived() {
    const DOCS: i32 = 12;
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let info = segment_info(&directory, DOCS);
    write_segment(&directory, &info, DOCS);

    let original = read_file(&directory, "_0.fdt");
    assert!(
        original.len() > 128,
        "the fixture must produce a real chunk"
    );

    // Sweep the whole file rather than a window: the header, the chunk header,
    // the packed `numStoredFields`/`lengths` arrays, the compressed payload and
    // the footer are all attacker-reachable.
    let mut survived = 0;
    for (at, byte) in original.iter().enumerate() {
        for value in [0x00u8, 0x40, 0x7F, 0xFF] {
            if *byte == value {
                continue;
            }
            let corrupt = corrupt_copy(&directory, "_0.fdt", at, value);
            let corrupt_info = SegmentInfo::new(
                Arc::clone(&corrupt),
                Version::LATEST,
                Some(Version::LATEST),
                "_0".to_string(),
                DOCS,
                false,
                false,
                codec(),
                HashMap::new(),
                info.id(),
                HashMap::new(),
                Default::default(),
            )
            .expect("segment info");
            corrupt_info.put_attribute(
                "Lucene90StoredFieldsFormat.mode".to_string(),
                "BEST_SPEED".to_string(),
            );
            visit_all(&corrupt, &corrupt_info, DOCS);
            survived += 1;
        }
    }
    assert!(
        survived > 1_000,
        "the sweep must actually exercise the reader, got {survived} cases"
    );
}

#[test]
fn every_single_byte_corruption_of_the_index_files_is_survived() {
    const DOCS: i32 = 8;
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let info = segment_info(&directory, DOCS);
    write_segment(&directory, &info, DOCS);

    let mut survived = 0;
    for extension in ["fdx", "fdm"] {
        let name = format!("_0.{extension}");
        let original = read_file(&directory, &name);
        for (at, byte) in original.iter().enumerate() {
            for value in [0x00u8, 0xFF] {
                if *byte == value {
                    continue;
                }
                let corrupt = corrupt_copy(&directory, &name, at, value);
                let corrupt_info = SegmentInfo::new(
                    Arc::clone(&corrupt),
                    Version::LATEST,
                    Some(Version::LATEST),
                    "_0".to_string(),
                    DOCS,
                    false,
                    false,
                    codec(),
                    HashMap::new(),
                    info.id(),
                    HashMap::new(),
                    Default::default(),
                )
                .expect("segment info");
                corrupt_info.put_attribute(
                    "Lucene90StoredFieldsFormat.mode".to_string(),
                    "BEST_SPEED".to_string(),
                );
                visit_all(&corrupt, &corrupt_info, DOCS);
                survived += 1;
            }
        }
    }
    assert!(
        survived > 100,
        "the sweep must actually exercise the reader, got {survived} cases"
    );
}

#[test]
fn a_truncated_data_file_is_survived_at_every_length() {
    const DOCS: i32 = 8;
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let info = segment_info(&directory, DOCS);
    write_segment(&directory, &info, DOCS);

    let original = read_file(&directory, "_0.fdt");
    for length in (0..original.len()).step_by(3) {
        let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        write_file(&corrupt, "_0.fdt", &original[..length]);
        for extension in ["fdx", "fdm"] {
            let name = format!("_0.{extension}");
            let bytes = read_file(&directory, &name);
            write_file(&corrupt, &name, &bytes);
        }
        let corrupt_info = SegmentInfo::new(
            Arc::clone(&corrupt),
            Version::LATEST,
            Some(Version::LATEST),
            "_0".to_string(),
            DOCS,
            false,
            false,
            codec(),
            HashMap::new(),
            info.id(),
            HashMap::new(),
            Default::default(),
        )
        .expect("segment info");
        corrupt_info.put_attribute(
            "Lucene90StoredFieldsFormat.mode".to_string(),
            "BEST_SPEED".to_string(),
        );
        visit_all(&corrupt, &corrupt_info, DOCS);
    }
}

#[test]
fn a_valid_segment_still_reads_after_all_that() {
    // Guards the sweeps above against becoming vacuous: if the fixture stopped
    // producing a readable segment, every corruption would be "survived" by
    // failing to open, and the tests would prove nothing.
    const DOCS: i32 = 8;
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let info = segment_info(&directory, DOCS);
    write_segment(&directory, &info, DOCS);

    #[derive(Debug, Default)]
    struct Counting {
        values: usize,
    }
    impl StoredFieldVisitor for Counting {
        fn binary_field(&mut self, _i: &FieldInfo, _v: &[u8]) -> rucene::error::Result<()> {
            self.values += 1;
            Ok(())
        }
        fn string_field(&mut self, _i: &FieldInfo, _v: &str) -> rucene::error::Result<()> {
            self.values += 1;
            Ok(())
        }
        fn long_field(&mut self, _i: &FieldInfo, _v: i64) -> rucene::error::Result<()> {
            self.values += 1;
            Ok(())
        }
        fn float_field(&mut self, _i: &FieldInfo, v: f32) -> rucene::error::Result<()> {
            assert_eq!(v, -1.0, "the small-integer float form must decode");
            self.values += 1;
            Ok(())
        }
        fn double_field(&mut self, _i: &FieldInfo, v: f64) -> rucene::error::Result<()> {
            assert_eq!(v, -1.0, "the small-integer double form must decode");
            self.values += 1;
            Ok(())
        }
        fn needs_field(
            &mut self,
            _info: &FieldInfo,
        ) -> rucene::error::Result<StoredFieldVisitorStatus> {
            Ok(StoredFieldVisitorStatus::Yes)
        }
    }

    let reader = codec()
        .stored_fields_format()
        .fields_reader(
            directory.as_ref(),
            &info,
            &field_infos(),
            &*DEFAULT_IO_CONTEXT,
        )
        .expect("fields reader");
    reader.check_integrity().expect("integrity");
    for doc in 0..DOCS {
        let mut visitor = Counting::default();
        reader.document(doc, &mut visitor).expect("document");
        assert_eq!(visitor.values, 6, "doc {doc}");
    }
}
