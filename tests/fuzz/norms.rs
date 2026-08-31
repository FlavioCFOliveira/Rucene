//! Defensive fuzz-style tests for the norms reader.
//!
//! Everything the reader consumes comes straight off disk and is therefore
//! untrusted. Lucene's own decoder answers corruption with an exception —
//! `CorruptIndexException`, `EOFException`, `ArrayIndexOutOfBoundsException` —
//! or, where Java's arithmetic wraps, with a nonsensical value; none of them
//! aborts the JVM. A Rust port must match that: an `Err`, or a wrapped value
//! where Java wraps, but never a panic and never an allocation sized by an
//! attacker-controlled length.
//!
//! These tests take a valid segment, corrupt it systematically, and assert
//! exactly that. The byte sweeps try **every one of the 255 other values** at
//! every position, not a sample: the values that actually break a decoder are
//! the ones that name a length, a count or an offset, and those are spread
//! across the whole range.
//!
//! # Shapes, and why one is not enough
//!
//! Value space is only half of it: a defect can need a particular **shape**.
//! The norms format branches on shape three times over — once in the metadata
//! (`docsWithFieldOffset` is `-2` for a field nobody has, `-1` for a field
//! everybody has, and an offset otherwise), once on the value width
//! (`bytesPerNorm` of 0, 1, 2, 4 or 8, where 0 means the value lives in the
//! metadata and the data file holds none), and once inside `IndexedDISI`, whose
//! block encoding is SPARSE below 4096 documents, DENSE (bitmap plus rank
//! table) above it, and ALL for a full 65536-document block. A fixture with one
//! dense field reaches exactly one of those paths.
//!
//! The sweeps therefore run over four fixtures:
//!
//! * `dense` — one all-documents field, one byte per norm: no docs-with-field
//!   stream at all, so the data file is nothing but packed values and a
//!   corrupt `norms_offset` points straight into them;
//! * `sparse` — one field over a third of the segment, which `IndexedDISI`
//!   writes as shorts;
//! * `mixed` — four fields whose metadata entries **disagree**: an empty one
//!   (`-2`), a constant all-documents one (`bytesPerNorm == 0`, whose
//!   `normsOffset` is a value and not an offset), a sparse one with eight-byte
//!   norms, and a dense one with one-byte norms. This is the only shape where
//!   one field's offset can be made to point into another field's region, and
//!   the only one where a corrupt `bytesPerNorm` reinterprets bytes that were
//!   written with a different width;
//! * `denseblock` — enough documents in one block for `IndexedDISI` to switch
//!   to its bitmap-plus-rank encoding.
//!
//! Whenever a guard is added over a *product* — and this reader multiplies
//! `numDocsWithField` by `bytesPerNorm`, and shifts an ordinal by the width —
//! ask what shape makes its factors disagree in sign.
//!
//! # The metadata checksum, and why it is not the end of the story
//!
//! `.nvm` is read through a checksumming input, so any single-byte corruption
//! of it is ultimately reported as a checksum mismatch. That does **not** make
//! the sweep vacuous: Lucene reads the entries *first* and checks the footer
//! afterwards (`Lucene90NormsProducer.java:59-66`, the `priorE` pattern), so
//! the entry decoder still sees the garbage and still has to survive it. But it
//! does mean a corrupt `.nvm` never reaches `getNorms`.
//!
//! [`hostile_metadata_that_passes_its_checksum_is_survived`] closes that gap: it
//! rewrites one metadata word, **re-signs the footer**, and then reads the
//! segment. That is the only way an offset, a length, a jump-table count, a
//! rank power or a document count reaches the iterators, and it is where a
//! `slice(offset, length)` computed from two untrusted numbers is actually
//! tested.
//!
//! # Shapes these fixtures cannot express
//!
//! * **The `IndexedDISI` ALL encoding.** It needs a full 65536-document block,
//!   which no fixture small enough to sweep exhaustively can contain. The
//!   portability suite covers it for correctness (`disiall`), and the hostile
//!   metadata sweep reaches the same reader entry points, but no byte of an ALL
//!   block is ever flipped here.
//! * **More than one `IndexedDISI` block.** Every fixture fits in block zero,
//!   so a corrupt jump-table entry that points at a *different* block is out of
//!   reach.
//! * **The whole data region of `denseblock`.** Its `.nvd` is around thirteen
//!   kilobytes and every attempt costs a walk over four thousand documents, so
//!   the exhaustive sweep runs over a window at each end of the file — the
//!   index header, the block header, the rank table and the head of the bitmap
//!   at one end; the tail of the packed values, the jump table and the footer
//!   at the other. The middle of the bitmap is pure data: flipping a bit there
//!   adds or removes a document and reaches no branch the ends do not. Its
//!   end-of-stream sentinel block is not swept either, but it is byte for byte
//!   the same shape as the sentinel blocks of `sparse` and `mixed`, whose data
//!   files are swept whole.
//! * **Multi-byte corruption.** Every sweep flips exactly one byte; a
//!   corruption that must be internally consistent across two fields — a length
//!   and the offset that describes it — is only reachable through the hostile
//!   metadata sweep, which patches one word at a time.
//! * **A corrupt `.si`.** `maxDoc` reaches the reader from the segment info,
//!   not from the norms files, so it is trusted here.

use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use rucene::codecs::codec_util;
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::stub::BufferedUpdates;
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::index::{FieldInfo, FieldInfos, IndexOptions, NormValuesWriter, SegmentInfo};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::StringHelper;
use rucene::util::{default_info_stream, Version};

/// The two files a norms segment is made of.
const EXTENSIONS: [&str; 2] = ["nvd", "nvm"];

fn codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

/// Every field name any fixture uses, in a fixed numbering they all share.
const FIELDS: [&str; 4] = ["body", "title", "tags", "extra"];

fn field_infos() -> FieldInfos {
    let mut infos = Vec::new();
    for (number, name) in FIELDS.into_iter().enumerate() {
        let mut info = FieldInfo::new(name, number as i32);
        info.index_options = IndexOptions::DOCS_AND_FREQS;
        infos.push(info);
    }
    FieldInfos::new(infos).expect("field infos")
}

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

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// One fixture: its name, how many documents it spans, and the norms of each
/// field, as `(field number, [(doc, norm)])`.
struct Fixture {
    name: &'static str,
    max_doc: i32,
    fields: Vec<(i32, Vec<(i32, i64)>)>,
}

/// The four shapes every sweep runs over. See the module documentation for what
/// each one exists to express.
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "dense",
            max_doc: 40,
            // Every document, one byte per norm, values that span the signed
            // byte range so the width is genuinely one and not zero.
            fields: vec![(
                0,
                (0..40)
                    .map(|doc| (doc, (doc as i64 * 7 - 120).clamp(-128, 127)))
                    .collect(),
            )],
        },
        Fixture {
            name: "sparse",
            max_doc: 100,
            fields: vec![(
                0,
                (0..100)
                    .filter(|doc| doc % 3 == 0)
                    .map(|doc| (doc, 1 + (doc as i64 % 17)))
                    .collect(),
            )],
        },
        Fixture {
            name: "mixed",
            max_doc: 100,
            fields: vec![
                // Empty: `docsWithFieldOffset == -2`, and the metadata carries
                // `Long.MAX_VALUE` as the singleton nobody reads.
                (0, Vec::new()),
                // Constant over every document: `bytesPerNorm == 0`, so the
                // "offset" in the metadata is really a value.
                (1, (0..100).map(|doc| (doc, 42)).collect()),
                // Sparse with eight-byte norms: the widest ordinal shift.
                (
                    2,
                    (0..100)
                        .filter(|doc| doc % 7 == 1)
                        .map(|doc| (doc, doc as i64 * 0x0123_4567_89AB_CDEF))
                        .collect(),
                ),
                // Dense with one-byte norms, so two fields' data regions sit
                // side by side with different widths.
                (
                    3,
                    (0..100)
                        .map(|doc| (doc, (doc as i64 % 200) - 100))
                        .collect(),
                ),
            ],
        },
        Fixture {
            name: "denseblock",
            max_doc: 4_300,
            // More than the 4095 entries `IndexedDISI` stores as shorts, so the
            // block switches to a bitmap plus a rank table.
            fields: vec![(
                0,
                (0..4_300)
                    .filter(|doc| doc % 43 != 0)
                    .map(|doc| (doc, 1 + (doc as i64 % 11)))
                    .collect(),
            )],
        },
    ]
}

/// Writes one fixture through the real [`NormValuesWriter`] and the real norms
/// consumer.
fn write_segment(directory: &Arc<dyn Directory>, info: &SegmentInfo, fixture: &Fixture) {
    let infos = field_infos();
    let write_state = SegmentWriteState::new(
        default_info_stream(),
        directory.as_ref(),
        info,
        &infos,
        &BufferedUpdates,
        &*DEFAULT_IO_CONTEXT,
    );
    let mut consumer = codec()
        .norms_format()
        .norms_consumer(&write_state)
        .expect("norms consumer");
    let bytes_used = Arc::new(AtomicI64::new(0));
    for (number, values) in &fixture.fields {
        let info = infos
            .field_info_by_number(*number)
            .expect("field info")
            .clone();
        let mut writer = NormValuesWriter::new(info, Arc::clone(&bytes_used));
        for (doc, value) in values {
            writer.add_value(*doc, *value).expect("add value");
        }
        writer.finish(fixture.max_doc);
        writer.flush(consumer.as_mut()).expect("flush");
    }
    consumer.close().expect("close");
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

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

/// Walks every norm of every field, reporting only whether it panicked.
///
/// The reader is expected to answer corruption with an `Err`, or with whatever
/// value Java's wrapping arithmetic would produce; the one outcome this asserts
/// against is an abort.
///
/// Returns how many norms it managed to decode, which the sweeps use to prove
/// that they are actually reaching the decoder rather than being turned away at
/// the file header.
fn visit_all(directory: &Arc<dyn Directory>, info: &SegmentInfo, max_doc: i32) -> usize {
    let infos = field_infos();
    let read_state = SegmentReadState::new(directory.as_ref(), info, &infos, &*DEFAULT_IO_CONTEXT);
    let producer = match codec().norms_format().norms_producer(&read_state) {
        Ok(producer) => producer,
        // Refusing to open a corrupt segment is a perfectly good answer.
        Err(_) => return 0,
    };
    let _ = producer.check_integrity();
    let mut decoded = 0;
    for field in infos.iter() {
        // Every field is asked for twice: state a failed read left behind must
        // not be served to the second one.
        for pass in 0..2 {
            let Ok(mut norms) = producer.get_norms(field) else {
                continue;
            };
            // A corrupt entry can describe a run far longer than the segment,
            // so the walk is bounded by what the segment can hold rather than
            // by what the file claims.
            let mut steps = 0;
            loop {
                match norms.next_doc() {
                    Ok(NO_MORE_DOCS) | Err(_) => break,
                    Ok(_) => {}
                }
                let _ = norms.doc_id();
                if norms.long_value().is_ok() {
                    decoded += 1;
                }
                steps += 1;
                if steps > max_doc as i64 + 16 {
                    break;
                }
            }
            if pass == 1 {
                // The random-access entry points take their own path through
                // the ordinal arithmetic, including the shifts.
                for target in [0, 1, max_doc / 2, max_doc - 1, max_doc, max_doc + 1] {
                    let mut fresh = match producer.get_norms(field) {
                        Ok(fresh) => fresh,
                        Err(_) => continue,
                    };
                    let _ = fresh.advance(target);
                    let _ = fresh.long_value();
                    let _ = fresh.advance_exact(target);
                    let _ = fresh.long_value();
                    let _ = fresh.cost();
                }
            }
        }
    }
    // The merge instance must be no more trusting than the primary one.
    if let Ok(merge) = producer.get_merge_instance() {
        for field in infos.iter() {
            let Ok(mut norms) = merge.get_norms(field) else {
                continue;
            };
            let mut steps = 0;
            loop {
                match norms.next_doc() {
                    Ok(NO_MORE_DOCS) | Err(_) => break,
                    Ok(_) => {}
                }
                let _ = norms.long_value();
                steps += 1;
                if steps > max_doc as i64 + 16 {
                    break;
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

/// Builds a valid segment for `fixture` in a fresh directory.
fn build(fixture: &Fixture) -> (Arc<dyn Directory>, SegmentInfo, [u8; 16]) {
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    let id = StringHelper::random_id();
    let info = segment_info(&directory, fixture.max_doc, id);
    write_segment(&directory, &info, fixture);
    (directory, info, id)
}

/// The positions of `.nvd` a sweep visits.
///
/// Small files are swept whole. `denseblock` is not: its data file is thirteen
/// kilobytes and every attempt costs a walk over four thousand documents, so it
/// is swept over a window at each end, which is where every structural field
/// lives. See the module documentation.
fn data_positions(shape: &str, length: usize) -> Vec<usize> {
    const WINDOW: usize = 320;
    if shape != "denseblock" || length <= WINDOW * 2 {
        return (0..length).collect();
    }
    let mut positions: Vec<usize> = (0..WINDOW).collect();
    positions.extend((length - WINDOW)..length);
    positions
}

// ---------------------------------------------------------------------------
// Sweeps
// ---------------------------------------------------------------------------

#[test]
fn every_value_at_every_byte_of_the_data_file_is_survived() {
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let original = read_file(&directory, "_0.nvd");
        assert!(
            original.len() > 48,
            "[{}] the fixture must produce a real data file, got {} bytes",
            fixture.name,
            original.len()
        );

        let positions = data_positions(fixture.name, original.len());
        let mut survived = 0;
        let mut decoded = 0;
        for at in positions.iter().copied() {
            for value in 0..=u8::MAX {
                if original[at] == value {
                    continue;
                }
                let corrupt = corrupt_copy(&directory, "_0.nvd", at, value);
                let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
                decoded += visit_all(&corrupt, &corrupt_info, fixture.max_doc);
                survived += 1;
            }
        }
        assert_eq!(
            survived,
            positions.len() * 255,
            "[{}] every swept position must have been tried with every other value",
            fixture.name
        );
        assert!(
            decoded > 100,
            "[{}] the sweep must reach the decoder, not be turned away at the \
             header; only {decoded} norms were decoded",
            fixture.name
        );
    }
}

#[test]
fn every_value_at_every_byte_of_the_metadata_file_is_survived() {
    // A corrupt `.nvm` is always caught by its checksum in the end, but only
    // *after* the entry decoder has read the garbage: Lucene checks the footer
    // with the prior exception in hand rather than before parsing. What this
    // sweep proves is that the entry decoder survives every byte, not that the
    // checksum works.
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let original = read_file(&directory, "_0.nvm");
        let mut survived = 0;
        for (at, byte) in original.iter().enumerate() {
            for value in 0..=u8::MAX {
                if *byte == value {
                    continue;
                }
                let corrupt = corrupt_copy(&directory, "_0.nvm", at, value);
                let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
                visit_all(&corrupt, &corrupt_info, fixture.max_doc);
                survived += 1;
            }
        }
        assert_eq!(
            survived,
            original.len() * 255,
            "[{}] every position of the metadata must have been tried with \
             every other value",
            fixture.name
        );
    }
}

#[test]
fn a_truncated_data_file_is_survived_at_every_length() {
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let original = read_file(&directory, "_0.nvd");
        let meta = read_file(&directory, "_0.nvm");
        let step = if original.len() > 1_024 { 7 } else { 1 };
        for length in (0..original.len()).step_by(step) {
            let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            write_file(&corrupt, "_0.nvd", &original[..length]);
            write_file(&corrupt, "_0.nvm", &meta);
            let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
            visit_all(&corrupt, &corrupt_info, fixture.max_doc);
        }
    }
}

#[test]
fn a_truncated_metadata_file_is_survived_at_every_length() {
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let data = read_file(&directory, "_0.nvd");
        let original = read_file(&directory, "_0.nvm");
        for length in 0..original.len() {
            let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            write_file(&corrupt, "_0.nvd", &data);
            write_file(&corrupt, "_0.nvm", &original[..length]);
            let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
            visit_all(&corrupt, &corrupt_info, fixture.max_doc);
        }
    }
}

// ---------------------------------------------------------------------------
// Hostile metadata that survives its own checksum
// ---------------------------------------------------------------------------

/// The layout of one metadata entry, as `Lucene90NormsConsumer.addNormsField`
/// writes it: the field number, then the seven words below.
///
/// Each tuple is `(name, offset within the entry, width in bytes)`. The entry
/// begins immediately after the four-byte field number.
const ENTRY_WORDS: [(&str, usize, usize); 7] = [
    ("docs_with_field_offset", 0, 8),
    ("docs_with_field_length", 8, 8),
    ("jump_table_entry_count", 16, 2),
    ("dense_rank_power", 18, 1),
    ("num_docs_with_field", 19, 4),
    ("bytes_per_norm", 23, 1),
    ("norms_offset", 24, 8),
];

/// One metadata entry is `4 + 8 + 8 + 2 + 1 + 4 + 1 + 8` bytes long.
const ENTRY_LENGTH: usize = 36;

/// The values a hostile writer would choose for a word of the metadata.
///
/// They are the boundaries of the two's-complement ranges rather than a random
/// sample, because that is where a product overflows, a subtraction underflows
/// and a shift leaves its width.
fn hostile_values(width: usize) -> Vec<i64> {
    let mut values = vec![
        0,
        1,
        -1,
        -2,
        -3,
        2,
        4,
        8,
        16,
        127,
        128,
        255,
        256,
        4_095,
        4_096,
        65_535,
        65_536,
        i32::MAX as i64,
        i32::MIN as i64,
    ];
    if width == 8 {
        values.extend([
            i64::MAX,
            i64::MIN,
            i64::MAX / 2,
            i64::MIN / 2,
            1 << 40,
            -(1 << 40),
            (1 << 62) + 1,
        ]);
    }
    values
}

/// Writes `value` into `bytes[at..at + width]` little-endian, the way
/// `DataOutput.writeShort/writeInt/writeLong` do.
fn patch(bytes: &mut [u8], at: usize, width: usize, value: i64) {
    let raw = value.to_le_bytes();
    bytes[at..at + width].copy_from_slice(&raw[..width]);
}

/// Rebuilds a `.nvm` whose footer matches its (patched) body.
///
/// Without this the reader stops at the checksum and `getNorms` is never
/// reached, so every offset, length, count and width in the metadata would go
/// untested against the iterators.
fn resign(body: &[u8]) -> Vec<u8> {
    let directory: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    {
        let mut output = directory
            .create_output("resigned", &*DEFAULT_IO_CONTEXT)
            .expect("create");
        // The footer is a checksum over everything written before it, so the
        // body must go through the same output that computes it.
        output
            .write_bytes(body, 0, body.len())
            .expect("write patched body");
        codec_util::write_footer(output.as_mut()).expect("footer");
        output.close().expect("close");
    }
    read_file(&directory, "resigned")
}

#[test]
fn hostile_metadata_that_passes_its_checksum_is_survived() {
    // This is the sweep that actually reaches `get_norms`. Every metadata word
    // of every field of every shape is replaced with each boundary value, the
    // footer is recomputed so the reader accepts the file, and the segment is
    // then read. An offset, a length, a document count, a jump-table count, a
    // rank power and a value width all arrive at the iterators as attacker
    // input.
    let footer_length = 16;
    let mut attempts = 0;
    let mut opened = 0;
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let original = read_file(&directory, "_0.nvm");
        let data = read_file(&directory, "_0.nvd");
        let body = &original[..original.len() - footer_length];
        // The entries start after the index header and run until the `-1`
        // end-of-metadata marker, so the header length is whatever is left.
        let entries_length = fixture.fields.len() * ENTRY_LENGTH;
        assert!(
            body.len() > entries_length + 4,
            "[{}] the metadata must hold {} entries",
            fixture.name,
            fixture.fields.len()
        );
        let header_length = body.len() - entries_length - 4;

        for entry in 0..fixture.fields.len() {
            let entry_start = header_length + entry * ENTRY_LENGTH + 4;
            for (word, offset, width) in ENTRY_WORDS {
                for value in hostile_values(width) {
                    let mut patched = body.to_vec();
                    patch(&mut patched, entry_start + offset, width, value);
                    let signed = resign(&patched);

                    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
                    write_file(&corrupt, "_0.nvd", &data);
                    write_file(&corrupt, "_0.nvm", &signed);
                    let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
                    // The only assertion is that this returns at all.
                    let decoded = visit_all(&corrupt, &corrupt_info, fixture.max_doc);
                    attempts += 1;
                    if decoded > 0 {
                        opened += 1;
                    }
                    let _ = word;
                }
            }
        }
    }
    assert!(attempts > 1_000, "the sweep must be broad, ran {attempts}");
    assert!(
        opened > 100,
        "the sweep must reach the iterators rather than stop at the entry \
         validation; only {opened} of {attempts} attempts decoded anything"
    );
}

#[test]
fn a_metadata_entry_naming_an_unknown_field_is_refused() {
    // `readFields` looks every field number up in the field infos and refuses a
    // number it does not know, before any offset is used.
    let fixture = &fixtures()[0];
    let (directory, _, id) = build(fixture);
    let original = read_file(&directory, "_0.nvm");
    let data = read_file(&directory, "_0.nvd");
    let body = &original[..original.len() - 16];
    // The metadata is `header || entries || -1`, so the last entry's field
    // number sits one entry plus the four-byte end marker from the end.
    let entry_start = body.len() - 4 - ENTRY_LENGTH;

    for number in [FIELDS.len() as i32, 1_000, i32::MAX, -2, i32::MIN] {
        let mut patched = body.to_vec();
        patch(&mut patched, entry_start, 4, number as i64);
        let signed = resign(&patched);
        let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        write_file(&corrupt, "_0.nvd", &data);
        write_file(&corrupt, "_0.nvm", &signed);
        let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);

        let infos = field_infos();
        let read_state = SegmentReadState::new(
            corrupt.as_ref(),
            &corrupt_info,
            &infos,
            &*DEFAULT_IO_CONTEXT,
        );
        assert!(
            codec().norms_format().norms_producer(&read_state).is_err(),
            "a metadata entry for field number {number} must be refused"
        );
    }
}

#[test]
fn a_field_the_metadata_never_mentions_is_refused_not_answered() {
    // Asking for a field with no entry must be an error, never a silent read of
    // another field's bytes.
    let fixture = &fixtures()[0];
    let (directory, info, _) = build(fixture);
    let infos = field_infos();
    let read_state = SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
    let producer = codec()
        .norms_format()
        .norms_producer(&read_state)
        .expect("norms producer");
    // Only field 0 was written.
    for number in 1..FIELDS.len() as i32 {
        let field = infos.field_info_by_number(number).expect("field info");
        assert!(
            producer.get_norms(field).is_err(),
            "field {number} has no norms entry and must be refused"
        );
    }
}

#[test]
fn a_document_id_outside_the_segment_is_never_answered_with_a_value() {
    for fixture in fixtures() {
        let (directory, info, _) = build(&fixture);
        let infos = field_infos();
        let read_state =
            SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
        let producer = codec()
            .norms_format()
            .norms_producer(&read_state)
            .expect("norms producer");
        for (number, _) in &fixture.fields {
            let field = infos.field_info_by_number(*number).expect("field info");
            for doc in [fixture.max_doc, fixture.max_doc + 1, i32::MAX - 1] {
                let mut norms = producer.get_norms(field).expect("norms");
                // Out of range is `NO_MORE_DOCS` or an error, never a value
                // read from some other document's bytes.
                if let Ok(found) = norms.advance(doc) {
                    assert_eq!(
                        found, NO_MORE_DOCS,
                        "[{}] advance({doc}) past the segment must be exhausted",
                        fixture.name
                    );
                    assert!(
                        norms.long_value().is_err(),
                        "[{}] an exhausted iterator must not answer with a value",
                        fixture.name
                    );
                }
            }
        }
    }
}
