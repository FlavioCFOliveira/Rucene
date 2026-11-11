//! Parity of the index readers and reader wrappers of
//! `org.apache.lucene.tests.index` (rmp #344) with Lucene 10.5.0:
//! `iw_wrappers.tsv`, written by
//! `crates/randomizedtesting/tests/fixtures/oracle/src/org/apache/lucene/tests/index/IndexWrappersOracle.java`
//! over the default-codec corpus of `mdw_corpus.tsv` (`nocfs` and `cfs`).
//!
//! Each row is `case, scenario, step, outcome` (`ok<TAB>value` or
//! `err<TAB>class<TAB>message`); this target replays every scenario through
//! the Rust port and compares the rows. Per wrapped leaf it records the
//! wrapper's `toString()`, its document counts and live docs, which
//! accessors answer for a fixed set of fields, its cache-helper identity
//! relations, the field numbers its stored fields report, and the digest of
//! `SimpleTextReadOracle.dumpLeaf` through it (`oracle::dump_leaf` here),
//! with the dump's `leaf`/`live`/`fi` lines in full. It also replays the
//! `Mismatched*` shuffles per seed (and the draws they leave), the
//! `PerThreadPKLookup` lookups, `OwnCacheKeyMultiReader`'s key and
//! listeners, the reopen of each directory wrapper, the misuse of the
//! asserting views, and `PointsStackTracker` over an index of 5000 points.
//!
//! Rows whose outcome the port cannot reproduce for a declared reason are
//! listed in [`DECLARED`] with the reason, and the dump's `   live` lines
//! (rmp #334) are compared apart; every other row must match.

#[path = "../../rucene-codecs/tests/oracle/mod.rs"]
mod oracle;
mod oracle_support;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use oracle::{bytes, dump_leaf, id, install_codecs};
use randomizedtesting::jdk::exception::QuietPanics;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::thread::Thread;
use rucene::analysis::standard::StandardAnalyzer;
use rucene::document::{Document, IntPoint, LongPoint, Store, StringField};
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::filter_directory_reader::{
    DirectoryReaderWrapper, ExitableDirectoryReaderWrapper, SlowCodecReaderWrapper,
};
use rucene::index::filter_reader::{FilterCodecReader, FilterLeafReader};
use rucene::index::index_reader::{CacheHelper, CacheKey, ClosedListener, StoredFields};
use rucene::index::index_utilities::QueryTimeout;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::leaf_reader::TermVectors;
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{
    FieldInfo, FieldInfos, IndexReader, IntersectVisitor, LeafReader, Relation, StoredFieldVisitor,
    StoredFieldVisitorStatus, Term, POSTINGS_ENUM_FREQS,
};
use rucene::search::NO_MORE_DOCS;
use rucene::store::java_nio_file::{java_class_of, java_message_of};
use rucene::store::{ByteBuffersDirectory, Directory, DEFAULT_IO_CONTEXT};
use rucene::util::resource_loader::ClassLoader;
use rucene::util::Bits;
use rucene_test_framework::index::mismatched_leaf_reader::{shuffle_infos, MismatchedVisitor};
use rucene_test_framework::index::{
    AllDeletedFilterReader, AssertingDirectoryReader, AssertingLeafReader, FieldFilterLeafReader,
    MergeReaderWrapper, MergingCodecReader, MergingDirectoryReaderWrapper, MismatchedCodecReader,
    MismatchedDirectoryReader, MismatchedLeafReader, OwnCacheKeyMultiReader, PerThreadPKLookup,
    PointsStackTracker,
};

/// The "class path" `oracle::install_codecs` loads the codecs from.
fn test_class_loader() -> &'static Arc<ClassLoader> {
    rucene_test_framework::class_loader()
}

/// Rows the port cannot reproduce (by key suffix), with the reason; their
/// outcome is compared on the error class only (`class`), or not at all
/// (`skip`).
const DECLARED: &[(&str, &str, &str)] = &[
    (
        "\tmisuse\tmerging_dir_cast",
        "class",
        "the ClassCastException message names run-time classes the port does not have",
    ),
    (
        "\tmergewrapper\tskipper",
        "class",
        "the JDK's helpful NullPointerException message names Java's expression",
    ),
    (
        "\tmisuse\tdv_thread",
        "skip",
        "a doc-values iterator is not Send: Rust refuses to hand it to another thread",
    ),
];

const PROBE_FIELDS: [&str; 13] = [
    "id",
    "body",
    "tv",
    "dv_num",
    "dv_bin",
    "dv_sorted",
    "dv_snum",
    "dv_sset",
    "p_int",
    "p_long",
    "vf",
    "vb",
    "nope",
];

const FILTER_FIELDS: [&str; 9] = [
    "body", "dv_bin", "dv_num", "p_int", "s_str", "tv", "vb", "id", "dv_sset",
];

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

/// The oracle's `m(o)`: identity hashes as `@HASH`, thread ids as `#N`, line
/// breaks and tabs escaped.
fn m(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        out.push(c);
        i += 1;
        if c == '@' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || ('a'..='f').contains(&chars[i]))
            {
                i += 1;
            }
            if i > start {
                out.push_str("HASH");
            }
        }
    }
    let mut masked = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(at) = rest.find("Thread[#") {
        masked.push_str(&rest[..at + "Thread[#".len()]);
        let tail = &rest[at + "Thread[#".len()..];
        let digits = tail.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && tail[digits..].starts_with(',') {
            masked.push('N');
            rest = &tail[digits..];
        } else {
            rest = tail;
        }
    }
    masked.push_str(rest);
    masked.replace('\n', "\\n").replace('\t', "\\t")
}

/// `class` and `getMessage()` of an error, as Java names them.
fn class_and_message(e: &LuceneError) -> (String, String) {
    let null = || "<null>".to_string();
    let or_null = |message: &String| {
        if message.is_empty() {
            null()
        } else {
            message.clone()
        }
    };
    match e {
        LuceneError {
            kind: LuceneErrorKind::Io(io),
            ..
        } => (
            java_class_of(io).to_string(),
            java_message_of(io).unwrap_or_else(null),
        ),
        LuceneError {
            kind: LuceneErrorKind::Unchecked { class, message, .. },
            ..
        } => (class.to_string(), message.clone().unwrap_or_else(null)),
        LuceneError {
            kind: LuceneErrorKind::AssertionError(message),
            ..
        } => ("java.lang.AssertionError".to_string(), or_null(message)),
        LuceneError {
            kind: LuceneErrorKind::AlreadyClosed { message, .. },
            ..
        } => (
            "org.apache.lucene.store.AlreadyClosedException".to_string(),
            message.clone(),
        ),
        LuceneError {
            kind: LuceneErrorKind::IllegalState { message, .. },
            ..
        } => (
            "java.lang.IllegalStateException".to_string(),
            message.clone(),
        ),
        LuceneError {
            kind: LuceneErrorKind::IllegalArgument { message, .. },
            ..
        } => (
            "java.lang.IllegalArgumentException".to_string(),
            or_null(message),
        ),
        other => (
            format!("rucene::{}", other.java_simple_name()),
            other.to_string(),
        ),
    }
}

/// The outcome of a step: `ok<TAB>value`, or `err<TAB>class<TAB>message` of
/// the error it returned or the Java `Throwable` it threw (a panic).
fn outcome(f: impl FnOnce() -> Result<String>) -> String {
    let result = {
        let _quiet = QuietPanics::new();
        catch_unwind(AssertUnwindSafe(f))
    };
    match result {
        Ok(Ok(v)) => format!("ok\t{}", m(&v)),
        Ok(Err(e)) => {
            let (class, message) = class_and_message(&e);
            format!("err\t{class}\t{}", m(&message))
        }
        Err(payload) => {
            let (class, message) = oracle_support::panic_error(payload);
            format!("err\t{class}\t{}", m(&message))
        }
    }
}

#[derive(Default)]
struct Rows(BTreeMap<String, String>);

impl Rows {
    fn row(&mut self, c: &str, scenario: &str, step: &str, value: String) {
        let key = format!("{c}\t{scenario}\t{step}");
        assert!(
            self.0.insert(key.clone(), value).is_none(),
            "duplicate row {key}"
        );
    }

    fn rec(&mut self, c: &str, scenario: &str, step: &str, f: impl FnOnce() -> Result<String>) {
        let value = outcome(f);
        self.row(c, scenario, step, value);
    }
}

/// `IndexWrappersOracle.digest`: FNV-1a 64 over each line (its length as 4
/// little-endian bytes, then its UTF-16 units as 2 little-endian bytes
/// each), and the line count.
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

/// `IndexWrappersOracle.notLive`: the dump without its `   live` lines.
fn not_live(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| !l.starts_with("   live"))
        .cloned()
        .collect()
}

/// `load_corpus(case)`: the files of `mdw_corpus.tsv` for `case`.
fn load_corpus(case: &str) -> Arc<dyn Directory> {
    let dir = ByteBuffersDirectory::new();
    for row in oracle_support::rows("mdw_corpus.tsv") {
        if row[0] != case {
            continue;
        }
        let b: Vec<u8> = (0..row[2].len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&row[2][i..i + 2], 16).unwrap())
            .collect();
        let mut out = dir.create_output(&row[1], &*DEFAULT_IO_CONTEXT).unwrap();
        out.write_bytes(&b, 0, b.len()).unwrap();
        out.close().unwrap();
    }
    Arc::new(dir)
}

fn as_index_reader(r: &Arc<dyn LeafReader>) -> Arc<dyn IndexReader> {
    Arc::clone(r).as_index_reader()
}

// ---------------------------------------------------------------------------
// What is recorded of one wrapped leaf
// ---------------------------------------------------------------------------

fn present<T>(f: impl FnOnce() -> Result<Option<T>>) -> char {
    let result = {
        let _quiet = QuietPanics::new();
        catch_unwind(AssertUnwindSafe(f))
    };
    match result {
        Ok(Ok(None)) => '0',
        Ok(Ok(Some(_))) => '1',
        _ => 'E',
    }
}

fn probe(r: &dyn LeafReader) -> String {
    let mut parts = Vec::new();
    for f in PROBE_FIELDS {
        let mut sb = format!("{f}=");
        sb.push(present(|| r.terms(f)));
        sb.push(present(|| r.get_numeric_doc_values(f)));
        sb.push(present(|| r.get_binary_doc_values(f)));
        sb.push(present(|| r.get_sorted_doc_values(f)));
        sb.push(present(|| r.get_sorted_numeric_doc_values(f)));
        sb.push(present(|| r.get_sorted_set_doc_values(f)));
        sb.push(present(|| r.get_norm_values(f)));
        sb.push(present(|| r.get_point_values(f)));
        sb.push(present(|| r.get_float_vector_values(f)));
        sb.push(present(|| r.get_byte_vector_values(f)));
        parts.push(sb);
    }
    parts.join(",")
}

fn same_key(a: &dyn CacheHelper, b: &dyn CacheHelper) -> bool {
    a.get_key() == b.get_key()
}

fn helper_relation(
    mine: Option<Box<dyn CacheHelper>>,
    inner: Option<Box<dyn CacheHelper>>,
    again: Option<Box<dyn CacheHelper>>,
) -> String {
    let Some(mine) = mine else {
        return "null".to_string();
    };
    let rel = if inner.is_some_and(|inner| same_key(mine.as_ref(), inner.as_ref())) {
        "same"
    } else {
        "own"
    };
    let stable = if again.is_some_and(|again| same_key(again.as_ref(), mine.as_ref())) {
        "+stable"
    } else {
        "+unstable"
    };
    format!("{rel}{stable}")
}

fn cache(r: &dyn LeafReader, inner: &dyn LeafReader) -> String {
    format!(
        "core={} reader={}",
        helper_relation(
            r.get_core_cache_helper(),
            inner.get_core_cache_helper(),
            r.get_core_cache_helper()
        ),
        helper_relation(
            r.get_reader_cache_helper(),
            inner.get_reader_cache_helper(),
            r.get_reader_cache_helper()
        )
    )
}

struct NumbersVisitor<'a>(&'a mut String);

impl StoredFieldVisitor for NumbersVisitor<'_> {
    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        self.0
            .push_str(&format!(" {}#{}", field_info.name, field_info.number));
        Ok(StoredFieldVisitorStatus::Yes)
    }
}

fn stored_numbers(r: &dyn LeafReader) -> Result<String> {
    let mut sb = String::new();
    for doc in 0..std::cmp::min(3, r.max_doc()) {
        sb.push_str(&format!("{doc}:"));
        r.stored_fields()?
            .document_with_visitor(doc, &mut NumbersVisitor(&mut sb))?;
        sb.push(';');
    }
    Ok(sb)
}

fn live_docs(r: &dyn LeafReader) -> String {
    let Some(live) = r.get_live_docs() else {
        return "null".to_string();
    };
    let mut sb: String = (0..live.length())
        .map(|i| if live.get(i) { '1' } else { '0' })
        .collect();
    match r.get_live_docs().and_then(Bits::into_live_docs) {
        Some(ld) => sb.push_str(&format!(" deleted={}", ld.deleted_count())),
        None => sb.push_str(" bits"),
    }
    sb
}

fn leaf(
    rows: &mut Rows,
    c: &str,
    scenario: &str,
    ord: i32,
    r: &Arc<dyn LeafReader>,
    inner: &Arc<dyn LeafReader>,
) {
    let p = format!("leaf{ord}.");
    rows.rec(c, scenario, &format!("{p}toString"), || Ok(r.to_string()));
    rows.rec(c, scenario, &format!("{p}docs"), || {
        let ir = as_index_reader(r);
        Ok(format!(
            "{}/{}/{}/{}",
            ir.max_doc(),
            ir.num_docs(),
            ir.num_deleted_docs(),
            ir.has_deletions()
        ))
    });
    rows.rec(c, scenario, &format!("{p}live"), || {
        Ok(live_docs(r.as_ref()))
    });
    rows.rec(c, scenario, &format!("{p}probe"), || Ok(probe(r.as_ref())));
    rows.rec(c, scenario, &format!("{p}cache"), || {
        Ok(cache(r.as_ref(), inner.as_ref()))
    });
    rows.rec(c, scenario, &format!("{p}stored"), || {
        stored_numbers(r.as_ref())
    });
    let mut lines = Vec::new();
    rows.rec(c, scenario, &format!("{p}dump"), || {
        dump_leaf(ord, r.as_ref(), &mut lines)?;
        Ok(digest(&not_live(&lines)))
    });
    // the dump's "   live" lines (impacts read through a view), digested apart
    let live: Vec<String> = lines
        .iter()
        .filter(|l| l.starts_with("   live"))
        .cloned()
        .collect();
    rows.row(
        c,
        scenario,
        &format!("{p}dumplive"),
        format!("ok\t{}", digest(&live)),
    );
    // Diagnostic aid: `IW_DUMP_DIR=<dir>` writes each dump in full, to diff
    // against the Java oracle's lines when a digest differs.
    if let Ok(dump_dir) = std::env::var("IW_DUMP_DIR") {
        let path = format!("{dump_dir}/{c}_{scenario}_{ord}.txt");
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }
    let head: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .filter(|l| l.starts_with("leaf ") || l.starts_with(" live ") || l.starts_with(" fi "))
        .collect();
    rows.row(
        c,
        scenario,
        &format!("{p}fi"),
        format!("ok\t{}", m(&head.join("|"))),
    );
}

fn dir_reader(
    rows: &mut Rows,
    c: &str,
    scenario: &str,
    d: &Arc<dyn DirectoryReader>,
    inner: &Arc<dyn DirectoryReader>,
) {
    rows.rec(c, scenario, "toString", || Ok(d.to_string()));
    rows.rec(c, scenario, "docs", || {
        Ok(format!(
            "{}/{}/{}",
            d.max_doc(),
            d.num_docs(),
            Arc::clone(d).leaves()?.len()
        ))
    });
    rows.rec(c, scenario, "cache", || {
        Ok(helper_relation(
            d.get_reader_cache_helper(),
            inner.get_reader_cache_helper(),
            d.get_reader_cache_helper(),
        ))
    });
    let mine = Arc::clone(d).leaves().unwrap();
    let theirs = Arc::clone(inner).leaves().unwrap();
    for (i, (a, b)) in mine.iter().zip(theirs.iter()).enumerate() {
        leaf(
            rows,
            c,
            scenario,
            i as i32,
            &a.leaf_reader(),
            &b.leaf_reader(),
        );
    }
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

fn codec(r: &Arc<dyn LeafReader>) -> Arc<dyn CodecReader> {
    Arc::clone(r).as_codec_reader().expect("a segment reader")
}

fn wrappers(rows: &mut Rows, c: &str, reader: &Arc<dyn DirectoryReader>, shuffle_seeds: &[i64]) {
    let leaves = Arc::clone(reader).leaves().unwrap();
    rows.rec(c, "plain", "docs", || {
        Ok(format!(
            "{}/{}/{}",
            reader.max_doc(),
            reader.num_docs(),
            leaves.len()
        ))
    });
    rows.rec(c, "plain", "toString", || Ok(reader.to_string()));
    for ctx in &leaves {
        let r = ctx.leaf_reader();
        leaf(rows, c, "plain", ctx.ord(), &r, &r);
    }

    let asserting: Arc<dyn DirectoryReader> =
        Arc::new(AssertingDirectoryReader::new(Arc::clone(reader)).unwrap());
    dir_reader(rows, c, "asserting_dir", &asserting, reader);

    for ctx in &leaves {
        let r#in = ctx.leaf_reader();
        let ord = ctx.ord();
        let mut inner_rows = Rows::default();
        rows.rec(c, "alldeleted", &format!("leaf{ord}.new"), || {
            let r: Arc<dyn LeafReader> = Arc::new(AllDeletedFilterReader::new(Arc::clone(&r#in))?);
            leaf(&mut inner_rows, c, "alldeleted", ord, &r, &r#in);
            Ok("built".to_string())
        });
        rows.rec(c, "asserting_alldeleted", &format!("leaf{ord}.new"), || {
            let all: Arc<dyn LeafReader> =
                Arc::new(AllDeletedFilterReader::new(Arc::clone(&r#in))?);
            let r: Arc<dyn LeafReader> = Arc::new(AssertingLeafReader::new(all)?);
            leaf(&mut inner_rows, c, "asserting_alldeleted", ord, &r, &r#in);
            Ok("built".to_string())
        });
        rows.0.extend(inner_rows.0);
        for negate in [false, true] {
            let scenario = if negate {
                "fieldfilter_neg"
            } else {
                "fieldfilter"
            };
            let r: Arc<dyn LeafReader> = Arc::new(
                FieldFilterLeafReader::new(Arc::clone(&r#in), FILTER_FIELDS, negate).unwrap(),
            );
            leaf(rows, c, scenario, ord, &r, &r#in);
        }
        let single = FieldFilterLeafReader::new(Arc::clone(&r#in), ["body"], false).unwrap();
        rows.rec(
            c,
            "fieldfilter_single",
            &format!("leaf{ord}.toString"),
            || Ok(single.to_string()),
        );
        let none =
            FieldFilterLeafReader::new(Arc::clone(&r#in), Vec::<String>::new(), false).unwrap();
        rows.rec(
            c,
            "fieldfilter_empty",
            &format!("leaf{ord}.toString"),
            || Ok(none.to_string()),
        );
        rows.rec(c, "fieldfilter_empty", &format!("leaf{ord}.tv"), || {
            let tv = LeafReader::term_vectors(&none)?;
            let mut sb = String::new();
            for d in 0..std::cmp::min(4, LeafReader::max_doc(&none)) {
                sb.push(if tv.get(d)?.is_none() { 'n' } else { 'f' });
            }
            Ok(sb)
        });
    }

    for seed in [0i64, 42, 5843011988881251843] {
        let s = format!("seed{seed}");
        let random: Arc<dyn Random> = Arc::new(JdkRandom::new(seed));
        let md: Arc<dyn DirectoryReader> = Arc::new(
            MismatchedDirectoryReader::new(Arc::clone(reader), Arc::clone(&random)).unwrap(),
        );
        dir_reader(rows, c, &format!("mismatched_dir_{s}"), &md, reader);
        rows.rec(c, &format!("mismatched_dir_{s}"), "next", || {
            Ok(random.next_long().to_string())
        });

        let r2 = JdkRandom::new(seed);
        for ctx in &leaves {
            let r#in = ctx.leaf_reader();
            let r: Arc<dyn LeafReader> =
                Arc::new(MismatchedLeafReader::new(Arc::clone(&r#in), &r2).unwrap());
            leaf(
                rows,
                c,
                &format!("mismatched_leaf_{s}"),
                ctx.ord(),
                &r,
                &r#in,
            );
        }
        rows.rec(c, &format!("mismatched_leaf_{s}"), "next", || {
            Ok(r2.next_long().to_string())
        });

        let r3 = JdkRandom::new(seed);
        for ctx in &leaves {
            let r#in = ctx.leaf_reader();
            let r: Arc<dyn LeafReader> =
                Arc::new(MismatchedCodecReader::new(codec(&r#in), &r3).unwrap());
            leaf(
                rows,
                c,
                &format!("mismatched_codec_{s}"),
                ctx.ord(),
                &r,
                &r#in,
            );
        }
        rows.rec(c, &format!("mismatched_codec_{s}"), "next", || {
            Ok(r3.next_long().to_string())
        });
    }
    // the shuffle alone, for every seed of the grid
    for &seed in shuffle_seeds {
        let random = JdkRandom::new(seed);
        rows.rec(c, "shuffle", &format!("seed{seed}"), || {
            let infos = shuffle_infos(&leaves[0].leaf_reader().get_field_infos(), &random)?;
            let mut sb = String::new();
            for fi in infos.iter() {
                sb.push_str(&format!("{}#{} ", fi.name, fi.number));
            }
            Ok(format!("{sb}{}", random.next_long()))
        });
    }

    let merging: Arc<dyn DirectoryReader> =
        Arc::new(MergingDirectoryReaderWrapper::new(Arc::clone(reader)).unwrap());
    dir_reader(rows, c, "merging_dir", &merging, reader);
    for ctx in &leaves {
        let r#in = ctx.leaf_reader();
        let mc: Arc<dyn LeafReader> = Arc::new(MergingCodecReader::new(codec(&r#in)));
        leaf(rows, c, "merging_codec", ctx.ord(), &mc, &r#in);
        let mw: Arc<dyn LeafReader> = Arc::new(MergeReaderWrapper::new(codec(&r#in)).unwrap());
        leaf(rows, c, "merge_wrapper", ctx.ord(), &mw, &r#in);
    }

    pk_lookup(rows, c, reader);
}

fn lookups(pk: &mut PerThreadPKLookup, step: usize) -> Result<String> {
    let mut sb = String::new();
    for i in (0..340).step_by(step) {
        sb.push_str(&format!("{} ", pk.lookup(&bytes(&id(i)))?));
    }
    Ok(sb)
}

/// `MultiReaderHolder`: an `OwnCacheKeyMultiReader` over sub-readers it must
/// not close, each `incRef`'ed first.
fn holder(subs: &[Arc<dyn IndexReader>]) -> Result<Arc<OwnCacheKeyMultiReader>> {
    for r in subs {
        r.inc_ref()?;
    }
    Ok(Arc::new(OwnCacheKeyMultiReader::new(subs.to_vec())?))
}

fn pk_lookup(rows: &mut Rows, c: &str, reader: &Arc<dyn DirectoryReader>) {
    let as_reader: Arc<dyn IndexReader> = Arc::clone(reader) as Arc<dyn IndexReader>;
    let mut pk = PerThreadPKLookup::new(&as_reader, "id").unwrap();
    rows.rec(c, "pklookup", "lookups", || lookups(&mut pk, 1));
    rows.rec(c, "pklookup", "missing", || {
        Ok(pk.lookup(&bytes("nope"))?.to_string())
    });
    let mut again = pk.reopen(Some(&as_reader)).unwrap().unwrap();
    rows.rec(c, "pklookup", "reopen", || lookups(&mut again, 7));
    rows.rec(c, "pklookup", "reopen_null", || {
        Ok(match pk.reopen(None)? {
            None => "null".to_string(),
            Some(_) => "lookup".to_string(),
        })
    });
    rows.rec(c, "pklookup", "old_after_reopen", || {
        Ok(pk.lookup(&bytes("id-0042"))?.to_string())
    });
    let subs: Vec<Arc<dyn IndexReader>> = Arc::clone(reader)
        .leaves()
        .unwrap()
        .iter()
        .map(|ctx| {
            Arc::new(AllDeletedFilterReader::new(ctx.leaf_reader()).unwrap())
                as Arc<dyn IndexReader>
        })
        .collect();
    rows.rec(c, "pklookup", "alldeleted", || {
        let h: Arc<dyn IndexReader> = holder(&subs)?;
        let mut dead = PerThreadPKLookup::new(&h, "id")?;
        lookups(&mut dead, 13)
    });
}

/// `IndexWrappersOracle.probeClosed`: each member's `ensureOpen()` outcome.
fn probe_closed(rows: &mut Rows, c: &str, s: &str, what: &str, r: &Arc<dyn LeafReader>) {
    let present = |o: bool, yes: &str| if o { yes } else { "null" }.to_string();
    rows.rec(c, s, &format!("{what}.terms"), || {
        Ok(present(r.terms("body")?.is_some(), "terms"))
    });
    rows.rec(c, s, &format!("{what}.numeric"), || {
        Ok(present(r.get_numeric_doc_values("dv_num")?.is_some(), "dv"))
    });
    rows.rec(c, s, &format!("{what}.norms"), || {
        Ok(present(r.get_norm_values("body")?.is_some(), "norms"))
    });
    rows.rec(c, s, &format!("{what}.points"), || {
        Ok(present(r.get_point_values("p_int")?.is_some(), "points"))
    });
    rows.rec(c, s, &format!("{what}.pointsMissing"), || {
        Ok(present(r.get_point_values("nope")?.is_some(), "points"))
    });
    rows.rec(c, s, &format!("{what}.floatVectors"), || {
        Ok(present(
            r.get_float_vector_values("vf")?.is_some(),
            "vectors",
        ))
    });
    rows.rec(c, s, &format!("{what}.storedFields"), || {
        LeafReader::stored_fields(r.as_ref())?;
        Ok("storedFields".to_string())
    });
    rows.rec(c, s, &format!("{what}.termVectors"), || {
        LeafReader::term_vectors(r.as_ref())?;
        Ok("termVectors".to_string())
    });
    rows.rec(c, s, &format!("{what}.liveDocs"), || {
        Ok(present(r.get_live_docs().is_some(), "bits"))
    });
    rows.rec(c, s, &format!("{what}.fieldInfos"), || {
        Ok(r.get_field_infos().size().to_string())
    });
    rows.rec(c, s, &format!("{what}.metaData"), || {
        Ok(r.get_meta_data().created_version_major().to_string())
    });
    rows.rec(c, s, &format!("{what}.checkIntegrity"), || {
        r.check_integrity()?;
        Ok("checked".to_string())
    });
    rows.rec(c, s, &format!("{what}.numDocs"), || {
        Ok(format!("{}/{}", r.num_docs(), r.max_doc()))
    });
    rows.rec(c, s, &format!("{what}.leaves"), || {
        Ok(as_index_reader(r).leaves()?.len().to_string())
    });
}

/// A listener that does nothing: Java's `k -> {}`.
struct Nothing;

impl ClosedListener for Nothing {
    fn on_close(&self, _key: CacheKey) -> Result<()> {
        Ok(())
    }
}

/// The term the composite scenarios count.
fn the() -> Term {
    Term::from_text("body", "the")
}

/// `IndexWrappersOracle.coreLifecycle`: the reader state of the core readers
/// (rmp #344 round 2).
fn core_lifecycle(rows: &mut Rows, c: &str) {
    // SegmentReader: the bounds of CodecReader.storedFields(), then every
    // member once closed
    {
        let reader = directory_reader::open(load_corpus(c)).unwrap();
        let seg = Arc::clone(&reader).leaves().unwrap()[0].leaf_reader();
        let core = seg.get_core_cache_helper().unwrap();
        let mine = seg.get_reader_cache_helper().unwrap();
        let s = "core_segment";
        let max_doc = seg.max_doc();
        let mut visitor = rucene::document::DocumentStoredFieldVisitor::new();
        rows.rec(c, s, "doc.maxDoc", || {
            LeafReader::stored_fields(seg.as_ref())?
                .document_with_visitor(max_doc, &mut visitor)?;
            Ok("doc".to_string())
        });
        rows.rec(c, s, "doc.minus1", || {
            LeafReader::stored_fields(seg.as_ref())?.document_with_visitor(-1, &mut visitor)?;
            Ok("doc".to_string())
        });
        rows.rec(c, s, "prefetch.maxDoc", || {
            LeafReader::stored_fields(seg.as_ref())?.prefetch(max_doc)?;
            Ok("prefetch".to_string())
        });
        let ir = as_index_reader(&seg);
        rows.rec(c, s, "close", || {
            reader.close()?;
            Ok(format!("{}/{}", ir.get_ref_count(), reader.get_ref_count()))
        });
        probe_closed(rows, c, s, "closed", &seg);
        rows.rec(c, s, "closed.coreHelper.add", || {
            core.add_closed_listener(Arc::new(Nothing));
            Ok("added".to_string())
        });
        rows.rec(c, s, "closed.readerHelper.add", || {
            mine.add_closed_listener(Arc::new(Nothing));
            Ok("added".to_string())
        });
        // registerParentReader calls ensureOpen() on the closed child
        rows.rec(c, s, "closed.multiReader", || {
            Ok(
                rucene::index::MultiReader::new_closing(vec![Arc::clone(&ir)])?
                    .max_doc()
                    .to_string(),
            )
        });
        rows.rec(c, s, "closed.filterLeaf", || {
            Ok(LeafReader::max_doc(&FilterLeafReader::new(Arc::clone(&seg))?).to_string())
        });
        rows.rec(c, s, "closed.parallelLeaf", || {
            Ok(
                LeafReader::max_doc(&rucene::index::ParallelLeafReader::new(vec![Arc::clone(
                    &seg,
                )])?)
                .to_string(),
            )
        });
    }
    // StandardDirectoryReader is a parent of its segment readers
    {
        let reader = directory_reader::open(load_corpus(c)).unwrap();
        let seg = as_index_reader(&Arc::clone(&reader).leaves().unwrap()[0].leaf_reader());
        let s = "core_directory";
        rows.rec(c, s, "child_close", || {
            seg.close()?;
            Ok(format!(
                "{}/{}",
                seg.get_ref_count(),
                reader.get_ref_count()
            ))
        });
        rows.rec(c, s, "leaves", || {
            Ok(Arc::clone(&reader).leaves()?.len().to_string())
        });
        rows.rec(c, s, "docFreq", || Ok(reader.doc_freq(&the())?.to_string()));
        rows.rec(c, s, "storedFields", || {
            reader.stored_fields()?;
            Ok("storedFields".to_string())
        });
        rows.rec(c, s, "termVectors", || {
            reader.term_vectors()?;
            Ok("termVectors".to_string())
        });
        rows.rec(c, s, "numDocs", || {
            Ok(format!("{}/{}", reader.num_docs(), reader.max_doc()))
        });
        rows.rec(c, s, "close", || {
            reader.close()?;
            Ok("closed".to_string())
        });
    }
    // MultiReader and ParallelCompositeReader are parents of their
    // sub-readers, transitively
    for name in ["multi", "parallel_composite"] {
        let reader = directory_reader::open(load_corpus(c)).unwrap();
        let composite: Arc<dyn IndexReader> = if name == "multi" {
            Arc::new(
                rucene::index::MultiReader::new_closing(vec![
                    Arc::clone(&reader) as Arc<dyn IndexReader>
                ])
                .unwrap(),
            )
        } else {
            Arc::new(
                rucene::index::ParallelCompositeReader::new(vec![
                    Arc::clone(&reader) as Arc<dyn rucene::index::index_reader::CompositeReader>
                ])
                .unwrap(),
            )
        };
        let s = format!("core_{name}");
        rows.rec(c, &s, "child_close", || {
            reader.close()?;
            Ok(format!(
                "{}/{}",
                composite.get_ref_count(),
                reader.get_ref_count()
            ))
        });
        rows.rec(c, &s, "leaves", || {
            Ok(Arc::clone(&composite).leaves()?.len().to_string())
        });
        rows.rec(c, &s, "docFreq", || {
            Ok(composite.doc_freq(&the())?.to_string())
        });
        rows.rec(c, &s, "storedFields", || {
            composite.stored_fields()?;
            Ok("storedFields".to_string())
        });
        rows.rec(c, &s, "close", || {
            composite.close()?;
            Ok(format!(
                "{}/{}",
                composite.get_ref_count(),
                reader.get_ref_count()
            ))
        });
    }
    // FilterLeafReader, ParallelLeafReader (parents of the wrapped reader,
    // close it when closed), FilterCodecReader (calls its doClose) and
    // SlowCodecReaderWrapper (does nothing on close)
    for name in ["filter_leaf", "parallel_leaf", "filter_codec", "slow_codec"] {
        for what in ["inner_closed", "wrapper_closed"] {
            let reader = directory_reader::open(load_corpus(c)).unwrap();
            let r#in = Arc::clone(&reader).leaves().unwrap()[0].leaf_reader();
            let w: Arc<dyn LeafReader> = match name {
                "filter_leaf" => Arc::new(FilterLeafReader::new(Arc::clone(&r#in)).unwrap()),
                "parallel_leaf" => Arc::new(
                    rucene::index::ParallelLeafReader::new(vec![Arc::clone(&r#in)]).unwrap(),
                ),
                "filter_codec" => Arc::new(FilterCodecReader::new(codec(&r#in))),
                _ => {
                    let filter: Arc<dyn LeafReader> =
                        Arc::new(FilterLeafReader::new(Arc::clone(&r#in)).unwrap());
                    let slow = SlowCodecReaderWrapper::wrap(filter).unwrap();
                    slow as Arc<dyn LeafReader>
                }
            };
            let s = format!("core_{name}");
            let wr = as_index_reader(&w);
            let ir = as_index_reader(&r#in);
            rows.rec(c, &s, &format!("{what}.close"), || {
                if what == "inner_closed" {
                    reader.close()?;
                } else {
                    wr.close()?;
                }
                Ok(format!("{}/{}", wr.get_ref_count(), ir.get_ref_count()))
            });
            probe_closed(rows, c, &s, what, &w);
            if what == "wrapper_closed" {
                rows.rec(c, &s, &format!("{what}.reader_close"), || {
                    reader.close()?;
                    Ok("closed".to_string())
                });
            }
        }
    }
    // FilterDirectoryReader (ExitableDirectoryReader): a parent of its
    // wrapped leaves, closes the wrapped reader when closed
    for what in ["inner_closed", "wrapper_closed"] {
        let reader = directory_reader::open(load_corpus(c)).unwrap();
        let w = ExitableDirectoryReaderWrapper::new(Arc::new(NeverExits))
            .wrap(Arc::clone(&reader))
            .unwrap();
        let s = "core_filter_directory";
        rows.rec(c, s, &format!("{what}.close"), || {
            if what == "inner_closed" {
                reader.close()?;
            } else {
                w.close()?;
            }
            Ok(format!("{}/{}", w.get_ref_count(), reader.get_ref_count()))
        });
        rows.rec(c, s, &format!("{what}.leaves"), || {
            Ok(Arc::clone(&w).leaves()?.len().to_string())
        });
        rows.rec(c, s, &format!("{what}.docFreq"), || {
            Ok(w.doc_freq(&the())?.to_string())
        });
        rows.rec(c, s, &format!("{what}.storedFields"), || {
            w.stored_fields()?;
            Ok("storedFields".to_string())
        });
        rows.rec(c, s, &format!("{what}.numDocs"), || {
            Ok(format!("{}/{}", w.num_docs(), w.max_doc()))
        });
        rows.rec(c, s, &format!("{what}.version"), || {
            Ok((w.version()? > 0).to_string())
        });
    }
    core_round3(rows, c);
}

/// Restores `IndexWriter`'s document limit when dropped, as the oracle's
/// `finally { setMaxDocs(IndexWriter.MAX_DOCS); }` does (the limit it had).
struct RestoreMaxDocs(i32);

impl Drop for RestoreMaxDocs {
    fn drop(&mut self) {
        rucene::index::index_writer::set_max_docs(self.0).unwrap();
    }
}

/// `IndexWrappersOracle.coreRound3`: the last core items of rmp #344 (round 3).
fn core_round3(rows: &mut Rows, c: &str) {
    // BaseCompositeReader.readerIndex bounds the document id by maxDoc
    {
        let reader = directory_reader::open(load_corpus(c)).unwrap();
        let s = "core_directory_bounds";
        let max = reader.max_doc();
        let mut visitor = rucene::document::DocumentStoredFieldVisitor::new();
        for (step, doc) in [
            ("storedFields.last", max - 1),
            ("storedFields.maxDoc", max),
            ("storedFields.minus1", -1),
        ] {
            rows.rec(c, s, step, || {
                reader
                    .stored_fields()?
                    .document_with_visitor(doc, &mut visitor)?;
                Ok("doc".to_string())
            });
        }
        rows.rec(c, s, "storedFields.prefetch.maxDoc", || {
            reader.stored_fields()?.prefetch(max)?;
            Ok("prefetch".to_string())
        });
        rows.rec(c, s, "termVectors.last", || {
            Ok(if reader.term_vectors()?.get(max - 1)?.is_none() {
                "null"
            } else {
                "fields"
            }
            .to_string())
        });
        for (step, doc) in [("termVectors.maxDoc", max), ("termVectors.minus1", -1)] {
            rows.rec(c, s, step, || {
                reader.term_vectors()?.get(doc)?;
                Ok("fields".to_string())
            });
        }
        rows.rec(c, s, "termVectors.prefetch.maxDoc", || {
            reader.term_vectors()?.prefetch(max)?;
            Ok("prefetch".to_string())
        });
        reader.close().unwrap();
    }
    // A ParallelCompositeReader over more documents than the limit is refused
    // as a MultiReader is (BaseCompositeReader's constructor)
    {
        let reader = directory_reader::open(load_corpus(c)).unwrap();
        let s = "core_max_docs";
        {
            let _restore = RestoreMaxDocs(rucene::index::index_writer::get_actual_max_docs());
            rucene::index::index_writer::set_max_docs(reader.max_doc() - 1).unwrap();
            rows.rec(c, s, "parallel_composite", || {
                Ok(rucene::index::ParallelCompositeReader::with_close_flag(
                    false,
                    vec![Arc::clone(&reader)
                        as Arc<dyn rucene::index::index_reader::CompositeReader>],
                )?
                .max_doc()
                .to_string())
            });
            rows.rec(c, s, "multi", || {
                Ok(rucene::index::MultiReader::new(
                    vec![Arc::clone(&reader) as Arc<dyn IndexReader>],
                    false,
                )?
                .max_doc()
                .to_string())
            });
        }
        rows.rec(c, s, "refCount", || Ok(reader.get_ref_count().to_string()));
        rows.rec(c, s, "close", || {
            reader.close()?;
            Ok(reader.get_ref_count().to_string())
        });
    }
    // ParallelLeafReader closes or dereferences its sub-readers with
    // IndexReader.decRef, which notifies their reader-closed listeners
    for close_subs in [true, false] {
        let reader = directory_reader::open(load_corpus(c)).unwrap();
        let seg = Arc::clone(&reader).leaves().unwrap()[0].leaf_reader();
        let reader_closed = Arc::new(AtomicI32::new(0));
        let core_closed = Arc::new(AtomicI32::new(0));
        seg.get_reader_cache_helper()
            .unwrap()
            .add_closed_listener(Arc::new(Counting(Arc::clone(&reader_closed))));
        seg.get_core_cache_helper()
            .unwrap()
            .add_closed_listener(Arc::new(Counting(Arc::clone(&core_closed))));
        let w: Arc<dyn LeafReader> = Arc::new(
            rucene::index::ParallelLeafReader::with_close_flag(close_subs, vec![Arc::clone(&seg)])
                .unwrap(),
        );
        let s = format!(
            "core_parallel_leaf_listeners_{}",
            if close_subs { "close" } else { "decref" }
        );
        let ir = as_index_reader(&seg);
        let state = || {
            format!(
                "{}/{}/{}",
                reader_closed.load(Ordering::SeqCst),
                core_closed.load(Ordering::SeqCst),
                ir.get_ref_count()
            )
        };
        let wr = as_index_reader(&w);
        rows.rec(c, &s, "wrapper_close", || {
            wr.close()?;
            Ok(state())
        });
        rows.rec(c, &s, "reader_close", || {
            reader.close()?;
            Ok(state())
        });
    }
    // the wrapper's decRef is the sub-reader's last: it closes the sub-reader
    {
        let reader = directory_reader::open(load_corpus(c)).unwrap();
        let seg = Arc::clone(&reader).leaves().unwrap()[0].leaf_reader();
        let reader_closed = Arc::new(AtomicI32::new(0));
        let core_closed = Arc::new(AtomicI32::new(0));
        seg.get_reader_cache_helper()
            .unwrap()
            .add_closed_listener(Arc::new(Counting(Arc::clone(&reader_closed))));
        seg.get_core_cache_helper()
            .unwrap()
            .add_closed_listener(Arc::new(Counting(Arc::clone(&core_closed))));
        let w: Arc<dyn LeafReader> = Arc::new(
            rucene::index::ParallelLeafReader::with_close_flag(false, vec![Arc::clone(&seg)])
                .unwrap(),
        );
        let s = "core_parallel_leaf_listeners_decref_last";
        let ir = as_index_reader(&seg);
        let state = || {
            format!(
                "{}/{}/{}",
                reader_closed.load(Ordering::SeqCst),
                core_closed.load(Ordering::SeqCst),
                ir.get_ref_count()
            )
        };
        let wr = as_index_reader(&w);
        rows.rec(c, s, "reader_close", || {
            reader.close()?;
            Ok(state())
        });
        rows.rec(c, s, "wrapper_close", || {
            wr.close()?;
            Ok(state())
        });
    }
}

/// `() -> false`: a timeout that never fires.
#[derive(Debug)]
struct NeverExits;

impl QueryTimeout for NeverExits {
    fn should_exit(&self) -> bool {
        false
    }
}

struct Counting(Arc<AtomicI32>);

impl ClosedListener for Counting {
    fn on_close(&self, _key: CacheKey) -> Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn own_cache_key(rows: &mut Rows, c: &str, reader: &Arc<dyn DirectoryReader>) {
    let subs: Vec<Arc<dyn IndexReader>> = Arc::clone(reader)
        .leaves()
        .unwrap()
        .iter()
        .map(|ctx| ctx.leaf_reader().as_index_reader())
        .collect();
    let r = holder(&subs).unwrap();
    rows.rec(c, "owncachekey", "toString", || Ok(r.to_string()));
    rows.rec(c, "owncachekey", "docs", || {
        Ok(format!(
            "{}/{}/{}",
            r.max_doc(),
            r.num_docs(),
            Arc::clone(&r).leaves()?.len()
        ))
    });
    rows.rec(c, "owncachekey", "key", || {
        let helper = r.get_reader_cache_helper().unwrap();
        let mut sb = if same_key(
            helper.as_ref(),
            r.get_reader_cache_helper().unwrap().as_ref(),
        ) {
            "stable".to_string()
        } else {
            "unstable".to_string()
        };
        for sub in &subs {
            let same = sub
                .get_reader_cache_helper()
                .is_some_and(|sh| same_key(sh.as_ref(), helper.as_ref()));
            sb.push_str(if same { " same" } else { " other" });
        }
        Ok(sb)
    });
    let calls = Arc::new(AtomicI32::new(0));
    rows.rec(c, "owncachekey", "addListener", || {
        r.get_reader_cache_helper()
            .unwrap()
            .add_closed_listener(Arc::new(Counting(Arc::clone(&calls))));
        r.get_reader_cache_helper()
            .unwrap()
            .add_closed_listener(Arc::new(Counting(Arc::clone(&calls))));
        Ok(calls.load(Ordering::SeqCst).to_string())
    });
    rows.rec(c, "owncachekey", "close", || {
        r.close()?;
        Ok(format!(
            "{} refCount={} subRefCount={}",
            calls.load(Ordering::SeqCst),
            r.get_ref_count(),
            subs[0].get_ref_count()
        ))
    });
    rows.rec(c, "owncachekey", "addListenerAfterClose", || {
        r.get_reader_cache_helper()
            .unwrap()
            .add_closed_listener(Arc::new(Counting(Arc::clone(&calls))));
        Ok(calls.load(Ordering::SeqCst).to_string())
    });
}

/// `inThread(name, body)`: runs `body` in a new `Thread` named `name`, waits
/// for it, and rethrows what it threw.
fn in_thread<T: Send + 'static>(
    name: &str,
    body: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let result: Arc<Mutex<Option<std::thread::Result<Result<T>>>>> = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&result);
    let t = Thread::new(name, move || {
        let r = catch_unwind(AssertUnwindSafe(body));
        *slot.lock().unwrap() = Some(r);
    });
    t.start();
    t.join().unwrap();
    let r = result.lock().unwrap().take().unwrap();
    match r {
        Ok(r) => r,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

struct YesVisitor;

impl StoredFieldVisitor for YesVisitor {
    fn needs_field(&mut self, _field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        Ok(StoredFieldVisitorStatus::Yes)
    }
}

fn misuse(rows: &mut Rows, c: &str, reader: &Arc<dyn DirectoryReader>) {
    let leaves = Arc::clone(reader).leaves().unwrap();
    let r#in = leaves[0].leaf_reader();
    let with_deletions = leaves
        .iter()
        .map(|ctx| ctx.leaf_reader())
        .find(|l| as_index_reader(l).has_deletions())
        .unwrap();
    let r = Arc::new(AssertingLeafReader::new(Arc::clone(&r#in)).unwrap());
    rows.rec(c, "misuse", "termvectors_thread", || {
        let acquiring = Arc::clone(&r);
        let tv: Box<dyn TermVectors> = in_thread("iw-acquire", move || {
            LeafReader::term_vectors(acquiring.as_ref())
        })?;
        in_thread("iw-consume", move || {
            Ok(tv.get(0)?.map_or("null", |_| "fields").to_string())
        })
    });
    rows.rec(c, "misuse", "storedfields_thread", || {
        let acquiring = Arc::clone(&r);
        let sf: Box<dyn StoredFields> = in_thread("iw-acquire", move || {
            LeafReader::stored_fields(acquiring.as_ref())
        })?;
        in_thread("iw-consume", move || {
            sf.document(0)?;
            Ok("read".to_string())
        })
    });
    let rd = Arc::new(AssertingLeafReader::new(Arc::clone(&with_deletions)).unwrap());
    rows.rec(c, "misuse", "bits_thread", || {
        let acquiring = Arc::clone(&rd);
        let live: Box<dyn Bits> =
            in_thread("iw-acquire", move || Ok(acquiring.get_live_docs().unwrap()))?;
        in_thread("iw-consume", move || Ok(live.get(0).to_string()))
    });
    rows.rec(c, "misuse", "bits_out_of_range", || {
        let live = rd.get_live_docs().unwrap();
        Ok(live.get(live.length()).to_string())
    });
    rows.rec(c, "misuse", "bits_kind", || {
        Ok(match rd.get_live_docs().and_then(Bits::into_live_docs) {
            Some(ld) => format!("livedocs deleted={}", ld.deleted_count()),
            None => "bits".to_string(),
        })
    });
    rows.rec(c, "misuse", "nextdoc_after_end", || {
        let mut te = r.terms("body")?.unwrap().iterator()?;
        te.next()?;
        let mut pe = te.postings(None, POSTINGS_ENUM_FREQS)?;
        while pe.next_doc()? != NO_MORE_DOCS {}
        Ok(pe.next_doc()?.to_string())
    });
    rows.rec(c, "misuse", "term_unpositioned", || {
        let mut te = r.terms("body")?.unwrap().iterator()?;
        while te.next()?.is_some() {}
        Ok(format!("{:?}", te.term()?))
    });
    rows.rec(c, "misuse", "dv_unpositioned", || {
        let dv = r.get_numeric_doc_values("dv_num")?.unwrap();
        Ok(dv.long_value()?.to_string())
    });
    rows.row(c, "misuse", "dv_thread", "not expressible".to_string());
    rows.rec(c, "misuse", "mismatched_bogus", || {
        let infos = r#in.get_field_infos();
        let fi = infos.field_info("s_str").unwrap();
        let empty = FieldInfos::new(Vec::new())?;
        let mut yes = YesVisitor;
        MismatchedVisitor::new(&mut yes, &empty).string_field(fi, "x")?;
        Ok("visited".to_string())
    });
    rows.rec(c, "misuse", "merging_dir_cast", || {
        let asserting: Arc<dyn DirectoryReader> =
            Arc::new(AssertingDirectoryReader::new(Arc::clone(reader))?);
        Ok(MergingDirectoryReaderWrapper::new(asserting)?.to_string())
    });
}

/// `addOne(dir)`: one more document committed.
fn add_one(dir: &Arc<dyn Directory>) -> Result<()> {
    let config = IndexWriterConfig::with_analyzer(Some(Arc::new(StandardAnalyzer::new())));
    config.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))?;
    let iw = IndexWriter::new(Arc::clone(dir), config)?;
    let mut d = Document::new();
    d.add(Box::new(StringField::new(
        "id",
        "id-9999".to_string(),
        Store::YES,
    )?));
    iw.add_document(&d)?;
    iw.commit()?;
    iw.close()
}

fn reopen(rows: &mut Rows, c: &str) {
    for name in ["asserting_dir", "mismatched_dir", "merging_dir"] {
        let dir = load_corpus(c);
        let reader = directory_reader::open(Arc::clone(&dir)).unwrap();
        let wrapped: Arc<dyn DirectoryReader> = match name {
            "asserting_dir" => Arc::new(AssertingDirectoryReader::new(reader).unwrap()),
            "mismatched_dir" => Arc::new(
                MismatchedDirectoryReader::new(reader, Arc::new(JdkRandom::new(42))).unwrap(),
            ),
            _ => Arc::new(MergingDirectoryReaderWrapper::new(reader).unwrap()),
        };
        rows.rec(c, "reopen", &format!("{name}.unchanged"), || {
            Ok(
                match directory_reader::open_if_changed(Arc::clone(&wrapped))? {
                    None => "null".to_string(),
                    Some(r) => r.to_string(),
                },
            )
        });
        add_one(&dir).unwrap();
        rows.rec(c, "reopen", &format!("{name}.changed"), || {
            let r = directory_reader::open_if_changed(Arc::clone(&wrapped))?.unwrap();
            let s = r.to_string();
            let result = format!(
                "{} {}/{}/{}",
                &s[..s.find('(').unwrap()],
                r.max_doc(),
                r.num_docs(),
                Arc::clone(&r).leaves()?.len()
            );
            r.close()?;
            Ok(result)
        });
        wrapped.close().unwrap();
    }
    // MergeReaderWrapper is a reader of its own: closing it closes the wrapped leaf
    let dir = load_corpus(c);
    let reader = directory_reader::open(dir).unwrap();
    let leaf0 = Arc::clone(&reader).leaves().unwrap()[0].leaf_reader();
    let mrw = Arc::new(MergeReaderWrapper::new(codec(&leaf0)).unwrap());
    rows.rec(c, "mergewrapper_close", "close", || {
        mrw.close()?;
        Ok(format!(
            "{}/{}",
            mrw.get_ref_count(),
            Arc::clone(&reader).leaves()?[0]
                .leaf_reader()
                .as_index_reader()
                .get_ref_count()
        ))
    });
    rows.rec(c, "mergewrapper_close", "terms", || {
        Ok(match mrw.terms("body")? {
            None => "null".to_string(),
            Some(_) => "terms".to_string(),
        })
    });
    rows.rec(c, "mergewrapper_close", "storedFields", || {
        LeafReader::stored_fields(mrw.as_ref())?;
        Ok("storedFields".to_string())
    });
    // the directory reader then decRefs a leaf already closed
    rows.rec(c, "mergewrapper_close", "reader_close", || {
        reader.close()?;
        Ok("closed".to_string())
    });
    lifecycle(rows, c);
}

/// `IndexWrappersOracle.lifecycle`: each wrapper is a reader of its own.
fn lifecycle(rows: &mut Rows, c: &str) {
    let names = [
        "asserting",
        "alldeleted",
        "fieldfilter",
        "mismatched",
        "merging_codec",
        "mismatched_codec",
    ];
    for name in names {
        for what in ["inner_closed", "wrapper_closed"] {
            let reader = directory_reader::open(load_corpus(c)).unwrap();
            let r#in = Arc::clone(&reader).leaves().unwrap()[0].leaf_reader();
            let w: Arc<dyn LeafReader> = match name {
                "asserting" => Arc::new(AssertingLeafReader::new(Arc::clone(&r#in)).unwrap()),
                "alldeleted" => Arc::new(AllDeletedFilterReader::new(Arc::clone(&r#in)).unwrap()),
                "fieldfilter" => Arc::new(
                    FieldFilterLeafReader::new(Arc::clone(&r#in), FILTER_FIELDS, false).unwrap(),
                ),
                "mismatched" => Arc::new(
                    MismatchedLeafReader::new(Arc::clone(&r#in), &JdkRandom::new(7)).unwrap(),
                ),
                "merging_codec" => Arc::new(MergingCodecReader::new(codec(&r#in))),
                _ => {
                    Arc::new(MismatchedCodecReader::new(codec(&r#in), &JdkRandom::new(7)).unwrap())
                }
            };
            let s = format!("lifecycle_{name}");
            let wr = as_index_reader(&w);
            let ir = as_index_reader(&r#in);
            rows.rec(c, &s, &format!("{what}.close"), || {
                if what == "inner_closed" {
                    reader.close()?;
                } else {
                    wr.close()?;
                }
                Ok(format!("{}/{}", wr.get_ref_count(), ir.get_ref_count()))
            });
            rows.rec(c, &s, &format!("{what}.terms"), || {
                Ok(if w.terms("body")?.is_none() {
                    "null"
                } else {
                    "terms"
                }
                .to_string())
            });
            rows.rec(c, &s, &format!("{what}.numeric"), || {
                Ok(if w.get_numeric_doc_values("dv_num")?.is_none() {
                    "null"
                } else {
                    "dv"
                }
                .to_string())
            });
            rows.rec(c, &s, &format!("{what}.liveDocs"), || {
                Ok(if w.get_live_docs().is_none() {
                    "null"
                } else {
                    "bits"
                }
                .to_string())
            });
            rows.rec(c, &s, &format!("{what}.fieldInfos"), || {
                Ok(w.get_field_infos().size().to_string())
            });
            rows.rec(c, &s, &format!("{what}.leaves"), || {
                Ok(Arc::clone(&wr).leaves()?.len().to_string())
            });
            if what == "wrapper_closed" {
                rows.rec(c, &s, &format!("{what}.reader_close"), || {
                    reader.close()?;
                    Ok("closed".to_string())
                });
            }
        }
    }
    // FieldFilterLeafReader does not filter getNumericDocValues, which
    // AssertingLeafReader checks against the (filtered) field infos
    let reader = directory_reader::open(load_corpus(c)).unwrap();
    let r#in = Arc::clone(&reader).leaves().unwrap()[0].leaf_reader();
    let filtered: Arc<dyn LeafReader> =
        Arc::new(FieldFilterLeafReader::new(r#in, ["body"], false).unwrap());
    let a = AssertingLeafReader::new(filtered).unwrap();
    let presence = |o: bool| if o { "dv" } else { "null" }.to_string();
    rows.rec(c, "asserting_fieldfilter", "numeric", || {
        Ok(presence(a.get_numeric_doc_values("dv_num")?.is_some()))
    });
    rows.rec(c, "asserting_fieldfilter", "binary", || {
        Ok(presence(a.get_binary_doc_values("dv_bin")?.is_some()))
    });
    rows.rec(c, "asserting_fieldfilter", "norms", || {
        Ok(presence(a.get_norm_values("tv")?.is_some()))
    });
    rows.rec(c, "asserting_fieldfilter", "points", || {
        Ok(if a.get_point_values("p_int")?.is_none() {
            "null"
        } else {
            "points"
        }
        .to_string())
    });
    reader.close().unwrap();
    core_lifecycle(rows, c);
}

/// `IndexWrappersOracle.pkDuplicates`: two segments holding the same ids; the
/// lookup answers the larger segment's document.
fn pk_duplicates(rows: &mut Rows) {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::with_analyzer(Some(Arc::new(StandardAnalyzer::new())));
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    let iw = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for batch in [&["a", "b"][..], &["a", "c", "d", "b"][..]] {
        for id in batch {
            let mut d = Document::new();
            d.add(Box::new(
                StringField::new("id", id.to_string(), Store::YES).unwrap(),
            ));
            iw.add_document(&d).unwrap();
        }
        iw.commit().unwrap();
    }
    iw.close().unwrap();
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    let mut pk = PerThreadPKLookup::new(&reader, "id").unwrap();
    let mut sb = format!("{}:", Arc::clone(&reader).leaves().unwrap().len());
    for id in ["a", "b", "c", "d", "e"] {
        sb.push_str(&format!(" {}", pk.lookup(&bytes(id)).unwrap()));
    }
    rows.row("pkdup", "pklookup", "lookups", format!("ok\t{sb}"));
    reader.close().unwrap();
}

// ---------------------------------------------------------------------------
// PointsStackTracker
// ---------------------------------------------------------------------------

fn points_index() -> Arc<dyn Directory> {
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let config = IndexWriterConfig::with_analyzer(Some(Arc::new(StandardAnalyzer::new())));
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    let iw = IndexWriter::new(Arc::clone(&dir), config).unwrap();
    for i in 0..5000i32 {
        let mut d = Document::new();
        d.add(Box::new(
            IntPoint::new("p", &[(i * 7919) % 10007, (i * 104729) % 10009]).unwrap(),
        ));
        d.add(Box::new(
            LongPoint::new("q", &[(i64::from(i) * 2654435761) % 1000003 - 500000]).unwrap(),
        ));
        iw.add_document(&d).unwrap();
    }
    iw.force_merge(1).unwrap();
    iw.commit().unwrap();
    iw.close().unwrap();
    dir
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct TrackingVisitor<'a> {
    tracker: &'a RefCell<PointsStackTracker>,
    sizes: &'a RefCell<Vec<String>>,
}

impl IntersectVisitor for TrackingVisitor<'_> {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Ok(())
    }

    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        Ok(())
    }

    fn compare(&self, min: &[u8], max: &[u8]) -> Result<Relation> {
        let mut tracker = self.tracker.borrow_mut();
        tracker.on_compare(min, max);
        self.sizes
            .borrow_mut()
            .push(tracker.stack.len().to_string());
        Ok(Relation::CellCrossesQuery)
    }
}

fn tracker(rows: &mut Rows, leaf: &dyn LeafReader, field: &str, num_dims: i32, bytes: i32) {
    let tracker = RefCell::new(PointsStackTracker::new(num_dims, bytes));
    let sizes = RefCell::new(Vec::new());
    leaf.get_point_values(field)
        .unwrap()
        .unwrap()
        .intersect(&mut TrackingVisitor {
            tracker: &tracker,
            sizes: &sizes,
        })
        .unwrap();
    let sizes = sizes.into_inner();
    rows.row(
        "points",
        "tracker",
        &format!("{field}.compares"),
        format!("ok\t{} {}", digest(&sizes), sizes.join(",")),
    );
    let tracker = tracker.into_inner();
    let stack: String = tracker
        .stack
        .iter()
        .map(|cell| {
            format!(
                "{}-{} ",
                hex(&cell.min_packed_value),
                hex(&cell.max_packed_value)
            )
        })
        .collect();
    rows.row(
        "points",
        "tracker",
        &format!("{field}.stack"),
        format!("ok\t{stack}"),
    );
    let n = tracker.stack.len();
    if n >= 2 {
        rows.row(
            "points",
            "tracker",
            &format!("{field}.contains"),
            format!(
                "ok\t{} {}",
                tracker.stack[n - 2].contains(&tracker.stack[n - 1]),
                tracker.stack[n - 1].contains(&tracker.stack[n - 2])
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// Every row of `iw_wrappers.tsv` reproduced through the port.
#[test]
fn index_wrappers_match_the_java_oracle() {
    install_codecs();
    let expected: BTreeMap<String, String> = oracle_support::rows("iw_wrappers.tsv")
        .into_iter()
        .map(|r| (r[..3].join("\t"), r[3..].join("\t")))
        .collect();
    let shuffle_seeds: Vec<i64> = expected
        .keys()
        .filter_map(|k| k.strip_prefix("nocfs\tshuffle\tseed"))
        .map(|s| s.parse().unwrap())
        .collect();
    let mut rows = Rows::default();
    for c in ["nocfs", "cfs"] {
        let reader = directory_reader::open(load_corpus(c)).unwrap();
        wrappers(&mut rows, c, &reader, &shuffle_seeds);
        misuse(&mut rows, c, &reader);
        own_cache_key(&mut rows, c, &reader);
        reader.close().unwrap();
        reopen(&mut rows, c);
    }
    let dir = points_index();
    let reader = directory_reader::open(dir).unwrap();
    let leaf = Arc::clone(&reader).leaves().unwrap()[0].leaf_reader();
    tracker(&mut rows, leaf.as_ref(), "p", 2, 4);
    tracker(&mut rows, leaf.as_ref(), "q", 1, 8);
    // a segment without doc values: getDocValuesSkipper dereferences the
    // missing producer
    let mrw = MergeReaderWrapper::new(codec(&leaf)).unwrap();
    let presence = |o: bool| if o { "values" } else { "null" }.to_string();
    rows.rec("points", "mergewrapper", "skipper", || {
        Ok(presence(mrw.get_doc_values_skipper("p")?.is_some()))
    });
    rows.rec("points", "mergewrapper", "numeric", || {
        Ok(presence(mrw.get_numeric_doc_values("p")?.is_some()))
    });
    rows.rec("points", "mergewrapper", "norms", || {
        Ok(presence(mrw.get_norm_values("p")?.is_some()))
    });
    pk_duplicates(&mut rows);

    let mut actual = rows.0;
    let mut expected = expected;
    // The dump's `   live` lines read the impacts of a postings enum through a
    // view; core's `BlockPostingsEnum::get_impacts` returns a snapshot (rmp
    // #334, the exclusion of `oracle::assert_same_lines_but_live`). They are
    // compared apart and must still differ somewhere, so that this exclusion
    // fails once the core is fixed.
    let live_keys: Vec<String> = expected
        .keys()
        .filter(|k| k.ends_with(".dumplive"))
        .cloned()
        .collect();
    let mut live_differences = 0;
    for key in live_keys {
        let e = expected.remove(&key);
        let a = actual.remove(&key);
        assert!(a.is_some(), "missing {key}");
        if e != a {
            live_differences += 1;
        }
    }
    assert!(
        live_differences > 0,
        "every dumplive row matches: the core's live impacts are fixed, remove the exclusion"
    );
    for (suffix, how, _reason) in DECLARED {
        let keys: Vec<String> = expected
            .keys()
            .filter(|k| k.ends_with(suffix))
            .cloned()
            .collect();
        assert!(!keys.is_empty(), "declared row *{suffix} missing");
        for key in keys {
            let (Some(e), Some(a)) = (expected.get(&key).cloned(), actual.get(&key).cloned())
            else {
                panic!("declared row {key} missing");
            };
            // A declared row that starts matching is no longer a divergence:
            // its declaration must go.
            assert_ne!(
                a, e,
                "{key} now matches the oracle: remove it from DECLARED"
            );
            if *how == "class" {
                let class = |s: &str| s.split('\t').take(2).collect::<Vec<_>>().join("\t");
                assert_eq!(class(&a), class(&e), "{key}: the error class differs");
            }
            expected.remove(&key);
            actual.remove(&key);
        }
    }
    oracle_support::assert_same("iw_wrappers.tsv", &expected, &actual);
}
