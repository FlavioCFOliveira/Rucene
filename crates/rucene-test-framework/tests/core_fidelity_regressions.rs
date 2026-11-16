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

//! Regression tests of the core fidelity fixes of sprint 13 (rmp #495,
//! #496, #498, #499, #500, #501 and #502): each test reproduces a divergence
//! from Lucene 10.5.0 (or JDK 25, where Lucene calls it) that the port had,
//! and fails without its fix. Where the expected value is the JVM's, the test
//! computes it with the local JDK 25 (and the Lucene 10.5.0 jars of the query
//! oracle harness), each JVM in a work directory of its own under `TMPDIR`,
//! so the tests run in parallel safely.

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rucene::analysis::{default_token_attribute_factory, CachingTokenFilter, TokenStream};
use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{DateTools, Document, Resolution, Store, TextField};
use rucene::error::{LuceneError, Result};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{directory_reader, FieldInfos, IndexReader, PrefixCodedTermsBuilder, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::phrase_query::{self, PhraseQuery};
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::Query;
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::task_executor::{
    Command as ExecutorCommand, Executor, RejectedExecutionException, TaskExecutor,
};
use rucene::search::term_query::TermQuery;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::attribute::AttributeSource;
use rucene::util::concurrent::panic_to_error;
use rucene::util::Accountable;
use rucene_test_framework::analysis::attributes::{char_term_id, Att};
use rucene_test_framework::util::java_exceptions::java_class_name;

use query_oracle::{lucene_jars, WorkDir};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The Java exception class and message `f` throws, as an error or a panic.
fn thrown<T>(f: impl FnOnce() -> Result<T>) -> (String, Option<String>) {
    let error = match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(_)) => panic!("expected an exception"),
        Ok(Err(e)) => e,
        Err(payload) => panic_to_error(payload.as_ref()),
    };
    (java_class_name(&error), java_message(&error))
}

/// `getMessage()` of the throwable `error` stands for.
fn java_message(error: &LuceneError) -> Option<String> {
    rucene_test_framework::analysis::base_token_stream_test_case::java_message(error)
}

/// Runs the single-file Java program `source` (class `class`) with `args`
/// on the Lucene 10.5.0 class path, in a work directory of its own, and
/// returns its standard output.
fn run_jvm(class: &str, source: &str, args: &[String]) -> String {
    let work = WorkDir::new(&format!("core-fidelity-{}", class.to_lowercase()));
    let file = work.path().join(format!("{class}.java"));
    std::fs::write(&file, source).expect("write the Java source");
    let class_path = std::env::join_paths(lucene_jars()).expect("a class path");
    let output = Command::new("java")
        .current_dir(work.path())
        .arg(format!("-Djava.io.tmpdir={}", work.path().display()))
        .arg("-Xmx512m")
        .arg("-cp")
        .arg(class_path)
        .arg(&file)
        .args(args)
        .output()
        .expect("a JDK 25 is required");
    assert!(
        output.status.success(),
        "{class}.java failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("the JVM prints UTF-8")
}

// ---------------------------------------------------------------------------
// #495: TaskGroup panic propagation
// ---------------------------------------------------------------------------

/// An executor that runs every command on a thread of its own.
struct ThreadPerCommand;

impl Executor for ThreadPerCommand {
    fn execute(
        &self,
        command: ExecutorCommand,
    ) -> std::result::Result<(), RejectedExecutionException> {
        std::thread::spawn(command);
        Ok(())
    }
}

/// rmp #495: a task that panics on a forked thread must fail the
/// `invokeAll` caller, as Java's `FutureTask` records the throwable and
/// `collectResults` rethrows it. Before the fix the panic unwound out of the
/// forked thread with the task's slot still running, and `invoke_all` waited
/// forever: the call is bounded here, and a timeout fails the test.
#[test]
fn a_panicking_forked_task_fails_invoke_all() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // The thread that calls `invoke_all`: a task runs on it or on a
        // forked thread.
        let caller = std::thread::current().id();
        let forked_started = Arc::new(AtomicBool::new(false));
        let executor = TaskExecutor::new(Arc::new(ThreadPerCommand));
        let tasks: Vec<Box<dyn FnOnce() -> Result<i32> + Send + 'static>> = (0..2)
            .map(|_| {
                let forked_started = Arc::clone(&forked_started);
                let task: Box<dyn FnOnce() -> Result<i32> + Send + 'static> = Box::new(move || {
                    if std::thread::current().id() != caller {
                        forked_started.store(true, Ordering::SeqCst);
                        panic!("java.lang.IllegalStateException: boom");
                    }
                    // The task the calling thread runs waits (boundedly)
                    // for a forked one to start, so that a task does run,
                    // and panic, on the forked thread.
                    for _ in 0..3_000 {
                        if forked_started.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Ok(1)
                });
                task
            })
            .collect();
        let _ = tx.send(executor.invoke_all(tasks));
    });
    let outcome = rx
        .recv_timeout(Duration::from_secs(120))
        .expect("invoke_all returns: a panicking task must not hang the caller");
    let error = outcome.expect_err("the panic fails invoke_all");
    assert_eq!(java_class_name(&error), "java.lang.IllegalStateException");
    assert_eq!(java_message(&error).as_deref(), Some("boom"));
}

// ---------------------------------------------------------------------------
// #496 and #499: Term
// ---------------------------------------------------------------------------

/// rmp #496: Java's `new Term(null, "foo")` is legal; `toString`, `equals`,
/// `hashCode`, `compareTo` and `ramBytesUsed` behave as `Term.java` defines
/// them for a `null` field.
#[test]
fn a_term_with_a_null_field_behaves_as_lucene() {
    let null_field = Term::from_text_nullable_field(None, "foo");
    assert_eq!(null_field.field_opt(), None);
    // `field + ":" + text()`
    assert_eq!(null_field.to_string(), "null:foo");
    // `equals`: two null fields are equal, a null field differs from "".
    assert_eq!(null_field, Term::from_text_nullable_field(None, "foo"));
    assert_ne!(null_field, Term::from_text("", "foo"));
    // `hashCode`: `31 * (31 * 1 + 0) + bytes.hashCode()`.
    assert_eq!(
        null_field.hash_code(),
        31i32
            .wrapping_mul(31)
            .wrapping_add(null_field.bytes().hash_code())
    );
    // `ramBytesUsed`: `sizeOfObject(null)` is 0.
    let empty_field = Term::from_text("", "foo");
    assert!(null_field.ram_bytes_used() < empty_field.ram_bytes_used());
    // `compareTo`: `field.equals(other.field)` dereferences a null field.
    let (class, _) = thrown(|| Ok(null_field.cmp(&empty_field)));
    assert_eq!(class, "java.lang.NullPointerException");
    let (class, _) = thrown(|| Ok(empty_field.cmp(&null_field)));
    assert_eq!(class, "java.lang.NullPointerException");
}

/// rmp #496: the multi-term queries reject a `null` field with
/// `MultiTermQuery`'s `Objects.requireNonNull(field, "field must not be
/// null")`, a `TermQuery` keeps it and throws Java's helpful
/// `NullPointerException` in `toString`, and a `PhraseQuery` throws it when a
/// second term is added.
#[test]
fn queries_over_a_null_field_term_behave_as_lucene() {
    let term = || Term::from_text_nullable_field(None, "foo");
    let field_npe = (
        "java.lang.NullPointerException".to_string(),
        Some("field must not be null".to_string()),
    );
    assert_eq!(thrown(|| WildcardQuery::new(term())), field_npe);
    assert_eq!(thrown(|| PrefixQuery::new(term())), field_npe);
    assert_eq!(thrown(|| RegexpQuery::new(term())), field_npe);
    assert_eq!(thrown(|| FuzzyQuery::new(term())), field_npe);

    let helpful = (
        "java.lang.NullPointerException".to_string(),
        Some(rucene::index::terms::TERM_FIELD_EQUALS_NPE.to_string()),
    );
    let tq = TermQuery::new(term());
    assert_eq!(thrown(|| Ok(tq.to_query_string("body"))), helpful);

    let mut builder = phrase_query::Builder::new();
    builder.add_at(term(), 0).expect("the first term");
    assert_eq!(thrown(|| builder.add_at(term(), 1).map(|_| ())), helpful);
    // A one-term phrase of a null field is legal: its field is null.
    let pq = PhraseQuery::new(0, vec![term()], vec![0]).expect("one term");
    assert_eq!(pq.get_field(), None);
}

/// rmp #499: `Term.compareTo` orders fields by `String.compareTo`, UTF-16
/// code units: a supplementary character (a surrogate pair, 0xD800..) sorts
/// before U+E000, where UTF-8 order puts it after. Before the fix the port
/// compared UTF-8 bytes.
#[test]
fn term_fields_order_by_utf16_code_units() {
    let bmp = Term::from_text("\u{E000}", "a");
    let supplementary = Term::from_text("\u{10000}", "a");
    assert_eq!(bmp.cmp(&supplementary), std::cmp::Ordering::Greater);
    assert_eq!(supplementary.cmp(&bmp), std::cmp::Ordering::Less);
    // Same field: the bytes in unsigned order.
    assert_eq!(
        Term::from_text("f", "\u{E000}").cmp(&Term::from_text("f", "\u{10000}")),
        std::cmp::Ordering::Less
    );
    // `PrefixCodedTerms.Builder.add` asserts strictly increasing terms in
    // `compareTo` order (`PrefixCodedTerms.java:74`): Java's order is
    // accepted.
    let mut builder = PrefixCodedTermsBuilder::new();
    builder.add(&supplementary).expect("first term");
    builder.add(&bmp).expect("a later field in Java's order");
}

// ---------------------------------------------------------------------------
// #498: PhraseQuery.visit
// ---------------------------------------------------------------------------

/// Records the calls `Query.visit` makes, sub-visitors included.
struct Recorder {
    log: Rc<RefCell<Vec<String>>>,
}

impl QueryVisitor for Recorder {
    fn consume_terms(&mut self, query: &dyn Query, terms: &[Term]) {
        let terms: Vec<String> = terms.iter().map(Term::to_string).collect();
        self.log.borrow_mut().push(format!(
            "consumeTerms({}, [{}])",
            query.java_class_name(),
            terms.join(", ")
        ));
    }

    fn visit_leaf(&mut self, query: &dyn Query) {
        self.log
            .borrow_mut()
            .push(format!("visitLeaf({})", query.java_class_name()));
    }

    fn accept_field(&self, field: &str) -> bool {
        self.log.borrow_mut().push(format!("acceptField({field})"));
        true
    }

    fn accept_null_field(&self) -> bool {
        self.log.borrow_mut().push("acceptField(null)".to_string());
        true
    }

    fn get_sub_visitor<'a>(
        &'a mut self,
        occur: Occur,
        parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'a>
    where
        Self: 'a,
    {
        self.log.borrow_mut().push(format!(
            "getSubVisitor({occur:?}, {})",
            parent.java_class_name()
        ));
        Box::new(Recorder {
            log: Rc::clone(&self.log),
        })
    }
}

/// rmp #498: `new PhraseQuery.Builder().build()` visited by a recording
/// visitor makes Java's calls (`PhraseQuery.java:321-327`):
/// `acceptField(null)`, `getSubVisitor(MUST, query)`, `consumeTerms(query)`.
/// Before the fix the port returned without calling the visitor.
#[test]
fn an_empty_phrase_query_visits_a_null_field() {
    let query = phrase_query::Builder::new()
        .build()
        .expect("an empty phrase");
    let log = Rc::new(RefCell::new(Vec::new()));
    query.visit(&mut Recorder {
        log: Rc::clone(&log),
    });
    assert_eq!(
        *log.borrow(),
        vec![
            "acceptField(null)".to_string(),
            "getSubVisitor(MUST, org.apache.lucene.search.PhraseQuery)".to_string(),
            "consumeTerms(org.apache.lucene.search.PhraseQuery, [])".to_string(),
        ]
    );
}

// ---------------------------------------------------------------------------
// #500: FieldInfos.getIndexedFields
// ---------------------------------------------------------------------------

/// Collects the arguments with `Collectors.toSet()` and prints the set in its
/// iteration order.
const TO_SET: &str = r#"
import java.util.Arrays;
import java.util.stream.Collectors;

public class ToSet {
  public static void main(String[] args) {
    System.out.println(String.join("\n", Arrays.stream(args).collect(Collectors.toSet())));
  }
}
"#;

/// rmp #500: `FieldInfos.getIndexedFields` collects the indexed names with
/// `Collectors.toSet()`, a `java.util.HashSet`; its iteration order, computed
/// by the JVM over the same names in the same stream order (field-number
/// order), is the port's. Before the fix it returned a Rust `HashSet`, whose
/// order is random.
#[test]
fn indexed_fields_iterate_in_java_hash_set_order() {
    let _ = register_codec("Lucene104", Lucene104Codec::new());
    // Names with colliding `String.hashCode` ("Aa"/"BB", "AaAa"/"BBBB"), with
    // supplementary characters, and more than the initial capacity of 16
    // would hold under a 0.75 load factor.
    let mut names: Vec<String> = (0..20).map(|i| format!("field{i}")).collect();
    names.extend(["Aa", "BB", "AaAa", "BBBB", "\u{1F600}x", "z"].map(String::from));
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
    let mut doc = Document::new();
    for name in &names {
        doc.add(TextField::new(name, "some text".to_string(), Store::NO).unwrap());
    }
    writer.add_document(&doc).unwrap();
    writer.commit().unwrap();
    writer.close().unwrap();
    let reader: Arc<dyn IndexReader> = directory_reader::open(dir).unwrap();

    let rust: Vec<String> = FieldInfos::get_indexed_fields(Arc::clone(&reader))
        .unwrap()
        .iter()
        .cloned()
        .collect();
    let java: Vec<String> = run_jvm("ToSet", TO_SET, &names)
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(java.len(), names.len());
    assert_eq!(rust, java, "Collectors.toSet() iteration order");
}

// ---------------------------------------------------------------------------
// #501: DateTools
// ---------------------------------------------------------------------------

/// Prints, per timestamp of the file `args[0]`, and per `Resolution`,
/// `timeToString`, `round` and `stringToTime(timeToString(...))` (or the
/// `ParseException` message).
const DATE_TOOLS: &str = r#"
import java.nio.file.Files;
import java.nio.file.Path;
import org.apache.lucene.document.DateTools;

public class DateToolsOracle {
  public static void main(String[] args) throws Exception {
    StringBuilder out = new StringBuilder();
    for (String line : Files.readAllLines(Path.of(args[0]))) {
      long time = Long.parseLong(line);
      out.append(time);
      for (DateTools.Resolution r : DateTools.Resolution.values()) {
        String s = DateTools.timeToString(time, r);
        String back;
        try {
          back = Long.toString(DateTools.stringToTime(s));
        } catch (java.text.ParseException e) {
          back = "ParseException:" + e.getMessage();
        }
        out.append('\t').append(s).append('\t').append(DateTools.round(time, r)).append('\t').append(back);
      }
      out.append('\n');
    }
    System.out.print(out);
  }
}
"#;

/// The same line, computed by Rucene.
fn date_tools_line(time: i64) -> String {
    let mut out = time.to_string();
    for r in [
        Resolution::YEAR,
        Resolution::MONTH,
        Resolution::DAY,
        Resolution::HOUR,
        Resolution::MINUTE,
        Resolution::SECOND,
        Resolution::MILLISECOND,
    ] {
        let s = DateTools::time_to_string(time, r);
        let back = match DateTools::string_to_time(&s) {
            Ok(t) => t.to_string(),
            Err(e) => format!(
                "ParseException:{}",
                java_message(&e).unwrap_or_else(|| "null".into())
            ),
        };
        out.push_str(&format!("\t{s}\t{}\t{back}", DateTools::round(time, r)));
    }
    out
}

/// The timestamps of the DateTools oracle: fixed boundaries (the Gregorian
/// cutover of 1582-10-15, the epoch, the turn of the eras, leap days of
/// both calendars, years -10000 and 10000) and seeded instants spanning
/// years -10000 to 10000.
fn date_tools_timestamps() -> Vec<i64> {
    const DAY: i64 = 86_400_000;
    // 1582-10-15T00:00:00Z, `GregorianCalendar.DEFAULT_GREGORIAN_CUTOVER`.
    const CUTOVER: i64 = -12_219_292_800_000;
    // Proleptic Gregorian 0001-01-01T00:00:00Z.
    const YEAR_1: i64 = -62_135_596_800_000;
    // About years -10000 and 10000.
    const MIN: i64 = -377_705_116_800_000;
    const MAX: i64 = 253_402_300_800_000;
    let mut times = vec![0, -1, 1, MIN, MAX, MAX + 366 * DAY, MIN - 366 * DAY];
    for d in -40..40 {
        times.push(CUTOVER + d * DAY);
        times.push(CUTOVER + d * DAY - 1);
        times.push(CUTOVER + d * DAY + 13 * 3_600_000 + 7 * 60_000 + 9_123);
        times.push(YEAR_1 + d * DAY);
        times.push(YEAR_1 + d * DAY + DAY / 2 + 1);
    }
    // Around February 29 of 1500 (Julian leap year), 1600 and 2000
    // (Gregorian leap years), 1700 and 1900 (not), and 4 BCE.
    for year_start in [
        -14_831_769_600_000i64,
        -11_676_096_000_000,
        946_684_800_000,
        -8_520_336_000_000,
        -2_208_988_800_000,
        -62_261_827_200_000,
    ] {
        for d in 50..70 {
            times.push(year_start + d * DAY + 1234);
        }
    }
    // SplitMix64, seeded: instants uniform over [MIN, MAX].
    let mut state: u64 = 0x0501_da7e_7001_5eed;
    let span = (MAX - MIN) as u64;
    for _ in 0..20_000 {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        times.push(MIN + (z % span) as i64);
    }
    times
}

/// rmp #501: `DateTools.timeToString`, `round` and `stringToTime` equal
/// Lucene's (run on JDK 25's `GregorianCalendar` and `SimpleDateFormat`) for
/// every `Resolution` over timestamps spanning years -10000 to 10000, the
/// Julian/Gregorian cutover included. Before the fix the port used the
/// proleptic Gregorian calendar and a signed year.
#[test]
fn date_tools_match_lucene_across_the_julian_cutover() {
    let times = date_tools_timestamps();
    let work = WorkDir::new("date-tools-input");
    let input = work.path().join("times.txt");
    let text: String = times.iter().map(|t| format!("{t}\n")).collect();
    std::fs::write(&input, text).expect("write the timestamps");
    let java = run_jvm(
        "DateToolsOracle",
        DATE_TOOLS,
        &[input.display().to_string()],
    );
    let java: Vec<&str> = java.lines().collect();
    assert_eq!(java.len(), times.len(), "one line per timestamp");
    let mut differences = Vec::new();
    for (time, java_line) in times.iter().zip(&java) {
        let rust_line = date_tools_line(*time);
        if rust_line != *java_line {
            differences.push(format!("java: {java_line}\nrust: {rust_line}"));
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} timestamps differ; first:\n{}",
        differences.len(),
        times.len(),
        differences
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---------------------------------------------------------------------------
// #502: CachingTokenFilter
// ---------------------------------------------------------------------------

/// A token stream of `a`, `b`, ... that fails at its `fail_at`-th token
/// (1-based) with an `IOException`.
#[derive(Debug)]
struct FailingStream {
    source: AttributeSource,
    term_att: Att,
    emitted: usize,
    fail_at: usize,
}

impl FailingStream {
    fn new(fail_at: usize) -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term_att = Att::add(&mut source, char_term_id())?;
        Ok(Self {
            source,
            term_att,
            emitted: 0,
            fail_at,
        })
    }
}

impl TokenStream for FailingStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.emitted + 1 == self.fail_at {
            return Err(LuceneError::from(std::io::Error::other("read failure")));
        }
        self.source.clear_attributes();
        let text = ((b'a' + self.emitted as u8) as char).to_string();
        self.term_att.char_term_mut().append_string(Some(&text));
        self.emitted += 1;
        Ok(true)
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// rmp #502: Java's `incrementToken` assigns `cache = new ArrayList<>(64)`
/// before `fillCache()` reads the input (`CachingTokenFilter.java:66-72`), so
/// an input failing at its third token leaves two cached states, the filter
/// counts as cached, a `reset()` replays them, and an `incrementToken()`
/// without that `reset()` throws the `NullPointerException` of the `null`
/// iterator. Before the fix the cache stayed empty and the filter uncached.
#[test]
fn caching_token_filter_keeps_the_states_cached_before_an_error() {
    let mut filter = CachingTokenFilter::new(Box::new(FailingStream::new(3).unwrap()));
    filter.reset().unwrap();
    let error = filter.increment_token().expect_err("the input fails");
    assert!(error.kind().is_io_exception());
    assert!(filter.is_cached(), "the partial cache is kept");

    let (class, message) = thrown(|| filter.increment_token());
    assert_eq!(class, "java.lang.NullPointerException");
    assert_eq!(
        message.as_deref(),
        Some("Cannot invoke \"java.util.Iterator.hasNext()\" because \"this.iterator\" is null")
    );

    filter.reset().unwrap();
    let mut replayed = Vec::new();
    while filter.increment_token().unwrap() {
        let att = filter
            .attribute_source()
            .get_attribute_interface::<dyn rucene::analysis::CharTermAttribute>()
            .unwrap();
        replayed.push(att.buffer()[..att.length()].iter().collect::<String>());
    }
    assert_eq!(replayed, vec!["a".to_string(), "b".to_string()]);
}

// ---------------------------------------------------------------------------
// #496: a null field name through Analyzer and QueryBuilder
// ---------------------------------------------------------------------------

/// rmp #496: Java's `Analyzer.tokenStream(null, text)` and
/// `normalize(null, text)` hand the `null` field to the reuse strategy and
/// the hooks (`Analyzer.java:139-264`); `PER_FIELD_REUSE_STRATEGY`'s
/// `HashMap` keeps the `null` field as a key of its own; `QueryBuilder`
/// builds terms of the `null` field (`QueryBuilder.java:244-411`).
#[test]
fn analyzer_and_query_builder_accept_a_null_field_name() {
    use rucene::analysis::standard::StandardAnalyzer;
    use rucene::analysis::{Analyzer, PerFieldReuseStrategy, ReuseStrategy};
    use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};

    let analyzer = StandardAnalyzer::new();
    assert_eq!(
        analyzer
            .normalize_nullable_field(None, "FoO")
            .unwrap()
            .utf8_to_string()
            .unwrap(),
        "foo"
    );
    let stream = analyzer
        .token_stream_from_str_nullable_field(None, "Hello World")
        .unwrap();
    let mut stream = stream.borrow_mut();
    stream.reset().unwrap();
    let mut n = 0;
    while stream.increment_token().unwrap() {
        n += 1;
    }
    stream.end().unwrap();
    stream.close().unwrap();
    assert_eq!(n, 2);
    drop(stream);

    // The per-field strategy stores the `null` field's components apart (on
    // a fresh analyzer, whose stored value the global strategy has not set).
    let analyzer = StandardAnalyzer::new();
    let state = analyzer.analyzer_state();
    let per_field = PerFieldReuseStrategy;
    let components = std::sync::Arc::new(rucene::util::atomic_ref_cell::AtomicRefCell::new(
        analyzer.create_components(None).unwrap(),
    ));
    per_field.set_reusable_components(state, None, std::sync::Arc::clone(&components));
    assert!(per_field.get_reusable_components(state, None).is_some());
    assert!(per_field
        .get_reusable_components(state, Some("f"))
        .is_none());

    // `createBooleanQuery(null, "Foo bar")`: two term queries of the null
    // field, whose `toString` dereferences it.
    let builder = QueryBuilder::new(Some(std::sync::Arc::new(StandardAnalyzer::new())));
    let q = builder
        .create_boolean_query(None, "Foo bar")
        .unwrap()
        .expect("a query");
    let mut terms = Vec::new();
    q.visit(&mut rucene::search::query_visitor::TermCollectorVisitor::new(&mut terms));
    let mut terms: Vec<String> = terms.iter().map(Term::to_string).collect();
    terms.sort();
    assert_eq!(terms, vec!["null:bar".to_string(), "null:foo".to_string()]);
    let (class, _) = thrown(|| Ok(q.to_query_string("body")));
    assert_eq!(class, "java.lang.NullPointerException");
}
