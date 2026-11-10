/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Parity of rmp #347 with Lucene 10.5.0: `RandomCodec`, `RandomIndexWriter`,
//! `RandomPostingsTester` and `DocHelper` against the fixtures
//! `org.apache.lucene.tests.index.RandomIndexOracle` writes
//! (`crates/randomizedtesting/tests/fixtures/oracle/`).

mod oracle_support;

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, Once};

use oracle_support::{assert_same, rows};
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::lang::double_to_string;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{JUnitCore, Request, Test};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::codecs::Codec;
use rucene::document::{
    Document, IntPoint, NumericDocValuesField, SortedDocValuesField, Store, StringField, TextField,
};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::point_values::PointTree;
use rucene::index::terms::Term;
use rucene::index::{IndexOptions, IndexableField, LeafReader};
use rucene::search::term_query::TermQuery;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::BytesRef;
use rucene_codecs::blocktreeords::BlockTreeOrdsPostingsFormat;
use rucene_codecs::memory::{DirectPostingsFormat, FSTPostingsFormat};
use rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message;
use rucene_test_framework::index::doc_helper::NameValue;
use rucene_test_framework::index::random_postings_tester::{TestOption, TestOptions};
use rucene_test_framework::index::{
    DocHelper, RandomCodec, RandomIndexWriter, RandomPostingsTester,
};
use rucene_test_framework::util::lucene_test_case::{LiveIWCFlushMode, LuceneTestCase};
use rucene_test_framework::util::TestUtil;

/// The fields whose per-field formats `random_codec.tsv` records.
const FIELDS: [&str; 15] = [
    "id",
    "body",
    "title",
    "f0",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "\u{e9}t\u{e9}",
    "_x",
];

const WORDS: [&str; 8] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta",
];

/// The process-wide setup the oracle's `main` makes: the service files of
/// `lucene-test-framework` and `lucene-codecs` on the class path, and the live
/// flush mode `maybeChangeLiveIndexWriterConfig` switches on.
fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let loader = rucene_test_framework::class_loader();
        rucene::codecs::reload_codecs(loader).unwrap();
        rucene::codecs::postings::reload_postings_formats(loader).unwrap();
        rucene::codecs::doc_values::reload_doc_values_formats(loader).unwrap();
    });
}

/// `LuceneTestCase.liveIWCFlushMode = EITHER`, held for a whole test: the
/// runner-based test of this file replaces it with the class environment's.
static ENV: Mutex<()> = Mutex::new(());

fn with_live_flush_mode<T>(f: impl FnOnce() -> T) -> T {
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    LuceneTestCase::set_live_iwc_flush_mode(Some(LiveIWCFlushMode::Either));
    let out = f();
    LuceneTestCase::set_live_iwc_flush_mode(None);
    out
}

fn units(s: &str) -> String {
    let units: Vec<String> = s.encode_utf16().map(|u| format!("{u:04x}")).collect();
    if units.is_empty() {
        "-".to_string()
    } else {
        units.join(".")
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `IndexWrappersOracle.digest(lines)`.
fn digest(lines: &[String]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut b = |v: u32| {
        h ^= u64::from(v & 0xff);
        h = h.wrapping_mul(0x100000001b3);
    };
    for line in lines {
        let units: Vec<u16> = line.encode_utf16().collect();
        let n = units.len() as u32;
        for k in 0..4 {
            b(n >> (8 * k));
        }
        for unit in units {
            b(u32::from(unit));
            b(u32::from(unit) >> 8);
        }
    }
    format!("{h:016x}/{}", lines.len())
}

// -----------------------------------------------------------------------------
// RandomCodec
// -----------------------------------------------------------------------------

/// `RandomIndexOracle.codecRow`.
fn codec_row(r: &JdkRandom, avoid: Option<HashSet<String>>) -> String {
    let c = match avoid {
        None => RandomCodec::new(r),
        Some(avoid) => RandomCodec::new_avoiding(r, avoid),
    };
    let mut parts = Vec::new();
    let formats: Vec<String> = c.formats().iter().map(|p| p.to_java_string()).collect();
    parts.push(format!("formats={}", formats.join("|")));
    let dv: Vec<String> = c.dv_formats().iter().map(|d| d.to_string()).collect();
    parts.push(format!("dvFormats={}", dv.join("|")));
    let names: Vec<&str> = c.format_names.iter().map(String::as_str).collect();
    parts.push(format!("formatNames={}", names.join("|")));
    let dv_names: Vec<&str> = c.dv_format_names.iter().map(String::as_str).collect();
    parts.push(format!("dvFormatNames={}", dv_names.join("|")));
    parts.push(format!("perFieldSeed={}", c.per_field_seed()));
    parts.push(format!(
        "maxPointsInLeafNode={}",
        c.max_points_in_leaf_node()
    ));
    parts.push(format!(
        "maxMBSortInHeap={}",
        double_to_string(c.max_mb_sort_in_heap())
    ));
    parts.push(format!("bkdSplitRandomSeed={}", c.bkd_split_random_seed()));
    for f in FIELDS {
        parts.push(format!(
            "{}={}/{}",
            units(f),
            c.get_postings_format_for_field(f).to_java_string(),
            c.get_doc_values_format_for_field(f)
        ));
    }
    parts.push(format!("toString={c}"));
    parts.push(format!("next={}", r.next_long()));
    parts.join("\t")
}

#[test]
fn random_codec_matches_lucene() {
    setup();
    let mut expected = BTreeMap::new();
    let mut actual = BTreeMap::new();
    for (i, row) in rows("random_codec.tsv").into_iter().enumerate() {
        let seed: i64 = row[0].parse().unwrap();
        let avoid = (row[1] != "-").then(|| row[1].split('|').map(str::to_string).collect());
        let label = format!("{i:02} {} {}", row[0], row[1]);
        expected.insert(label.clone(), row[2..].join("\t"));
        actual.insert(label, codec_row(&JdkRandom::new(seed), avoid));
    }
    assert_eq!(expected.len(), 29);
    assert_same("RandomCodec", &expected, &actual);
}

/// `RandomIndexOracle.walk`: every node of the tree, depth first.
fn walk(tree: &mut dyn PointTree, depth: usize, lines: &mut Vec<String>) {
    lines.push(format!(
        "{depth} {} {} {}",
        hex(tree.min_packed_value().unwrap()),
        hex(tree.max_packed_value().unwrap()),
        tree.size().unwrap()
    ));
    if tree.move_to_child().unwrap() {
        loop {
            walk(tree, depth + 1, lines);
            if !tree.move_to_sibling().unwrap() {
                break;
            }
        }
        tree.move_to_parent().unwrap();
    }
}

fn config(codec: Arc<RandomCodec>) -> IndexWriterConfig {
    let c = IndexWriterConfig::with_analyzer(Some(Arc::new(StandardAnalyzer::new())));
    c.set_codec(codec).unwrap();
    c.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    c.set_merge_policy(Some(Arc::new(LogMergePolicy::by_doc_count())))
        .unwrap();
    c
}

/// `RandomIndexOracle.trees`.
fn trees(dir: &Arc<dyn Directory>) -> String {
    let reader = directory_reader::open(Arc::clone(dir)).unwrap();
    let mut parts = Vec::new();
    for ctx in Arc::clone(&reader).leaves().unwrap() {
        let leaf = ctx.leaf_reader();
        for field in ["p2", "p3"] {
            let mut lines = Vec::new();
            let values = leaf.get_point_values(field).unwrap().unwrap();
            walk(&mut *values.point_tree().unwrap(), 0, &mut lines);
            parts.push(format!("{field}:{}:{}", lines.len(), digest(&lines)));
        }
    }
    reader.close().unwrap();
    parts.join("|")
}

/// `RandomIndexOracle.pointsRow`.
fn points_row(seed: i64) -> String {
    let values = JdkRandom::new(seed);
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let codec = Arc::new(RandomCodec::new(&JdkRandom::new(seed)));
    let w = IndexWriter::new(Arc::clone(&dir), config(codec)).unwrap();
    for _ in 0..3 {
        for _ in 0..1500 {
            let mut d = Document::new();
            let a = values.next_int_bound(1000);
            let b = values.next_int();
            d.add(Box::new(IntPoint::new("p2", &[a, b]).unwrap()));
            let a = values.next_int_bound(50);
            let b = values.next_int_bound(100000);
            let c = values.next_int();
            d.add(Box::new(IntPoint::new("p3", &[a, b, c]).unwrap()));
            w.add_document(&d).unwrap();
        }
        w.commit().unwrap();
    }
    let mut parts = vec![format!("flushed={}", trees(&dir))];
    w.force_merge(1).unwrap();
    w.commit().unwrap();
    parts.push(format!("merged={}", trees(&dir)));
    w.close().unwrap();
    parts.join("\t")
}

#[test]
fn random_codec_points_match_lucene() {
    setup();
    let mut expected = BTreeMap::new();
    let mut actual = BTreeMap::new();
    for row in rows("random_codec_points.tsv") {
        let seed: i64 = row[0].parse().unwrap();
        expected.insert(row[0].clone(), row[1..].join("\t"));
        actual.insert(row[0].clone(), points_row(seed));
    }
    assert_eq!(expected.len(), 6);
    assert_same("RandomCodec points", &expected, &actual);
}

/// `RandomIndexOracle.filesRow`.
fn files_row(seed: i64) -> String {
    let mut parts = Vec::new();
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let codec = Arc::new(RandomCodec::new(&JdkRandom::new(seed)));
    let c = config(Arc::clone(&codec));
    c.set_use_compound_file(false).unwrap();
    let w = IndexWriter::new(Arc::clone(&dir), c).unwrap();
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", "0".to_string(), Store::NO).unwrap(),
    ));
    for i in 0..10 {
        d.add(Box::new(
            StringField::new(&format!("f{i}"), format!("v{i}"), Store::NO).unwrap(),
        ));
        d.add(Box::new(NumericDocValuesField::new(&format!("d{i}"), i)));
    }
    w.add_document(&d).unwrap();
    w.commit().unwrap();
    let list = |dir: &Arc<dyn Directory>| {
        let mut files = dir.list_all().unwrap();
        files.sort();
        files.join("|")
    };
    parts.push(format!("files={}", list(&dir)));
    let id0 = || Term::new("id", BytesRef::from_char_sequence("0"));
    w.update_numeric_doc_value(id0(), "d1", 10).unwrap();
    w.update_numeric_doc_value(id0(), "d4", 40).unwrap();
    w.update_numeric_doc_value(id0(), "d7", 70).unwrap();
    w.commit().unwrap();
    parts.push(format!("updated={}", list(&dir)));
    w.close().unwrap();
    parts.push(format!("codec={codec}"));
    parts.join("\t")
}

#[test]
fn random_codec_files_match_lucene() {
    setup();
    let mut expected = BTreeMap::new();
    let mut actual = BTreeMap::new();
    for row in rows("random_codec_files.tsv") {
        let seed: i64 = row[0].parse().unwrap();
        expected.insert(row[0].clone(), row[1..].join("\t"));
        actual.insert(row[0].clone(), files_row(seed));
    }
    assert_eq!(expected.len(), 20);
    assert_same("RandomCodec files", &expected, &actual);
}

// -----------------------------------------------------------------------------
// RandomIndexWriter
// -----------------------------------------------------------------------------

/// `RandomIndexOracle.doc`.
fn doc(i: i32) -> Document {
    let mut d = Document::new();
    d.add(Box::new(
        StringField::new("id", i.to_string(), Store::YES).unwrap(),
    ));
    d.add(Box::new(
        TextField::new(
            "body",
            format!(
                "{} {} w{}",
                WORDS[(i as usize) % WORDS.len()],
                WORDS[((i * 3) as usize) % WORDS.len()],
                i % 17
            ),
            Store::NO,
        )
        .unwrap(),
    ));
    d.add(Box::new(NumericDocValuesField::new("num", i64::from(i))));
    d.add(Box::new(SortedDocValuesField::new(
        "cat",
        BytesRef::from_char_sequence(&format!("c{}", i % 5)),
    )));
    d.add(Box::new(IntPoint::new("pt", &[i, (i * 7) % 13]).unwrap()));
    d
}

fn id(i: i32) -> Term {
    Term::new("id", BytesRef::from_char_sequence(&i.to_string()))
}

/// `RandomIndexOracle.session`.
fn session(seed: i64) -> rucene::error::Result<String> {
    let mut parts = Vec::new();
    let r = JdkRandom::new(seed);
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let codec = Arc::new(RandomCodec::new(&JdkRandom::new(seed.wrapping_add(1))));
    let c = config(Arc::clone(&codec));
    let w = RandomIndexWriter::new_with_config(&r, Arc::clone(&dir), c)?;
    for i in 0..80 {
        let op = i % 10;
        if i == 25 {
            w.add_documents(&[doc(1000 + i), doc(2000 + i)])?;
        } else if i == 45 {
            w.update_documents(id(5), &[doc(3000), doc(3001)])?;
        } else if op == 7 {
            w.update_document(id(i - 3), &doc(i))?;
        } else {
            w.add_document(&doc(i))?;
        }
        if op == 9 {
            w.delete_documents(id(i - 5))?;
        }
        if i == 30 {
            parts.push(format!("commit30={}", w.commit()? > 0));
        }
        if i == 50 {
            let rd = w.get_reader()?;
            parts.push(format!("reader50={}/{}", rd.num_docs(), rd.max_doc()));
            rd.close()?;
        }
        if i == 55 {
            w.update_numeric_doc_value(id(10), "num", 777)?;
        }
        if i == 60 {
            w.delete_documents_by_query(Arc::new(TermQuery::new(Term::new(
                "body",
                BytesRef::from_char_sequence("gamma"),
            ))))?;
        }
        if i == 65 {
            w.flush()?;
        }
        if i == 70 {
            w.force_merge_deletes()?;
        }
    }
    let ds = w.get_doc_stats()?;
    parts.push(format!("docStats={}/{}", ds.max_doc, ds.num_docs));
    w.close()?;
    parts.push(format!("codec={codec}"));
    let reader = directory_reader::open(Arc::clone(&dir))?;
    let leaves = Arc::clone(&reader).leaves()?;
    parts.push(format!(
        "reader={}/{}/{}",
        reader.num_docs(),
        reader.max_doc(),
        leaves.len()
    ));
    for ctx in &leaves {
        let leaf = ctx.leaf_reader();
        let sr = leaf.as_segment_reader().expect("a SegmentReader");
        let mut lines = Vec::new();
        if let Some(pv) = leaf.get_point_values("pt")? {
            walk(&mut *pv.point_tree()?, 0, &mut lines);
        }
        parts.push(format!(
            "leaf={}/{}/{}/{}/points={}:{}",
            sr.get_segment_name(),
            (&*leaf as &dyn LeafReader).max_doc(),
            (&*leaf as &dyn LeafReader).num_docs(),
            sr.get_segment_info()
                .info
                .codec()
                .map_or("null".to_string(), |c| c.name().to_string()),
            lines.len(),
            digest(&lines)
        ));
    }
    reader.close()?;
    let mut files = dir.list_all()?;
    files.sort();
    parts.push(format!("files={}", files.join("|")));
    parts.push(format!("next={}", r.next_long()));
    Ok(parts.join("\t"))
}

#[test]
fn random_index_writer_sessions_match_lucene() {
    setup();
    with_live_flush_mode(|| {
        let mut expected = BTreeMap::new();
        let mut actual = BTreeMap::new();
        for row in rows("random_iw.tsv") {
            let seed: i64 = row[0].parse().unwrap();
            expected.insert(row[0].clone(), row[1..].join("\t"));
            let got = match session(seed) {
                Ok(row) => row,
                Err(e) => {
                    let (class, message) = java_class_and_message(&e);
                    format!("error\t{class}\t{message}")
                }
            };
            actual.insert(row[0].clone(), got);
        }
        assert_eq!(expected.len(), 12);
        assert_same("RandomIndexWriter sessions", &expected, &actual);
    });
}

// -----------------------------------------------------------------------------
// RandomPostingsTester
// -----------------------------------------------------------------------------

/// `RandomIndexOracle.postingsLines`.
fn postings_lines(t: &RandomPostingsTester) -> Vec<String> {
    let mut lines = Vec::new();
    for (field, terms) in t.postings_seeds() {
        for (term, seed) in terms {
            let term = term.utf8_to_string().unwrap();
            let mut sp = RandomPostingsTester::get_seed_postings(
                &term,
                seed,
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
                true,
            );
            lines.push(format!(
                "{field}:{term} seed={seed} docFreq={}",
                sp.doc_freq
            ));
            use rucene::index::PostingsEnum;
            use rucene::search::DocIdSetIterator;
            loop {
                let doc = sp.next_doc().unwrap();
                if doc == rucene::search::NO_MORE_DOCS {
                    break;
                }
                let freq = sp.freq().unwrap();
                let mut sb = format!("{doc} {freq}");
                for _ in 0..freq {
                    let pos = sp.next_position().unwrap();
                    let payload = sp.payload().map_or("-".to_string(), hex);
                    sb.push_str(&format!(
                        " {pos},{},{},{payload}",
                        sp.start_offset(),
                        sp.end_offset()
                    ));
                }
                lines.push(sb);
            }
        }
    }
    lines
}

#[test]
fn random_postings_match_lucene() {
    let mut expected = BTreeMap::new();
    let mut actual = BTreeMap::new();
    for row in rows("random_postings.tsv") {
        let seed: i64 = row[0].parse().unwrap();
        expected.insert(row[0].clone(), row[1..].join("\t"));
        let r: Arc<dyn Random> = Arc::new(JdkRandom::new(seed));
        let t = RandomPostingsTester::new(Arc::clone(&r)).unwrap();
        let names: Vec<String> = t
            .postings_seeds()
            .iter()
            .map(|(f, terms)| format!("{f}:{}", terms.len()))
            .collect();
        let all: Vec<String> = t
            .all_terms()
            .iter()
            .map(|ft| {
                format!(
                    "{}:{}:{}",
                    ft.field(),
                    ft.term().utf8_to_string().unwrap(),
                    ft.ord()
                )
            })
            .collect();
        let lines = postings_lines(&t);
        let d_all = digest(&all);
        let d_lines = digest(&lines);
        actual.insert(
            row[0].clone(),
            format!(
                "fields={}\tmaxDoc={}\tallTerms={}:{}\tpostings={}:{}\tnext={}",
                names.join("|"),
                t.max_doc(),
                all.len(),
                d_all,
                lines.len(),
                d_lines,
                r.next_long()
            ),
        );
    }
    assert_eq!(expected.len(), 20);
    assert_same("RandomPostingsTester postings", &expected, &actual);
}

/// `RandomIndexOracle.codecOf`.
fn codec_of(name: &str) -> Arc<dyn Codec> {
    match name {
        "default" => TestUtil::get_default_codec(),
        "FST50" => TestUtil::always_postings_format(Arc::new(FSTPostingsFormat::new())),
        "BlockTreeOrds" => {
            TestUtil::always_postings_format(Arc::new(BlockTreeOrdsPostingsFormat::new()))
        }
        "Direct" => TestUtil::always_postings_format(Arc::new(DirectPostingsFormat::new())),
        _ => unreachable!("{name}"),
    }
}

fn index_options(name: &str) -> IndexOptions {
    match name {
        "DOCS" => IndexOptions::DOCS,
        "DOCS_AND_FREQS" => IndexOptions::DOCS_AND_FREQS,
        "DOCS_AND_FREQS_AND_POSITIONS" => IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
        "DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS" => {
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
        }
        _ => unreachable!("{name}"),
    }
}

/// A failure of the tester: its error, or the `AssertionError` it panics
/// with.
fn outcome(f: impl FnOnce() -> rucene::error::Result<()>) -> Result<(), String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            let (class, message) = java_class_and_message(&e);
            Err(format!("{class}\t{message}"))
        }
        Err(payload) => Err(randomizedtesting::jdk::exception::panic_message(&*payload)),
    }
}

#[test]
fn random_postings_test_terms_match_lucene() {
    setup();
    let mut expected = BTreeMap::new();
    let mut actual = BTreeMap::new();
    for row in rows("random_postings_terms.tsv") {
        let seed: i64 = row[0].parse().unwrap();
        let (codec, max, payloads, threads) = (
            row[1].as_str(),
            index_options(&row[2]),
            row[3] == "true",
            row[4] == "true",
        );
        let label = row[..5].join(" ");
        expected.insert(label.clone(), row[5..].join("\t"));
        let r: Arc<dyn Random> = Arc::new(JdkRandom::new(seed));
        let result = outcome(|| {
            let mut t = RandomPostingsTester::new(Arc::clone(&r))?;
            let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
            let mut fp = t.build_index(codec_of(codec), Arc::clone(&dir), max, payloads, false)?;
            t.test_fields(&*fp);
            let mut options = TestOptions::all_of();
            if !threads {
                options.remove(TestOption::Threads);
            }
            t.test_terms(&*fp, options, max, max, false)?;
            fp.close()?;
            // `dir.close()`: closing a `ByteBuffersDirectory` drops its files.
            drop(dir);
            Ok(())
        });
        actual.insert(
            label,
            match result {
                Ok(()) => format!("ok\t{}", r.next_long()),
                Err(e) => format!("error\t{e}"),
            },
        );
    }
    assert_eq!(expected.len(), 32);
    assert_same("RandomPostingsTester.testTerms", &expected, &actual);
}

static FULL: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The body of `TestRandomPostingsFull.testFull`.
fn test_full() {
    for codec in ["default", "FST50", "BlockTreeOrds"] {
        let random = LuceneTestCase::random();
        let result = outcome(|| {
            RandomPostingsTester::new(random)?.test_full(
                codec_of(codec),
                &LuceneTestCase::create_temp_dir(),
                IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
                true,
            )
        });
        let outcome = match result {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("error {e}"),
        };
        FULL.lock().unwrap().push(format!("{codec}={outcome}"));
    }
    let next = LuceneTestCase::random().next_long();
    FULL.lock().unwrap().push(format!("next={next}"));
}

#[test]
fn random_postings_test_full_matches_lucene_under_the_runner() {
    setup();
    let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let class = Class::builder("org.apache.lucene.tests.index.TestRandomPostingsFull")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(
            Method::new("testFull")
                .annotate(Test::new())
                .body_any(test_full),
        )
        .build();
    let class = Arc::new(class);
    let mut expected = BTreeMap::new();
    let mut actual = BTreeMap::new();
    for row in rows("random_postings_full.tsv") {
        expected.insert(row[0].clone(), row[1..].join("\t"));
    }
    for seed in expected.keys() {
        FULL.lock().unwrap().clear();
        system::set_property("tests.seed", seed);
        let result = JUnitCore::new().run(&Request::a_class(&class));
        if let Some(f) = result.failures().first() {
            Throwable::new(
                "java.lang.AssertionError",
                format!("seed {seed}: {}", f.exception()),
            )
            .throw();
        }
        actual.insert(seed.clone(), FULL.lock().unwrap().join("\t"));
    }
    system::clear_property("tests.seed");
    assert_eq!(expected.len(), 2);
    assert_same("RandomPostingsTester.testFull", &expected, &actual);
}

// -----------------------------------------------------------------------------
// DocHelper
// -----------------------------------------------------------------------------

/// `RandomIndexOracle.describe` without the field's runtime class name (the
/// second `:` part), which a Rust field has no counterpart of.
fn describe(f: &dyn IndexableField) -> String {
    let t = f.field_type();
    let value = match f.string_value() {
        Some(s) if units(&s).len() > 200 => format!("len{}", s.encode_utf16().count()),
        Some(s) => units(&s),
        None => match f.binary_value() {
            Some(b) => format!("bin{}", hex(b.slice())),
            None => "-".to_string(),
        },
    };
    format!(
        "{}:stored={}:tokenized={}:{:?}:omitNorms={}:tv={}{}{}:{:?}:{value}",
        f.name(),
        t.stored(),
        t.tokenized(),
        t.index_options(),
        t.omit_norms(),
        t.store_term_vectors(),
        t.store_term_vector_positions(),
        t.store_term_vector_offsets(),
        t.doc_values_type(),
    )
}

/// The Java row of a field without its class name.
fn without_class(described: &str) -> String {
    let mut parts: Vec<&str> = described.splitn(3, ':').collect();
    parts.remove(1);
    parts.join(":")
}

#[test]
fn doc_helper_matches_lucene() {
    let mut expected = Vec::new();
    for row in rows("doc_helper.tsv") {
        let value = match row[0].as_str() {
            "field" => without_class(&row[1]),
            "createDocument" => row[1]
                .split('|')
                .map(without_class)
                .collect::<Vec<_>>()
                .join("|"),
            _ => row[1].clone(),
        };
        expected.push(format!("{}\t{value}", row[0]));
    }
    let mut actual = Vec::new();
    for f in DocHelper::fields() {
        actual.push(format!("field\t{}", describe(&*f)));
    }
    let maps = [
        ("all", DocHelper::all()),
        ("indexed", DocHelper::indexed()),
        ("stored", DocHelper::stored()),
        ("unstored", DocHelper::unstored()),
        ("unindexed", DocHelper::unindexed()),
        ("termvector", DocHelper::termvector()),
        ("notermvector", DocHelper::notermvector()),
        ("lazy", DocHelper::lazy()),
        ("noNorms", DocHelper::no_norms()),
        ("noTf", DocHelper::no_tf()),
    ];
    for (name, map) in maps {
        let keys: Vec<&str> = map.keys().map(String::as_str).collect();
        actual.push(format!("map.{name}\t{}", keys.join("|")));
    }
    let name_values = DocHelper::name_values();
    let keys: Vec<&str> = name_values.keys().map(String::as_str).collect();
    actual.push(format!("nameValues\t{}", keys.join("|")));
    assert_eq!(
        name_values.get(rucene_test_framework::index::doc_helper::LAZY_FIELD_BINARY_KEY),
        Some(&NameValue::Bytes(
            b"These are some binary field bytes".to_vec()
        ))
    );
    let mut d = Document::new();
    DocHelper::setup_doc(&mut d);
    actual.push(format!("setupDoc\t{}", DocHelper::num_fields(&d)));
    let c = DocHelper::create_document(7, "idx", 4);
    let fs: Vec<String> = c.get_fields().iter().map(|f| describe(&**f)).collect();
    actual.push(format!("createDocument\t{}", fs.join("|")));
    assert_eq!(expected, actual);
}
