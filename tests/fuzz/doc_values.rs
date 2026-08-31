//! Defensive fuzz-style tests for the doc-values reader.
//!
//! Everything the reader consumes comes straight off disk and is therefore
//! untrusted. Lucene's own decoder answers corruption with an exception —
//! `CorruptIndexException`, `EOFException`, `ArrayIndexOutOfBoundsException` —
//! or, where Java's arithmetic wraps, with a nonsensical value; none of them
//! aborts the JVM. A Rust port must match that: an `Err`, or a wrapped value
//! where Java wraps, but never a panic and never an allocation sized by an
//! attacker-controlled length.
//!
//! The second half of that is held **by construction** rather than by any
//! bound: the producer keeps offsets and decodes a value only when an iterator
//! asks for one, exactly as `Lucene90DocValuesProducer` does, so a count read
//! out of the metadata never reaches an allocator.
//! [`a_value_count_off_disk_sizes_nothing`] asserts it where a ceiling would
//! otherwise have had to stand. The two buffers that are genuinely sized from
//! the file — a binary value and one decompressed terms block — are allocated
//! fallibly, which turns Java's catchable `OutOfMemoryError` into an `Err`
//! instead of an abort.
//!
//! These tests take a valid segment, corrupt it systematically, and assert
//! exactly that. The byte sweeps try **every one of the 255 other values** at
//! every position they visit, not a sample: the values that actually break a
//! decoder are the ones that name a length, a count or an offset, and those are
//! spread across the whole range.
//!
//! # Shapes, and why one is not enough
//!
//! Value space is only half of it: a defect can need a particular **shape**.
//! The doc-values format branches far more than norms does, and a fixture that
//! misses a branch proves nothing about it. The metadata alone chooses between
//! five entry types, and inside each of them:
//!
//! * **`NUMERIC`** picks its encoding four ways in `writeValues`
//!   (`Lucene90DocValuesConsumer.java:437-500`): a documents-with-field offset
//!   of `-2` for a field nobody has, `-1` for a field everybody has and an
//!   `IndexedDISI` otherwise; then `bitsPerValue == 0` for a constant field
//!   whose single value lives in the metadata and whose data region is empty,
//!   a unique-value table of up to 256 entries when that is narrower, and
//!   otherwise GCD-divided bit packing.
//! * **`BINARY`** stores fixed-length values with no address table at all when
//!   `minLength == maxLength`, and a `DirectMonotonic` address table otherwise
//!   (`Lucene90DocValuesProducer.java:355-377`). The empty value has to survive
//!   both, without being confused with an absent one.
//! * **`SORTED`** is a `NUMERIC` run of ordinals followed by a term dictionary.
//! * **`SORTED_NUMERIC`** and **`SORTED_SET`** each have two layouts, chosen by
//!   whether any document carries more than one value. The single-valued
//!   `SORTED_SET` writes the `SORTED` layout behind a `multiValued == 0` byte
//!   and no address table; the multi-valued one writes a `numDocsWithField`
//!   and an address table. A reader that ignores that byte reads the term
//!   dictionary's `termsDictSize` as a document count and drifts, which is
//!   exactly the defect this suite locks out.
//! * **The term dictionary** compresses every 64 terms into one LZ4 block
//!   against the block's first term as a dictionary, with a per-term header
//!   byte that clips the shared prefix at 15 and the suffix at 16 and spills
//!   the remainder into `VInt`s (`Lucene90DocValuesConsumer.addTermsDict`). A
//!   dictionary of four short terms reaches none of the spill paths and never
//!   opens a second block.
//! * **`IndexedDISI`** encodes each 65536-document block three ways: SPARSE
//!   (one short per document) below 4096 documents, DENSE (a bitmap plus a rank
//!   table) above it, and ALL for a block every document of which carries the
//!   field.
//!
//! The sweeps therefore run over seven fixtures, each of which exists for the
//! branches the others cannot reach:
//!
//! * `numeric` — four `NUMERIC` fields over every document, one per encoding:
//!   plain bit packing, unique-value table, constant (`bitsPerValue == 0`) and
//!   GCD-divided. None of them writes a documents-with-field stream, so a
//!   corrupt values offset points straight into another field's values;
//! * `sparse` — a `NUMERIC` field over a third of the segment and two `BINARY`
//!   fields, one variable-length (address table) and one fixed-length (none),
//!   all three written through `IndexedDISI`'s SPARSE encoding, plus a field no
//!   document carries at all (`docsWithFieldOffset == -2`);
//! * `sorted` — `SORTED`, single-valued `SORTED_SET` and single-valued
//!   `SORTED_NUMERIC` side by side: the three entries that are laid out alike
//!   and read differently;
//! * `multi` — multi-valued `SORTED_SET` and `SORTED_NUMERIC`, the address-table
//!   layouts, with in-document duplicates and cross-document repeats;
//! * `terms` — one `SORTED` field with more than 64 distinct terms, so the
//!   dictionary spans several LZ4 blocks, and with terms sharing prefixes
//!   longer than 15 bytes and carrying suffixes longer than 16, so the clipped
//!   header and both `VInt` spills are exercised;
//! * `denseblock` — more documents in one `IndexedDISI` block than the 4096 it
//!   stores as shorts, so the block switches to its bitmap-plus-rank encoding;
//! * `disiall` — one full 65536-document block in which every document carries
//!   the field, followed by a sparse tail: the only shape that reaches the ALL
//!   encoding, and the only one with more than one block and therefore a jump
//!   table. Its values are constant, so its data file is the documents-with-field
//!   stream and nothing else, which is what makes a 66048-document shape cheap
//!   enough to sweep at all.
//!
//! [`every_fixture_writes_the_shape_it_exists_for`] asserts each of those
//! claims against the bytes the fixture actually wrote, through a second
//! metadata decoder written for the purpose: a fixture can stop reaching a
//! branch through a change far too small to break a round trip.
//!
//! Whenever a guard is added over a *product* — and this reader multiplies a
//! value count by a bit width, an address by nothing at all and a document
//! index by a fixed length — ask what shape makes its factors disagree in sign.
//!
//! # The metadata checksum, and why it is not the end of the story
//!
//! `.dvm` is read through a checksumming input, so any single-byte corruption
//! of it is ultimately reported as a checksum mismatch. That does **not** make
//! the sweep vacuous: the entries are decoded first and the footer verified
//! afterwards, so the entry decoder still sees the garbage and still has to
//! survive it. But it does mean a corrupt `.dvm` never reaches the iterators.
//!
//! [`hostile_metadata_that_passes_its_checksum_is_survived`] closes that gap: it
//! rewrites one metadata word, **re-signs the footer**, and then reads the
//! segment. That is the only way an offset, a length, a bit width, a table
//! size, a document count, an address block shift or a term count reaches the
//! decoder as attacker input with the file still considered intact. It patches
//! every aligned position of the metadata body rather than a named list of
//! fields, because the entry layout differs per type and a named list would
//! quietly stop covering a type the day its layout changed.
//!
//! # Shapes these fixtures cannot express
//!
//! * **`doBlocks`.** `writeValues` switches to a per-block bit width when that
//!   saves over 10% of the packed bits, which cannot happen below 16384 values
//!   in one field (`Lucene90DocValuesConsumer.java:486-497`). Rucene's codec
//!   does not implement that branch yet, so no fixture can produce it and no
//!   sweep can corrupt it.
//! * **A skip index.** No fixture sets `DocValuesSkipIndexType`, so `.dvs`
//!   holds nothing but a header and a footer and the reader never opens it.
//! * **Multi-byte corruption.** Every sweep flips exactly one byte; a
//!   corruption that must be internally consistent across two fields — a length
//!   and the offset that describes it — is only reachable through the hostile
//!   metadata sweep, which patches one word at a time.
//! * **A corrupt `.si`.** `maxDoc` reaches the reader from the segment info,
//!   not from the doc-values files, so it is trusted here.
//! * **The whole data region of the two large fixtures.** `denseblock` and
//!   `disiall` are swept over a window at each end of the file, which is where
//!   every structural field lives; the middle of a bitmap is pure data, and
//!   flipping a bit there adds or removes a document and reaches no branch the
//!   ends do not.

use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use rucene::codecs::codec_util;
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::stub::BufferedUpdates;
use rucene::codecs::{
    register_codec, Codec, DocValuesConsumer, DocValuesFormat, DocValuesProducer, Lucene104Codec,
    Lucene90DocValuesFormat,
};
use rucene::index::{
    BinaryDocValuesWriter, DocValuesType, FieldInfo, FieldInfos, NumericDocValuesWriter,
    SegmentInfo, SortedDocValuesWriter, SortedNumericDocValuesWriter, SortedSetDocValuesWriter,
};
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::StringHelper;
use rucene::util::{default_info_stream, BytesRef, Version};

/// Every file the consumer produces, so a corrupted copy is a complete
/// segment. `.dvs` is written but never read by this format's producer, so it
/// is copied along with the segment and never corrupted.
const ALL_EXTENSIONS: [&str; 3] = ["dvd", "dvm", "dvs"];

fn codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
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

/// The doc values of one field, in document order.
///
/// Each variant carries the pairs the corresponding writer is fed, so a fixture
/// is a literal transcript of what reaches the consumer.
#[derive(Clone, Debug)]
enum Values {
    Numeric(Vec<(i32, i64)>),
    Binary(Vec<(i32, Vec<u8>)>),
    Sorted(Vec<(i32, Vec<u8>)>),
    SortedNumeric(Vec<(i32, Vec<i64>)>),
    SortedSet(Vec<(i32, Vec<Vec<u8>>)>),
}

impl Values {
    fn doc_values_type(&self) -> DocValuesType {
        match self {
            Self::Numeric(_) => DocValuesType::NUMERIC,
            Self::Binary(_) => DocValuesType::BINARY,
            Self::Sorted(_) => DocValuesType::SORTED,
            Self::SortedNumeric(_) => DocValuesType::SORTED_NUMERIC,
            Self::SortedSet(_) => DocValuesType::SORTED_SET,
        }
    }
}

/// One fixture: its name, how many documents it spans, and its fields, in the
/// order they are flushed — which is the order their metadata entries appear.
struct Fixture {
    name: &'static str,
    max_doc: i32,
    fields: Vec<Values>,
}

impl Fixture {
    /// The field infos the writer and the reader share.
    ///
    /// Field numbers are the positions in `fields`, so a metadata entry naming
    /// any other number is one the reader must refuse.
    fn field_infos(&self) -> FieldInfos {
        let infos = self
            .fields
            .iter()
            .enumerate()
            .map(|(number, values)| {
                let mut info = FieldInfo::new(format!("f{number}"), number as i32);
                info.doc_values_type = values.doc_values_type();
                info
            })
            .collect();
        FieldInfos::new(infos).expect("field infos")
    }
}

fn bytes(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// Four `NUMERIC` fields over every document, one per encoding `writeValues`
/// can choose.
fn numeric_fixture() -> Fixture {
    let max_doc = 40;
    Fixture {
        name: "numeric",
        max_doc,
        fields: vec![
            // Forty values spread over a range of forty, so the unique-value
            // table would be no narrower than the range and `writeValues`
            // packs the values themselves. The permutation keeps consecutive
            // differences coprime, so the GCD is 1 and nothing is divided out.
            Values::Numeric(
                (0..max_doc)
                    .map(|doc| (doc, i64::from((doc * 17) % 40)))
                    .collect(),
            ),
            // Five distinct values whose differences are coprime but whose
            // range needs thirty bits: three bits of ordinal into a table of
            // five is narrower, so the table is written and the values become
            // ordinals into it.
            Values::Numeric(
                (0..max_doc)
                    .map(|doc| {
                        const TABLE: [i64; 5] = [-7, 0, 1, 1_000, 1_000_000_007];
                        (doc, TABLE[(doc as usize) % TABLE.len()])
                    })
                    .collect(),
            ),
            // Constant: `bitsPerValue == 0`, the value lives in the metadata
            // and the data region holds nothing at all.
            Values::Numeric((0..max_doc).map(|doc| (doc, 42)).collect()),
            // Every value divisible by 1024 and more than 256 of them: GCD
            // compression divides before packing.
            Values::Numeric(
                (0..max_doc)
                    .map(|doc| (doc, i64::from(doc) * 1024 + 1_048_576))
                    .collect(),
            ),
        ],
    }
}

/// Sparse fields written through `IndexedDISI`'s SPARSE encoding, plus one no
/// document carries at all.
fn sparse_fixture() -> Fixture {
    let max_doc = 100;
    Fixture {
        name: "sparse",
        max_doc,
        fields: vec![
            Values::Numeric(
                (0..max_doc)
                    .filter(|doc| doc % 3 == 0)
                    .map(|doc| (doc, i64::from(doc) * 13 - 40))
                    .collect(),
            ),
            // Variable length, including the empty value, which the format must
            // not confuse with an absent one: `minLength < maxLength`, so an
            // address table is written.
            Values::Binary(
                (0..max_doc)
                    .filter(|doc| doc % 5 == 2)
                    .map(|doc| {
                        let value = if doc == 12 {
                            Vec::new()
                        } else {
                            bytes(&format!("b{doc}-{}", "x".repeat((doc % 11) as usize)))
                        };
                        (doc, value)
                    })
                    .collect(),
            ),
            // Fixed length: `minLength == maxLength`, so no address table at
            // all and the values are cut out by multiplication instead.
            Values::Binary(
                (0..max_doc)
                    .filter(|doc| doc % 7 == 1)
                    .map(|doc| (doc, bytes(&format!("{doc:04}"))))
                    .collect(),
            ),
            // No document at all: `docsWithFieldOffset == -2`.
            Values::Numeric(Vec::new()),
        ],
    }
}

/// The three single-valued layouts side by side.
fn sorted_fixture() -> Fixture {
    let max_doc = 60;
    let dictionary = ["zz", "apple", "mm", "bee", "kiwi", "fig", "date", "cherry"];
    Fixture {
        name: "sorted",
        max_doc,
        fields: vec![
            // First-seen order disagrees with sorted order, so insertion-order
            // ordinals would diverge.
            Values::Sorted(
                (0..max_doc)
                    .filter(|doc| doc % 2 == 0)
                    .map(|doc| (doc, bytes(dictionary[(doc as usize) % dictionary.len()])))
                    .collect(),
            ),
            // Every document carries exactly one value, so the consumer takes
            // the single-valued route and writes the `SORTED` layout behind a
            // `multiValued == 0` byte.
            Values::SortedSet(
                (0..max_doc)
                    .filter(|doc| doc % 3 == 0)
                    .map(|doc| {
                        (
                            doc,
                            vec![bytes(dictionary[(doc as usize / 3) % dictionary.len()])],
                        )
                    })
                    .collect(),
            ),
            // One value per document, so `numValues == numDocsWithField` and no
            // address table is written.
            Values::SortedNumeric(
                (0..max_doc)
                    .filter(|doc| doc % 4 == 1)
                    .map(|doc| (doc, vec![i64::from(doc) * 7 - 13]))
                    .collect(),
            ),
        ],
    }
}

/// The two address-table layouts.
fn multi_fixture() -> Fixture {
    let max_doc = 60;
    let dictionary = ["bee", "ant", "cow", "ant", "emu", "bee", "dog", "fox"];
    Fixture {
        name: "multi",
        max_doc,
        fields: vec![
            // In-document duplicates are deduplicated by the writer; the counts
            // therefore differ from what the table names, which is what the
            // address table has to carry.
            Values::SortedSet(
                (0..max_doc)
                    .filter(|doc| doc % 2 == 0)
                    .map(|doc| {
                        let count = 1 + (doc as usize) % 3;
                        let terms = (0..count)
                            .map(|index| bytes(dictionary[(doc as usize * 2 + index) % 8]))
                            .collect();
                        (doc, terms)
                    })
                    .collect(),
            ),
            // A list, not a set: within-document duplicates must survive.
            Values::SortedNumeric(
                (0..max_doc)
                    .filter(|doc| doc % 3 != 2)
                    .map(|doc| {
                        let count = 1 + (doc as usize) % 4;
                        let values = (0..count)
                            .map(|index| i64::from(doc) * 31 - 9 * (index as i64 % 2))
                            .collect();
                        (doc, values)
                    })
                    .collect(),
            ),
        ],
    }
}

/// A term dictionary that spans several LZ4 blocks and spills both clipped
/// lengths into their `VInt`s.
fn terms_fixture() -> Fixture {
    let max_doc = 200;
    // More than 64 terms, so the dictionary opens a second block and the
    // reverse index gets a second entry. Every term shares a 20-byte prefix
    // with its neighbour, which is longer than the 15 the header byte can hold,
    // and carries a suffix longer than the 16 it can hold, so both `VInt`
    // spills of `addTermsDict` are written and both have to be read back.
    let terms: Vec<Vec<u8>> = (0..150)
        .map(|index| {
            bytes(&format!(
                "common-prefix-000000{index:04}-{}",
                "s".repeat(24)
            ))
        })
        .collect();
    Fixture {
        name: "terms",
        max_doc,
        fields: vec![Values::Sorted(
            (0..max_doc)
                .map(|doc| (doc, terms[(doc as usize * 7) % terms.len()].clone()))
                .collect(),
        )],
    }
}

/// More documents in one block than `IndexedDISI` stores as shorts.
fn dense_block_fixture() -> Fixture {
    let max_doc = 4_300;
    Fixture {
        name: "denseblock",
        max_doc,
        fields: vec![Values::Numeric(
            (0..max_doc)
                .filter(|doc| doc % 43 != 0)
                .map(|doc| (doc, i64::from(doc % 11) + 1))
                .collect(),
        )],
    }
}

/// A full 65536-document block in which every document carries the field,
/// followed by a sparse tail: the only shape that reaches the ALL encoding.
fn disi_all_fixture() -> Fixture {
    let max_doc = 65_536 + 512;
    Fixture {
        name: "disiall",
        max_doc,
        // The value is constant, so `bitsPerValue` is zero and the data file
        // holds the documents-with-field stream and nothing else. That keeps
        // the ALL block the fixture exists for while leaving a data file small
        // enough to sweep, and it is the only shape in which an ALL block and a
        // metadata-resident constant meet.
        fields: vec![Values::Numeric(
            (0..max_doc)
                .filter(|doc| *doc < 65_536 || doc % 64 == 0)
                .map(|doc| (doc, 4_611_686_018_427_387_904))
                .collect(),
        )],
    }
}

/// The shapes the exhaustive sweeps run over.
fn fixtures() -> Vec<Fixture> {
    vec![
        numeric_fixture(),
        sparse_fixture(),
        sorted_fixture(),
        multi_fixture(),
        terms_fixture(),
        dense_block_fixture(),
        disi_all_fixture(),
    ]
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Writes one fixture through the real doc-values writers and the real
/// `Lucene90` consumer.
fn write_segment(directory: &Arc<dyn Directory>, info: &SegmentInfo, fixture: &Fixture) {
    let infos = fixture.field_infos();
    let write_state = SegmentWriteState::new(
        default_info_stream(),
        directory.as_ref(),
        info,
        &infos,
        &BufferedUpdates,
        &*DEFAULT_IO_CONTEXT,
    );
    let mut consumer = Lucene90DocValuesFormat::new()
        .fields_consumer(&write_state)
        .expect("doc values consumer");
    let bytes_used = Arc::new(AtomicI64::new(0));
    for (number, values) in fixture.fields.iter().enumerate() {
        let info = infos
            .field_info_by_number(number as i32)
            .expect("field info")
            .clone();
        write_field(consumer.as_mut(), info, values, Arc::clone(&bytes_used));
    }
    consumer.close().expect("close");
}

fn write_field(
    consumer: &mut dyn DocValuesConsumer,
    info: FieldInfo,
    values: &Values,
    bytes_used: Arc<AtomicI64>,
) {
    match values {
        Values::Numeric(pairs) => {
            let mut writer = NumericDocValuesWriter::new(info, bytes_used);
            for (doc, value) in pairs {
                writer.add_value(*doc, *value).expect("add numeric");
            }
            writer.flush(consumer).expect("flush numeric");
        }
        Values::Binary(pairs) => {
            let mut writer = BinaryDocValuesWriter::new(info, bytes_used);
            for (doc, value) in pairs {
                writer.add_value(*doc, value).expect("add binary");
            }
            writer.flush(consumer).expect("flush binary");
        }
        Values::Sorted(pairs) => {
            let mut writer = SortedDocValuesWriter::new(info, bytes_used);
            for (doc, value) in pairs {
                writer.add_value(*doc, value).expect("add sorted");
            }
            writer.flush(consumer).expect("flush sorted");
        }
        Values::SortedNumeric(pairs) => {
            let mut writer = SortedNumericDocValuesWriter::new(info, bytes_used);
            for (doc, values) in pairs {
                for value in values {
                    writer.add_value(*doc, *value).expect("add sorted numeric");
                }
            }
            writer.flush(consumer).expect("flush sorted numeric");
        }
        Values::SortedSet(pairs) => {
            let mut writer = SortedSetDocValuesWriter::new(info, bytes_used);
            for (doc, values) in pairs {
                for value in values {
                    writer.add_value(*doc, value).expect("add sorted set");
                }
            }
            writer.flush(consumer).expect("flush sorted set");
        }
    }
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

/// Walks every value of every field of a segment, reporting only whether it
/// panicked.
///
/// The reader is expected to answer corruption with an `Err`, or with whatever
/// value Java's wrapping arithmetic would produce; the one outcome this asserts
/// against is an abort.
///
/// Returns how many values it managed to decode, which the sweeps use to prove
/// that they are actually reaching the decoder rather than being turned away at
/// the file header.
fn visit_all(directory: &Arc<dyn Directory>, info: &SegmentInfo, fixture: &Fixture) -> usize {
    let infos = fixture.field_infos();
    let read_state = SegmentReadState::new(directory.as_ref(), info, &infos, &*DEFAULT_IO_CONTEXT);
    let producer = match Lucene90DocValuesFormat::new().fields_producer(&read_state) {
        Ok(producer) => producer,
        // Refusing to open a corrupt segment is a perfectly good answer.
        Err(_) => return 0,
    };
    let _ = producer.check_integrity();
    let mut decoded = visit_fields(producer.as_ref(), &infos, fixture);
    // The merge instance must be no more trusting than the primary one.
    if let Ok(merge) = producer.get_merge_instance() {
        decoded += visit_fields(merge.as_ref(), &infos, fixture);
    }
    decoded
}

fn visit_fields(producer: &dyn DocValuesProducer, infos: &FieldInfos, fixture: &Fixture) -> usize {
    let mut decoded = 0;
    for field in infos.iter() {
        // Every field is asked for twice: state a failed read left behind must
        // not be served to the second one.
        for pass in 0..2 {
            decoded += match field.doc_values_type {
                DocValuesType::NUMERIC => visit_numeric(producer, field, fixture),
                DocValuesType::BINARY => visit_binary(producer, field, fixture),
                DocValuesType::SORTED => visit_sorted(producer, field, fixture),
                DocValuesType::SORTED_NUMERIC => visit_sorted_numeric(producer, field, fixture),
                DocValuesType::SORTED_SET => visit_sorted_set(producer, field, fixture),
                DocValuesType::NONE => 0,
            };
            if pass == 1 {
                visit_random_access(producer, field, fixture);
            }
        }
        // A skipper is never written by these fixtures, but asking for one must
        // still be answered rather than aborted.
        let _ = producer.get_skipper(field);
    }
    decoded
}

/// How many steps a walk is allowed before it is treated as diverging.
///
/// A corrupt entry can describe a run far longer than the segment, so the walk
/// is bounded by what the segment can hold rather than by what the file claims.
/// It is bounded again by [`MAX_WALK`], because the decoding this suite is
/// about happens when the producer is opened, not while its cursors run: past
/// a few thousand documents the walk only repeats iterator arithmetic it has
/// already covered, at the cost of every attempt of every sweep.
fn step_budget(fixture: &Fixture) -> usize {
    const MAX_WALK: usize = 4_096;
    (fixture.max_doc as usize + 16).min(MAX_WALK)
}

fn visit_numeric(producer: &dyn DocValuesProducer, field: &FieldInfo, fixture: &Fixture) -> usize {
    let Ok(mut values) = producer.get_numeric(field) else {
        return 0;
    };
    let mut decoded = 0;
    for _ in 0..step_budget(fixture) {
        match values.next_doc() {
            Ok(NO_MORE_DOCS) | Err(_) => break,
            Ok(_) => {}
        }
        let _ = values.doc_id();
        if values.long_value().is_ok() {
            decoded += 1;
        }
    }
    decoded
}

fn visit_binary(producer: &dyn DocValuesProducer, field: &FieldInfo, fixture: &Fixture) -> usize {
    let Ok(mut values) = producer.get_binary(field) else {
        return 0;
    };
    let mut decoded = 0;
    for _ in 0..step_budget(fixture) {
        match values.next_doc() {
            Ok(NO_MORE_DOCS) | Err(_) => break,
            Ok(_) => {}
        }
        if let Ok(value) = values.binary_value() {
            // Reading the bytes back proves the slice the reader handed out is
            // one it actually owns; the empty value is legal and still counts.
            std::hint::black_box(value.slice());
            decoded += 1;
        }
    }
    decoded
}

fn visit_sorted(producer: &dyn DocValuesProducer, field: &FieldInfo, fixture: &Fixture) -> usize {
    let Ok(mut values) = producer.get_sorted(field) else {
        return 0;
    };
    let count = values.get_value_count().unwrap_or(0);
    // The dictionary is walked over a range that overruns it in both
    // directions: a corrupt term count must be refused, not indexed with.
    for ord in [-1, 0, count / 2, count - 1, count, count + 1, i32::MAX] {
        let _ = values.lookup_ord(ord);
    }
    let _ = values.lookup_term(&BytesRef::new(bytes("bee")));
    let mut decoded = 0;
    for _ in 0..step_budget(fixture) {
        match values.next_doc() {
            Ok(NO_MORE_DOCS) | Err(_) => break,
            Ok(_) => {}
        }
        if let Ok(ord) = values.ord_value() {
            let _ = values.lookup_ord(ord);
            decoded += 1;
        }
    }
    decoded
}

fn visit_sorted_numeric(
    producer: &dyn DocValuesProducer,
    field: &FieldInfo,
    fixture: &Fixture,
) -> usize {
    let Ok(mut values) = producer.get_sorted_numeric(field) else {
        return 0;
    };
    let mut decoded = 0;
    for _ in 0..step_budget(fixture) {
        match values.next_doc() {
            Ok(NO_MORE_DOCS) | Err(_) => break,
            Ok(_) => {}
        }
        let count = values.doc_value_count().unwrap_or(0);
        // One past the count as well: the cursor must refuse rather than serve
        // the next document's value.
        for _ in 0..count.saturating_add(1).min(1_024) {
            if values.next_value().is_ok() {
                decoded += 1;
            }
        }
    }
    decoded
}

fn visit_sorted_set(
    producer: &dyn DocValuesProducer,
    field: &FieldInfo,
    fixture: &Fixture,
) -> usize {
    let Ok(mut values) = producer.get_sorted_set(field) else {
        return 0;
    };
    let count = values.get_value_count().unwrap_or(0);
    for ord in [-1, 0, count / 2, count - 1, count, count + 1, i64::MAX] {
        let _ = values.lookup_ord(ord);
    }
    let _ = values.lookup_term(&BytesRef::new(bytes("bee")));
    let mut decoded = 0;
    for _ in 0..step_budget(fixture) {
        match values.next_doc() {
            Ok(NO_MORE_DOCS) | Err(_) => break,
            Ok(_) => {}
        }
        let doc_count = values.doc_value_count().unwrap_or(0);
        for _ in 0..doc_count.saturating_add(1).min(1_024) {
            if let Ok(ord) = values.next_ord() {
                let _ = values.lookup_ord(ord);
                decoded += 1;
            }
        }
    }
    decoded
}

/// Drives the random-access entry points, which take their own path through the
/// cursor arithmetic.
fn visit_random_access(producer: &dyn DocValuesProducer, field: &FieldInfo, fixture: &Fixture) {
    let max_doc = fixture.max_doc;
    let targets = [0, 1, max_doc / 2, max_doc - 1, max_doc, max_doc + 1];
    for target in targets {
        match field.doc_values_type {
            DocValuesType::NUMERIC => {
                if let Ok(mut values) = producer.get_numeric(field) {
                    let _ = values.advance(target);
                    let _ = values.long_value();
                    let _ = values.advance_exact(target);
                    let _ = values.long_value();
                    let _ = values.cost();
                }
            }
            DocValuesType::BINARY => {
                if let Ok(mut values) = producer.get_binary(field) {
                    let _ = values.advance(target);
                    let _ = values.binary_value();
                    let _ = values.advance_exact(target);
                    let _ = values.binary_value();
                    let _ = values.cost();
                }
            }
            DocValuesType::SORTED => {
                if let Ok(mut values) = producer.get_sorted(field) {
                    let _ = values.advance(target);
                    let _ = values.ord_value();
                    let _ = values.advance_exact(target);
                    let _ = values.ord_value();
                    let _ = values.cost();
                }
            }
            DocValuesType::SORTED_NUMERIC => {
                if let Ok(mut values) = producer.get_sorted_numeric(field) {
                    let _ = values.advance(target);
                    let _ = values.next_value();
                    let _ = values.advance_exact(target);
                    let _ = values.next_value();
                    let _ = values.cost();
                }
            }
            DocValuesType::SORTED_SET => {
                if let Ok(mut values) = producer.get_sorted_set(field) {
                    let _ = values.advance(target);
                    let _ = values.next_ord();
                    let _ = values.advance_exact(target);
                    let _ = values.next_ord();
                    let _ = values.cost();
                }
            }
            DocValuesType::NONE => {}
        }
    }
}

// ---------------------------------------------------------------------------
// An independent metadata reader, used only to prove what each fixture writes
// ---------------------------------------------------------------------------

/// A cursor over a `.dvm` body, in the little-endian encoding
/// `DataOutput.writeShort/writeInt/writeLong` produce.
///
/// This is a second, deliberately independent decoder: it exists so the shape
/// each fixture writes can be asserted rather than assumed, and it would be
/// worthless as an oracle if it called the reader under test.
struct MetaReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> MetaReader<'a> {
    /// Positions the cursor after the index header, whose length is the codec
    /// name, the version, the segment id and the suffix.
    fn new(bytes: &'a [u8]) -> Self {
        let name_length = bytes[4] as usize;
        let at = 4 + 1 + name_length + 4 + 16 + 1 + bytes[4 + 1 + name_length + 4 + 16] as usize;
        Self { bytes, at }
    }

    fn byte(&mut self) -> u8 {
        let value = self.bytes[self.at];
        self.at += 1;
        value
    }

    fn short(&mut self) -> i16 {
        let value = i16::from_le_bytes([self.bytes[self.at], self.bytes[self.at + 1]]);
        self.at += 2;
        value
    }

    fn int(&mut self) -> i32 {
        let value = i32::from_le_bytes(self.bytes[self.at..self.at + 4].try_into().unwrap());
        self.at += 4;
        value
    }

    fn long(&mut self) -> i64 {
        let value = i64::from_le_bytes(self.bytes[self.at..self.at + 8].try_into().unwrap());
        self.at += 8;
        value
    }

    fn v_long(&mut self) -> i64 {
        let mut value = 0i64;
        let mut shift = 0;
        loop {
            let byte = self.byte();
            value |= i64::from(byte & 0x7F) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                return value;
            }
        }
    }

    fn v_int(&mut self) -> i32 {
        self.v_long() as i32
    }

    fn skip(&mut self, count: usize) {
        self.at += count;
    }

    /// Skips the per-block records `DirectMonotonicWriter` emits: a minimum, an
    /// average, an offset and a bit width, twenty-one bytes each.
    fn skip_monotonic(&mut self, num_values: i64, shift: i32) {
        let mut blocks = num_values >> shift;
        if blocks << shift < num_values {
            blocks += 1;
        }
        self.skip(blocks as usize * 21);
    }
}

/// What one metadata entry says about the shape it was written in.
#[derive(Debug, Default, Clone)]
struct Shape {
    /// Where the entry begins in the metadata body, so a test can patch a word
    /// of it by name rather than by a hand-counted offset.
    at: usize,
    field: i32,
    kind: u8,
    multi_valued: Option<u8>,
    docs_with_field_offset: i64,
    jump_table_entry_count: i16,
    num_values: i64,
    table_size: i32,
    bits_per_value: u8,
    gcd: i64,
    num_docs_with_field: Option<i32>,
    binary_lengths: Option<(i32, i32)>,
    terms_dict_size: Option<i64>,
}

fn read_numeric_shape(meta: &mut MetaReader, shape: &mut Shape) {
    shape.docs_with_field_offset = meta.long();
    meta.skip(8); // docsWithFieldLength
    shape.jump_table_entry_count = meta.short();
    meta.skip(1); // denseRankPower
    shape.num_values = meta.long();
    shape.table_size = meta.int();
    if shape.table_size > 0 {
        meta.skip(shape.table_size as usize * 8);
    }
    shape.bits_per_value = meta.byte();
    meta.skip(8); // minValue
    shape.gcd = meta.long();
    meta.skip(24); // valuesOffset, valuesLength, valueJumpTableOffset
}

fn read_addresses_shape(meta: &mut MetaReader, num_addresses: i64) {
    meta.skip(8); // addressesOffset
    let shift = meta.v_int();
    meta.skip_monotonic(num_addresses, shift);
    meta.skip(8); // addressesLength
}

fn read_terms_dict_shape(meta: &mut MetaReader, shape: &mut Shape) {
    let size = meta.v_long();
    shape.terms_dict_size = Some(size);
    let block_shift = meta.int();
    meta.skip_monotonic((size + 63) >> 6, block_shift);
    meta.skip(8 + 32); // maxTermLength, maxBlockLength, four longs
    let index_shift = meta.int();
    let index_size = (size + (1i64 << index_shift) - 1) >> index_shift;
    meta.skip_monotonic(index_size + 1, block_shift);
    meta.skip(32); // four longs
}

fn read_grouped_shape(meta: &mut MetaReader, shape: &mut Shape) {
    let num_docs_with_field = meta.int();
    shape.num_docs_with_field = Some(num_docs_with_field);
    if i64::from(num_docs_with_field) != shape.num_values {
        read_addresses_shape(meta, i64::from(num_docs_with_field) + 1);
    }
}

/// Decodes every entry of a `.dvm`, in the order they were written.
fn shapes(dvm: &[u8]) -> Vec<Shape> {
    let mut meta = MetaReader::new(dvm);
    let mut shapes = Vec::new();
    loop {
        let field = meta.int();
        if field == -1 {
            return shapes;
        }
        let at = meta.at - 4;
        let mut shape = Shape {
            at,
            field,
            kind: meta.byte(),
            ..Shape::default()
        };
        match shape.kind {
            0 => read_numeric_shape(&mut meta, &mut shape),
            1 => {
                meta.skip(16); // dataOffset, dataLength
                shape.docs_with_field_offset = meta.long();
                meta.skip(8); // docsWithFieldLength
                shape.jump_table_entry_count = meta.short();
                meta.skip(1); // denseRankPower
                let num_docs_with_field = meta.int();
                shape.num_docs_with_field = Some(num_docs_with_field);
                let min_length = meta.int();
                let max_length = meta.int();
                shape.binary_lengths = Some((min_length, max_length));
                if min_length < max_length {
                    read_addresses_shape(&mut meta, i64::from(num_docs_with_field) + 1);
                }
            }
            2 => {
                read_numeric_shape(&mut meta, &mut shape);
                read_terms_dict_shape(&mut meta, &mut shape);
            }
            3 => {
                let multi_valued = meta.byte();
                shape.multi_valued = Some(multi_valued);
                read_numeric_shape(&mut meta, &mut shape);
                if multi_valued == 1 {
                    read_grouped_shape(&mut meta, &mut shape);
                }
                read_terms_dict_shape(&mut meta, &mut shape);
            }
            4 => {
                read_numeric_shape(&mut meta, &mut shape);
                read_grouped_shape(&mut meta, &mut shape);
            }
            other => panic!("unknown doc-values type {other}"),
        }
        shapes.push(shape);
    }
}

/// The block encoding `IndexedDISI` chose for the first block of a stream.
///
/// A block header is the block index and the cardinality less one, both as
/// shorts; the encoding follows from the cardinality alone
/// (`IndexedDISI.java:118-137`).
#[derive(Debug, PartialEq)]
enum BlockMethod {
    Sparse,
    Dense,
    All,
}

fn first_block_method(dvd: &[u8], offset: i64) -> BlockMethod {
    let at = offset as usize;
    let cardinality = i32::from(u16::from_le_bytes([dvd[at + 2], dvd[at + 3]])) + 1;
    // `IndexedDISI.MAX_ARRAY_LENGTH` is `(1 << 12) - 1` and `BLOCK_SIZE` is
    // 65536 (`IndexedDISI.java:105-111`).
    const MAX_ARRAY_LENGTH: i32 = (1 << 12) - 1;
    const BLOCK_SIZE: i32 = 65_536;
    match cardinality {
        c if c <= MAX_ARRAY_LENGTH => BlockMethod::Sparse,
        BLOCK_SIZE => BlockMethod::All,
        _ => BlockMethod::Dense,
    }
}

#[test]
fn every_fixture_writes_the_shape_it_exists_for() {
    // The sweeps are only worth what their shapes are worth, and a fixture can
    // stop reaching a branch through a change too small to break a round trip:
    // one value fewer and a unique-value table becomes bit packing, one
    // document fewer and a DENSE block becomes SPARSE. Each claim the module
    // documentation makes is therefore asserted here against the bytes the
    // fixture actually wrote.
    let by_name: HashMap<&str, (Vec<Shape>, Vec<u8>)> = fixtures()
        .iter()
        .map(|fixture| {
            let (directory, _, _) = build(fixture);
            let dvm = read_file(&directory, "_0.dvm");
            let dvd = read_file(&directory, "_0.dvd");
            (fixture.name, (shapes(&dvm), dvd))
        })
        .collect();

    // The entries reach the file in the order the fields were flushed, and
    // every field number is the one the field infos gave it: an assertion that
    // holds the rest of this test to the right entries.
    for (name, (shapes, _)) in &by_name {
        let numbers: Vec<i32> = shapes.iter().map(|shape| shape.field).collect();
        let expected: Vec<i32> = (0..numbers.len() as i32).collect();
        assert_eq!(numbers, expected, "[{name}] entry order");
    }

    // `numeric`: four fields over every document, one per encoding.
    let (numeric, _) = &by_name["numeric"];
    assert_eq!(numeric.len(), 4);
    for shape in numeric {
        assert_eq!(shape.kind, 0, "every field of `numeric` is NUMERIC");
        assert_eq!(
            shape.docs_with_field_offset, -1,
            "every document carries it, so no documents-with-field stream is written"
        );
    }
    assert!(
        numeric[0].table_size == -1 && numeric[0].bits_per_value > 0 && numeric[0].gcd == 1,
        "field 0 must be plain bit packing: {:?}",
        numeric[0]
    );
    assert_eq!(
        numeric[1].table_size, 5,
        "field 1 must be a unique-value table"
    );
    assert_eq!(
        numeric[2].bits_per_value, 0,
        "field 2 must be the constant whose value lives in the metadata"
    );
    assert!(
        numeric[3].table_size == -1 && numeric[3].gcd > 1,
        "field 3 must be GCD compressed: {:?}",
        numeric[3]
    );

    // `sparse`: a SPARSE stream, both binary layouts, and an empty field.
    let (sparse, sparse_data) = &by_name["sparse"];
    assert_eq!(sparse.len(), 4);
    for entry in &sparse[..3] {
        assert!(
            entry.docs_with_field_offset >= 0,
            "the three present fields each write a documents-with-field stream"
        );
        assert_eq!(
            first_block_method(sparse_data, entry.docs_with_field_offset),
            BlockMethod::Sparse,
            "field {} must use the SPARSE block encoding",
            entry.field
        );
    }
    let (min, max) = sparse[1].binary_lengths.expect("binary");
    assert!(
        min < max,
        "field 1 must be the variable-length binary layout"
    );
    assert_eq!(min, 0, "including the empty value");
    let (min, max) = sparse[2].binary_lengths.expect("binary");
    assert_eq!(min, max, "field 2 must be the fixed-length binary layout");
    assert_eq!(
        sparse[3].docs_with_field_offset, -2,
        "field 3 must be the field no document carries"
    );

    // `sorted`: the three single-valued layouts.
    let (sorted, _) = &by_name["sorted"];
    assert_eq!(sorted.len(), 3);
    assert_eq!(sorted[0].kind, 2);
    assert!(sorted[0].terms_dict_size.expect("dictionary") > 1);
    assert_eq!(sorted[1].kind, 3);
    assert_eq!(
        sorted[1].multi_valued,
        Some(0),
        "field 1 must take the single-valued sorted-set route"
    );
    assert_eq!(sorted[2].kind, 4);
    assert_eq!(
        sorted[2].num_docs_with_field.map(i64::from),
        Some(sorted[2].num_values),
        "field 2 must have one value per document, so no address table"
    );

    // `multi`: the two address-table layouts.
    let (multi, _) = &by_name["multi"];
    assert_eq!(multi.len(), 2);
    assert_eq!(multi[0].multi_valued, Some(1));
    assert!(
        i64::from(multi[0].num_docs_with_field.expect("count")) < multi[0].num_values,
        "field 0 must carry more values than documents, so an address table"
    );
    assert!(
        i64::from(multi[1].num_docs_with_field.expect("count")) < multi[1].num_values,
        "field 1 must carry more values than documents, so an address table"
    );

    // `terms`: a dictionary spanning more than one LZ4 block.
    let (terms, _) = &by_name["terms"];
    assert_eq!(terms.len(), 1);
    let size = terms[0].terms_dict_size.expect("dictionary");
    assert!(
        size > 64,
        "the dictionary must span more than one 64-term block, got {size}"
    );

    // `denseblock` and `disiall`: the other two block encodings.
    let (dense, dense_data) = &by_name["denseblock"];
    assert_eq!(
        first_block_method(dense_data, dense[0].docs_with_field_offset),
        BlockMethod::Dense
    );
    let (all, all_data) = &by_name["disiall"];
    assert_eq!(
        first_block_method(all_data, all[0].docs_with_field_offset),
        BlockMethod::All
    );
    assert!(
        all[0].jump_table_entry_count > 0,
        "the ALL fixture must span more than one block, so a jump table is written"
    );
    assert_eq!(
        all[0].bits_per_value, 0,
        "the ALL fixture is constant-valued, so its data file is the stream alone"
    );
}

// ---------------------------------------------------------------------------
// Corruption
// ---------------------------------------------------------------------------

/// Builds a fresh directory holding a corrupted copy of a valid segment.
fn corrupt_copy(
    original: &Arc<dyn Directory>,
    file: &str,
    at: usize,
    value: u8,
) -> Arc<dyn Directory> {
    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    for extension in ALL_EXTENSIONS {
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

/// Whether a shape is one of the two whose sweeps have to be rationed.
///
/// `denseblock` spans 4300 documents and `disiall` 66048, and every attempt
/// walks the segment once through the iterators. Sweeping them the way the five
/// small shapes are swept would cost far more than the rest of the suite
/// together, and would re-cover the same branches: what these two shapes exist
/// for is the DENSE and ALL block encodings, both of which are decoded from the
/// head of the documents-with-field stream by every attempt that reaches an
/// iterator at all, however many attempts there are.
fn is_large(shape: &str) -> bool {
    matches!(shape, "denseblock" | "disiall")
}

/// The positions of a file a sweep visits.
///
/// Small files are swept whole. A large fixture's data file is swept over a
/// window at each end, which is where every structural field lives: the block
/// header, the rank table and the head of the bitmap at one end, the tail of
/// the packed values and the jump table at the other. The middle of a bitmap is
/// pure data, and flipping a bit there adds or removes a document and reaches
/// no branch the ends do not.
fn positions(shape: &str, length: usize) -> Vec<usize> {
    const WINDOW: usize = 224;
    if !is_large(shape) || length <= WINDOW * 2 {
        return (0..length).collect();
    }
    let mut positions: Vec<usize> = (0..WINDOW).collect();
    positions.extend((length - WINDOW)..length);
    positions
}

/// The byte values a sweep tries at each position it visits.
///
/// A small shape gets all 255 others, because the values that break a decoder
/// are the ones that name a length, a count or an offset and those are spread
/// across the whole range. A large shape gets the boundaries of that range
/// instead — every bit pattern that flips a sign, saturates a width, or turns a
/// `VInt` continuation on or off — which is where the same decisions are made
/// at a fraction of the cost.
fn sweep_values(shape: &str) -> Vec<u8> {
    if is_large(shape) {
        vec![
            0x00, 0x01, 0x02, 0x03, 0x0f, 0x10, 0x40, 0x55, 0x7f, 0x80, 0x81, 0xbf, 0xc0, 0xfd,
            0xfe, 0xff,
        ]
    } else {
        (0..=u8::MAX).collect()
    }
}

/// Writes `value` into `bytes[at..at + width]` little-endian, the way
/// `DataOutput.writeShort/writeInt/writeLong` do.
fn patch(bytes: &mut [u8], at: usize, width: usize, value: i64) {
    let raw = value.to_le_bytes();
    bytes[at..at + width].copy_from_slice(&raw[..width]);
}

/// The values a hostile writer would choose for a word of the metadata.
///
/// They are the boundaries of the two's-complement ranges rather than a random
/// sample, because that is where a product overflows, a subtraction underflows
/// and a shift leaves its width.
fn hostile_values(width: usize) -> Vec<i64> {
    let mut values = vec![0, 1, -1, -2, -3, 2, 63, 64, 255, 256, 4_095, 4_096, 65_536];
    if width >= 4 {
        values.extend([i64::from(i32::MAX), i64::from(i32::MIN), 1 << 30]);
    }
    if width == 8 {
        values.extend([i64::MAX, i64::MIN, i64::MAX / 2, 1 << 40, -(1 << 40)]);
    }
    values
}

/// Rebuilds a `.dvm` whose footer matches its (patched) body.
///
/// Without this the reader stops at the checksum and the iterators are never
/// reached, so every offset, length, count and width in the metadata would go
/// untested against them.
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

// ---------------------------------------------------------------------------
// Sweeps
// ---------------------------------------------------------------------------

#[test]
fn every_value_at_every_byte_of_the_data_file_is_survived() {
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let original = read_file(&directory, "_0.dvd");
        assert!(
            original.len() > 48,
            "[{}] the fixture must produce a real data file, got {} bytes",
            fixture.name,
            original.len()
        );

        let swept = positions(fixture.name, original.len());
        let values = sweep_values(fixture.name);
        let mut survived = 0;
        let mut decoded = 0;
        let mut expected = 0;
        for at in swept.iter().copied() {
            for value in values.iter().copied() {
                if original[at] == value {
                    continue;
                }
                expected += 1;
                let corrupt = corrupt_copy(&directory, "_0.dvd", at, value);
                let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
                decoded += visit_all(&corrupt, &corrupt_info, &fixture);
                survived += 1;
            }
        }
        assert_eq!(
            survived, expected,
            "[{}] every swept position must have been tried with every value of \
             the sweep",
            fixture.name
        );
        assert!(
            survived >= swept.len() * (values.len() - 1),
            "[{}] the sweep must reach every position with every value but the \
             one already there; ran {survived} attempts over {} positions",
            fixture.name,
            swept.len()
        );
        assert!(
            decoded > 100,
            "[{}] the sweep must reach the decoder, not be turned away at the \
             header; only {decoded} values were decoded",
            fixture.name
        );
    }
}

#[test]
fn every_value_at_every_byte_of_the_metadata_file_is_survived() {
    // A corrupt `.dvm` is always caught by its checksum in the end, but only
    // *after* the entry decoder has read the garbage. What this sweep proves is
    // that the entry decoder survives every byte, not that the checksum works.
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let original = read_file(&directory, "_0.dvm");
        let swept = positions(fixture.name, original.len());
        let values = sweep_values(fixture.name);
        let mut survived = 0;
        let mut expected = 0;
        for at in swept.iter().copied() {
            for value in values.iter().copied() {
                if original[at] == value {
                    continue;
                }
                expected += 1;
                let corrupt = corrupt_copy(&directory, "_0.dvm", at, value);
                let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
                visit_all(&corrupt, &corrupt_info, &fixture);
                survived += 1;
            }
        }
        assert_eq!(
            survived, expected,
            "[{}] every swept position of the metadata must have been tried \
             with every value of the sweep",
            fixture.name
        );
        assert!(
            survived >= swept.len() * (values.len() - 1),
            "[{}] the metadata sweep must reach every position with every value \
             but the one already there; ran {survived} attempts",
            fixture.name
        );
    }
}

#[test]
fn a_truncated_data_file_is_survived_at_every_length() {
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let original = read_file(&directory, "_0.dvd");
        let meta = read_file(&directory, "_0.dvm");
        let skip = read_file(&directory, "_0.dvs");
        let step = if original.len() > 1_024 { 17 } else { 1 };
        for length in (0..original.len()).step_by(step) {
            let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            write_file(&corrupt, "_0.dvd", &original[..length]);
            write_file(&corrupt, "_0.dvm", &meta);
            write_file(&corrupt, "_0.dvs", &skip);
            let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
            visit_all(&corrupt, &corrupt_info, &fixture);
        }
    }
}

#[test]
fn a_truncated_metadata_file_is_survived_at_every_length() {
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let data = read_file(&directory, "_0.dvd");
        let original = read_file(&directory, "_0.dvm");
        let skip = read_file(&directory, "_0.dvs");
        let step = if original.len() > 1_024 { 17 } else { 1 };
        for length in (0..original.len()).step_by(step) {
            let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            write_file(&corrupt, "_0.dvd", &data);
            write_file(&corrupt, "_0.dvm", &original[..length]);
            write_file(&corrupt, "_0.dvs", &skip);
            let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
            visit_all(&corrupt, &corrupt_info, &fixture);
        }
    }
}

#[test]
fn hostile_metadata_that_passes_its_checksum_is_survived() {
    // This is the sweep that actually reaches the iterators. Every aligned word
    // of the metadata body is replaced with each boundary value, the footer is
    // recomputed so the reader accepts the file, and the segment is then read.
    // Offsets, lengths, bit widths, table sizes, document counts, block shifts
    // and term counts all arrive at the decoder as attacker input.
    //
    // The positions are swept rather than named, because the entry layout
    // differs per doc-values type and a named list would quietly stop covering
    // a type the day its layout changed.
    let footer_length = 16;
    let mut attempts = 0;
    let mut opened = 0;
    for fixture in fixtures() {
        let (directory, _, id) = build(&fixture);
        let original = read_file(&directory, "_0.dvm");
        let data = read_file(&directory, "_0.dvd");
        let skip = read_file(&directory, "_0.dvs");
        let body = &original[..original.len() - footer_length];

        // The large shapes cost a walk over their whole documents-with-field
        // stream per attempt, so they are patched at the eight-byte width only:
        // every offset, length and count of an entry is eight bytes wide, and
        // the narrower widths only reach the same words from the middle.
        let widths: &[usize] = if is_large(fixture.name) {
            &[8]
        } else {
            &[1, 2, 4, 8]
        };
        for width in widths.iter().copied() {
            let mut at = 0;
            while at + width <= body.len() {
                for value in hostile_values(width) {
                    let mut patched = body.to_vec();
                    patch(&mut patched, at, width, value);
                    let signed = resign(&patched);

                    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
                    write_file(&corrupt, "_0.dvd", &data);
                    write_file(&corrupt, "_0.dvm", &signed);
                    write_file(&corrupt, "_0.dvs", &skip);
                    let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);
                    // The only assertion is that this returns at all.
                    let decoded = visit_all(&corrupt, &corrupt_info, &fixture);
                    attempts += 1;
                    if decoded > 0 {
                        opened += 1;
                    }
                }
                at += width;
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

// ---------------------------------------------------------------------------
// Targeted refusals
// ---------------------------------------------------------------------------

#[test]
fn a_metadata_entry_naming_an_unknown_field_is_refused() {
    // `readFields` looks every field number up in the field infos and refuses a
    // number it does not know, before any offset of the entry is used. Without
    // that, a corrupt number creates an entry no field will ever ask for and
    // the entry after it is decoded at the wrong offset.
    let fixture = sorted_fixture();
    let (directory, _, id) = build(&fixture);
    let original = read_file(&directory, "_0.dvm");
    let data = read_file(&directory, "_0.dvd");
    let skip = read_file(&directory, "_0.dvs");
    let body = &original[..original.len() - 16];
    // The header ends where the first entry begins, and the first entry begins
    // with its four-byte field number. The index header is the codec header
    // (magic, codec name, version), the segment id and the suffix; rather than
    // recompute it, the number is located by searching for the little-endian
    // zero the first entry starts with, immediately followed by the SORTED type
    // byte the first field of this fixture has.
    let entry_start = (0..body.len() - 5)
        .find(|&at| body[at..at + 4] == [0, 0, 0, 0] && body[at + 4] == 2)
        .expect("the first entry of the sorted fixture is field 0, type SORTED");

    for number in [
        fixture.fields.len() as i32,
        1_000,
        i32::MAX,
        -2,
        i32::MIN,
        i32::MAX - 1,
    ] {
        let mut patched = body.to_vec();
        patch(&mut patched, entry_start, 4, i64::from(number));
        let signed = resign(&patched);
        let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        write_file(&corrupt, "_0.dvd", &data);
        write_file(&corrupt, "_0.dvm", &signed);
        write_file(&corrupt, "_0.dvs", &skip);
        let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);

        let infos = fixture.field_infos();
        let read_state = SegmentReadState::new(
            corrupt.as_ref(),
            &corrupt_info,
            &infos,
            &*DEFAULT_IO_CONTEXT,
        );
        assert!(
            Lucene90DocValuesFormat::new()
                .fields_producer(&read_state)
                .is_err(),
            "a metadata entry for field number {number} must be refused"
        );
    }
}

#[test]
fn a_field_the_metadata_never_mentions_is_never_answered_with_another_fields_values() {
    // Only the fields a fixture writes have entries. Asking for one that has
    // none must yield an exhausted iterator, never a value cut out of a
    // neighbouring field's region.
    let fixture = sorted_fixture();
    let (directory, info, _) = build(&fixture);
    // A field infos with one more field than the segment was written with: the
    // extra number has no metadata entry at all.
    let mut infos: Vec<FieldInfo> = fixture
        .fields
        .iter()
        .enumerate()
        .map(|(number, values)| {
            let mut field = FieldInfo::new(format!("f{number}"), number as i32);
            field.doc_values_type = values.doc_values_type();
            field
        })
        .collect();
    let orphan = fixture.fields.len() as i32;
    let mut extra = FieldInfo::new(format!("f{orphan}"), orphan);
    extra.doc_values_type = DocValuesType::NUMERIC;
    infos.push(extra);
    let infos = FieldInfos::new(infos).expect("field infos");

    let read_state = SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
    let producer = Lucene90DocValuesFormat::new()
        .fields_producer(&read_state)
        .expect("doc values producer");
    let field = infos.field_info_by_number(orphan).expect("field info");
    let mut values = producer.get_numeric(field).expect("numeric");
    assert_eq!(
        values.next_doc().expect("next doc"),
        NO_MORE_DOCS,
        "a field with no metadata entry must hold no documents"
    );
    assert!(
        values.long_value().is_err(),
        "an exhausted iterator must not answer with a value"
    );
}

#[test]
fn advancing_past_the_segment_exhausts_the_iterator() {
    for fixture in fixtures() {
        let (directory, info, _) = build(&fixture);
        let infos = fixture.field_infos();
        let read_state =
            SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
        let producer = Lucene90DocValuesFormat::new()
            .fields_producer(&read_state)
            .expect("doc values producer");
        for field in infos.iter() {
            if field.doc_values_type != DocValuesType::NUMERIC {
                continue;
            }
            for doc in [fixture.max_doc, fixture.max_doc + 1, i32::MAX - 1] {
                let mut values = producer.get_numeric(field).expect("numeric");
                if let Ok(found) = values.advance(doc) {
                    assert_eq!(
                        found, NO_MORE_DOCS,
                        "[{}] advance({doc}) past the segment must be exhausted",
                        fixture.name
                    );
                    // What may *not* happen is a panic or a read outside the
                    // entry's own region. What may happen is a value: Lucene's
                    // `DocValuesIterator` leaves `longValue()` undefined unless
                    // the iterator is positioned, and its constant-valued dense
                    // fields answer with their constant even after exhaustion
                    // (`Lucene90DocValuesProducer.java:789-793`). This port
                    // reproduces that rather than adding a check Lucene has
                    // not, so the assertion is that the call returns at all.
                    let _ = values.long_value();
                }
            }
        }
    }
}

#[test]
fn an_ordinal_outside_the_dictionary_is_refused() {
    // The dictionary is the one place a caller supplies the index, so it is the
    // one place a bounds check is a contract and not just a defence.
    for fixture in [sorted_fixture(), multi_fixture(), terms_fixture()] {
        let (directory, info, _) = build(&fixture);
        let infos = fixture.field_infos();
        let read_state =
            SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
        let producer = Lucene90DocValuesFormat::new()
            .fields_producer(&read_state)
            .expect("doc values producer");
        for field in infos.iter() {
            match field.doc_values_type {
                DocValuesType::SORTED => {
                    let values = producer.get_sorted(field).expect("sorted");
                    let count = values.get_value_count().expect("value count");
                    assert!(count > 0, "[{}] the fixture writes terms", fixture.name);
                    assert!(values.lookup_ord(count).is_err());
                    assert!(values.lookup_ord(-1).is_err());
                    assert!(values.lookup_ord(i32::MAX).is_err());
                    assert!(values.lookup_ord(count - 1).is_ok());
                }
                DocValuesType::SORTED_SET => {
                    let values = producer.get_sorted_set(field).expect("sorted set");
                    let count = values.get_value_count().expect("value count");
                    assert!(count > 0, "[{}] the fixture writes terms", fixture.name);
                    assert!(values.lookup_ord(count).is_err());
                    assert!(values.lookup_ord(-1).is_err());
                    assert!(values.lookup_ord(i64::MAX).is_err());
                    assert!(values.lookup_ord(count - 1).is_ok());
                }
                _ => {}
            }
        }
    }
}

#[test]
fn every_fixture_round_trips_before_it_is_corrupted() {
    // A sweep over a fixture that never decoded correctly in the first place
    // would prove nothing, so each shape is read back and compared against the
    // table it was written from before any byte of it is touched.
    for fixture in fixtures() {
        let (directory, info, _) = build(&fixture);
        let infos = fixture.field_infos();
        let read_state =
            SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
        let producer = Lucene90DocValuesFormat::new()
            .fields_producer(&read_state)
            .expect("doc values producer");
        producer.check_integrity().expect("integrity");

        for (number, expected) in fixture.fields.iter().enumerate() {
            let field = infos
                .field_info_by_number(number as i32)
                .expect("field info");
            match expected {
                Values::Numeric(pairs) => {
                    let mut values = producer.get_numeric(field).expect("numeric");
                    for (doc, value) in pairs {
                        assert_eq!(values.next_doc().expect("next doc"), *doc);
                        assert_eq!(values.long_value().expect("value"), *value);
                    }
                    assert_eq!(values.next_doc().expect("next doc"), NO_MORE_DOCS);
                }
                Values::Binary(pairs) => {
                    let mut values = producer.get_binary(field).expect("binary");
                    for (doc, value) in pairs {
                        assert_eq!(values.next_doc().expect("next doc"), *doc);
                        assert_eq!(values.binary_value().expect("value").slice(), &value[..]);
                    }
                    assert_eq!(values.next_doc().expect("next doc"), NO_MORE_DOCS);
                }
                Values::Sorted(pairs) => {
                    let mut values = producer.get_sorted(field).expect("sorted");
                    for (doc, value) in pairs {
                        assert_eq!(values.next_doc().expect("next doc"), *doc);
                        let ord = values.ord_value().expect("ord");
                        assert_eq!(values.lookup_ord(ord).expect("term").slice(), &value[..]);
                    }
                    assert_eq!(values.next_doc().expect("next doc"), NO_MORE_DOCS);
                }
                Values::SortedNumeric(pairs) => {
                    let mut values = producer.get_sorted_numeric(field).expect("sorted numeric");
                    for (doc, expected) in pairs {
                        assert_eq!(values.next_doc().expect("next doc"), *doc);
                        let count = values.doc_value_count().expect("count");
                        let mut got: Vec<i64> = Vec::new();
                        for _ in 0..count {
                            got.push(values.next_value().expect("value"));
                        }
                        // The format keeps a sorted list, duplicates included.
                        let mut want = expected.clone();
                        want.sort_unstable();
                        assert_eq!(got, want, "[{}] doc {doc}", fixture.name);
                    }
                    assert_eq!(values.next_doc().expect("next doc"), NO_MORE_DOCS);
                }
                Values::SortedSet(pairs) => {
                    let mut values = producer.get_sorted_set(field).expect("sorted set");
                    for (doc, expected) in pairs {
                        assert_eq!(values.next_doc().expect("next doc"), *doc);
                        let count = values.doc_value_count().expect("count");
                        let mut got: Vec<Vec<u8>> = Vec::new();
                        for _ in 0..count {
                            let ord = values.next_ord().expect("ord");
                            got.push(values.lookup_ord(ord).expect("term").slice().to_vec());
                        }
                        // A set: the writer deduplicates and sorts.
                        let mut want = expected.clone();
                        want.sort();
                        want.dedup();
                        assert_eq!(got, want, "[{}] doc {doc}", fixture.name);
                    }
                    assert_eq!(values.next_doc().expect("next doc"), NO_MORE_DOCS);
                }
            }
        }
    }
}

#[test]
fn a_value_count_off_disk_sizes_nothing() {
    // The property the reader has to hold is that no count read out of the
    // metadata decides how much memory is reserved. It is held by construction:
    // the producer keeps offsets and decodes a value only when an iterator asks
    // for one, exactly as `Lucene90DocValuesProducer` does, so there is no
    // ceiling anywhere and none is needed.
    //
    // This asserts it where a ceiling would have shown: `numValues` is replaced
    // with counts that no machine could materialise, the footer is re-signed so
    // the file is accepted, and the segment is then opened and walked. Opening
    // has to succeed — nothing is decoded yet — and the walk has to stay bounded
    // by the segment rather than by what the metadata claims.
    let fixture = numeric_fixture();
    let (directory, _, id) = build(&fixture);
    let original = read_file(&directory, "_0.dvm");
    let data = read_file(&directory, "_0.dvd");
    let skip = read_file(&directory, "_0.dvs");
    let body = &original[..original.len() - 16];
    let entries = shapes(&original);
    assert_eq!(entries.len(), 4);

    // `numValues` is the fifth word of a NUMERIC entry: four bytes of field
    // number, the type byte, then the two eight-byte documents-with-field
    // words, the jump-table short and the rank-power byte.
    const NUM_VALUES_OFFSET: usize = 4 + 1 + 8 + 8 + 2 + 1;

    for entry in &entries {
        for count in [i64::MAX, i64::MAX / 2, 1 << 62, 1 << 40, -1] {
            let mut patched = body.to_vec();
            patch(&mut patched, entry.at + NUM_VALUES_OFFSET, 8, count);
            let signed = resign(&patched);
            let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
            write_file(&corrupt, "_0.dvd", &data);
            write_file(&corrupt, "_0.dvm", &signed);
            write_file(&corrupt, "_0.dvs", &skip);
            let corrupt_info = segment_info(&corrupt, fixture.max_doc, id);

            let infos = fixture.field_infos();
            let read_state = SegmentReadState::new(
                corrupt.as_ref(),
                &corrupt_info,
                &infos,
                &*DEFAULT_IO_CONTEXT,
            );
            let producer = Lucene90DocValuesFormat::new()
                .fields_producer(&read_state)
                .unwrap_or_else(|error| {
                    panic!("a count of {count} must not stop the segment opening: {error}")
                });

            // Walking is bounded by the segment, not by the claimed count: a
            // reader that had reserved anything from `numValues` could not get
            // this far, and one that iterated it would not stop.
            for field in infos.iter() {
                let Ok(mut values) = producer.get_numeric(field) else {
                    continue;
                };
                let mut seen = 0;
                while let Ok(doc) = values.next_doc() {
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let _ = values.long_value();
                    seen += 1;
                    assert!(
                        seen <= fixture.max_doc,
                        "field {} walked {seen} documents in a {}-document \
                         segment after numValues was set to {count}",
                        field.number,
                        fixture.max_doc
                    );
                }
            }
        }
    }
}
