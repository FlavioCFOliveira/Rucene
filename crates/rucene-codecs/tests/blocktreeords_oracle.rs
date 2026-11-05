//! Oracle parity of `org.apache.lucene.codecs.blocktreeords` (rmp #335:
//! `BlockTreeOrdsPostingsFormat`) against Apache Lucene 10.5.0.
//!
//! The Java oracle (`PostingsFormatOracle`, in the workspace harness
//! `crates/randomizedtesting/tests/fixtures/oracle/`) writes the corpus of
//! `SimpleTextOracle` with `Lucene104Codec` routing every field but `e_id` to
//! `BlockTreeOrds`. The tests write the same corpus with Rucene and compare
//! every `.tio` and `.tipo` file (and the postings files the format writes
//! under its name) byte for byte, Lucene's random segment id replaced by
//! Rucene's; read both sides' files with Rucene; run Lucene's reader on
//! Rucene's files; replay the enum trace, whose `ord` line exercises
//! `seekExact(long)` and `ord()`, the format's reason to exist; compare the
//! errors of corrupted copies; and resolve the format's service name.

mod oracle;

use std::sync::Arc;

use oracle::*;
use rucene::codecs::Codec;
use rucene::util::resource_loader::ClassLoader;
use rucene_codecs::blocktreeords::BlockTreeOrdsPostingsFormat;

fn test_class_loader() -> &'static Arc<ClassLoader> {
    rucene_codecs::class_loader()
}

fn block_tree_ords_codec() -> Arc<dyn Codec> {
    per_field_postings_codec(Arc::new(BlockTreeOrdsPostingsFormat::new()))
}

/// `BlockTreeOrds` reads postings with `Lucene104PostingsReader`, whose
/// `impacts` (core defect 1) the dump's `live` lines meet.
const BLOCK_TREE_ORDS: FormatOracle = FormatOracle {
    prefix: "blocktreeords",
    format_name: "BlockTreeOrds",
    extensions: &["tio", "tipo"],
    codec: block_tree_ords_codec,
    extra_jars: &[],
    live_impacts_differ: true,
};

#[test]
fn block_tree_ords_writes_lucenes_bytes() {
    assert!(BLOCK_TREE_ORDS.assert_writes_lucenes_bytes() > 0);
}

#[test]
fn block_tree_ords_reads_what_lucene_reads() {
    BLOCK_TREE_ORDS.assert_reads_what_lucene_reads();
}

#[test]
fn block_tree_ords_enum_trace_matches_lucenes() {
    BLOCK_TREE_ORDS.assert_enum_trace_matches();
}

#[test]
fn lucene_reads_what_block_tree_ords_writes() {
    BLOCK_TREE_ORDS.assert_lucene_reads_what_rucene_writes();
}

#[test]
fn block_tree_ords_corrupted_files_raise_lucenes_errors() {
    BLOCK_TREE_ORDS.assert_corrupted_files_raise_lucenes_errors();
}

/// `PostingsFormat.forName("BlockTreeOrds")` resolves once the crate's
/// service files are loaded; the format's own `toString()` is
/// `BlockTreeOrds(blocksize=128)`.
#[test]
fn block_tree_ords_resolves_by_name() {
    install_codecs();
    let spi = BLOCK_TREE_ORDS.spi();
    let format =
        rucene::codecs::postings_for_name("BlockTreeOrds").expect("BlockTreeOrds resolves");
    assert_eq!(format.name(), spi["getName"]);
    assert_eq!(
        BlockTreeOrdsPostingsFormat::new().to_string(),
        spi["toString"]
    );
    assert_eq!(
        BlockTreeOrdsPostingsFormat::new().to_string(),
        spi["forName.toString"]
    );
    assert_eq!(
        spi["forName.getClass"],
        "org.apache.lucene.codecs.blocktreeords.BlockTreeOrdsPostingsFormat"
    );
}

// ---------------------------------------------------------------------------
// The ord API over every term: `seekExact(long)`, `ord()`, `seekExact(term)`,
// `seekCeil` and `seekExact(term, state)` against the enumeration `next()`
// gives (which the dump above pins to Lucene's), on Lucene's files.
// ---------------------------------------------------------------------------

use rucene::document::{Document, Store, StringField};
use rucene::index::index_writer::IndexWriter;
use rucene::index::{directory_reader, IndexReader, SeekStatus, Terms};
use rucene::util::automaton::compiled_automaton::CompiledAutomaton;
use rucene::util::automaton::reg_exp::RegExp;
use rucene::util::BytesRef;

/// Every term of `terms`, in `next()` order, with the `ord()` after each.
fn all_terms(terms: &dyn Terms) -> Vec<(Vec<u8>, i64)> {
    let mut te = terms.iterator().unwrap();
    let mut out = Vec::new();
    while let Some(t) = te.next().unwrap() {
        out.push((t.slice().to_vec(), te.ord().unwrap()));
    }
    out
}

/// The checks of `TestOrdsBlockTree.testEnum`, made exhaustive (every ord
/// and every term, in both directions, where Lucene draws 1000 random
/// ones), plus `seekCeil` and `seekExact(term, state)` on every term.
fn check_ord_api(terms: &dyn Terms) -> usize {
    let all = all_terms(terms);
    assert_eq!(all.len() as i64, terms.size().unwrap());
    for (i, (_, ord)) in all.iter().enumerate() {
        assert_eq!(*ord, i as i64, "ord() after next()");
    }
    let mut te = terms.iterator().unwrap();
    // seekExact(long), descending as Lucene's loop, then ascending.
    for i in (0..all.len()).rev().chain(0..all.len()) {
        te.seek_ord(i as i64).unwrap();
        assert_eq!(te.ord().unwrap(), i as i64);
        assert_eq!(te.term().unwrap().slice(), &all[i].0[..], "seekExact({i})");
        // `seekExact(long)` leaves the enum unpositioned; `next()` re-seeks.
        let next = te.next().unwrap();
        assert_eq!(
            next.map(|t| t.slice().to_vec()),
            all.get(i + 1).map(|t| t.0.clone())
        );
    }
    // seekExact(term) / seekCeil(term), interleaved with seekExact(long).
    for (i, (t, _)) in all.iter().enumerate().rev().chain(all.iter().enumerate()) {
        assert!(te.seek_exact(&BytesRef::new(t.clone())).unwrap());
        assert_eq!(te.ord().unwrap(), i as i64, "ord after seekExact(term)");
        let j = (i * 7 + 3) % all.len();
        te.seek_ord(j as i64).unwrap();
        assert_eq!(te.term().unwrap().slice(), &all[j].0[..]);
        assert_eq!(
            te.seek_ceil(&BytesRef::new(t.clone())).unwrap(),
            SeekStatus::FOUND
        );
        assert_eq!(te.ord().unwrap(), i as i64, "ord after seekCeil(term)");
        // A target just past the term lands on the next one.
        let mut past = t.clone();
        past.push(0);
        let status = te.seek_ceil(&BytesRef::new(past.clone())).unwrap();
        match all.get(i + 1) {
            Some((n, _)) if *n == past => assert_eq!(status, SeekStatus::FOUND),
            Some((n, _)) => {
                assert_eq!(status, SeekStatus::NOT_FOUND);
                assert_eq!(te.term().unwrap().slice(), &n[..]);
                assert_eq!(te.ord().unwrap(), i as i64 + 1);
            }
            None => assert_eq!(status, SeekStatus::END),
        }
    }
    // termState() carries the ord; seekExact(term, state) restores it.
    let mut states = Vec::new();
    let mut it = terms.iterator().unwrap();
    while it.next().unwrap().is_some() {
        states.push(it.term_state().unwrap());
    }
    let mut fresh = terms.iterator().unwrap();
    for (i, (t, _)) in all.iter().enumerate().rev() {
        fresh
            .seek_term_state(&BytesRef::new(t.clone()), states[i].as_ref())
            .unwrap();
        assert_eq!(
            fresh.ord().unwrap(),
            i as i64,
            "ord after seekExact(term, state)"
        );
        assert_eq!(fresh.term().unwrap().slice(), &t[..]);
    }
    all.len()
}

/// `intersect(compiled, null)` lists exactly the terms the automaton
/// accepts, in order.
fn check_intersect(terms: &dyn Terms, regexps: &[&str]) -> usize {
    let all = all_terms(terms);
    let mut listed = 0;
    for r in regexps {
        let automaton = RegExp::new(r).unwrap().to_automaton().unwrap();
        let compiled = CompiledAutomaton::new(automaton, false, false, false).unwrap();
        let runnable = compiled.get_byte_runnable().unwrap();
        let expected: Vec<Vec<u8>> = all
            .iter()
            .map(|(t, _)| t.clone())
            .filter(|t| {
                let mut s = 0;
                for &b in t {
                    s = runnable.step(s, i32::from(b));
                    if s == -1 {
                        return false;
                    }
                }
                runnable.is_accept(s)
            })
            .collect();
        let mut ie = terms.intersect(&compiled, None).unwrap();
        let mut actual = Vec::new();
        while let Some(t) = ie.next().unwrap() {
            ie.doc_freq().unwrap();
            actual.push(t.slice().to_vec());
        }
        assert_eq!(actual, expected, "intersect {r}");
        listed += actual.len();
    }
    listed
}

/// The ord API over every field of the oracle corpus, on Lucene's files.
#[test]
fn block_tree_ords_ord_api_on_lucenes_files() {
    install_codecs();
    let mut checked = 0;
    for case in ["nocfs", "cfs", "edge"] {
        let dir = TempDir::new(&format!("blocktreeords-ords-{case}"));
        write_files(&dir.0, &java_files("blocktreeords", case));
        let reader: Arc<dyn IndexReader> = directory_reader::open(open_dir(&dir.0)).unwrap();
        for ctx in Arc::clone(&reader).leaves().unwrap() {
            let leaf = ctx.leaf_reader();
            let fis: Vec<_> = leaf.get_field_infos().iter().cloned().collect();
            for fi in fis.iter().filter(|fi| fi.name != "e_id") {
                if let Some(terms) = leaf.terms(&fi.name).unwrap() {
                    checked += check_ord_api(terms.as_ref());
                    check_intersect(terms.as_ref(), &REGEXPS);
                }
            }
        }
    }
    assert!(checked > 1000, "{checked} terms checked");
}

/// An index of one segment whose field `field` holds `docs`, one document
/// per entry, each the listed terms (`StringField`s; Lucene's tests index
/// single-token `TextField`s, which give the same terms).
fn ords_index(dir: &TempDir, docs: &[Vec<String>]) -> Arc<dyn IndexReader> {
    let w = IndexWriter::new(open_dir(&dir.0), new_config(block_tree_ords_codec())).unwrap();
    for terms in docs {
        let mut d = Document::new();
        for t in terms {
            d.add(Box::new(
                StringField::new("field", t.clone(), Store::NO).unwrap(),
            ));
        }
        w.add_document(&d).unwrap();
    }
    w.force_merge(1).unwrap();
    w.commit().unwrap();
    w.close().unwrap();
    directory_reader::open(open_dir(&dir.0)).unwrap()
}

fn field_terms(reader: &Arc<dyn IndexReader>) -> Box<dyn Terms> {
    let leaves = Arc::clone(reader).leaves().unwrap();
    assert_eq!(leaves.len(), 1);
    leaves[0].leaf_reader().terms("field").unwrap().unwrap()
}

fn one_per_doc(terms: &[String]) -> Vec<Vec<String>> {
    terms.iter().map(|t| vec![t.clone()]).collect()
}

fn ch(c: u32) -> String {
    char::from_u32(c).unwrap().to_string()
}

const INTERSECTIONS: [&str; 7] = ["m.*", "a.", ".*a", "[d-k]", "mo[c-x]", "b.*|m[q-t]", ".*"];

/// `TestOrdsBlockTree.testBasic`.
#[test]
fn test_basic() {
    let dir = TempDir::new("blocktreeords-basic");
    let r = ords_index(&dir, &[vec!["a".into(), "b".into(), "c".into()]]);
    let terms = field_terms(&r);
    let mut te = terms.iterator().unwrap();
    for (i, t) in ["a", "b", "c"].iter().enumerate() {
        assert_eq!(te.next().unwrap().unwrap().slice(), t.as_bytes());
        assert_eq!(te.ord().unwrap(), i as i64);
    }
    assert!(te.next().unwrap().is_none());
    for (t, ord) in [("b", 1), ("a", 0), ("c", 2)] {
        assert!(te
            .seek_exact(&BytesRef::new(t.as_bytes().to_vec()))
            .unwrap());
        assert_eq!(te.ord().unwrap(), ord);
    }
    for (ord, t) in [(1, "b"), (0, "a"), (2, "c")] {
        te.seek_ord(ord).unwrap();
        assert_eq!(te.term().unwrap().slice(), t.as_bytes());
    }
    check_ord_api(terms.as_ref());
}

/// `TestOrdsBlockTree.testTwoBlocks` and `testThreeBlocks`.
#[test]
fn test_two_and_three_blocks() {
    for blocks in [2, 3] {
        let mut terms: Vec<String> = (0..36).map(|i| ch(97 + i)).collect();
        terms.extend((0..36).map(|i| format!("m{}", ch(97 + i))));
        if blocks == 3 {
            terms.extend((0..36).map(|i| format!("mo{}", ch(97 + i))));
        }
        let dir = TempDir::new(&format!("blocktreeords-blocks{blocks}"));
        let r = ords_index(&dir, &one_per_doc(&terms));
        let t = field_terms(&r);
        let mut te = t.iterator().unwrap();
        assert!(te.seek_exact(&BytesRef::new(b"mo".to_vec())).unwrap());
        assert_eq!(te.ord().unwrap(), 27);
        te.seek_ord(if blocks == 2 { 54 } else { 90 }).unwrap();
        assert_eq!(te.term().unwrap().slice(), b"s");
        assert_eq!(check_ord_api(t.as_ref()), terms.len());
        check_intersect(t.as_ref(), &INTERSECTIONS);
    }
}

/// `TestOrdsBlockTree.testFloorBlocks`.
#[test]
fn test_floor_blocks() {
    let terms: Vec<String> = (0..128).map(ch).collect();
    let dir = TempDir::new("blocktreeords-floor");
    let r = ords_index(&dir, &one_per_doc(&terms));
    let t = field_terms(&r);
    let mut te = t.iterator().unwrap();
    assert!(te.seek_exact(&BytesRef::new(b"a".to_vec())).unwrap());
    assert_eq!(te.ord().unwrap(), 97);
    te.seek_ord(98).unwrap();
    assert_eq!(te.term().unwrap().slice(), b"b");
    assert!(te.seek_exact(&BytesRef::new(b"z".to_vec())).unwrap());
    assert_eq!(te.ord().unwrap(), 122);
    assert_eq!(check_ord_api(t.as_ref()), 128);
    check_intersect(t.as_ref(), &INTERSECTIONS);
}

/// `TestOrdsBlockTree.testNonRootFloorBlocks`.
#[test]
fn test_non_root_floor_blocks() {
    let mut terms: Vec<String> = (0..36).map(|i| ch(97 + i)).collect();
    terms.extend((0..128).map(|i| format!("m{}", ch(i))));
    let dir = TempDir::new("blocktreeords-nonroot-floor");
    let r = ords_index(&dir, &one_per_doc(&terms));
    let t = field_terms(&r);
    assert_eq!(check_ord_api(t.as_ref()), terms.len());
    check_intersect(t.as_ref(), &INTERSECTIONS);
}

/// `TestOrdsBlockTree.testSeveralNonRootBlocks`.
#[test]
fn test_several_non_root_blocks() {
    let mut terms = Vec::new();
    for i in 0..30 {
        for j in 0..30 {
            terms.push(format!("{}{}", ch(97 + i), ch(97 + j)));
        }
    }
    let dir = TempDir::new("blocktreeords-several");
    let r = ords_index(&dir, &one_per_doc(&terms));
    let t = field_terms(&r);
    let mut te = t.iterator().unwrap();
    for (k, term) in terms.iter().enumerate() {
        assert_eq!(te.next().unwrap().unwrap().slice(), term.as_bytes());
        assert_eq!(te.ord().unwrap(), k as i64);
    }
    assert_eq!(check_ord_api(t.as_ref()), terms.len());
    te.seek_ord(0).unwrap();
    assert_eq!(te.term().unwrap().slice(), b"aa");
    check_intersect(t.as_ref(), &INTERSECTIONS);
}

/// `TestOrdsBlockTree.testSeekCeilNotFound`.
#[test]
fn test_seek_ceil_not_found() {
    let mut docs = vec![vec![String::new()]];
    for i in 0..36 {
        docs.push(vec![ch(97 + i), format!("a{}", ch(97 + i))]);
    }
    let dir = TempDir::new("blocktreeords-ceil");
    let r = ords_index(&dir, &docs);
    let t = field_terms(&r);
    let mut te = t.iterator().unwrap();
    assert_eq!(
        te.seek_ceil(&BytesRef::new(vec![0x22])).unwrap(),
        SeekStatus::NOT_FOUND
    );
    assert_eq!(te.term().unwrap().slice(), b"a");
    assert_eq!(te.ord().unwrap(), 1);
    check_ord_api(t.as_ref());
}
