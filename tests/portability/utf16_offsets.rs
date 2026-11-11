//! Offsets in UTF-16 code units through the index, between Rucene and Apache
//! Lucene Core 10.5.0 (rmp #357, E9 block 3).
//!
//! Java's offsets count `char`s, UTF-16 code units, so a supplementary
//! character advances them by two; they are stored in the postings of a field
//! indexed with `DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS` and in its term
//! vectors. The documents of `Utf16OffsetsFixture.java`
//! (`tests/fixtures/java-codec-harness`) are supplementary text analyzed by
//! `StandardAnalyzer`, one value per document, then one document with two
//! values (the offset gap between values) and one with every text.
//!
//! * Lucene writes `L` (`Utf16OffsetsFixture write`); Rucene reads every
//!   posting and term vector of `L` and must print what Lucene prints
//!   (`Utf16OffsetsFixture dump`).
//! * Rucene writes the same documents into `R`; Lucene's dump of `R` and
//!   Rucene's own must both equal Lucene's dump of `L`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::analysis::StandardAnalyzer;
use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, Field, FieldType, Store, StringField};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::{IndexOptions, IndexReader, PostingsEnum, StoredFields, POSTINGS_ENUM_ALL};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

/// `Utf16OffsetsFixture.TEXTS`.
const TEXTS: [&str; 10] = [
    "x\u{10400}y \u{1D11E}\u{1D11E} zz \u{10428}",
    "\u{130}STANBUL \u{3A3}\u{391}\u{3A3} \u{C0}\u{D6}\u{F8}\u{4E00}\u{9FCC}",
    "\u{29B05}\u{8271}\u{935F}\u{43F9}\u{612F}\u{701B}",
    "\u{1F4A9} \u{1F4A9}\u{1F4A9}",
    "poo\u{1F4A9}poo",
    "\u{1F4A9}\u{4E2D}\u{570B}\u{1F4A9}",
    "12345678\u{10300}",
    "ab cd toolong xy z",
    "A bc defg hiJklmn opqrstuv wxy z ",
    "  \t\r\n  leading whitespace",
];

/// `Utf16OffsetsFixture.DOCS`.
const DOCS: usize = TEXTS.len() + 2;

/// `Utf16OffsetsFixture.bodyType()`.
fn body_type() -> FieldType {
    let t = FieldType::new();
    t.set_tokenized(true).unwrap();
    t.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
        .unwrap();
    t.set_store_term_vectors(true).unwrap();
    t.set_store_term_vector_positions(true).unwrap();
    t.set_store_term_vector_offsets(true).unwrap();
    t.freeze();
    t
}

/// `Utf16OffsetsFixture.doc(int)`.
fn doc(i: usize) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", i.to_string(), Store::YES).unwrap(),
    ));
    let body = |text: &str| Box::new(Field::new("body", text.to_string(), body_type()).unwrap());
    if i < TEXTS.len() {
        d.add(body(TEXTS[i]));
    } else if i == TEXTS.len() {
        d.add(body(TEXTS[0]));
        d.add(body(TEXTS[3]));
    } else {
        for text in TEXTS {
            d.add(body(text));
        }
    }
    d
}

/// `Utf16OffsetsFixture.write`.
fn write(path: &Path) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let config = IndexWriterConfig::with_analyzer(Some(Arc::new(StandardAnalyzer::new())));
    config.set_open_mode(OpenMode::CREATE).unwrap();
    let w = IndexWriter::new(dir, config).unwrap();
    for i in 0..DOCS {
        w.add_document(&doc(i)).unwrap();
    }
    w.commit().unwrap();
    w.close().unwrap();
}

fn hex(b: &BytesRef) -> String {
    b.slice().iter().map(|x| format!("{x:02x}")).collect()
}

fn stored_id(stored: &dyn StoredFields, doc: i32) -> String {
    stored
        .document(doc)
        .unwrap()
        .get_field("id")
        .and_then(|f| f.string_value())
        .unwrap_or_else(|| "null".to_string())
}

/// `Utf16OffsetsFixture.positions`.
fn positions(pe: &mut dyn PostingsEnum) -> String {
    let freq = pe.freq().unwrap();
    let mut s = format!(" freq={freq}");
    for _ in 0..freq {
        let pos = pe.next_position().unwrap();
        write!(s, " [{pos},{},{}]", pe.start_offset(), pe.end_offset()).unwrap();
    }
    s
}

/// `Utf16OffsetsFixture.dump`, run by Rucene.
fn rucene_dump(path: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(path).unwrap());
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let stored = reader.stored_fields().unwrap();
    let mut out = String::new();
    for ctx in Arc::clone(&reader).leaves().unwrap() {
        let Some(terms) = ctx.leaf_reader().terms("body").unwrap() else {
            continue;
        };
        let mut te = terms.iterator().unwrap();
        while let Some(text) = te.next().unwrap() {
            let mut pe = te.postings(None, POSTINGS_ENUM_ALL).unwrap().unwrap();
            let mut d = pe.next_doc().unwrap();
            while d != NO_MORE_DOCS {
                let id = stored_id(stored.as_ref(), ctx.doc_base() + d);
                writeln!(out, "post {} id={id}{}", hex(&text), positions(&mut *pe)).unwrap();
                d = pe.next_doc().unwrap();
            }
        }
    }
    let vectors = reader.term_vectors().unwrap();
    for d in 0..reader.max_doc() {
        let id = stored_id(stored.as_ref(), d);
        let terms = match vectors.get(d).unwrap() {
            Some(fields) => fields.terms("body").unwrap(),
            None => None,
        };
        let Some(terms) = terms else {
            writeln!(out, "tv id={id} none").unwrap();
            continue;
        };
        writeln!(
            out,
            "tv id={id} offsets={} positions={}",
            terms.has_offsets(),
            terms.has_positions()
        )
        .unwrap();
        let mut te = terms.iterator().unwrap();
        while let Some(text) = te.next().unwrap() {
            let mut pe = te.postings(None, POSTINGS_ENUM_ALL).unwrap().unwrap();
            pe.next_doc().unwrap();
            writeln!(out, " {}{}", hex(&text), positions(&mut *pe)).unwrap();
        }
    }
    out
}

fn lucene_jar() -> PathBuf {
    let home = std::env::var("HOME").expect("HOME is set");
    let jar = Path::new(&home)
        .join(".m2/repository/org/apache/lucene/lucene-core/10.5.0/lucene-core-10.5.0.jar");
    assert!(
        jar.is_file(),
        "the Lucene 10.5.0 jar is required: {}",
        jar.display()
    );
    jar
}

/// Runs `Utf16OffsetsFixture <mode> <dir>` and returns its standard output.
fn run_java(mode: &str, dir: &Path) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         Utf16OffsetsFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .arg(mode)
        .arg(dir)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "Utf16OffsetsFixture {mode} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("the fixture prints ASCII")
}

/// The first line where `left` and `right` differ, or `None`.
fn first_difference(left: &str, right: &str) -> Option<String> {
    let (l, r): (Vec<&str>, Vec<&str>) = (left.lines().collect(), right.lines().collect());
    for i in 0..l.len().max(r.len()) {
        let (a, b) = (l.get(i), r.get(i));
        if a != b {
            return Some(format!("line {}:\n  java ={a:?}\n  rucene={b:?}", i + 1));
        }
    }
    None
}

#[test]
fn offsets_in_utf16_units_cross_the_index_both_ways() {
    let root = tempfile::tempdir().unwrap();
    let lucene_index = root.path().join("L");
    let rucene_index = root.path().join("R");
    std::fs::create_dir_all(&lucene_index).unwrap();
    std::fs::create_dir_all(&rucene_index).unwrap();

    run_java("write", &lucene_index);
    let expected = run_java("dump", &lucene_index);
    // The corpus reaches supplementary offsets: the first document's last
    // token, U+10428, starts at UTF-16 unit 13 (character 10).
    assert!(
        expected.contains("tv id=0 offsets=true positions=true\n"),
        "{expected}"
    );
    assert!(
        expected.contains(" f09090a8 freq=1 [2,13,15]\n"),
        "{expected}"
    );

    // Rucene reads Lucene's index.
    if let Some(d) = first_difference(&expected, &rucene_dump(&lucene_index)) {
        panic!("Rucene's reading of Lucene's index differs at {d}");
    }

    // Lucene and Rucene read Rucene's index.
    write(&rucene_index);
    if let Some(d) = first_difference(&expected, &run_java("dump", &rucene_index)) {
        panic!("Lucene's reading of Rucene's index differs at {d}");
    }
    if let Some(d) = first_difference(&expected, &rucene_dump(&rucene_index)) {
        panic!("Rucene's reading of its own index differs at {d}");
    }
}
