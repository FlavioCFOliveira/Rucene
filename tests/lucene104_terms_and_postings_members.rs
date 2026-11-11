//! Members of the `Lucene104` terms and postings readers that the
//! `BasePostingsFormatTestCase` suites reach (rmp #351):
//!
//! * `BlockPostingsEnum.intoBitSet` (`Lucene104PostingsReader.java:970-1033`)
//!   and `docIDRunEnd` (`:1100-1128`) over packed and unary (dense) blocks.
//! * `SegmentTermsEnum.seekExact(BytesRef, TermState)`
//!   (`SegmentTermsEnum.java:1016-1035`, rmp #245) fed by the states
//!   `IntersectTermsEnum.termState()` hands out, whose `termBlockOrd` is the
//!   number of terms its frame decoded (`IntersectTermsEnumFrame.java:322`).

use std::sync::Arc;

use rucene::codecs::{register_codec, BlockTermState, Lucene104Codec};
use rucene::document::{Document, Store, StringField};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::postings_enum::POSTINGS_ENUM_NONE;
use rucene::index::LeafReader;
use rucene::search::{DocIdSetIterator, NO_MORE_DOCS};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::{BytesRef, FixedBitSet};

/// One segment over `docs`, each document a list of `(field, value)`.
fn index(docs: &[Vec<(&str, String)>]) -> (Arc<dyn Directory>, Arc<dyn DirectoryReader>) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    for fields in docs {
        let mut doc = Document::new();
        for (name, value) in fields {
            doc.add(Box::new(
                StringField::new(name, value.clone(), Store::NO).unwrap(),
            ));
        }
        writer.add_document(&doc).unwrap();
    }
    writer.force_merge(1).unwrap();
    writer.close().unwrap();
    drop(writer);
    let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
    (dir, reader)
}

fn only_leaf(reader: &Arc<dyn DirectoryReader>) -> Arc<dyn LeafReader> {
    let leaves = Arc::clone(reader).leaves().unwrap();
    assert_eq!(leaves.len(), 1);
    leaves[0].leaf_reader()
}

/// The docs `intoBitSet` loads equal those `nextDoc` walks, over a term whose
/// postings mix dense (unary) and sparse (packed) blocks, from several
/// starting points and bounds, and the enum stops on the first doc at or
/// above the bound.
#[test]
fn into_bit_set_matches_next_doc_over_unary_and_packed_blocks() {
    // Docs 0..2000 hold "t" (dense), then every third doc up to 6000 (packed).
    let max_doc = 6000;
    let docs: Vec<Vec<(&str, String)>> = (0..max_doc)
        .map(|d| {
            if d < 2000 || d % 3 == 0 {
                vec![("f", "t".to_string())]
            } else {
                vec![("f", "u".to_string())]
            }
        })
        .collect();
    let (_dir, reader) = index(&docs);
    let leaf = only_leaf(&reader);
    let terms = leaf.terms("f").unwrap().unwrap();
    let expected: Vec<i32> = (0..max_doc).filter(|d| *d < 2000 || d % 3 == 0).collect();
    for (start, up_to) in [
        (0, 300),
        (0, 2000),
        (5, 1999),
        (100, 4500),
        (1990, 6000),
        (0, NO_MORE_DOCS),
    ] {
        let mut te = terms.iterator().unwrap();
        assert!(te.seek_exact(&BytesRef::from_char_sequence("t")).unwrap());
        let mut postings = te.postings(None, POSTINGS_ENUM_NONE).unwrap().unwrap();
        postings.advance(start).unwrap();
        let offset = postings.doc_id();
        let mut bits = FixedBitSet::new((max_doc - offset) as usize + 1);
        postings.into_bit_set(up_to, &mut bits, offset).unwrap();
        let got: Vec<i32> = (0..bits.length())
            .filter(|&i| bits.get(i))
            .map(|i| i as i32 + offset)
            .collect();
        let want: Vec<i32> = expected
            .iter()
            .copied()
            .filter(|&d| d >= offset && d < up_to)
            .collect();
        assert_eq!(got, want, "start={start} up_to={up_to}");
        let next = expected
            .iter()
            .copied()
            .find(|&d| d >= up_to)
            .unwrap_or(NO_MORE_DOCS);
        assert_eq!(postings.doc_id(), next, "start={start} up_to={up_to}");
    }
    drop((terms, leaf));
    reader.close().unwrap();
}

/// In a run of dense blocks, `docIDRunEnd` reports the end of the dense
/// level-0 (or level-1) block, beyond `docID() + 1`, and every doc below it
/// matches; outside dense blocks it is `docID() + 1`.
#[test]
fn doc_id_run_end_spans_dense_blocks() {
    let max_doc = 20_000;
    let docs: Vec<Vec<(&str, String)>> = (0..max_doc)
        .map(|d| {
            if d < 10_000 || d % 7 == 0 {
                vec![("f", "t".to_string())]
            } else {
                vec![("f", "u".to_string())]
            }
        })
        .collect();
    let (_dir, reader) = index(&docs);
    let leaf = only_leaf(&reader);
    let terms = leaf.terms("f").unwrap().unwrap();
    let mut te = terms.iterator().unwrap();
    assert!(te.seek_exact(&BytesRef::from_char_sequence("t")).unwrap());
    let mut postings = te.postings(None, POSTINGS_ENUM_NONE).unwrap().unwrap();
    assert_eq!(postings.next_doc().unwrap(), 0);
    let run_end = postings.doc_id_run_end().unwrap();
    assert!(run_end > 1, "a dense block runs past doc 0: {run_end}");
    assert!(run_end <= 10_000, "{run_end}");
    postings.advance(12_005).unwrap();
    assert_eq!(postings.doc_id(), 12_005);
    assert_eq!(postings.doc_id_run_end().unwrap(), 12_006);
    drop((postings, te, terms, leaf));
    reader.close().unwrap();
}

/// rmp #245: a state an `IntersectTermsEnum` hands out records how many terms
/// of its block were decoded (`IntersectTermsEnumFrame.java:322`), and a
/// `SegmentTermsEnum` positioned by it (`SegmentTermsEnum.java:1016-1035`)
/// answers that term's statistics and postings; a later `next()` re-seeks to
/// the term and steps past it (`SegmentTermsEnum.java:900-912`).
#[test]
fn seek_exact_by_an_intersect_state_positions_the_terms_enum() {
    use rucene::util::automaton::automata::Automata;
    use rucene::util::automaton::CompiledAutomaton;
    // 100 terms over 250 docs: term `t{i}` is in docs i, i + 100, i + 200.
    let docs: Vec<Vec<(&str, String)>> = (0..250)
        .map(|d| vec![("f", format!("t{:03}", d % 100))])
        .collect();
    let (_dir, reader) = index(&docs);
    let leaf = only_leaf(&reader);
    let terms = leaf.terms("f").unwrap().unwrap();
    let wanted = ["t005", "t050", "t097"];
    let automaton =
        Automata::make_string_union(wanted.iter().map(|t| BytesRef::new(t.as_bytes().to_vec())))
            .unwrap();
    let compiled = CompiledAutomaton::new(automaton, true, false, false).unwrap();
    let mut intersect = terms.intersect(&compiled, None).unwrap();
    let mut states = Vec::new();
    while let Some(term) = intersect.next().unwrap() {
        let state = intersect.term_state().unwrap();
        let block_state = state.as_any().downcast_ref::<BlockTermState>().unwrap();
        assert!(block_state.term_block_ord > 0, "{block_state:?}");
        states.push((term, state));
    }
    assert_eq!(states.len(), 3);
    for (term, state) in &states {
        let i: i32 = std::str::from_utf8(term.slice()).unwrap()[1..]
            .parse()
            .unwrap();
        let expected: Vec<i32> = (0..250).filter(|d| d % 100 == i).collect();
        let mut te = terms.iterator().unwrap();
        // A prior seek, so that the terms input is open and `next()` re-seeks.
        assert!(te.seek_exact(&BytesRef::new(b"t000".to_vec())).unwrap());
        te.seek_term_state(term, state.as_ref()).unwrap();
        assert_eq!(te.term().unwrap(), Some(term.clone()));
        assert_eq!(te.doc_freq().unwrap(), expected.len() as i32);
        let mut postings = te.postings(None, POSTINGS_ENUM_NONE).unwrap().unwrap();
        let mut got = Vec::new();
        let mut doc = postings.next_doc().unwrap();
        while doc != NO_MORE_DOCS {
            got.push(doc);
            doc = postings.next_doc().unwrap();
        }
        assert_eq!(got, expected, "{term:?}");
        let next = te.next().unwrap().unwrap();
        assert_eq!(next, BytesRef::new(format!("t{:03}", i + 1).into_bytes()));
    }
    // The cursor positions from the state alone, without reading the terms
    // file (`SegmentTermsEnum.java:1022-1029`): the term is the target given,
    // the statistics and postings are the state's.
    let (_, state) = &states[1];
    let mut te = terms.iterator().unwrap();
    let target = BytesRef::new(b"zzz".to_vec());
    te.seek_term_state(&target, state.as_ref()).unwrap();
    assert_eq!(te.term().unwrap(), Some(target));
    // `t050` is in docs 50 and 150.
    assert_eq!(te.doc_freq().unwrap(), 2);
    drop((intersect, terms, leaf));
    reader.close().unwrap();
}
