//! `InetAddressPoint` against Apache Lucene Core 10.5.0 (rmp task #313).
//!
//! The oracle is `InetAddressPointFixture.java`, launched from source against
//! `lucene-core-10.5.0.jar`; the address list, the corpus and the query set
//! are defined by the same values and formulas here and there.
//!
//! * `values`: for every address of [`ADDRESSES`] — IPv4, IPv6, IPv4-mapped,
//!   IPv4-compatible and the boundary values — its host address and
//!   `getAddress()` length, `encode` (hex), `decode`, `nextUp`/`nextDown`
//!   (including the overflow and underflow errors at `MAX_VALUE`/`MIN_VALUE`),
//!   the field's `toString`, and the `toString` of its exact, prefix (every
//!   length of [`PREFIXES`], including the illegal ones and their errors) and
//!   set queries; then `MIN_VALUE`/`MAX_VALUE`, range and set queries over
//!   several addresses, and the errors of `decode` and `setBytesValue`. The
//!   two reports are compared line by line.
//! * Index: Rucene writes the corpus under `R`, Lucene under `L` (two commits,
//!   deletions in the second); both sides run the query set — exact, prefix,
//!   range (inclusive, exclusive through `nextUp`/`nextDown`, inverted) and
//!   set queries — over both indexes. Per query: its `toString`, the total
//!   hits and every hit's doc id and stored id. The four reports must agree.
//! * Positive control: a perturbed Rucene corpus (document 1 indexes one more
//!   address) must change the report, with the same document counts.

use std::fmt::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{Document, FieldValueSetters, InetAddressPoint, Store, StringField};
use rucene::error::{LuceneError, LuceneErrorKind};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::{IndexReader, Term};
use rucene::search::{IndexSearcher, Query, TotalHitsRelation};
use rucene::store::{Directory, FSDirectory};
use rucene::util::BytesRef;

/// `InetAddressPointFixture.ADDRESSES`.
const ADDRESSES: [&str; 19] = [
    "0.0.0.0",
    "1.2.3.4",
    "1.2.3.255",
    "10.0.0.1",
    "255.255.255.255",
    "::",
    "::1",
    "::ffff",
    "::1:0",
    "::fffe:ffff:ffff",
    "::ffff:1.2.3.4",
    "::1.2.3.4",
    "::1:0:0:0",
    "fec0::f66d",
    "2001:db8::a",
    "2001::a6bd:fc80",
    "fdc8:57ed:f042:0ad1:f66d:4ff:fe90:ce0c",
    "ffff:ffff:ffff:ffff:ffff:ffff:ffff:fffe",
    "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
];

/// `InetAddressPointFixture.PREFIXES`.
const PREFIXES: [i32; 18] = [
    -1, 0, 1, 7, 8, 9, 16, 24, 25, 31, 32, 33, 64, 96, 100, 127, 128, 129,
];

/// `InetAddressPointFixture.DOCS`.
const DOCS: i32 = 400;

/// `InetAddressPointFixture.TOP_N`.
const TOP_N: i32 = 1000;

/// `InetAddressPointFixture.SPECIAL`.
const SPECIAL: [&str; 7] = [
    "::",
    "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
    "255.255.255.255",
    "0.0.0.0",
    "::fffe:ffff:ffff",
    "::1:0:0:0",
    "::1.2.3.4",
];

/// `InetAddressPointFixture.ip(String)`: `InetAddress.getByName` of a literal.
fn ip(literal: &str) -> IpAddr {
    literal.parse().unwrap()
}

/// `InetAddressPointFixture.inet(byte[])`: `InetAddress.getByAddress`.
fn inet(b: &[u8]) -> IpAddr {
    InetAddressPoint::decode(b).unwrap()
}

/// `InetAddress.getAddress().length`: 4 for an IPv4 address, which is what
/// `decode` returns for an IPv4-mapped encoding.
fn address_len(a: IpAddr) -> usize {
    match InetAddressPoint::decode(&InetAddressPoint::encode(a)).unwrap() {
        IpAddr::V4(_) => 4,
        IpAddr::V6(_) => 16,
    }
}

/// `InetAddress.getHostAddress()`, read through the field's range-query
/// rendering, which prints `decode(value).getHostAddress()`.
fn host(a: IpAddr) -> String {
    let q = InetAddressPoint::new_exact_query("f", a)
        .unwrap()
        .to_query_string("");
    let inner = q.strip_prefix("f:[").unwrap().strip_suffix(']').unwrap();
    inner.split(" TO ").next().unwrap().to_string()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `InetAddressPointFixture.error(RuntimeException)`.
fn error(e: &LuceneError) -> String {
    let message = match e.kind() {
        LuceneErrorKind::IllegalArgument { message, .. } => message.clone(),
        LuceneErrorKind::Arithmetic(message) => message.clone(),
        other => format!("{other:?}"),
    };
    format!("! {} {message}", e.java_simple_name())
}

fn attempt(r: rucene::error::Result<String>) -> String {
    r.unwrap_or_else(|e| error(&e))
}

/// `InetAddressPointFixture.values()`, line for line.
fn values_report() -> String {
    let mut out = String::new();
    for literal in ADDRESSES {
        let a = ip(literal);
        writeln!(out, "addr {literal}").unwrap();
        writeln!(out, "  host {} len {}", host(a), address_len(a)).unwrap();
        let encoded = InetAddressPoint::encode(a);
        writeln!(out, "  encode {}", hex(&encoded)).unwrap();
        let decoded = InetAddressPoint::decode(&encoded).unwrap();
        writeln!(
            out,
            "  decode {} len {}",
            host(decoded),
            address_len(decoded)
        )
        .unwrap();
        writeln!(
            out,
            "  nextUp {}",
            attempt(InetAddressPoint::next_up(a).map(host))
        )
        .unwrap();
        writeln!(
            out,
            "  nextDown {}",
            attempt(InetAddressPoint::next_down(a).map(host))
        )
        .unwrap();
        writeln!(out, "  field {}", InetAddressPoint::new("f", a)).unwrap();
        writeln!(
            out,
            "  exact {}",
            InetAddressPoint::new_exact_query("f", a)
                .unwrap()
                .to_query_string("")
        )
        .unwrap();
        for p in PREFIXES {
            writeln!(
                out,
                "  prefix {p} {}",
                attempt(
                    InetAddressPoint::new_prefix_query("f", a, p).map(|q| q.to_query_string(""))
                )
            )
            .unwrap();
        }
        writeln!(
            out,
            "  set {}",
            InetAddressPoint::new_set_query("f", &[a])
                .unwrap()
                .to_query_string("")
        )
        .unwrap();
    }
    writeln!(out, "min {}", host(InetAddressPoint::MIN_VALUE)).unwrap();
    writeln!(out, "max {}", host(InetAddressPoint::MAX_VALUE)).unwrap();
    let mut i = 0;
    while i + 1 < ADDRESSES.len() {
        writeln!(
            out,
            "range {} {} {}",
            ADDRESSES[i],
            ADDRESSES[i + 1],
            InetAddressPoint::new_range_query("f", ip(ADDRESSES[i]), ip(ADDRESSES[i + 1]))
                .unwrap()
                .to_query_string("")
        )
        .unwrap();
        i += 3;
    }
    let all: Vec<IpAddr> = ADDRESSES.iter().map(|a| ip(a)).collect();
    writeln!(
        out,
        "set all {}",
        InetAddressPoint::new_set_query("f", &all)
            .unwrap()
            .to_query_string("")
    )
    .unwrap();
    writeln!(
        out,
        "set empty {}",
        InetAddressPoint::new_set_query("f", &[])
            .unwrap()
            .to_query_string("")
    )
    .unwrap();
    for len in [0usize, 3, 5, 15, 17] {
        writeln!(
            out,
            "decode len {len} {}",
            attempt(InetAddressPoint::decode(&vec![0u8; len]).map(host))
        )
        .unwrap();
    }
    let field = InetAddressPoint::new("f", ip("1.2.3.4"));
    writeln!(
        out,
        "setBytesValue {}",
        attempt(
            field
                .set_bytes_value(BytesRef::new(b"x".to_vec()))
                .map(|()| "ok".to_string())
        )
    )
    .unwrap();
    out
}

// ---------------------------------------------------------------------------
// Corpus: the formulas of InetAddressPointFixture
// ---------------------------------------------------------------------------

fn id(i: i32) -> String {
    format!("id-{i:04}")
}

/// `InetAddressPointFixture.v4(int)`.
fn v4(i: i32) -> Vec<u8> {
    vec![10, (i % 3) as u8, (i / 16) as u8, ((i * 37) % 256) as u8]
}

/// `InetAddressPointFixture.v6(int)`.
fn v6(i: i32) -> Vec<u8> {
    let mut b = vec![0u8; 16];
    b[0] = 0x20;
    b[1] = 0x01;
    b[2] = 0x0d;
    b[3] = 0xb8;
    b[13] = (i % 7) as u8;
    let low = (i * 13) % 65536;
    b[14] = (low >> 8) as u8;
    b[15] = low as u8;
    b
}

/// `InetAddressPointFixture.doc(int)`; the perturbed corpus gives document 1
/// one more address.
fn doc(i: i32, perturbed: bool) -> Document {
    let mut d = Document::new();
    d.add(Box::new(StringField::new("id", id(i), Store::YES).unwrap()));
    let mut add = |a: IpAddr| d.add(Box::new(InetAddressPoint::new("ip", a)));
    match i % 5 {
        1 => add(inet(&v4(i))),
        2 => add(inet(&v6(i))),
        3 => {
            add(inet(&v4(i)));
            add(inet(&v6(i)));
        }
        4 => {
            add(inet(&v4(i)));
            add(inet(&v4(i + 1)));
        }
        _ => {}
    }
    if i < SPECIAL.len() as i32 * 10 && i % 10 == 0 {
        add(ip(SPECIAL[(i / 10) as usize]));
    }
    if perturbed && i == 1 {
        add(ip("10.0.0.0"));
    }
    d
}

fn write(root: &Path, perturbed: bool) {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    std::fs::create_dir_all(root).unwrap();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(root).unwrap());
    let config = IndexWriterConfig::new();
    config.set_open_mode(OpenMode::CREATE).unwrap();
    config
        .set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
        .unwrap();
    let w = IndexWriter::new(dir, config).unwrap();
    for i in 0..DOCS / 2 {
        w.add_document(&doc(i, perturbed)).unwrap();
    }
    w.commit().unwrap();
    for i in DOCS / 2..DOCS {
        w.add_document(&doc(i, perturbed)).unwrap();
    }
    let mut i = 7;
    while i < DOCS {
        w.delete_documents_by_terms(vec![Term::new("id", BytesRef::new(id(i).into_bytes()))])
            .unwrap();
        i += 50;
    }
    w.commit().unwrap();
    w.close().unwrap();
}

/// `InetAddressPointFixture.queries()`, in the same order.
fn queries() -> Vec<(String, Arc<dyn Query>)> {
    let mut q: Vec<(String, Arc<dyn Query>)> = Vec::new();
    for a in [
        "10.1.0.37",
        "2001:db8::2:1a",
        "::",
        "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        "0.0.0.0",
        "255.255.255.255",
        "::fffe:ffff:ffff",
        "::1.2.3.4",
        "9.9.9.9",
    ] {
        q.push((
            format!("exact {a}"),
            InetAddressPoint::new_exact_query("ip", ip(a)).unwrap(),
        ));
    }
    for (a, p) in [
        ("10.0.0.0", 8),
        ("10.1.0.0", 16),
        ("10.2.3.0", 24),
        ("10.0.1.0", 20),
        ("0.0.0.0", 0),
        ("2001:db8::", 32),
        ("2001:db8::3:0", 112),
        ("2001:db8::3:27", 128),
        ("::", 0),
        ("::", 96),
        ("::1:0:0:0", 80),
    ] {
        q.push((
            format!("prefix {a}/{p}"),
            InetAddressPoint::new_prefix_query("ip", ip(a), p).unwrap(),
        ));
    }
    for (a, b) in [
        ("10.0.0.0", "10.1.255.255"),
        ("::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
        ("::", "::fffe:ffff:ffff"),
        ("0.0.0.0", "255.255.255.255"),
        ("2001:db8::", "2001:db8::4:0"),
        ("10.2.0.0", "10.0.0.0"),
    ] {
        q.push((
            format!("range {a} {b}"),
            InetAddressPoint::new_range_query("ip", ip(a), ip(b)).unwrap(),
        ));
    }
    q.push((
        "range exclusive 10.0.0.37 10.2.255.255".to_string(),
        InetAddressPoint::new_range_query(
            "ip",
            InetAddressPoint::next_up(ip("10.0.0.37")).unwrap(),
            InetAddressPoint::next_down(ip("10.2.255.255")).unwrap(),
        )
        .unwrap(),
    ));
    q.push((
        "set mixed".to_string(),
        InetAddressPoint::new_set_query(
            "ip",
            &[
                inet(&v4(1)),
                inet(&v4(6)),
                inet(&v6(2)),
                ip("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
                ip("9.9.9.9"),
            ],
        )
        .unwrap(),
    ));
    q.push((
        "set empty".to_string(),
        InetAddressPoint::new_set_query("ip", &[]).unwrap(),
    ));
    q.push((
        "set boundaries".to_string(),
        InetAddressPoint::new_set_query(
            "ip",
            &[ip("0.0.0.0"), ip("255.255.255.255"), ip("::fffe:ffff:ffff")],
        )
        .unwrap(),
    ));
    q.push((
        "set duplicates".to_string(),
        InetAddressPoint::new_set_query("ip", &[inet(&v4(11)), inet(&v4(11))]).unwrap(),
    ));
    q
}

/// `InetAddressPointFixture.query(String)`, line for line.
fn query_report(root: &Path) -> String {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    let mut out = String::new();
    let dir: Arc<dyn Directory> = Arc::new(FSDirectory::open(root).unwrap());
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();
    writeln!(
        out,
        "reader maxDoc={} numDocs={} leaves={}",
        reader.max_doc(),
        reader.num_docs(),
        Arc::clone(&reader).leaves().unwrap().len()
    )
    .unwrap();
    let searcher = IndexSearcher::new(Arc::clone(&reader)).unwrap();
    let stored = searcher.stored_fields().unwrap();
    for (name, query) in queries() {
        let rendered = query.to_query_string("");
        let top = searcher.search(query, TOP_N).unwrap();
        let relation = match top.total_hits.relation() {
            TotalHitsRelation::EQUAL_TO => "EQUAL_TO",
            TotalHitsRelation::GREATER_THAN_OR_EQUAL_TO => "GREATER_THAN_OR_EQUAL_TO",
        };
        writeln!(
            out,
            "query {name} | {rendered} | total={} {relation}",
            top.total_hits.value()
        )
        .unwrap();
        for hit in &top.score_docs {
            let document = stored.document(hit.doc).unwrap();
            let id = document
                .get_field("id")
                .and_then(|f| f.string_value())
                .unwrap_or_else(|| "null".to_string());
            writeln!(out, "  hit doc={} id={id}", hit.doc).unwrap();
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Java oracle
// ---------------------------------------------------------------------------

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

/// Runs `InetAddressPointFixture <args>` and returns its stdout; panics with
/// its stderr when it fails.
fn run_java(args: &[&str]) -> String {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         InetAddressPointFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .args(args)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "InetAddressPointFixture {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The first differing lines of two reports, or `None` when they agree.
fn diff(left_name: &str, left: &str, right_name: &str, right: &str) -> Option<String> {
    if left == right {
        return None;
    }
    let l: Vec<&str> = left.lines().collect();
    let r: Vec<&str> = right.lines().collect();
    let mut out = format!(
        "{left_name} ({} lines) differs from {right_name} ({} lines):\n",
        l.len(),
        r.len()
    );
    let mut shown = 0;
    for k in 0..l.len().max(r.len()) {
        let a = l.get(k).copied().unwrap_or("<end>");
        let b = r.get(k).copied().unwrap_or("<end>");
        if a != b {
            writeln!(
                out,
                "  line {}\n    {left_name}: {a}\n    {right_name}: {b}",
                k + 1
            )
            .unwrap();
            shown += 1;
            if shown == 30 {
                out.push_str("  ...\n");
                break;
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

/// Encoding, decoding, `nextUp`/`nextDown`, `toString` of the field and of
/// every query, and every error, line for line against Lucene.
#[test]
fn inet_address_point_values_match_lucene() {
    let oracle = run_java(&["values"]);
    // Vacuity guards: the oracle exercises both error paths of nextUp and
    // nextDown, the illegal prefix lengths, and both address families.
    for needle in [
        "nextUp ! ArithmeticException Overflow: there is no greater InetAddress than \
         ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        "nextDown ! ArithmeticException Underflow: there is no smaller InetAddress than \
         0:0:0:0:0:0:0:0",
        "prefix 33 ! IllegalArgumentException illegal prefixLength '33'",
        "prefix 129 ! IllegalArgumentException illegal prefixLength '129'",
        "prefix -1 ! IllegalArgumentException illegal prefixLength '-1'",
        "field InetAddressPoint <f:1.2.3.4>",
        "field InetAddressPoint <f:[0:0:0:0:0:0:102:304]>",
    ] {
        assert!(oracle.contains(needle), "oracle lacks: {needle}");
    }
    let rucene = values_report();
    if let Some(d) = diff("Lucene", &oracle, "Rucene", &rucene) {
        panic!("{d}");
    }
    println!("values: {} lines identical", oracle.lines().count());
}

/// Exact, prefix, range and set queries over indexes written by both sides,
/// run by both sides.
#[test]
fn inet_address_point_queries_match_lucene() {
    let tmp = tempfile::Builder::new()
        .prefix("rucene-inet-address-point")
        .tempdir()
        .unwrap();
    let r = tmp.path().join("rucene");
    let l = tmp.path().join("lucene");
    let p = tmp.path().join("rucene-perturbed");
    write(&r, false);
    std::fs::create_dir_all(&l).unwrap();
    run_java(&["write", l.to_str().unwrap()]);

    let lucene_on_l = run_java(&["query", l.to_str().unwrap()]);
    let lucene_on_r = run_java(&["query", r.to_str().unwrap()]);
    let rucene_on_l = query_report(&l);
    let rucene_on_r = query_report(&r);

    // Vacuity guards: every query is reported, and all but the ones that
    // must be empty have hits.
    let headers: Vec<&str> = lucene_on_l
        .lines()
        .filter(|l| l.starts_with("query "))
        .collect();
    assert_eq!(headers.len(), queries().len());
    let empty: Vec<&&str> = headers.iter().filter(|h| h.contains(" total=0 ")).collect();
    let expected_empty = ["exact 9.9.9.9 ", "range 10.2.0.0 10.0.0.0 ", "set empty "];
    assert_eq!(
        empty.len(),
        expected_empty.len(),
        "unexpected empty queries: {empty:?}"
    );
    for e in expected_empty {
        assert!(
            empty.iter().any(|h| h.starts_with(&format!("query {e}"))),
            "{e} must be empty"
        );
    }

    let mut failures = Vec::new();
    for (a_name, a, b_name, b) in [
        ("Lucene on L", &lucene_on_l, "Rucene on L", &rucene_on_l),
        ("Lucene on R", &lucene_on_r, "Rucene on R", &rucene_on_r),
        ("Lucene on L", &lucene_on_l, "Lucene on R", &lucene_on_r),
    ] {
        if let Some(d) = diff(a_name, a, b_name, b) {
            failures.push(d);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    // Positive control: one more address in document 1 changes the report.
    write(&p, true);
    let perturbed = query_report(&p);
    assert_eq!(
        perturbed.lines().next(),
        lucene_on_l.lines().next(),
        "the perturbed corpus keeps the document counts"
    );
    assert!(
        diff("Lucene on L", &lucene_on_l, "Rucene on P", &perturbed).is_some(),
        "the comparison must fail on the perturbed corpus"
    );
    println!(
        "queries: {} queries, {} lines identical on L and R",
        headers.len(),
        lucene_on_l.lines().count()
    );
}
