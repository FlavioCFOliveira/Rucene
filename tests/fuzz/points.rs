//! Defensive fuzz-style tests for the points reader.
//!
//! Everything the reader consumes comes straight off disk and is therefore
//! untrusted. Lucene's own decoder answers corruption with an exception —
//! `CorruptIndexException`, `EOFException`, `ArrayIndexOutOfBoundsException` —
//! or, where Java's arithmetic wraps, with a nonsensical value; none of them
//! aborts the JVM. A Rust port must match that: an `Err`, or a wrapped value
//! where Java wraps, but never a panic and never an allocation sized by an
//! attacker-controlled length.
//!
//! These tests take a valid segment written by [`PointValuesWriter`] through
//! the real `Lucene90PointsFormat`, corrupt it systematically, and assert
//! exactly that. They are the reader-side counterpart of
//! `tests/portability/points.rs`, which proves the same files are the ones
//! Lucene writes.
//!
//! [`PointValuesWriter`]: rucene::index::point_values_writer::PointValuesWriter
//!
//! # Shapes, and why one is not enough
//!
//! The BKD format branches on shape in ways a single fixture cannot reach:
//!
//! * `leaf` — twenty points in one leaf, so the root *is* the leaf and the
//!   index holds no split value at all;
//! * `deep` — sixty points over a leaf size of eight, so the tree is three
//!   levels deep and the index is a real recursive structure of split values,
//!   left-subtree byte counts and leaf-block file pointers;
//! * `nd` — the same depth over two indexed dimensions, which is the only
//!   shape where the split *dimension* byte is not always zero and where a
//!   leaf carries the narrowed per-dimension bounds;
//! * `multi` — two fields in one segment, one of each of the two dimension
//!   counts, which is the only shape where a corrupt offset in one field's
//!   metadata entry can be made to point into another field's region;
//! * `const` — two indexed dimensions where **dimension 0 never varies**, so
//!   every leaf stores `commonPrefixLengths[0] == bytesPerDim`. It is the only
//!   shape in which a corrupt sorted-dimension byte can name a dimension whose
//!   common prefix already covers the whole dimension;
//! * `const3` — the same, over three dimensions with the constant one in the
//!   middle, so the offending dimension is neither the first nor the last;
//! * `lowcard` — a handful of distinct values repeated across the segment, so
//!   `BKDWriter` finds the low-cardinality encoding cheaper and writes
//!   `compressedDim == -2`. It is the only shape that reaches
//!   `visitSparseDocValues` at all.
//!
//! # Shape is a property of the *guard*, not only of the format
//!
//! A guard nobody's fixture can reach is a guard nobody is testing, and two of
//! the guards in this reader were added over an arithmetic relation rather than
//! over a single field. Whenever a guard is added over a **sum** or a
//! **difference**, ask which fixture shape makes its terms disagree in sign or
//! sit exactly on the boundary — and then build that shape.
//!
//! Both defects these fixtures were added for were of that kind:
//!
//! * the compressed-dimension guard was written over the *global* byte offset
//!   `compressedDim * bytesPerDim + commonPrefixLengths[compressedDim]`, which
//!   only reaches the end of the packed value when the named dimension is the
//!   **last** one. Every fixture had all dimensions varying, so no leaf could
//!   ever name a dimension with a full common prefix and the guard was never
//!   reached; `const`/`const3` are the shapes that make the difference
//!   `bytesPerDim - commonPrefixLengths[compressedDim]` go negative;
//! * the leaf-count guard was reached from the two per-value read sites but not
//!   from `BKDPointTree::addAll`, because every visitor here answered
//!   `CellCrossesQuery` and the bulk path only runs for a cell the visitor
//!   accepts whole. [`InsideVisitor`] is the missing *visitor* shape.
//!
//! # Budget
//!
//! The three files of these fixtures are hundreds of bytes, not kilobytes, so
//! the metadata and index files are swept with **every one of the 255 other
//! values** at every position. The data file — which is mostly packed values
//! and doc IDs rather than structure — is swept with a boundary set instead,
//! because sweeping it exhaustively multiplies the suite by sixteen for bytes
//! that name nothing. The leaf size of eight is what keeps the deep shapes
//! small enough for that to be affordable.
//!
//! # What these tests deliberately do not cover
//!
//! * **Multi-byte corruption.** Every sweep flips exactly one byte; a
//!   corruption that must be internally consistent across two words is only
//!   reachable through the hostile-metadata sweep, which patches one word at a
//!   time and re-signs the footer so the patched word actually reaches the
//!   decoder.
//! * **A corrupt `.si` or `.fnm`.** `maxDoc` and the per-field dimension
//!   counts reach the reader from the segment info and the field infos, not
//!   from the points files, so they are trusted here.

use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use rucene::codecs::codec_util;
use rucene::codecs::lucene90::points::{
    Lucene90PointsWriter, VERSION_CURRENT as POINTS_VERSION_CURRENT,
};
use rucene::codecs::points::{PointsWriter, Relation};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::stub::BufferedUpdates;
use rucene::codecs::{register_codec, Codec, Lucene104Codec};
use rucene::index::point_values::IntersectVisitor;
use rucene::index::point_values_writer::PointValuesWriter;
use rucene::index::{FieldInfo, FieldInfos, SegmentInfo};
use rucene::store::{Directory, RamDirectory, DEFAULT_IO_CONTEXT};
use rucene::util::string_helper::StringHelper;
use rucene::util::{default_info_stream, BytesRef, Version};

/// The three files a points segment is made of.
const EXTENSIONS: [&str; 3] = ["kdd", "kdi", "kdm"];

/// How many points a leaf holds in the fixtures that want a deep tree.
const SMALL_LEAF: i32 = 8;

fn codec() -> Arc<dyn Codec> {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    rucene::codecs::default_codec().expect("Lucene104 codec is registered")
}

/// Every field any fixture uses, in a fixed numbering they all share.
///
/// `one` is one-dimensional, `two` has two indexed dimensions and `three` has
/// three, all four bytes wide. `three` exists so that a constant dimension can
/// sit in the *middle* of the packed value, where neither the first nor the
/// last dimension is the one at fault.
fn field_infos() -> FieldInfos {
    let mut one = FieldInfo::new("one", 0);
    one.point_dimension_count = 1;
    one.point_index_dimension_count = 1;
    one.point_num_bytes = 4;
    let mut two = FieldInfo::new("two", 1);
    two.point_dimension_count = 2;
    two.point_index_dimension_count = 2;
    two.point_num_bytes = 4;
    let mut three = FieldInfo::new("three", 2);
    three.point_dimension_count = 3;
    three.point_index_dimension_count = 3;
    three.point_num_bytes = 4;
    FieldInfos::new(vec![one, two, three]).expect("field infos")
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

/// The packed values of one field, as `(docID, packedValue)`.
type FieldPoints = Vec<(i32, Vec<u8>)>;

/// One fixture: its name, how many documents it spans, the leaf size and, per
/// field number, the packed values of each document.
struct Fixture {
    name: &'static str,
    max_doc: i32,
    max_points_in_leaf: i32,
    fields: Vec<(i32, FieldPoints)>,
}

fn packed(values: &[i32]) -> Vec<u8> {
    let mut bytes = vec![0u8; values.len() * 4];
    for (dim, value) in values.iter().enumerate() {
        // The sortable int encoding, so the tree orders values as Lucene does.
        let biased = (*value as u32) ^ 0x8000_0000;
        bytes[dim * 4..dim * 4 + 4].copy_from_slice(&biased.to_be_bytes());
    }
    bytes
}

fn one_dim(count: i32) -> FieldPoints {
    (0..count)
        .map(|doc| (doc, packed(&[(doc * 7919) % 101 - 50])))
        .collect()
}

fn two_dim(count: i32) -> FieldPoints {
    (0..count)
        .map(|doc| {
            (
                doc,
                packed(&[(doc * 7919) % 101 - 50, (doc * 104729) % 37 - 18]),
            )
        })
        .collect()
}

/// Two indexed dimensions of which **dimension 0 never varies**.
///
/// A dimension that holds one value across a whole leaf is stored with
/// `commonPrefixLengths[dim] == bytesPerDim`, which is exactly what
/// `readCommonPrefixes` is allowed to read back. It is the only way to build a
/// leaf where flipping the sorted-dimension byte names a dimension with no byte
/// left to compress.
fn const_dim(count: i32) -> FieldPoints {
    (0..count)
        .map(|doc| (doc, packed(&[42, (doc * 7919) % 1009])))
        .collect()
}

/// Three indexed dimensions with the constant one in the middle.
///
/// Same idea as [`const_dim`], but the offending dimension is neither the first
/// nor the last, so a guard that happens to be right at either end of the
/// packed value is not right here.
fn const_middle_dim(count: i32) -> FieldPoints {
    (0..count)
        .map(|doc| {
            (
                doc,
                packed(&[(doc * 7919) % 1009, 42, (doc * 104729) % 61 - 30]),
            )
        })
        .collect()
}

/// One dimension taking very few distinct values.
///
/// `BKDWriter` compares the cost of run-length compressing the sorted
/// dimension's first non-prefix byte against the cost of storing one
/// `(cardinality, suffix)` pair per distinct value, and writes
/// `compressedDim == -2` when the second is cheaper or equal
/// (`BKDWriter.java:1370-1373`). Two distinct values per leaf of eight is well
/// inside that: the low-cardinality decoder, and therefore its sub-block
/// length guard, is unreachable without a shape like this one.
fn low_cardinality(count: i32) -> FieldPoints {
    (0..count)
        .map(|doc| (doc, packed(&[doc / 8 * 2 + (doc % 2)])))
        .collect()
}

/// The seven shapes every sweep runs over. See the module documentation for
/// what each one exists to express.
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "leaf",
            max_doc: 20,
            max_points_in_leaf: 512,
            fields: vec![(0, one_dim(20))],
        },
        Fixture {
            name: "deep",
            max_doc: 60,
            max_points_in_leaf: SMALL_LEAF,
            fields: vec![(0, one_dim(60))],
        },
        Fixture {
            name: "nd",
            max_doc: 60,
            max_points_in_leaf: SMALL_LEAF,
            fields: vec![(1, two_dim(60))],
        },
        Fixture {
            name: "multi",
            max_doc: 40,
            max_points_in_leaf: SMALL_LEAF,
            fields: vec![(0, one_dim(40)), (1, two_dim(40))],
        },
        Fixture {
            name: "const",
            max_doc: 60,
            max_points_in_leaf: SMALL_LEAF,
            fields: vec![(1, const_dim(60))],
        },
        Fixture {
            name: "const3",
            max_doc: 60,
            max_points_in_leaf: SMALL_LEAF,
            fields: vec![(2, const_middle_dim(60))],
        },
        Fixture {
            name: "lowcard",
            max_doc: 60,
            max_points_in_leaf: SMALL_LEAF,
            fields: vec![(0, low_cardinality(60))],
        },
    ]
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

fn write_segment(directory: &Arc<dyn Directory>, info: &SegmentInfo, fixture: &Fixture) {
    let infos = field_infos();
    let updates = BufferedUpdates;
    let info_stream = default_info_stream();
    let state = SegmentWriteState::new(
        info_stream,
        directory.as_ref(),
        info,
        &infos,
        &updates,
        &*DEFAULT_IO_CONTEXT,
    );
    let mut writer = Lucene90PointsWriter::with_config(
        &state,
        fixture.max_points_in_leaf,
        16.0,
        POINTS_VERSION_CURRENT,
    )
    .expect("points writer");
    let bytes_used = Arc::new(AtomicI64::new(0));
    for (number, values) in &fixture.fields {
        let field_info = infos
            .field_info_by_number(*number)
            .expect("a known field")
            .clone();
        let mut field = PointValuesWriter::new(field_info, Arc::clone(&bytes_used));
        for (doc, value) in values {
            field
                .add_packed_value(*doc, &BytesRef::new(value.clone()))
                .expect("a well-formed value");
        }
        field.flush(&mut writer).expect("flush");
    }
    writer.finish().expect("finish");
    writer.close().expect("close");
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

/// Counts every point it is handed and never rejects a cell, so the whole tree
/// is walked.
#[derive(Default)]
struct CountingVisitor {
    seen: usize,
}

impl IntersectVisitor for CountingVisitor {
    fn visit(&mut self, _doc_id: i32) -> rucene::error::Result<()> {
        self.seen += 1;
        Ok(())
    }

    fn visit_with_value(
        &mut self,
        _doc_id: i32,
        _packed_value: &[u8],
    ) -> rucene::error::Result<()> {
        self.seen += 1;
        Ok(())
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> Relation {
        Relation::CellCrossesQuery
    }
}

/// Accepts every cell whole, so the reader takes its bulk doc-ID path.
///
/// [`CountingVisitor`] always answers `CellCrossesQuery`, which drives the
/// per-value decoder. A visitor that answers `CellInsideQuery` drives
/// `BKDPointTree::addAll` and `visitDocIDs` instead — a third read of the leaf
/// count, over a different decode path, that no `CellCrossesQuery` sweep can
/// reach. The relation, not the value space, is what selects it.
#[derive(Default)]
struct InsideVisitor {
    seen: usize,
}

impl IntersectVisitor for InsideVisitor {
    fn visit(&mut self, _doc_id: i32) -> rucene::error::Result<()> {
        self.seen += 1;
        Ok(())
    }

    fn visit_with_value(
        &mut self,
        _doc_id: i32,
        _packed_value: &[u8],
    ) -> rucene::error::Result<()> {
        self.seen += 1;
        Ok(())
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> Relation {
        Relation::CellInsideQuery
    }
}

/// Opens the segment and walks every point of every field, reporting only
/// whether it panicked.
///
/// The reader is expected to answer corruption with an `Err`, or with whatever
/// value Java's wrapping arithmetic would produce; the one outcome this asserts
/// against is an abort.
///
/// Returns how many points it managed to decode, which the sweeps use to prove
/// that they are actually reaching the decoder rather than being turned away at
/// the file header.
fn visit_all(directory: &Arc<dyn Directory>, info: &SegmentInfo, fixture: &Fixture) -> usize {
    let infos = field_infos();
    let read_state = SegmentReadState::new(directory.as_ref(), info, &infos, &*DEFAULT_IO_CONTEXT);
    let reader = match codec().points_format().fields_reader(&read_state) {
        Ok(reader) => reader,
        // Refusing to open a corrupt segment is a perfectly good answer.
        Err(_) => return 0,
    };
    let _ = reader.check_integrity();
    let mut decoded = 0;
    for (number, _) in &fixture.fields {
        let name = &infos
            .field_info_by_number(*number)
            .expect("a known field")
            .name;
        let Ok(values) = reader.get_values(name) else {
            continue;
        };
        let _ = values.size();
        let _ = values.doc_count();
        let _ = values.min_packed_value();
        let _ = values.max_packed_value();
        let mut visitor = CountingVisitor::default();
        let _ = values.intersect(&mut visitor);
        decoded += visitor.seen;
        // The same tree again, but accepted whole: `CellInsideQuery` is what
        // selects the bulk `addAll`/`visitDocIDs` path, which decodes the leaf
        // count and the doc IDs and never looks at a packed value. Sweeping
        // only with `CellCrossesQuery` leaves that path unexercised.
        let mut inside = InsideVisitor::default();
        let _ = values.intersect(&mut inside);
        decoded += inside.seen;
        // The tree navigation is the other half of the read path, and a cursor
        // can desynchronise where a full intersect does not.
        if let Ok(mut tree) = values.point_tree() {
            let mut steps = 0;
            // Bounded, because a corrupt tree can describe an arbitrarily deep
            // one and the walk is what the sweep is timing.
            while steps < 64 {
                steps += 1;
                match tree.move_to_child() {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(_) => break,
                }
                match tree.move_to_sibling() {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(_) => break,
                }
                match tree.move_to_parent() {
                    Ok(true) => {}
                    Ok(false) | Err(_) => break,
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

/// Builds a fresh directory holding a truncated copy of a valid segment.
fn truncated_copy(original: &Arc<dyn Directory>, file: &str, length: usize) -> Arc<dyn Directory> {
    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    for extension in EXTENSIONS {
        let name = format!("_0.{extension}");
        let mut bytes = read_file(original, &name);
        if name == file {
            bytes.truncate(length);
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

/// The byte values the data-file sweep tries at each position.
///
/// Every boundary a length, a count, a bits-per-value or a doc-id encoding
/// selector can sit on, rather than all 255: the `.kdd` is mostly packed
/// values and doc IDs, and a byte that names nothing behaves the same for
/// every value it can take.
const BOUNDARY_VALUES: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x07, 0x0f, 0x10, 0x1f, 0x20, 0x40, 0x7f, 0x80, 0x81, 0xc0, 0xfe, 0xff,
];

// ---------------------------------------------------------------------------
// Sweeps
// ---------------------------------------------------------------------------

#[test]
fn every_value_at_every_byte_of_the_metadata_file_is_survived() {
    let mut attempts = 0;
    for fixture in fixtures() {
        let (original, info, _) = build(&fixture);
        let name = "_0.kdm";
        let valid = read_file(&original, name);
        for (at, &original_byte) in valid.iter().enumerate() {
            for value in 0..=u8::MAX {
                if value == original_byte {
                    continue;
                }
                let corrupt = corrupt_copy(&original, name, at, value);
                visit_all(&corrupt, &info, &fixture);
                attempts += 1;
            }
        }
    }
    assert!(
        attempts > 10_000,
        "the metadata sweep must actually run: {attempts} attempts"
    );
}

#[test]
fn every_value_at_every_byte_of_the_index_file_is_survived() {
    let mut attempts = 0;
    for fixture in fixtures() {
        let (original, info, _) = build(&fixture);
        let name = "_0.kdi";
        let valid = read_file(&original, name);
        for (at, &original_byte) in valid.iter().enumerate() {
            for value in 0..=u8::MAX {
                if value == original_byte {
                    continue;
                }
                let corrupt = corrupt_copy(&original, name, at, value);
                visit_all(&corrupt, &info, &fixture);
                attempts += 1;
            }
        }
    }
    assert!(
        attempts > 10_000,
        "the index sweep must actually run: {attempts} attempts"
    );
}

#[test]
fn boundary_values_at_every_byte_of_the_data_file_are_survived() {
    let mut attempts = 0;
    let mut decoded_at_least_once = false;
    for fixture in fixtures() {
        let (original, info, _) = build(&fixture);
        let name = "_0.kdd";
        let valid = read_file(&original, name);
        for (at, &original_byte) in valid.iter().enumerate() {
            for value in BOUNDARY_VALUES {
                if value == original_byte {
                    continue;
                }
                let corrupt = corrupt_copy(&original, name, at, value);
                decoded_at_least_once |= visit_all(&corrupt, &info, &fixture) > 0;
                attempts += 1;
            }
        }
    }
    assert!(
        attempts > 5_000,
        "the data sweep must actually run: {attempts} attempts"
    );
    assert!(
        decoded_at_least_once,
        "a corrupt data file must still reach the leaf decoder, or the sweep proves nothing"
    );
}

#[test]
fn a_truncated_file_is_survived_at_every_length() {
    for fixture in fixtures() {
        let (original, info, _) = build(&fixture);
        for extension in EXTENSIONS {
            let name = format!("_0.{extension}");
            let valid = read_file(&original, &name);
            for length in 0..valid.len() {
                let corrupt = truncated_copy(&original, &name, length);
                visit_all(&corrupt, &info, &fixture);
            }
            assert!(
                !valid.is_empty(),
                "[{}] {name} must exist to be truncated",
                fixture.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Hostile metadata that passes its own checksum
// ---------------------------------------------------------------------------

/// Rewrites the checksum in the last eight bytes so a patched body is accepted.
///
/// `.kdm` is read through a checksumming input, so a plain byte flip is
/// ultimately reported as a checksum mismatch. That does not make the sweep
/// above vacuous — Lucene reads the entries first and checks the footer
/// afterwards, so the decoder still sees the garbage — but it does mean the
/// patched value never reaches the tree. Re-signing closes that gap.
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

/// The hostile values tried for each word of the metadata.
fn hostile_values() -> Vec<i64> {
    vec![
        0,
        1,
        -1,
        2,
        -2,
        i32::MIN as i64,
        i32::MAX as i64,
        i64::MIN,
        i64::MAX,
        1 << 30,
        1 << 31,
        1 << 40,
        -(1 << 40),
        0x7fff_ffff_ffff,
    ]
}

#[test]
fn hostile_metadata_that_passes_its_checksum_is_survived() {
    // Every four- and eight-byte aligned word of the body, given a value chosen
    // to be hostile to a length, a count or an offset, with the footer re-signed
    // so the reader accepts the file and the patched word actually reaches the
    // decoder. This is where an allocation sized off disk shows up.
    let mut attempts = 0;
    for fixture in fixtures() {
        let (original, info, _) = build(&fixture);
        let name = "_0.kdm";
        let valid = read_file(&original, name);
        let body_end = valid.len() - 8;
        for at in (0..body_end.saturating_sub(8)).step_by(4) {
            for value in hostile_values() {
                for width in [4usize, 8] {
                    if at + width > body_end {
                        continue;
                    }
                    let mut patched = valid.clone();
                    if width == 4 {
                        patched[at..at + 4].copy_from_slice(&(value as i32).to_le_bytes());
                    } else {
                        patched[at..at + 8].copy_from_slice(&value.to_le_bytes());
                    }
                    let signed = resign(&patched[..body_end]);
                    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
                    for extension in EXTENSIONS {
                        let file = format!("_0.{extension}");
                        if file == name {
                            write_file(&corrupt, &file, &signed);
                        } else {
                            write_file(&corrupt, &file, &read_file(&original, &file));
                        }
                    }
                    visit_all(&corrupt, &info, &fixture);
                    attempts += 1;
                }
            }
        }
    }
    assert!(
        attempts > 1_000,
        "the hostile-metadata sweep must actually run: {attempts} attempts"
    );
}

// ---------------------------------------------------------------------------
// Directed regressions
// ---------------------------------------------------------------------------

/// Collects `(docID, packedValue)` for every point the reader hands over.
#[derive(Default)]
struct CollectingVisitor {
    points: Vec<(i32, Vec<u8>)>,
}

impl IntersectVisitor for CollectingVisitor {
    fn visit(&mut self, doc_id: i32) -> rucene::error::Result<()> {
        self.points.push((doc_id, Vec::new()));
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> rucene::error::Result<()> {
        self.points.push((doc_id, packed_value.to_vec()));
        Ok(())
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> Relation {
        Relation::CellCrossesQuery
    }
}

/// Collects the doc IDs the bulk whole-cell path hands over.
#[derive(Default)]
struct CollectingInsideVisitor {
    doc_ids: Vec<i32>,
}

impl IntersectVisitor for CollectingInsideVisitor {
    fn visit(&mut self, doc_id: i32) -> rucene::error::Result<()> {
        self.doc_ids.push(doc_id);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, _packed_value: &[u8]) -> rucene::error::Result<()> {
        self.doc_ids.push(doc_id);
        Ok(())
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> Relation {
        Relation::CellInsideQuery
    }
}

/// Opens one field of a segment and intersects it with `visitor`, returning
/// whatever the reader answered.
fn intersect_field(
    directory: &Arc<dyn Directory>,
    info: &SegmentInfo,
    number: i32,
    visitor: &mut dyn IntersectVisitor,
) -> rucene::error::Result<()> {
    let infos = field_infos();
    let read_state = SegmentReadState::new(directory.as_ref(), info, &infos, &*DEFAULT_IO_CONTEXT);
    let reader = codec().points_format().fields_reader(&read_state)?;
    let name = &infos
        .field_info_by_number(number)
        .expect("a known field")
        .name;
    let values = reader.get_values(name)?;
    values.intersect(visitor)
}

/// Replaces `patch.len()` bytes of one file, in place, keeping its length.
fn patched_copy(
    original: &Arc<dyn Directory>,
    file: &str,
    at: usize,
    patch: &[u8],
) -> Arc<dyn Directory> {
    let corrupt: Arc<dyn Directory> = Arc::new(RamDirectory::new());
    for extension in EXTENSIONS {
        let name = format!("_0.{extension}");
        let mut bytes = read_file(original, &name);
        if name == file {
            bytes[at..at + patch.len()].copy_from_slice(patch);
        }
        write_file(&corrupt, &name, &bytes);
    }
    corrupt
}

/// The `deep` shape: sixty one-dimensional points over leaves of eight.
fn deep_fixture() -> Fixture {
    fixtures()
        .into_iter()
        .find(|fixture| fixture.name == "deep")
        .expect("the deep shape exists")
}

/// The `const` shape: sixty two-dimensional points whose dimension 0 never
/// varies, over leaves of eight.
fn const_fixture() -> Fixture {
    fixtures()
        .into_iter()
        .find(|fixture| fixture.name == "const")
        .expect("the const shape exists")
}

/// Offset of the first leaf block's point-count vInt inside `_0.kdd`.
///
/// The data file opens with a codec header and the leaf blocks follow it back
/// to back, so the first byte after the header is the first leaf's count. The
/// tests below assert what they find there before corrupting it, so a change
/// in the header or the fixture fails loudly instead of sweeping a byte that
/// no longer means anything.
const FIRST_LEAF_COUNT_OFFSET: usize = 50;

#[test]
fn a_corrupt_leaf_count_is_refused_on_the_whole_cell_path_too() {
    // `BKDPointTree::addAll` is the third site that reads a leaf's point count,
    // and it is only reached when the visitor accepts the cell whole. It used
    // to size `scratch_doc_ids` straight from the file: a count of -1 became
    // `usize::MAX` and `Vec::resize` aborted the process with "capacity
    // overflow", and a count of 0x0FFFFFFF reserved a gigabyte from five
    // corrupt bytes.
    //
    // Java reads the count into `BKDReaderDocIDSetIterator.docIDs`, a fixed
    // `int[maxPointsInLeafNode]` reused across leaves
    // (`BKDReader.java:1045-1046`), so no count off disk can ever size an
    // allocation there.
    let fixture = deep_fixture();
    let (original, info, _) = build(&fixture);
    let valid = read_file(&original, "_0.kdd");
    assert_eq!(
        valid[FIRST_LEAF_COUNT_OFFSET], fixture.max_points_in_leaf as u8,
        "the first leaf of the deep shape must hold a full block, or this test \
         is corrupting something other than a leaf count"
    );

    // `-1` and `0x0FFFFFFF`, both as five-byte vInts so the file keeps its
    // length: the first is the sign trap, the second the allocation trap.
    for (name, patch) in [
        ("-1", [0xff, 0xff, 0xff, 0xff, 0x0f]),
        ("0x0FFFFFFF", [0xff, 0xff, 0xff, 0xff, 0x00]),
    ] {
        let corrupt = patched_copy(&original, "_0.kdd", FIRST_LEAF_COUNT_OFFSET, &patch);
        let mut visitor = CollectingInsideVisitor::default();
        let error = intersect_field(&corrupt, &info, 0, &mut visitor)
            .expect_err("a leaf count no leaf can hold must be refused");
        assert!(
            matches!(error, rucene::error::LuceneError::CorruptIndex(_)),
            "count={name}: {error:?}"
        );
    }
}

#[test]
fn a_compressed_dimension_with_a_full_common_prefix_is_refused() {
    // The sorted-dimension byte of a leaf names the dimension whose first
    // non-prefix byte is run-length compressed. `BKDWriter` never names a
    // dimension whose common prefix already covers the whole dimension
    // (`assert commonPrefixLengths[sortedDim] < config.bytesPerDim()`,
    // `BKDWriter.java:1345`), but nothing in the file forces that, and the
    // `const` shape has a dimension 0 that is constant in every leaf — so
    // `commonPrefixLengths[0] == bytesPerDim` and naming it makes
    // `bytesPerDim - commonPrefixLengths[compressedDim]` underflow.
    //
    // The guard used to be written over the *global* offset
    // `compressedDim * bytesPerDim + commonPrefixLengths[compressedDim]`
    // against the packed length, which only fires when the named dimension is
    // the last one; naming dimension 0 of a two-dimensional field passed it
    // and the subtraction panicked with "attempt to subtract with overflow".
    let fixture = const_fixture();
    let (original, info, _) = build(&fixture);
    let valid = read_file(&original, "_0.kdd");
    assert_eq!(
        valid[CONST_SORTED_DIM_OFFSET], 1,
        "the first leaf of the const shape must name dimension 1 as its sorted \
         dimension, or this test is corrupting something other than a \
         sorted-dimension byte"
    );

    let corrupt = patched_copy(&original, "_0.kdd", CONST_SORTED_DIM_OFFSET, &[0]);
    let mut visitor = CollectingVisitor::default();
    let error = intersect_field(&corrupt, &info, 1, &mut visitor)
        .expect_err("a compressed dimension with no byte left to compress must be refused");
    assert!(
        matches!(error, rucene::error::LuceneError::CorruptIndex(_)),
        "{error:?}"
    );
}

/// Offset of the first leaf block's sorted-dimension byte in the `const`
/// shape's `_0.kdd`.
///
/// The first leaf block runs: point count, doc IDs, then one
/// `(prefixLength, prefixBytes)` pair per dimension — `4` plus the four bytes
/// of the constant dimension 0, then `3` plus three bytes of dimension 1 —
/// and the sorted-dimension byte follows them. The test asserts the value it
/// finds before corrupting it, so a change in the fixture or in the doc-ID
/// encoding fails loudly instead of flipping a byte that means something else.
const CONST_SORTED_DIM_OFFSET: usize = 78;

#[test]
fn every_valid_shape_is_read_back_exactly_on_both_visitor_paths() {
    // A guard added to shared reader code fails classically by rejecting valid
    // input, and the two guards above sit on the path every leaf takes. The
    // boundary they were written for — a non-final indexed dimension whose
    // common prefix covers the whole dimension — is a *legal* leaf, produced
    // here by `const` in two dimensions and `const3` in three, and the
    // low-cardinality decoder they share a file with is produced by `lowcard`.
    // All of them must still read back exactly.
    for fixture in fixtures() {
        let (directory, info, _) = build(&fixture);
        for (number, expected) in &fixture.fields {
            let mut per_value = CollectingVisitor::default();
            intersect_field(&directory, &info, *number, &mut per_value)
                .unwrap_or_else(|e| panic!("[{}] field {number}: {e}", fixture.name));
            per_value.points.sort_by_key(|(doc, _)| *doc);
            assert_eq!(
                &per_value.points, expected,
                "[{}] field {number} must read back exactly what was written",
                fixture.name
            );

            let mut bulk = CollectingInsideVisitor::default();
            intersect_field(&directory, &info, *number, &mut bulk)
                .unwrap_or_else(|e| panic!("[{}] field {number} whole-cell: {e}", fixture.name));
            bulk.doc_ids.sort_unstable();
            let expected_docs: Vec<i32> = expected.iter().map(|(doc, _)| *doc).collect();
            assert_eq!(
                bulk.doc_ids, expected_docs,
                "[{}] field {number} must yield every doc ID on the whole-cell path",
                fixture.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Contract checks that need no corruption
// ---------------------------------------------------------------------------

#[test]
fn get_values_answers_exactly_as_lucene_does() {
    // `Lucene90PointsReader.getValues` throws `IllegalArgumentException` for a
    // name the field infos do not know and for a field whose info declares no
    // dimensions, and otherwise returns `readers.get(fieldInfo.number)` —
    // which is **null** when the segment carries no metadata entry for that
    // field (`Lucene90PointsReader.java:136`).
    //
    // `PointsReader::get_values` returns `Result<Box<dyn PointValues>>` and
    // has no way to say "null", so this crate answers that third case with an
    // empty view: size 0, docCount 0, no bounds. That is the closest available
    // answer and it is what every caller of Java's null does after checking
    // it. `leaf` writes only field 0, so field 1 is exactly that case.
    let fixture = &fixtures()[0];
    let (directory, info, _) = build(fixture);
    let infos = field_infos();
    let read_state = SegmentReadState::new(directory.as_ref(), &info, &infos, &*DEFAULT_IO_CONTEXT);
    let reader = codec()
        .points_format()
        .fields_reader(&read_state)
        .expect("a valid segment opens");
    reader.check_integrity().expect("integrity");

    let written = reader.get_values("one").unwrap_or_else(|error| {
        panic!(
            "[{}] the field the fixture wrote must be readable: {error}",
            fixture.name
        )
    });
    assert_eq!(written.size(), i64::from(fixture.max_doc));

    let absent = reader
        .get_values("two")
        .expect("a field with no metadata entry is Java's null, not a throw");
    assert_eq!(absent.size(), 0);
    assert_eq!(absent.doc_count(), 0);
    assert!(absent.min_packed_value().expect("no error").is_none());

    assert!(
        reader.get_values("nosuchfield").is_err(),
        "a name the field infos do not know must be refused"
    );
}
