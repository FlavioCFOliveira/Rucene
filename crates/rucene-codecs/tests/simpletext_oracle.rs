//! Oracle parity of the `SimpleText` codec (rmp #332, #333) against
//! Apache Lucene 10.5.0's `lucene-codecs`.
//!
//! The Java oracle (`SimpleTextOracle` and `SimpleTextReadOracle`, in the
//! workspace harness `crates/randomizedtesting/tests/fixtures/oracle/src/
//! org/apache/lucene/codecs/simpletext/`) writes three indexes with
//! `SimpleTextCodec` and records them in `tests/fixtures/`:
//!
//! * `nocfs` and `cfs`: the documents and operations of
//!   `tests/portability/index_interop.rs` (text with positions, offsets and
//!   payloads, term vectors, stored values of every type, the five doc-values
//!   types, points, float and byte vectors; deletes by term and by query,
//!   doc-values updates, several commits, `forceMerge(2)`,
//!   `forceMergeDeletes()`), without and with compound files;
//! * `edge`: twelve documents carrying every value kind at its limits
//!   (`Long.MIN_VALUE`/`MAX_VALUE`, NaN and signed zeros, empty and non-ASCII
//!   terms and strings, empty fields, every index option and term-vector
//!   option, the doc-values skip index, missing values, points of 1, 3, 4 and
//!   16 bytes per dimension with fewer index than data dimensions, vectors of
//!   every similarity), with a soft-deletes field and an index sort.
//!
//! The tests write the same indexes with Rucene and compare every file byte
//! for byte; read Lucene's files and Rucene's with Rucene and compare what is
//! read with what Lucene read; run Lucene's reader (`SimpleTextReadOracle`) on
//! Rucene's files; and compare the errors corrupted copies raise.
//!
//! # Determinism
//!
//! A `SimpleText` file carries no segment id, so the bytes are a function of
//! the documents and operations only, except two files: `segments_N`
//! (Lucene's own binary format, which holds the segment ids and is not
//! written by this codec) and the `.si`, whose id line holds the random
//! segment id and whose diagnostics hold the environment (operating system,
//! JVM, timestamp) and, in Java, iterate in an order drawn per JVM
//! (`Map.copyOf`; see `SegmentInfo::get_diagnostics`). The `.si` files are
//! therefore compared line by line with the id line skipped and the
//! diagnostics compared as a set of keys, with the values of the
//! environment keys skipped; the format-level `.si` test of
//! `simpletext_formats.rs` pins the id and the diagnostics and compares
//! bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::Codec;
use rucene::index::directory_reader;
use rucene::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, IndexOptions, LeafReader, StoredFieldVisitor,
    StoredFieldVisitorStatus, VectorEncoding, VectorSimilarityFunction,
};
use rucene::search::knn::TopKnnCollector;
use rucene::search::{BitsAcceptDocs, Sort, SortField, SortFieldType, NO_MORE_DOCS};
use rucene::store::Directory;
use rucene_codecs::simpletext::SimpleTextCodec;

mod oracle;

use oracle::*;

/// The loader `oracle::install_codecs` installs: this crate's service files.
fn test_class_loader() -> &'static Arc<rucene::util::resource_loader::ClassLoader> {
    rucene_codecs::class_loader()
}

fn simpletext() -> Arc<dyn Codec> {
    Arc::new(SimpleTextCodec::new())
}

/// `oracle::java_files` of `simpletext_files.tsv`.
fn java_files(case: &str) -> BTreeMap<String, Vec<u8>> {
    oracle::java_files("simpletext", case)
}

/// `oracle::java_reads` of `simpletext_reads.tsv`.
fn java_reads(case: &str) -> Vec<String> {
    oracle::java_reads("simpletext", case)
}

// ---------------------------------------------------------------------------
// Format-level files: SimpleTextOracle.writeFormats
// ---------------------------------------------------------------------------

/// `SimpleTextOracle.SEGMENT_ID`.
const SEGMENT_ID: [u8; 16] = [
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2, 0xe1, 0xf0,
];

/// `SimpleTextOracle.ArrayTree`: a mutable point tree over arrays.
struct ArrayTree {
    values: Vec<Vec<u8>>,
    docs: Vec<i32>,
    saved_values: Vec<Vec<u8>>,
    saved_docs: Vec<i32>,
}

impl ArrayTree {
    fn new(values: Vec<Vec<u8>>, docs: Vec<i32>) -> Self {
        let n = values.len();
        Self {
            values,
            docs,
            saved_values: vec![Vec::new(); n],
            saved_docs: vec![0; n],
        }
    }
}

impl rucene::index::PointTree for ArrayTree {
    fn clone_tree(&self) -> rucene::error::Result<Box<dyn rucene::index::PointTree>> {
        Err(rucene::error::LuceneError::unsupported_operation(
            String::new(),
        ))
    }

    fn move_to_child(&mut self) -> rucene::error::Result<bool> {
        Ok(false)
    }

    fn move_to_sibling(&mut self) -> rucene::error::Result<bool> {
        Ok(false)
    }

    fn move_to_parent(&mut self) -> rucene::error::Result<bool> {
        Ok(false)
    }

    fn min_packed_value(&self) -> rucene::error::Result<&[u8]> {
        Err(rucene::error::LuceneError::unsupported_operation(
            String::new(),
        ))
    }

    fn max_packed_value(&self) -> rucene::error::Result<&[u8]> {
        Err(rucene::error::LuceneError::unsupported_operation(
            String::new(),
        ))
    }

    fn size(&self) -> rucene::error::Result<i64> {
        Ok(self.values.len() as i64)
    }

    fn visit_doc_ids(
        &mut self,
        _visitor: &mut dyn rucene::index::IntersectVisitor,
    ) -> rucene::error::Result<()> {
        Err(rucene::error::LuceneError::unsupported_operation(
            String::new(),
        ))
    }

    fn visit_doc_values(
        &mut self,
        visitor: &mut dyn rucene::index::IntersectVisitor,
    ) -> rucene::error::Result<()> {
        for i in 0..self.values.len() {
            visitor.visit_with_value(self.docs[i], &self.values[i])?;
        }
        Ok(())
    }

    fn as_mutable(&mut self) -> Option<&mut dyn rucene::index::MutablePointTree> {
        Some(self)
    }
}

impl rucene::index::MutablePointTree for ArrayTree {
    fn value(&self, i: i32) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(&self.values[i as usize])
    }

    fn byte_at(&self, i: i32, k: i32) -> u8 {
        self.values[i as usize][k as usize]
    }

    fn doc_id(&self, i: i32) -> i32 {
        self.docs[i as usize]
    }

    fn swap(&mut self, i: i32, j: i32) {
        self.values.swap(i as usize, j as usize);
        self.docs.swap(i as usize, j as usize);
    }

    fn save(&mut self, i: i32, j: i32) {
        self.saved_values[j as usize] = self.values[i as usize].clone();
        self.saved_docs[j as usize] = self.docs[i as usize];
    }

    fn restore(&mut self, i: i32, j: i32) {
        for k in i as usize..j as usize {
            self.values[k] = self.saved_values[k].clone();
            self.docs[k] = self.saved_docs[k];
        }
    }
}

/// `SimpleTextOracle.formatValue`.
fn format_value(dims: i32, i: i32) -> Vec<u8> {
    if dims == 1 {
        let v = (i * 7919) % 100003;
        return v.to_be_bytes().to_vec();
    }
    let d = [(i * 31) % 997, (i * 17) % 1009, (i * 13) % 251];
    d.iter().flat_map(|v| [(v >> 8) as u8, *v as u8]).collect()
}

fn format_config(dims: i32) -> rucene::util::bkd::BKDConfig {
    if dims == 1 {
        rucene::util::bkd::BKDConfig::new(1, 1, 4, 512).unwrap()
    } else {
        rucene::util::bkd::BKDConfig::new(3, 2, 2, 512).unwrap()
    }
}

/// `SimpleTextOracle.writeBkd`.
fn write_bkd(dir: &Arc<dyn Directory>, file: &str, dims: i32, n: i32, max_mb: f64, mutable: bool) {
    use rucene_codecs::simpletext::bkd_writer::SimpleTextBKDWriter;
    let mut out = dir
        .create_output(file, &*rucene::store::DEFAULT_IO_CONTEXT)
        .unwrap();
    let mut w = SimpleTextBKDWriter::new(
        n,
        Arc::clone(dir),
        "_9",
        format_config(dims),
        max_mb,
        i64::from(n),
    )
    .unwrap();
    let fp = if mutable {
        let values = (0..n).map(|i| format_value(dims, i)).collect();
        let docs = (0..n).map(|i| (i * 37) % n).collect();
        let mut tree = ArrayTree::new(values, docs);
        w.write_field(out.as_mut(), "f", &mut tree).unwrap()
    } else {
        for i in 0..n {
            w.add(&format_value(dims, i), (i * 37) % n).unwrap();
        }
        w.finish(out.as_mut()).unwrap()
    };
    w.close().unwrap();
    rucene_codecs::simpletext::util::write_str(out.as_mut(), &format!("fp {fp}")).unwrap();
    rucene_codecs::simpletext::util::write_newline(out.as_mut()).unwrap();
    out.close().unwrap();
}

#[allow(clippy::too_many_arguments)]
fn format_field_info(
    name: &str,
    number: i32,
    io: IndexOptions,
    tv: bool,
    omit_norms: bool,
    payloads: bool,
    dv: DocValuesType,
    skip: DocValuesSkipIndexType,
    dv_gen: i64,
    points: (i32, i32, i32),
    vector: (i32, VectorEncoding, VectorSimilarityFunction),
    soft: bool,
    parent: bool,
) -> FieldInfo {
    FieldInfo::new_full(
        name,
        number,
        tv,
        omit_norms,
        payloads,
        io,
        dv,
        skip,
        dv_gen,
        rucene::util::java_collections::JavaStringMap::default(),
        points.0,
        points.1,
        points.2,
        vector.0,
        vector.1,
        vector.2,
        soft,
        parent,
    )
    .unwrap()
}

/// The pinned segment info of `SimpleTextOracle.writeFormats`.
fn format_segment_info(dir: &Arc<dyn Directory>) -> rucene::index::SegmentInfo {
    let si = rucene::index::SegmentInfo::new(
        Arc::clone(dir),
        rucene::util::Version::LATEST,
        Some(rucene::util::Version::from_bits(9, 12, 0).unwrap()),
        "_9".to_string(),
        7,
        true,
        false,
        Arc::new(SimpleTextCodec::new()),
        [("source".to_string(), "flush".to_string())]
            .into_iter()
            .collect(),
        SEGMENT_ID,
        std::collections::HashMap::new(),
        Some(
            Sort::new_fields(vec![
                SortField::new_reverse(Some("s".to_string()), SortFieldType::String, true).unwrap(),
                rucene::search::SortedNumericSortField::new("n", SortFieldType::Long)
                    .unwrap()
                    .into(),
            ])
            .unwrap(),
        ),
    )
    .unwrap();
    si.put_attribute("b", "2");
    si.put_attribute("a", "1");
    si.put_attribute("SimpleText.key", "line\nvalue");
    si.set_files(
        [
            "_9.inf", "_9.fld", "_9.pst", "_9.dat", "_9.tvc", "_9.len", "_9.vec",
        ]
        .iter()
        .map(|s| s.to_string()),
    )
    .unwrap();
    si
}

/// The field infos of `SimpleTextOracle.writeFormats`.
fn format_field_infos() -> rucene::index::FieldInfos {
    use DocValuesSkipIndexType as S;
    use DocValuesType as D;
    use VectorEncoding as E;
    use VectorSimilarityFunction as V;
    let all = format_field_info(
        "f_all",
        0,
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
        true,
        false,
        true,
        D::SORTED_SET,
        S::RANGE,
        3,
        (3, 2, 4),
        (8, E::BYTE, V::COSINE),
        false,
        false,
    );
    all.put_attribute("k2", "v2");
    all.put_attribute("k1", "v1");
    all.put_attribute("PerFieldPostingsFormat.format", "SimpleText");
    rucene::index::FieldInfos::new(vec![
        all,
        format_field_info(
            "f_custom",
            1,
            IndexOptions::DOCS_AND_CUSTOM_FREQS,
            false,
            true,
            false,
            D::NUMERIC,
            S::NONE,
            -1,
            (0, 0, 0),
            (0, E::FLOAT32, V::EUCLIDEAN),
            false,
            false,
        ),
        format_field_info(
            "f_soft",
            2,
            IndexOptions::NONE,
            false,
            false,
            false,
            D::NUMERIC,
            S::NONE,
            -1,
            (0, 0, 0),
            (0, E::FLOAT32, V::EUCLIDEAN),
            true,
            false,
        ),
        format_field_info(
            "f_parent",
            3,
            IndexOptions::NONE,
            false,
            false,
            false,
            D::NUMERIC,
            S::NONE,
            -1,
            (0, 0, 0),
            (0, E::FLOAT32, V::EUCLIDEAN),
            false,
            true,
        ),
        format_field_info(
            "f_mip",
            4,
            IndexOptions::NONE,
            false,
            false,
            false,
            D::NONE,
            S::NONE,
            -1,
            (0, 0, 0),
            (16, E::FLOAT32, V::MAXIMUM_INNER_PRODUCT),
            false,
            false,
        ),
        format_field_info(
            "f_dot",
            5,
            IndexOptions::DOCS,
            false,
            false,
            false,
            D::BINARY,
            S::NONE,
            9,
            (0, 0, 0),
            (2, E::FLOAT32, V::DOT_PRODUCT),
            false,
            false,
        ),
    ])
    .unwrap()
}

/// `SimpleTextOracle.writeFormats`.
fn write_formats(path: &Path) {
    let dir = open_dir(path);
    let codec = SimpleTextCodec::new();
    let si = format_segment_info(&dir);
    codec
        .segment_info_format()
        .write(dir.as_ref(), &si, &*rucene::store::DEFAULT_IO_CONTEXT)
        .unwrap();
    let infos = format_field_infos();
    for suffix in ["", "3"] {
        codec
            .field_infos_format()
            .write(
                dir.as_ref(),
                &si,
                suffix,
                &infos,
                &*rucene::store::DEFAULT_IO_CONTEXT,
            )
            .unwrap();
    }
    write_bkd(&dir, "_9.bkd1m", 1, 1200, 16.0, true);
    write_bkd(&dir, "_9.bkdnm", 3, 1100, 16.0, true);
    write_bkd(&dir, "_9.bkd1o", 1, 3000, 0.01, false);
    write_bkd(&dir, "_9.bkdno", 3, 2500, 0.02, false);
}

fn write_case(case: &str, path: &Path) {
    match case {
        "formats" => write_formats(path),
        "nocfs" => write_corpus(path, false, simpletext()),
        "cfs" => write_corpus(path, true, simpletext()),
        "edge" => write_edge(path, simpletext()),
        _ => unreachable!("{case}"),
    }
}

const ENVIRONMENT_KEYS: [&str; 6] = [
    "os",
    "os.arch",
    "os.version",
    "java.vendor",
    "java.runtime.version",
    "timestamp",
];

/// A `.si` without what varies between two writers of the same segment: the
/// id line and the checksum line are dropped, and the diagnostics block is
/// replaced by its sorted keys, each with its value unless the key is an
/// environment key.
fn normalized_si(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(n) = line.strip_prefix("    diagnostics ") {
            let n: usize = n.parse().unwrap();
            out.push(line.to_string());
            let mut diag = BTreeSet::new();
            for k in 0..n {
                let key = lines[i + 1 + 2 * k].strip_prefix("      key ").unwrap();
                let value = lines[i + 2 + 2 * k].strip_prefix("      value ").unwrap();
                if ENVIRONMENT_KEYS.contains(&key) {
                    diag.insert(format!("{key}=<env>"));
                } else {
                    diag.insert(format!("{key}={value}"));
                }
            }
            out.extend(diag);
            i += 1 + 2 * n;
            continue;
        }
        if line.starts_with("    id ") || line.starts_with("checksum ") {
            i += 1;
            continue;
        }
        out.push(line.to_string());
        i += 1;
    }
    out
}

fn assert_same_files(case: &str, rucene: &BTreeMap<String, Vec<u8>>) {
    let java = java_files(case);
    let java_names: Vec<&String> = java.keys().collect();
    let rucene_names: Vec<&String> = rucene.keys().collect();
    assert_eq!(rucene_names, java_names, "{case}: file listing");
    let mut compared = 0;
    for (name, java_bytes) in &java {
        let rust_bytes = &rucene[name];
        if name.starts_with("segments_") {
            continue;
        }
        if name.ends_with(".si") && case != "formats" {
            assert_eq!(
                normalized_si(rust_bytes),
                normalized_si(java_bytes),
                "{case}: {name} (normalized)"
            );
            continue;
        }
        if rust_bytes != java_bytes {
            let r = String::from_utf8_lossy(rust_bytes);
            let j = String::from_utf8_lossy(java_bytes);
            let line = r
                .lines()
                .zip(j.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            panic!(
                "{case}: {name} differs from Lucene's at line {}:\n rucene: {:?}\n lucene: {:?}",
                line + 1,
                r.lines().nth(line),
                j.lines().nth(line)
            );
        }
        compared += 1;
    }
    assert!(compared > 0);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Rucene writes the bytes Lucene writes, for every file of the three
/// indexes.
#[test]
fn rucene_writes_lucenes_bytes() {
    for case in ["formats", "nocfs", "cfs", "edge"] {
        let tmp = TempDir::new(&format!("simpletext-write-{case}"));
        write_case(case, &tmp.0);
        assert_same_files(case, &read_dir(&tmp.0));
    }
}

/// Rucene reads Lucene's files and its own and sees what Lucene sees.
#[test]
fn rucene_reads_what_lucene_reads() {
    for case in ["nocfs", "cfs", "edge"] {
        let expected = java_reads(case);
        let java_dir = TempDir::new(&format!("simpletext-java-{case}"));
        write_files(&java_dir.0, &java_files(case));
        assert_same_lines(
            &format!("{case}: Lucene's files"),
            &dump(&java_dir.0),
            &expected,
        );

        let rust_dir = TempDir::new(&format!("simpletext-rust-{case}"));
        write_case(case, &rust_dir.0);
        assert_same_lines(
            &format!("{case}: Rucene's files"),
            &dump(&rust_dir.0),
            &expected,
        );
    }
}

/// Lucene reads the files Rucene writes and sees what it sees in its own
/// (`java -cp lucene-core:lucene-codecs SimpleTextReadOracle.java <dir>`).
#[test]
fn lucene_reads_what_rucene_writes() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../randomizedtesting/tests/fixtures/oracle/src/org/apache/lucene/codecs/simpletext/\
         SimpleTextReadOracle.java",
    );
    let classpath = format!(
        "{}:{}",
        lucene_jar("lucene-core").display(),
        lucene_jar("lucene-codecs").display()
    );
    for case in ["nocfs", "cfs", "edge"] {
        let rust_dir = TempDir::new(&format!("simpletext-forjava-{case}"));
        write_case(case, &rust_dir.0);
        let output = Command::new("java")
            .arg("-ea")
            .arg("-cp")
            .arg(&classpath)
            .arg(&source)
            .arg(&rust_dir.0)
            .output()
            .expect("a JDK is required to run the Lucene oracle");
        assert!(
            output.status.success(),
            "SimpleTextReadOracle failed on Rucene's {case} index:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let lines: Vec<String> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_same_lines(
            &format!("{case}: Lucene on Rucene's files"),
            &lines,
            &java_reads(case),
        );
    }
}

/// `SimpleTextOracle.lastDigitBumped`.
fn last_digit_bumped(bytes: &[u8]) -> Vec<u8> {
    let mut b = bytes.to_vec();
    let at = b.len() - 2;
    b[at] = b'0' + ((b[at] - b'0' + 1) % 10);
    b
}

/// `SimpleTextOracle.replaceLast`.
fn replace_last(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let at = bytes
        .windows(from.len())
        .rposition(|w| w == from.as_bytes())
        .unwrap_or_else(|| panic!("no {from}"));
    let mut out = bytes[..at].to_vec();
    out.extend_from_slice(to.as_bytes());
    out.extend_from_slice(&bytes[at + from.len()..]);
    out
}

/// A corrupted copy of Lucene's `nocfs` index raises the exception Lucene
/// raises, with Lucene's message.
#[test]
fn corrupted_files_raise_lucenes_errors() {
    install_codecs();
    let expected: BTreeMap<String, (String, String)> = rows("simpletext_errors.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), (r[1].clone(), r[2].clone())))
        .collect();
    type Edit = fn(&[u8]) -> Vec<u8>;
    let cases: [(&str, &str, Edit); 6] = [
        ("pst-checksum", "pst", last_digit_bumped),
        ("fld-trailing", "fld", |b| {
            let mut c = b.to_vec();
            c.push(b'x');
            c
        }),
        ("inf-word", "inf", |b| {
            replace_last(b, "checksum ", "chksum   ")
        }),
        ("si-id", "si", |b| replace_last(b, "    id [", "    id [0 ")),
        ("si-version", "si", |b| {
            replace_last(b, "    version 1", "    version x")
        }),
        ("dat-checksum", "dat", last_digit_bumped),
    ];
    let java = java_files("nocfs");
    for (name, ext, edit) in cases {
        let tmp = TempDir::new(&format!("simpletext-corrupt-{name}"));
        write_files(&tmp.0, &java);
        let infos = rucene::index::SegmentInfos::read_latest_commit(open_dir(&tmp.0)).unwrap();
        let segment = infos.info(0).info.name.clone();
        let id =
            rucene::util::string_helper::StringHelper::id_to_string(Some(&infos.info(0).info.id()));
        let file = tmp.0.join(format!("{segment}.{ext}"));
        std::fs::write(&file, edit(&std::fs::read(&file).unwrap())).unwrap();
        let actual = match directory_reader::open(open_dir(&tmp.0)) {
            Ok(_) => ("none".to_string(), "-".to_string()),
            Err(e) => {
                let class = match &e {
                    rucene::error::LuceneError {
                        kind: rucene::error::LuceneErrorKind::CorruptIndex { .. },
                        ..
                    } => "org.apache.lucene.index.CorruptIndexException".to_string(),
                    other => format!("rucene::{}", other.java_simple_name()),
                };
                let message = match &e {
                    rucene::error::LuceneError {
                        kind: rucene::error::LuceneErrorKind::CorruptIndex { message: m, .. },
                        ..
                    } => m.clone(),
                    other => other.to_string(),
                };
                (
                    class,
                    message
                        .replace(&tmp.0.to_string_lossy().into_owned(), "$D")
                        .replace(&id, "$ID")
                        .replace('\n', " "),
                )
            }
        };
        assert_eq!(&actual, &expected[name], "{name}");
    }
}

/// `Codec.forName("SimpleText")` resolves once the crate's service files are
/// loaded, and its formats carry Lucene's names.
#[test]
fn codec_for_name_resolves() {
    install_codecs();
    let expected: BTreeMap<String, String> = rows("simpletext_spi.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), r[1].clone()))
        .collect();
    let codec = rucene::codecs::for_name("SimpleText").expect("SimpleText resolves");
    assert_eq!(codec.name(), expected["Codec.forName(SimpleText)"]);
    assert_eq!(codec.postings_format().name(), expected["postingsFormat"]);
    assert_eq!(
        codec.doc_values_format().name(),
        expected["docValuesFormat"]
    );
    assert_eq!(
        codec.knn_vectors_format().name(),
        expected["knnVectorsFormat"]
    );
    // `SimpleTextPostingsFormat` is no service of its own in Lucene either.
    assert_eq!(
        expected["PostingsFormat.forName(SimpleText)"],
        "java.lang.IllegalArgumentException"
    );
    assert!(rucene::codecs::postings_for_name("SimpleText").is_none());
}

/// Escaping round trip of the fixture encoding, so that a fixture decoding
/// defect cannot hide a byte difference.
#[test]
fn fixture_escape_round_trips() {
    let all: Vec<u8> = (0..=255u8).collect();
    assert_eq!(unescape(&escape(&all)), all);
}

/// Rucene reads Lucene's pinned segment info and field infos back to the
/// values Lucene wrote.
#[test]
fn rucene_reads_lucenes_segment_and_field_infos() {
    let tmp = TempDir::new("simpletext-formats-read");
    write_files(&tmp.0, &java_files("formats"));
    let dir = open_dir(&tmp.0);
    let codec = SimpleTextCodec::new();
    let read = codec
        .segment_info_format()
        .read(
            Arc::clone(&dir),
            "_9",
            &SEGMENT_ID,
            &*rucene::store::DEFAULT_IO_CONTEXT,
        )
        .unwrap();
    let expected = format_segment_info(&dir);
    expected.add_file("_9.si".to_string()).unwrap();
    assert_eq!(read.version(), expected.version());
    assert_eq!(read.min_version(), expected.min_version());
    assert_eq!(read.max_doc().unwrap(), 7);
    assert!(read.get_use_compound_file());
    assert!(!read.get_has_blocks());
    assert_eq!(
        read.get_diagnostics().to_std(),
        expected.get_diagnostics().to_std()
    );
    assert_eq!(read.get_attributes(), expected.get_attributes());
    assert_eq!(read.files().unwrap(), expected.files().unwrap());
    assert_eq!(read.id(), SEGMENT_ID);
    assert_eq!(read.index_sort(), expected.index_sort());

    let infos = codec
        .field_infos_format()
        .read(
            dir.as_ref(),
            &read,
            "3",
            &*rucene::store::DEFAULT_IO_CONTEXT,
        )
        .unwrap();
    let expected = format_field_infos();
    let read_all: Vec<&FieldInfo> = infos.iter().collect();
    let expected_all: Vec<&FieldInfo> = expected.iter().collect();
    assert_eq!(read_all, expected_all);
    for (r, e) in read_all.iter().zip(expected_all.iter()) {
        assert_eq!(
            r.attributes().to_std(),
            e.attributes().to_std(),
            "{}",
            r.name
        );
    }
}

// ---------------------------------------------------------------------------
// The unchecked exceptions of the readers: SimpleTextOracle.uncheckedCases
// ---------------------------------------------------------------------------

/// `SimpleTextOracle.replaceFirst`.
fn replace_first(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let at = bytes
        .windows(from.len())
        .position(|w| w == from.as_bytes())
        .unwrap_or_else(|| panic!("no {from}"));
    let mut out = bytes[..at].to_vec();
    out.extend_from_slice(to.as_bytes());
    out.extend_from_slice(&bytes[at + from.len()..]);
    out
}

/// `SimpleTextOracle.fixChecksum`: the last `checksum` line rewritten as
/// `SimpleTextUtil.writeChecksum` writes it for the bytes before it.
fn fix_checksum(bytes: &[u8]) -> Vec<u8> {
    let from = b"checksum ";
    let at = bytes
        .windows(from.len())
        .rposition(|w| w == from)
        .expect("a checksum line");
    let mut crc = rucene::store::BufferedChecksum::new();
    crc.update_bytes(bytes, 0, at).unwrap();
    let mut out = bytes[..at].to_vec();
    out.extend_from_slice(format!("checksum {:020}\n", crc.get_value()).as_bytes());
    out
}

/// `SimpleTextOracle.readState` over `dir`.
fn with_read_state<T>(
    dir: &dyn rucene::store::Directory,
    leaf: &dyn LeafReader,
    f: impl FnOnce(&rucene::codecs::SegmentReadState<'_>) -> rucene::error::Result<T>,
) -> rucene::error::Result<T> {
    let segment = leaf.as_segment_reader().expect("a segment reader");
    let field_infos = leaf.get_field_infos();
    let state = rucene::codecs::SegmentReadState {
        directory: dir,
        segment_info: &segment.get_segment_info().info,
        field_infos: &field_infos,
        context: &*rucene::store::DEFAULT_IO_CONTEXT,
        segment_suffix: String::new(),
    };
    f(&state)
}

/// Java's `getClass().getName()` and `String.valueOf(getMessage())` of `e`,
/// as `SimpleTextOracle` prints them: the shared mapping of the test
/// framework, with `null` for a `null` message.
fn java_class_and_message(e: &rucene::error::LuceneError) -> (String, String) {
    use rucene_test_framework::analysis::base_token_stream_test_case::java_message;
    (
        rucene_test_framework::util::java_exceptions::java_class_name(e),
        java_message(e).unwrap_or_else(|| "null".to_string()),
    )
}

/// The class and message a panic stands for: the `class: message` of a Java
/// exception the port panics with, or the `AssertionError` of a debug
/// assertion.
fn java_class_and_message_of_panic(payload: &(dyn std::any::Any + Send)) -> (String, String) {
    let text = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default();
    match text.split_once(": ") {
        Some((class, message)) if class.starts_with("java.") => {
            (class.to_string(), message.to_string())
        }
        _ => ("java.lang.AssertionError".to_string(), text),
    }
}

/// An edit of one file's bytes.
type Edit = fn(&[u8]) -> Vec<u8>;

/// An operation on the first leaf of an index, returning what it read.
type LeafOp = fn(&Path, &dyn LeafReader) -> rucene::error::Result<String>;

/// `SimpleTextOracle.unchecked`: the row of one case, `class, message,
/// cause class, cause message` or `none, result`.
fn unchecked_case(name: &str, file: Option<(&str, Edit)>, op: LeafOp) -> Vec<String> {
    let tmp = TempDir::new(&format!("simpletext-unchecked-{name}"));
    write_files(&tmp.0, &java_files("nocfs"));
    let infos = rucene::index::SegmentInfos::read_latest_commit(open_dir(&tmp.0)).unwrap();
    let segment = infos.info(0).info.name.clone();
    let id =
        rucene::util::string_helper::StringHelper::id_to_string(Some(&infos.info(0).info.id()));
    if let Some((file, edit)) = file {
        let path = if file.starts_with('_') {
            tmp.0.join(file)
        } else {
            tmp.0.join(format!("{segment}.{file}"))
        };
        std::fs::write(&path, edit(&std::fs::read(&path).unwrap())).unwrap();
    }
    let real = tmp.0.to_string_lossy().into_owned();
    let mask = |s: String| {
        s.replace(&real, "$D")
            .replace(&id, "$ID")
            .replace('\n', " ")
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let reader = directory_reader::open(open_dir(&tmp.0))?;
        let leaves = Arc::clone(&reader).leaves()?;
        let leaf = leaves[0].leaf_reader();
        op(&tmp.0, leaf.as_ref())
    }));
    let mut row = vec![name.to_string()];
    match outcome {
        Ok(Ok(result)) => row.extend(["none".to_string(), result]),
        Ok(Err(e)) => {
            let (class, message) = java_class_and_message(&e);
            row.extend([class, mask(message)]);
            // `#[source]` hands out the `Arc` the cause is kept in.
            let cause = std::error::Error::source(&e).and_then(|c| {
                c.downcast_ref::<Arc<rucene::error::LuceneError>>()
                    .map(|c| c.as_ref())
                    .or_else(|| c.downcast_ref::<rucene::error::LuceneError>())
            });
            match cause {
                Some(cause) => {
                    let (class, message) = java_class_and_message(cause);
                    row.extend([class, mask(message)]);
                }
                None => row.extend(["-".to_string(), "-".to_string()]),
            }
        }
        Err(payload) => {
            let (class, message) = java_class_and_message_of_panic(payload.as_ref());
            row.extend([class, mask(message), "-".to_string(), "-".to_string()]);
        }
    }
    row
}

/// The visitor of the direct stored-fields reads: every field, nothing kept.
struct AllFields;

impl StoredFieldVisitor for AllFields {
    fn needs_field(
        &mut self,
        _field_info: &FieldInfo,
    ) -> rucene::error::Result<StoredFieldVisitorStatus> {
        Ok(StoredFieldVisitorStatus::Yes)
    }
}

fn segment(leaf: &dyn LeafReader) -> &rucene::index::segment_reader::SegmentReader {
    leaf.as_segment_reader().expect("a segment reader")
}

/// `String.valueOf(SimpleTextTerms)`.
fn terms_to_string(terms: &dyn rucene::index::Terms) -> rucene::error::Result<String> {
    Ok(format!(
        "SimpleTextTerms(terms={},postings={},positions={},docs={})",
        terms.size()?,
        terms.sum_doc_freq()?,
        terms.sum_total_term_freq()?,
        terms.doc_count()?
    ))
}

/// The `.pst` whose field `tv` is renamed `tx`, which the field infos do not
/// know, read through `SimpleTextFieldsReader` itself.
fn unknown_field_terms(
    p: &Path,
    leaf: &dyn LeafReader,
    f: fn(&dyn rucene::index::Terms) -> rucene::error::Result<String>,
) -> rucene::error::Result<String> {
    use rucene::index::Fields;
    let dir = open_dir(p);
    with_read_state(dir.as_ref(), leaf, |state| {
        let fields = rucene_codecs::simpletext::fields_reader::SimpleTextFieldsReader::new(state)?;
        let terms = fields.terms("tx")?.expect("the renamed field");
        f(terms.as_ref())
    })
}

fn pst_rename(b: &[u8]) -> Vec<u8> {
    fix_checksum(&replace_first(b, "field tv\n", "field tx\n"))
}

/// The rows `java -da` writes where a Java `assert` decides the outcome
/// (the fixture is written with `-ea`), for release builds. Measured with
/// `java -da -cp <oracle classes>:<lucene jars>
/// org.apache.lucene.codecs.simpletext.SimpleTextOracle <out>`.
const DISABLED_ASSERTIONS_ROWS: [&str; 4] = [
    "pst-doc-out\tjava.lang.ArrayIndexOutOfBoundsException\tIndex 1562 out of bounds for length 3\t-\t-",
    "pst-doc-neg\tjava.lang.ArrayIndexOutOfBoundsException\tIndex -1 out of bounds for length 3\t-\t-",
    "pst-doc-ghost\tnone\t151",
    "vec-short-line\tjava.lang.NegativeArraySizeException\t-1\t-\t-",
];

/// Every unchecked exception the SimpleText readers throw on a copy of the
/// `nocfs` index, edited or misused, is the one Lucene throws: class,
/// message, and cause.
#[test]
fn unchecked_exceptions_are_lucenes() {
    install_codecs();
    let mut expected: BTreeMap<String, Vec<String>> = rows("simpletext_unchecked.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), r))
        .collect();
    if !cfg!(debug_assertions) {
        for row in DISABLED_ASSERTIONS_ROWS {
            let row: Vec<String> = row.split('\t').map(str::to_string).collect();
            expected.insert(row[0].clone(), row);
        }
    }
    type Case = (&'static str, Option<(&'static str, Edit)>, LeafOp);
    let cases: Vec<Case> = vec![
        ("dv-num-exhausted", None, |_, leaf| {
            let mut v = leaf.get_numeric_doc_values("dv_num")?.unwrap();
            while v.next_doc()? != NO_MORE_DOCS {}
            Ok(v.long_value()?.to_string())
        }),
        ("dv-bin-exhausted", None, |_, leaf| {
            let mut v = leaf.get_binary_doc_values("dv_bin")?.unwrap();
            while v.next_doc()? != NO_MORE_DOCS {}
            Ok(v.binary_value()?.to_string())
        }),
        ("dv-sorted-ord-neg", None, |_, leaf| {
            Ok(leaf
                .get_sorted_doc_values("dv_sorted")?
                .unwrap()
                .lookup_ord(-1)?
                .to_string())
        }),
        ("dv-sorted-ord-count", None, |_, leaf| {
            let v = leaf.get_sorted_doc_values("dv_sorted")?.unwrap();
            Ok(v.lookup_ord(v.get_value_count()?)?.to_string())
        }),
        ("dv-sset-ord-neg", None, |_, leaf| {
            Ok(leaf
                .get_sorted_set_doc_values("dv_sset")?
                .unwrap()
                .lookup_ord(-1)?
                .to_string())
        }),
        ("dv-sset-ord-count", None, |_, leaf| {
            let v = leaf.get_sorted_set_doc_values("dv_sset")?.unwrap();
            Ok(v.lookup_ord(v.get_value_count()?)?.to_string())
        }),
        ("dv-snum-past", None, |_, leaf| {
            let mut v = leaf.get_sorted_numeric_doc_values("dv_snum")?.unwrap();
            v.next_doc()?;
            for _ in 0..v.doc_value_count()? {
                v.next_value()?;
            }
            Ok(v.next_value()?.to_string())
        }),
        ("dv-sset-past", None, |_, leaf| {
            let mut v = leaf.get_sorted_set_doc_values("dv_sset")?.unwrap();
            v.next_doc()?;
            for _ in 0..v.doc_value_count()? {
                v.next_ord()?;
            }
            Ok(v.next_ord()?.to_string())
        }),
        (
            "dv-num-corrupt",
            Some(("dat", |b| {
                replace_first(b, "  pattern 0000\n1042\n", "  pattern 0000\nx042\n")
            })),
            |_, leaf| {
                let mut v = leaf.get_numeric_doc_values("dv_num")?.unwrap();
                v.next_doc()?;
                Ok(v.long_value()?.to_string())
            },
        ),
        (
            "dv-bin-corrupt",
            Some(("dat", |b| replace_first(b, "length 19\n", "length x9\n"))),
            |_, leaf| {
                let mut v = leaf.get_binary_doc_values("dv_bin")?.unwrap();
                v.next_doc()?;
                Ok(v.binary_value()?.to_string())
            },
        ),
        ("fld-doc-past", None, |_, leaf| {
            let reader = segment(leaf).get_fields_reader()?.unwrap();
            reader.document(leaf.max_doc(), &mut AllFields)?;
            Ok("read".to_string())
        }),
        ("fld-doc-neg", None, |_, leaf| {
            let reader = segment(leaf).get_fields_reader()?.unwrap();
            reader.document(-1, &mut AllFields)?;
            Ok("read".to_string())
        }),
        (
            "fld-extra-doc",
            Some(("fld", |b| replace_first(b, "doc 0\n", "doc 0\ndoc 0\n"))),
            |_, _| Ok("opened".to_string()),
        ),
        (
            "fld-unknown-type",
            Some(("fld", |b| {
                fix_checksum(&replace_first(b, "    type string\n", "    type strinx\n"))
            })),
            |_, leaf| {
                let reader = segment(leaf).get_fields_reader()?.unwrap();
                reader.document(0, &mut AllFields)?;
                Ok("read".to_string())
            },
        ),
        (
            "fld-unknown-field",
            Some(("fld", |b| {
                fix_checksum(&replace_first(b, "  field 0\n", "  field 99\n"))
            })),
            |_, leaf| {
                leaf.stored_fields()?.document(0)?;
                Ok("read".to_string())
            },
        ),
        ("tvc-doc-past", None, |_, leaf| {
            let reader = segment(leaf).get_term_vectors_reader()?.unwrap();
            Ok(reader
                .get(leaf.max_doc())?
                .map_or("null", |_| "fields")
                .to_string())
        }),
        (
            "tvc-extra-doc",
            Some(("tvc", |b| replace_first(b, "doc 0\n", "doc 0\ndoc 0\n"))),
            |_, _| Ok("opened".to_string()),
        ),
        ("tvc-term-unpositioned", None, |_, leaf| {
            let reader = segment(leaf).get_term_vectors_reader()?.unwrap();
            let fields = reader.get(0)?.unwrap();
            Ok(fields
                .terms("tv")?
                .unwrap()
                .iterator()?
                .term()?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?
                .to_string())
        }),
        ("tvc-ttf-unpositioned", None, |_, leaf| {
            let reader = segment(leaf).get_term_vectors_reader()?.unwrap();
            let fields = reader.get(0)?.unwrap();
            let mut te = fields.terms("tv")?.unwrap().iterator()?;
            Ok(te.total_term_freq()?.to_string())
        }),
        ("pst-term-unpositioned", None, |_, leaf| {
            Ok(leaf
                .terms("body")?
                .unwrap()
                .iterator()?
                .term()?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?
                .to_string())
        }),
        ("pst-term-exhausted", None, |_, leaf| {
            let mut te = leaf.terms("body")?.unwrap().iterator()?;
            while te.next()?.is_some() {}
            Ok(te
                .term()?
                .ok_or_else(rucene::error::LuceneError::null_pointer)?
                .to_string())
        }),
        (
            "pst-doc-out",
            Some(("pst", |b| {
                fix_checksum(&replace_first(b, "    doc 3\n", "    doc 99999\n"))
            })),
            |_, leaf| Ok(leaf.terms("body")?.unwrap().doc_count()?.to_string()),
        ),
        (
            "pst-doc-neg",
            Some(("pst", |b| {
                fix_checksum(&replace_first(b, "    doc 3\n", "    doc -1\n"))
            })),
            |_, leaf| Ok(leaf.terms("body")?.unwrap().doc_count()?.to_string()),
        ),
        (
            "pst-doc-ghost",
            Some(("pst", |b| {
                fix_checksum(&replace_first(b, "    doc 3\n", "    doc 191\n"))
            })),
            |_, leaf| Ok(leaf.terms("body")?.unwrap().doc_count()?.to_string()),
        ),
        ("pst-field-unknown", Some(("pst", pst_rename)), |p, leaf| {
            unknown_field_terms(p, leaf, terms_to_string)
        }),
        (
            "pst-field-unknown-iterator",
            Some(("pst", pst_rename)),
            |p, leaf| {
                unknown_field_terms(p, leaf, |t| {
                    t.iterator()?;
                    Ok("iterated".to_string())
                })
            },
        ),
        (
            "pst-field-unknown-has-freqs",
            Some(("pst", pst_rename)),
            |p, leaf| unknown_field_terms(p, leaf, |t| Ok(t.has_freqs().to_string())),
        ),
        (
            "pst-field-unknown-has-offsets",
            Some(("pst", pst_rename)),
            |p, leaf| unknown_field_terms(p, leaf, |t| Ok(t.has_offsets().to_string())),
        ),
        (
            "pst-field-unknown-has-positions",
            Some(("pst", pst_rename)),
            |p, leaf| unknown_field_terms(p, leaf, |t| Ok(t.has_positions().to_string())),
        ),
        (
            "pst-field-unknown-has-payloads",
            Some(("pst", pst_rename)),
            |p, leaf| unknown_field_terms(p, leaf, |t| Ok(t.has_payloads().to_string())),
        ),
        (
            "gri-field-unknown",
            Some(("gri", |b| {
                fix_checksum(&replace_first(b, "field-name vb\n", "field-name vz\n"))
            })),
            |_, _| Ok("opened".to_string()),
        ),
        (
            "vec-short-line",
            Some(("vec", |b| replace_first(b, "[20, -30, 0, -20]\n", "[\n"))),
            |_, leaf| {
                let v = leaf.get_byte_vector_values("vb")?.unwrap();
                Ok(format!("{:?}", v.vector_value(0)?))
            },
        ),
        ("knn-search-unknown", None, |p, leaf| {
            use rucene::codecs::KnnVectorsReader;
            let dir = open_dir(p);
            with_read_state(dir.as_ref(), leaf, |state| {
                let vectors =
                    rucene_codecs::simpletext::knn_vectors_reader::SimpleTextKnnVectorsReader::new(
                        state,
                    )?;
                let mut collector = TopKnnCollector::new(3, i64::from(i32::MAX));
                let mut accept = BitsAcceptDocs::from_live_docs(None, leaf.max_doc())?;
                vectors.search("nope", &[1.0, 2.0, 3.0, 4.0], &mut collector, &mut accept)?;
                Ok("searched".to_string())
            })
        }),
        ("knn-search-bytes-unknown", None, |p, leaf| {
            use rucene::codecs::KnnVectorsReader;
            let dir = open_dir(p);
            with_read_state(dir.as_ref(), leaf, |state| {
                let vectors =
                    rucene_codecs::simpletext::knn_vectors_reader::SimpleTextKnnVectorsReader::new(
                        state,
                    )?;
                let mut collector = TopKnnCollector::new(3, i64::from(i32::MAX));
                let mut accept = BitsAcceptDocs::from_live_docs(None, leaf.max_doc())?;
                vectors.search_byte("nope", &[1, 2, 3, 4], &mut collector, &mut accept)?;
                Ok("searched".to_string())
            })
        }),
        (
            "liv-negative",
            Some(("_1_1.liv", |b| replace_first(b, "  doc 0\n", "  doc -1\n"))),
            |_, _| Ok("opened".to_string()),
        ),
        ("fld-clone-closed", None, |p, leaf| {
            use rucene::codecs::StoredFieldsReader;
            let dir = open_dir(p);
            with_read_state(dir.as_ref(), leaf, |state| {
                let mut reader =
                    rucene_codecs::simpletext::stored_fields_reader::SimpleTextStoredFieldsReader::new(
                        state.directory,
                        state.segment_info,
                        state.field_infos,
                        state.context,
                    )?;
                reader.close()?;
                reader.clone_reader()?;
                Ok("cloned".to_string())
            })
        }),
        ("tvc-clone-closed", None, |p, leaf| {
            use rucene::codecs::TermVectorsReader;
            let dir = open_dir(p);
            with_read_state(dir.as_ref(), leaf, |state| {
                let mut reader =
                    rucene_codecs::simpletext::term_vectors_reader::SimpleTextTermVectorsReader::new(
                        state.directory,
                        state.segment_info,
                        state.context,
                    )?;
                reader.close()?;
                reader.clone_reader()?;
                Ok("cloned".to_string())
            })
        }),
    ];
    assert_eq!(
        cases.iter().map(|c| c.0).collect::<BTreeSet<_>>(),
        expected.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        "the cases are the oracle's"
    );
    for (name, file, op) in cases {
        let mut actual = unchecked_case(name, file, op);
        let mut want = expected[name].clone();
        // `CorruptIndexException` keeps no cause in the port, so the
        // `ParseException` Java chains under it is not compared.
        if name == "dv-bin-corrupt" {
            actual.truncate(3);
            want.truncate(3);
        }
        assert_eq!(actual, want, "{name}");
    }
}

/// Regression (rmp #351): `SimpleTextCompoundFormat.write` writes inside a
/// try-with-resources over the `.scf` output
/// (`SimpleTextCompoundFormat.java:171-212`), so a body that fails — here
/// the segment names a file that does not exist — still closes the output:
/// `MockDirectoryWrapper` then lets it be deleted, which it refuses for an
/// open file.
#[test]
fn a_failed_compound_write_closes_the_output() {
    use randomizedtesting::jdk::random::JdkRandom;
    use rucene::codecs::CompoundFormat;
    use rucene::index::SegmentInfo;
    use rucene::store::{ByteBuffersDirectory, DEFAULT_IO_CONTEXT};
    use rucene_test_framework::store::MockDirectoryWrapper;

    let mdw = MockDirectoryWrapper::new(&JdkRandom::new(0), Arc::new(ByteBuffersDirectory::new()));
    mdw.set_assert_no_delete_open_file(true);
    let dir = mdw.as_directory();
    let info = SegmentInfo::new(
        Arc::clone(&dir),
        rucene::util::Version::LATEST,
        Some(rucene::util::Version::LATEST),
        "_0".to_string(),
        1,
        false,
        false,
        Arc::new(SimpleTextCodec::new()),
        std::collections::HashMap::new(),
        [0u8; 16],
        std::collections::HashMap::new(),
        None,
    )
    .unwrap();
    info.set_files(["_0.missing".to_string()]).unwrap();
    rucene_codecs::simpletext::compound_format::SimpleTextCompoundFormat::new()
        .write(dir.as_ref(), &info, &*DEFAULT_IO_CONTEXT)
        .expect_err("the segment's file does not exist");
    dir.delete_file("_0.scf")
        .expect("the .scf output was closed");
}
