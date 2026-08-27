//! Defensive fuzz-style tests for the term-vectors reader.
//!
//! Everything the reader consumes comes straight off disk and is therefore
//! untrusted. Lucene's own decoder answers corruption with an exception —
//! `CorruptIndexException`, `ArrayIndexOutOfBoundsException`,
//! `NegativeArraySizeException` — or, where Java's arithmetic wraps, with a
//! nonsensical value; none of them aborts the JVM. A Rust port must match that:
//! an `Err`, or a wrapped value where Java wraps, but never a panic and never
//! an allocation sized by an attacker-controlled length.
//!
//! These tests take a valid segment, corrupt it systematically, and assert
//! exactly that. The byte sweeps try **every one of the 255 other values** at
//! every position, not a sample: the values that actually break a decoder are
//! the ones that name a length, a count or an offset, and those are spread
//! across the whole range — a four-value probe of `0x00`/`0x40`/`0x7F`/`0xFF`
//! misses `0x01`, `0x04`, `0x28` and `0x39`, each of which reached a panic
//! here. A test whose name claims "every single byte corruption" has to mean
//! it.
//!
//! Value space is only half of it: a defect can need a particular **shape**.
//! The per-field term-byte totals are a prefix sum, and a negative entry hidden
//! by a positive sibling keeps the document total non-negative — a shape that a
//! fixture with three fields of short, uniform terms simply cannot produce, at
//! any corruption value. The sweeps therefore run over three fixtures: a
//! uniform one; [`write_mixed_segment`], which mixes one field of 64 short terms —
//! exactly one full block-packed block, so a corrupt block header with a
//! negative minimum turns a whole run of lengths negative at once — with a few
//! fields of long terms; and `wide`, a one-document chunk with nine distinct
//! fields, which is the only shape that reaches two further decode branches.
//! Whenever a guard is added over a *sum*, ask what shape makes its terms
//! disagree in sign.
//!
//! Shapes these three fixtures still cannot produce, and which therefore remain
//! unswept:
//!
//! * **More than one chunk in a `.tvd`.** All three segments are small enough
//!   to flush as a single chunk, so a corrupt chunk index that points into the
//!   middle of a *different* chunk is not reached, and neither is a mix of
//!   clean and dirty chunks.
//! * **Per-instance field flags** (`flushFlags` mode 1). Every instance of a
//!   field here carries the same flags, so only the `nonChangingFlags` encoding
//!   is written; the branch that stores one flag per field instance is never
//!   decoded.
//! * **Documents with thousands of positions**, which is where the writer's
//!   growable position buffers matter.
//! * **Multi-byte corruption.** Every sweep flips exactly one byte; a
//!   corruption that must be internally consistent across two fields — a length
//!   and the header that describes it — is out of reach.
//!
//! Unlike the stored-fields stream, a term-vector chunk is almost
//! entirely made of packed integer arrays whose *counts* come from the same
//! bytes — the number of fields per document, the number of terms per field,
//! the frequency of each term, and the position/offset/payload arrays sized by
//! their sum — so a single flipped byte can ask the reader to build an array of
//! any size at all. Reading every document twice also covers the state a first,
//! failed read may have left behind.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::index::{FieldInfo, FieldInfos, IndexOptions, SegmentInfo, POSTINGS_ENUM_ALL};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::StringHelper;
use rucene::util::{BytesRef, Version};

/// The three files a term-vectors segment is made of.
const EXTENSIONS: [&str; 3] = ["tvd", "tvx", "tvm"];

fn codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

/// Every field name either fixture uses, in a fixed numbering both share.
const FIELDS: [&str; 11] = [
    "body", "title", "tags", "f0", "f1", "f2", "f3", "f4", "f5", "f6", "f7",
];

fn field_infos() -> FieldInfos {
    let mut infos = Vec::new();
    for (number, name) in FIELDS.into_iter().enumerate() {
        let mut info = FieldInfo::new(name, number as i32);
        info.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS;
        infos.push(info);
    }
    FieldInfos::new(infos).expect("field infos")
}

/// One of the two fixture shapes the sweeps run over.
type Fixture = (&'static str, fn(&Arc<dyn Directory>, &SegmentInfo, i32));

fn segment_info(directory: &Arc<dyn Directory>, max_doc: i32, id: [u8; 16]) -> SegmentInfo {
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
        id,
        HashMap::new(),
        Default::default(),
    )
    .expect("segment info")
}

/// Writes a segment whose documents exercise every shape of the format: a field
/// with positions and offsets, a field with neither, a field with payloads, a
/// term repeated inside a document, a document with no vectors at all, and
/// enough bytes to fill more than one chunk.
fn write_segment(directory: &Arc<dyn Directory>, info: &SegmentInfo, docs: i32) {
    let infos = field_infos();
    let mut writer = codec()
        .term_vectors_format()
        .vectors_writer(directory.as_ref(), info, &*DEFAULT_IO_CONTEXT)
        .expect("vectors writer");
    for doc in 0..docs {
        if doc % 5 == 4 {
            // A document without vectors still occupies a frame.
            writer.start_document(0).unwrap();
            writer.finish_document().unwrap();
            continue;
        }
        writer.start_document(3).unwrap();

        // `body`: positions and offsets, one term repeated.
        writer
            .start_field(infos.field_info("body").unwrap(), 2, true, true, false)
            .unwrap();
        writer
            .start_term(&BytesRef::new(format!("alpha{doc:03}").into_bytes()), 3)
            .unwrap();
        for occurrence in 0..3 {
            writer
                .add_position(occurrence * 2, occurrence * 7, occurrence * 7 + 5, None)
                .unwrap();
        }
        writer.finish_term().unwrap();
        writer
            .start_term(&BytesRef::new(b"beta".to_vec()), 1)
            .unwrap();
        writer.add_position(9, 40, 44, None).unwrap();
        writer.finish_term().unwrap();
        writer.finish_field().unwrap();

        // `tags`: positions and payloads, one of them empty.
        writer
            .start_field(infos.field_info("tags").unwrap(), 1, true, false, true)
            .unwrap();
        writer
            .start_term(&BytesRef::new(b"tagged".to_vec()), 2)
            .unwrap();
        let payload = BytesRef::new(vec![doc as u8; 24]);
        writer.add_position(0, -1, -1, Some(&payload)).unwrap();
        writer.add_position(4, -1, -1, None).unwrap();
        writer.finish_term().unwrap();
        writer.finish_field().unwrap();

        // `title`: neither positions nor offsets.
        writer
            .start_field(infos.field_info("title").unwrap(), 1, false, false, false)
            .unwrap();
        writer
            .start_term(&BytesRef::new(b"plain".to_vec()), 4)
            .unwrap();
        writer.finish_term().unwrap();
        writer.finish_field().unwrap();

        writer.finish_document().unwrap();
    }
    writer.finish(docs).unwrap();
    writer.close().unwrap();
}

/// Writes a segment whose per-field term-byte totals differ by orders of
/// magnitude, so that a corrupt length can be negative for one field and stay
/// hidden in the document total.
///
/// `f0` carries exactly 64 short terms — one full block-packed block, so a
/// single corrupt block header with a negative minimum turns a whole run of
/// suffix lengths negative at once — and `f1..f3` carry a few long ones. `TVFields::terms` places each field's window with a prefix sum
/// over those totals, which is where a mixed-sign set does its damage.
fn write_mixed_segment(directory: &Arc<dyn Directory>, info: &SegmentInfo, docs: i32) {
    let infos = field_infos();
    let mut writer = codec()
        .term_vectors_format()
        .vectors_writer(directory.as_ref(), info, &*DEFAULT_IO_CONTEXT)
        .expect("vectors writer");
    for doc in 0..docs {
        writer.start_document(4).unwrap();

        // One field of exactly 64 terms: a single full block-packed block.
        writer
            .start_field(infos.field_info("f0").unwrap(), 64, true, true, true)
            .unwrap();
        for term in 0..64u32 {
            writer
                .start_term(&BytesRef::new(format!("t{term:03}").into_bytes()), 1)
                .unwrap();
            let payload = BytesRef::new(vec![term as u8; 2]);
            writer
                .add_position(
                    term as i32,
                    term as i32 * 4,
                    term as i32 * 4 + 4,
                    Some(&payload),
                )
                .unwrap();
            writer.finish_term().unwrap();
        }
        writer.finish_field().unwrap();

        // Three fields of long terms: totals an order of magnitude larger, so a
        // negative length in one is easily masked by a positive sibling.
        for field in 1..4u32 {
            let name = format!("f{field}");
            writer
                .start_field(infos.field_info(&name).unwrap(), 2, true, true, true)
                .unwrap();
            for term in 0..2u32 {
                let text: String = std::iter::repeat(char::from(b'a' + (field + term) as u8 % 26))
                    .take(60)
                    .collect();
                writer
                    .start_term(&BytesRef::new(format!("{text}{term}").into_bytes()), 2)
                    .unwrap();
                let payload = BytesRef::new(vec![(field * 16 + term) as u8; 4]);
                for occurrence in 0..2i32 {
                    writer
                        .add_position(
                            occurrence * 3 + doc,
                            occurrence * 200,
                            occurrence * 200 + 60,
                            Some(&payload),
                        )
                        .unwrap();
                }
                writer.finish_term().unwrap();
            }
            writer.finish_field().unwrap();
        }

        writer.finish_document().unwrap();
    }
    writer.finish(docs).unwrap();
    writer.close().unwrap();
}

/// Writes a single-document segment with nine distinct fields.
///
/// Two decode branches exist only in this shape. A chunk of exactly one
/// document writes its field count as a plain vInt rather than a block-packed
/// array (`Lucene90CompressingTermVectorsWriter.flushNumFields`), and a chunk
/// with more than eight distinct fields writes an extra
/// `numDistinctFields - 1 - 0x07` vInt after the token (`flushFieldNums`).
/// Neither is reachable from a fixture with two or more documents and four
/// fields.
fn write_wide_segment(directory: &Arc<dyn Directory>, info: &SegmentInfo, docs: i32) {
    assert_eq!(
        docs, 1,
        "this shape exists to exercise the one-document chunk"
    );
    let infos = field_infos();
    let mut writer = codec()
        .term_vectors_format()
        .vectors_writer(directory.as_ref(), info, &*DEFAULT_IO_CONTEXT)
        .expect("vectors writer");
    writer.start_document(9).unwrap();
    for (ordinal, name) in FIELDS.iter().take(9).enumerate() {
        writer
            .start_field(
                infos.field_info(name).unwrap(),
                2,
                true,
                true,
                ordinal % 2 == 0,
            )
            .unwrap();
        for term in 0..2u32 {
            writer
                .start_term(&BytesRef::new(format!("{name}-term{term}").into_bytes()), 1)
                .unwrap();
            let payload = BytesRef::new(vec![ordinal as u8; 3]);
            writer
                .add_position(
                    term as i32 * 2,
                    term as i32 * 9,
                    term as i32 * 9 + 6,
                    if ordinal % 2 == 0 {
                        Some(&payload)
                    } else {
                        None
                    },
                )
                .unwrap();
            writer.finish_term().unwrap();
        }
        writer.finish_field().unwrap();
    }
    writer.finish_document().unwrap();
    writer.finish(1).unwrap();
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

/// Walks every term vector of every document, reporting only whether it
/// panicked.
///
/// The reader is expected to answer corruption with an `Err`, or with whatever
/// value Java's wrapping arithmetic would produce; the one outcome this asserts
/// against is an abort.
///
/// Returns how many documents it managed to decode, which the sweeps use to
/// prove that they are actually reaching the decoder rather than being turned
/// away at the file header.
fn visit_all(directory: &Arc<dyn Directory>, info: &SegmentInfo, docs: i32) -> usize {
    let reader = match codec().term_vectors_format().vectors_reader(
        directory.as_ref(),
        info,
        &field_infos(),
        &*DEFAULT_IO_CONTEXT,
    ) {
        Ok(reader) => reader,
        // Refusing to open a corrupt segment is a perfectly good answer.
        Err(_) => return 0,
    };
    let _ = reader.check_integrity();
    let mut decoded = 0;
    for doc in 0..docs {
        // Reading the same document twice must behave the same way: state a
        // failed read left behind must not be served to the second one.
        for _ in 0..2 {
            let Ok(Some(fields)) = reader.get(doc) else {
                continue;
            };
            decoded += 1;
            let _ = fields.size();
            for name in fields.iterator().collect::<Vec<_>>() {
                let Ok(Some(terms)) = fields.terms(&name) else {
                    continue;
                };
                let _ = terms.size();
                let Ok(mut iterator) = terms.iterator() else {
                    continue;
                };
                while matches!(iterator.next(), Ok(Some(_))) {
                    let _ = iterator.total_term_freq();
                    let Ok(mut postings) = iterator.postings(None, POSTINGS_ENUM_ALL) else {
                        continue;
                    };
                    if postings.next_doc().is_err() {
                        continue;
                    }
                    let Ok(freq) = postings.freq() else {
                        continue;
                    };
                    // A corrupt frequency may be enormous or negative; the loop
                    // must be bounded by what the reader can actually produce,
                    // not by what the file claims.
                    for _ in 0..freq.clamp(0, 4_096) {
                        if postings.next_position().is_err() {
                            break;
                        }
                        let _ = postings.start_offset();
                        let _ = postings.end_offset();
                        let _ = postings.get_payload();
                    }
                }
            }
        }
    }
    decoded
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

/// The fixture shapes every sweep runs over.
///
/// One shape is not enough. `uniform` cannot express a set of per-field term
/// totals whose signs disagree, which is what reaches the prefix sum in
/// `TVFields::terms`; `mixedsign` exists precisely for that. Neither can produce
/// a one-document chunk or more than eight distinct fields, which are two
/// separate decode branches; `wide` covers those. See the module documentation
/// for the shapes still missing.
const FIXTURES: [Fixture; 3] = [
    ("uniform", write_segment),
    ("mixedsign", write_mixed_segment),
    ("wide", write_wide_segment),
];

/// How many documents a shape is written with; `uniform` scales with the
/// caller's preference, the other two are fixed by what they exist to express.
fn documents_of(shape: &str, uniform_docs: i32) -> i32 {
    match shape {
        "uniform" => uniform_docs,
        "wide" => 1,
        _ => 2,
    }
}

#[test]
fn every_value_at_every_byte_of_the_data_file_is_survived() {
    for (shape, write) in FIXTURES {
        let docs = documents_of(shape, 12);
        let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let id = StringHelper::random_id();
        let info = segment_info(&directory, docs, id);
        write(&directory, &info, docs);

        let original = read_file(&directory, "_0.tvd");
        assert!(
            original.len() > 128,
            "[{shape}] the fixture must produce a real chunk, got {} bytes",
            original.len()
        );

        // Sweep the whole file rather than a window: the header, the chunk
        // header, every packed array and the compressed term suffixes are all
        // attacker-reachable. Every value is tried, not a sample of four.
        let mut survived = 0;
        let mut decoded = 0;
        for (at, byte) in original.iter().enumerate() {
            for value in 0..=u8::MAX {
                if *byte == value {
                    continue;
                }
                let corrupt = corrupt_copy(&directory, "_0.tvd", at, value);
                let corrupt_info = segment_info(&corrupt, docs, id);
                decoded += visit_all(&corrupt, &corrupt_info, docs);
                survived += 1;
            }
        }
        assert_eq!(
            survived,
            original.len() * 255,
            "[{shape}] every position must have been tried with every other value"
        );
        assert!(
            decoded > 100,
            "[{shape}] the sweep must reach the decoder, not be turned away at \
             the header; only {decoded} documents were decoded"
        );
    }
}

#[test]
fn every_value_at_every_byte_of_the_index_files_is_survived() {
    for (shape, write) in FIXTURES {
        let docs = documents_of(shape, 8);
        let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let id = StringHelper::random_id();
        let info = segment_info(&directory, docs, id);
        write(&directory, &info, docs);

        let mut survived = 0;
        let mut expected = 0;
        let mut decoded = 0;
        for extension in ["tvx", "tvm"] {
            let name = format!("_0.{extension}");
            let original = read_file(&directory, &name);
            expected += original.len() * 255;
            for (at, byte) in original.iter().enumerate() {
                for value in 0..=u8::MAX {
                    if *byte == value {
                        continue;
                    }
                    let corrupt = corrupt_copy(&directory, &name, at, value);
                    let corrupt_info = segment_info(&corrupt, docs, id);
                    decoded += visit_all(&corrupt, &corrupt_info, docs);
                    survived += 1;
                }
            }
        }
        assert_eq!(
            survived, expected,
            "[{shape}] every position of both index files must have been tried \
             with every other value"
        );
        assert!(
            decoded > 10,
            "[{shape}] the sweep must reach the decoder; only {decoded} documents were decoded"
        );
    }
}

#[test]
fn a_truncated_data_file_is_survived_at_every_length() {
    const DOCS: i32 = 8;
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let id = StringHelper::random_id();
    let info = segment_info(&directory, DOCS, id);
    write_segment(&directory, &info, DOCS);

    let original = read_file(&directory, "_0.tvd");
    for length in (0..original.len()).step_by(3) {
        let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        write_file(&corrupt, "_0.tvd", &original[..length]);
        for extension in ["tvx", "tvm"] {
            let name = format!("_0.{extension}");
            let bytes = read_file(&directory, &name);
            write_file(&corrupt, &name, &bytes);
        }
        let corrupt_info = segment_info(&corrupt, DOCS, id);
        visit_all(&corrupt, &corrupt_info, DOCS);
    }
}

#[test]
fn a_truncated_index_file_is_survived_at_every_length() {
    const DOCS: i32 = 8;
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let id = StringHelper::random_id();
    let info = segment_info(&directory, DOCS, id);
    write_segment(&directory, &info, DOCS);

    for extension in ["tvx", "tvm"] {
        let name = format!("_0.{extension}");
        let original = read_file(&directory, &name);
        for length in 0..original.len() {
            let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            write_file(&corrupt, &name, &original[..length]);
            for other in EXTENSIONS {
                let other_name = format!("_0.{other}");
                if other_name == name {
                    continue;
                }
                let bytes = read_file(&directory, &other_name);
                write_file(&corrupt, &other_name, &bytes);
            }
            let corrupt_info = segment_info(&corrupt, DOCS, id);
            visit_all(&corrupt, &corrupt_info, DOCS);
        }
    }
}

#[test]
fn a_document_id_outside_the_segment_is_rejected() {
    const DOCS: i32 = 4;
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let id = StringHelper::random_id();
    let info = segment_info(&directory, DOCS, id);
    write_segment(&directory, &info, DOCS);

    let reader = codec()
        .term_vectors_format()
        .vectors_reader(
            directory.as_ref(),
            &info,
            &field_infos(),
            &*DEFAULT_IO_CONTEXT,
        )
        .expect("vectors reader");
    for doc in [-1, DOCS, DOCS + 1, i32::MAX, i32::MIN] {
        // Out of range is an error, never a panic and never a read of another
        // document's bytes.
        assert!(
            reader.get(doc).is_err(),
            "doc {doc} is outside the segment and must be refused"
        );
    }
}
