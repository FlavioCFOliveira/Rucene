//! `StandardTokenizer`, `StandardAnalyzer`, `StopFilter` and `QueryBuilder`
//! against Apache Lucene Core 10.5.0 (rmp task #410).
//!
//! No index is involved: both sides print a dump and the dumps are compared
//! line by line.
//!
//! * The token dump: for every input of [`corpus`] (a word-break corpus of
//!   Latin text with punctuation, apostrophes, hyphens, URLs and e-mails,
//!   numbers with separators, CJK, Hiragana, Katakana, Hangul, Thai, Lao,
//!   Myanmar and Khmer, emoji with ZWJ sequences, flags, skin-tone modifiers,
//!   keycaps and tag sequences, supplementary characters, tokens longer than
//!   5 and 255 characters, and an input of more than 4096 UTF-16 units that
//!   crosses the scanner's buffer refills) and every configuration of
//!   [`stream_configs`] (`StandardTokenizer` default, with
//!   `setMaxTokenLength(5)` and `(255)`, `StandardAnalyzer` default, with
//!   `setMaxTokenLength(5)` and with a custom stop set, and a `StopFilter`
//!   directly over a `StandardTokenizer`), each token's text (UTF-16 units,
//!   escaped), type, position increment, position length and offsets, then
//!   the same attributes after `end()`. Each configuration's stream is reused
//!   across the corpus, as `AnalysisTokensFixture` reuses its own.
//! * The `QueryBuilder` dump: for a `StandardAnalyzer`, a `StandardAnalyzer`
//!   with a stop set and a graph analyzer ([`GraphTokenizerLogic`], the core
//!   stand-in for the test framework's `CannedTokenStream`: its input spells
//!   out `text:posInc:posLen` tokens, so position lengths above 1 form token
//!   graphs), every text, the four settings (defaults, graph queries off,
//!   multi-term synonym phrases on, position increments off) and the five
//!   methods (`createBooleanQuery` SHOULD and MUST, `createPhraseQuery` slop 0
//!   and 2, `createMinShouldMatchQuery` 0.5): the built query's class tree and
//!   `toString()`, or `null`.
//! * Positive control: Rucene dumps a perturbed corpus (the `latin` input
//!   gains the word `zebra`); its dump must differ from Lucene's.
//!
//! The corpus text is written with `[U+XXXX]` escapes, decoded by
//! [`unescape`], so this file and the fixture hold the same source text.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use rucene::analysis::{
    default_token_attribute_factory, new_standard_tokenizer, new_stop_filter, Analyzer,
    AnalyzerState, CharArraySet, CharReader, CharTermAttribute, GlobalReuseStrategy,
    OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute, SharedTokenStream,
    StandardAnalyzer, StandardTokenizer, StringCharReader, TokenStream, TokenStreamComponents,
    Tokenizer, TokenizerLogic, TypeAttribute,
};
use rucene::error::Result;
use rucene::search::{
    query_to_string, BooleanQuery, BoostQuery, MultiPhraseQuery, Occur, PhraseQuery, Query,
    SynonymQuery, TermQuery,
};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::{AttributeSource, QueryBuilder, QueryBuilderOps};

// ---------------------------------------------------------------------------
// Corpus: AnalysisTokensFixture.corpus()
// ---------------------------------------------------------------------------

/// `AnalysisTokensFixture.unescape`: every `[U+XXXX]` becomes the code point
/// it names.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find("[U+") {
        out.push_str(&rest[..at]);
        let close = rest[at..].find(']').expect("a closed escape") + at;
        let cp = u32::from_str_radix(&rest[at + 3..close], 16).expect("a hexadecimal escape");
        out.push(char::from_u32(cp).expect("a scalar value"));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// `AnalysisTokensFixture.longTokens()`.
fn long_tokens() -> String {
    format!(
        "{} bcdefghijkl 12345678901234 short [U+4E00][U+4E01] {} {} end",
        "a".repeat(300),
        "x".repeat(255),
        "y".repeat(256)
    )
}

/// `AnalysisTokensFixture.bufferRefill()`.
fn buffer_refill() -> String {
    let mut sb = String::new();
    for i in 0..900 {
        if i % 97 == 0 {
            sb.push_str("[U+20000]");
        }
        write!(sb, "w{i}").unwrap();
        sb.push_str(if i % 13 == 0 { ", " } else { " " });
        if i == 700 {
            sb.push_str(&"z".repeat(300));
            sb.push(' ');
        }
    }
    sb
}

/// `AnalysisTokensFixture.corpus()`, in order; the perturbed corpus appends
/// ` zebra` to the `latin` input.
fn corpus(perturbed: bool) -> Vec<(&'static str, String)> {
    let mut latin = String::from(
        "The quick-brown fox's 3rd jump, isn't it? O'Neil's e-mail: john.doe@example.com; \
         visit https://lucene.apache.org/core/10_5_0/ now! U.S.A. AT&T C++ C# wi-fi",
    );
    if perturbed {
        latin.push_str(" zebra");
    }
    vec![
        ("latin", latin),
        (
            "numbers",
            "1,234.56 3.14159 1-800-555-0199 2026-11-11 10:30 1e10 0x1F 12,34 $100 50% v2.0.1 3rd 4th"
                .to_string(),
        ),
        (
            "stopwords",
            "The of a the quick and to the brown it is a fox the a".to_string(),
        ),
        (
            "accents",
            "caf[U+00E9] na[U+00EF]ve [U+00C5]ngstr[U+00F6]m STRASSE stra[U+00DF]e [U+0130]stanbul \
             [U+0391][U+0392][U+0393] [U+0414][U+0416]"
                .to_string(),
        ),
        (
            "cjk",
            "[U+4E2D][U+6587][U+5206][U+8BCD] Lucene[U+662F][U+4E00][U+4E2A][U+5E93] [U+3007][U+3005]"
                .to_string(),
        ),
        (
            "hiragana",
            "[U+3072][U+3089][U+304C][U+306A][U+306E][U+30C6][U+30B9][U+30C8] \
             [U+3053][U+3093][U+306B][U+3061][U+306F]"
                .to_string(),
        ),
        (
            "katakana",
            "[U+30AB][U+30BF][U+30AB][U+30CA] [U+30EB][U+30FC][U+30BB][U+30F3] \
             [U+FF76][U+FF80][U+FF76][U+FF85] [U+30A2][U+30C3][U+30D7][U+30EB]123"
                .to_string(),
        ),
        (
            "hangul",
            "[U+D55C][U+AD6D][U+C5B4] [U+D14C][U+C2A4][U+D2B8][U+C785][U+B2C8][U+B2E4] \
             [U+1100][U+1161][U+11A8]"
                .to_string(),
        ),
        (
            "southeast_asian",
            "[U+0E20][U+0E32][U+0E29][U+0E32][U+0E44][U+0E17][U+0E22] \
             [U+0E9E][U+0EB2][U+0EAA][U+0EB2][U+0EA5][U+0EB2][U+0EA7] \
             [U+1019][U+103C][U+1014][U+103A][U+1019][U+102C] [U+1780][U+1781]"
                .to_string(),
        ),
        (
            "emoji",
            "I [U+2764][U+FE0F] Lucene [U+1F600][U+1F600] family \
             [U+1F468][U+200D][U+1F469][U+200D][U+1F467] flag [U+1F1F5][U+1F1F9][U+1F1FA][U+1F1F8] \
             skin [U+1F44D][U+1F3FD] keycap 1[U+FE0F][U+20E3] #[U+FE0F][U+20E3] [U+00A9] \
             [U+261D][U+1F3FB] [U+1F3F4][U+E0067][U+E0062][U+E0065][U+E006E][U+E0067][U+E007F] \
             a[U+1F600]b"
                .to_string(),
        ),
        (
            "supplementary",
            "[U+20000][U+20001] [U+1D400][U+1D401]bold [U+10400][U+10401]deseret [U+1F1E6] \
             x[U+10428]y"
                .to_string(),
        ),
        ("long_tokens", long_tokens()),
        ("buffer_refill", buffer_refill()),
    ]
}

/// `AnalysisTokensFixture.stopSet(boolean)`.
fn stop_set(ignore_case: bool) -> CharArraySet {
    CharArraySet::from_iter(
        ["the", "a", "an", "and", "of", "to", "is", "it", "isn't"],
        ignore_case,
    )
}

// ---------------------------------------------------------------------------
// Token dumps
// ---------------------------------------------------------------------------

/// `AnalysisTokensFixture.escape`: printable ASCII except the backslash as
/// is, every other UTF-16 unit as `\uXXXX`.
fn escape(s: &str) -> String {
    let mut out = String::new();
    for unit in s.encode_utf16() {
        if (0x20..=0x7e).contains(&unit) && unit != u16::from(b'\\') {
            out.push(char::from(unit as u8));
        } else {
            write!(out, "\\u{unit:04X}").unwrap();
        }
    }
    out
}

/// The position and offset attributes of `src`, as both dumps print them.
fn position_and_offsets(src: &AttributeSource) -> String {
    let pos_inc = src
        .get_attribute_interface::<dyn PositionIncrementAttribute>()
        .expect("added by consume")
        .get_position_increment();
    let pos_len = src
        .get_attribute_interface::<dyn PositionLengthAttribute>()
        .expect("added by consume")
        .get_position_length();
    let offset = src
        .get_attribute_interface::<dyn OffsetAttribute>()
        .expect("added by consume");
    format!(
        "posInc={pos_inc} posLen={pos_len} offset={}-{}",
        offset.start_offset(),
        offset.end_offset()
    )
}

/// `AnalysisTokensFixture.consume`: the full workflow over `ts`, printing
/// every token and the end state.
fn consume(ts: &mut dyn TokenStream, out: &mut String) -> Result<()> {
    {
        let src = ts.attribute_source_mut();
        src.add_attribute_interface::<dyn CharTermAttribute>()?;
        src.add_attribute_interface::<dyn TypeAttribute>()?;
        src.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        src.add_attribute_interface::<dyn PositionLengthAttribute>()?;
        src.add_attribute_interface::<dyn OffsetAttribute>()?;
    }
    ts.reset()?;
    while ts.increment_token()? {
        let src = ts.attribute_source();
        let term: String = {
            let term = src
                .get_attribute_interface::<dyn CharTermAttribute>()
                .expect("added above");
            term.buffer()[..term.length()].iter().collect()
        };
        let token_type = src
            .get_attribute_interface::<dyn TypeAttribute>()
            .expect("added above")
            .type_value()
            .to_string();
        writeln!(
            out,
            "  tok term={} type={token_type} {}",
            escape(&term),
            position_and_offsets(src)
        )
        .unwrap();
    }
    ts.end()?;
    writeln!(out, "  end {}", position_and_offsets(ts.attribute_source())).unwrap();
    ts.close()?;
    Ok(())
}

/// One token-stream configuration: consumes one text with its own reused
/// stream.
type StreamConfig = Box<dyn FnMut(&str, &mut String) -> Result<()>>;

/// A configuration over a reused `StandardTokenizer`.
fn tokenizer_config(max_token_length: Option<i32>) -> StreamConfig {
    let mut tokenizer = new_standard_tokenizer();
    if let Some(length) = max_token_length {
        tokenizer
            .logic_mut()
            .set_max_token_length(length)
            .expect("a valid length");
    }
    Box::new(move |text, out| {
        tokenizer.set_reader(Box::new(StringCharReader::new(text)))?;
        consume(&mut tokenizer, out)
    })
}

/// A configuration over a reused analyzer's `tokenStream("body", text)`.
fn analyzer_config(analyzer: StandardAnalyzer) -> StreamConfig {
    Box::new(move |text, out| {
        let stream = analyzer.token_stream_from_str("body", text)?;
        let mut ts = stream.borrow_mut();
        consume(&mut *ts, out)
    })
}

/// `AnalysisTokensFixture.streamConfigs()`, in order.
fn stream_configs() -> Vec<(&'static str, StreamConfig)> {
    let mut analyzer_max5 = StandardAnalyzer::new();
    analyzer_max5.set_max_token_length(5);

    let stop_source: Arc<AtomicRefCell<StandardTokenizer>> =
        Arc::new(AtomicRefCell::new(new_standard_tokenizer()));
    let shared: Arc<AtomicRefCell<dyn TokenStream>> = stop_source.clone();
    let mut stop_filter = new_stop_filter(Box::new(SharedTokenStream::new(shared)), stop_set(true))
        .expect("a stop filter");
    let stop_config: StreamConfig = Box::new(move |text, out| {
        stop_source
            .borrow_mut()
            .set_reader(Box::new(StringCharReader::new(text)))?;
        consume(&mut stop_filter, out)
    });

    vec![
        ("tokenizer", tokenizer_config(None)),
        ("tokenizer-max5", tokenizer_config(Some(5))),
        ("tokenizer-max255", tokenizer_config(Some(255))),
        ("analyzer", analyzer_config(StandardAnalyzer::new())),
        ("analyzer-max5", analyzer_config(analyzer_max5)),
        (
            "analyzer-stop",
            analyzer_config(StandardAnalyzer::with_stopwords(stop_set(false))),
        ),
        ("stopfilter", stop_config),
    ]
}

// ---------------------------------------------------------------------------
// QueryBuilder
// ---------------------------------------------------------------------------

/// `AnalysisTokensFixture.GraphToken`.
#[derive(Debug, Clone)]
struct GraphToken {
    text: String,
    pos_inc: i32,
    pos_len: i32,
    start: i32,
}

/// `AnalysisTokensFixture.GraphTokenizer`: emits the tokens its input spells
/// out as space-separated `text:posInc:posLen` items. Offsets are the item's
/// start and that plus the token text's length (the texts are ASCII, so
/// bytes and UTF-16 units coincide).
#[derive(Debug, Default)]
struct GraphTokenizerLogic {
    tokens: Vec<GraphToken>,
    upto: usize,
    length: i32,
}

impl TokenizerLogic for GraphTokenizerLogic {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        reader: &mut dyn CharReader,
    ) -> Result<bool> {
        if self.upto == self.tokens.len() {
            return Ok(false);
        }
        source.clear_attributes();
        let t = self.tokens[self.upto].clone();
        self.upto += 1;
        {
            let mut term = source
                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                .expect("added at construction");
            term.set_empty();
            term.append_string(Some(&t.text));
        }
        source
            .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
            .expect("added at construction")
            .set_position_increment(t.pos_inc);
        source
            .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
            .expect("added at construction")
            .set_position_length(t.pos_len);
        let input: &dyn CharReader = reader;
        let correct = |o: i32| Tokenizer::<Self>::correct_offset_of(Some(input), o);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("added at construction")
            .set_offset(
                correct(t.start),
                correct(t.start + t.text.encode_utf16().count() as i32),
            );
        Ok(true)
    }

    fn end(&mut self, source: &mut AttributeSource, input: Option<&dyn CharReader>) -> Result<()> {
        let final_offset = Tokenizer::<Self>::correct_offset_of(input, self.length);
        source
            .get_attribute_interface_mut::<dyn OffsetAttribute>()
            .expect("added at construction")
            .set_offset(final_offset, final_offset);
        Ok(())
    }

    fn reset(&mut self, _source: &mut AttributeSource, reader: &mut dyn CharReader) -> Result<()> {
        let mut units: Vec<u16> = Vec::new();
        let mut buffer = [0u16; 64];
        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            units.extend_from_slice(&buffer[..n]);
        }
        let s = String::from_utf16(&units).expect("a valid query text");
        self.length = units.len() as i32;
        self.tokens.clear();
        self.upto = 0;
        let mut start = 0usize;
        while start < s.len() {
            let end = s[start..].find(' ').map_or(s.len(), |p| p + start);
            if end > start {
                let parts: Vec<&str> = s[start..end].split(':').collect();
                self.tokens.push(GraphToken {
                    text: parts[0].to_string(),
                    pos_inc: parts[1].parse().expect("a position increment"),
                    pos_len: parts[2].parse().expect("a position length"),
                    start: start as i32,
                });
            }
            start = end + 1;
        }
        Ok(())
    }
}

/// `AnalysisTokensFixture.GraphAnalyzer`.
#[derive(Debug)]
struct GraphAnalyzer {
    state: AnalyzerState,
}

impl GraphAnalyzer {
    fn new() -> Self {
        Self {
            state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
        }
    }
}

impl Analyzer for GraphAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.rucene.codec.AnalysisTokensFixture$GraphAnalyzer"
    }

    /// The tokenizer's fields add the term, position increment, position
    /// length and offset attributes, in that order.
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        source.add_attribute_interface::<dyn CharTermAttribute>()?;
        source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
        source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
        source.add_attribute_interface::<dyn OffsetAttribute>()?;
        let tokenizer = Tokenizer::with_source(source, GraphTokenizerLogic::default());
        Ok(TokenStreamComponents::from_tokenizer(Arc::new(
            AtomicRefCell::new(tokenizer),
        )))
    }
}

/// `AnalysisTokensFixture.STANDARD_TEXTS`.
const STANDARD_TEXTS: [&str; 7] = [
    "quick brown fox",
    "the quick brown fox of the year",
    "Lucene",
    "wi-fi network",
    "[U+4E2D][U+6587][U+5206][U+8BCD]",
    "it is the",
    "",
];

/// `AnalysisTokensFixture.GRAPH_TEXTS`.
const GRAPH_TEXTS: [&str; 6] = [
    "wifi:1:2 wi:0:1 fi:1:1 network:1:1",
    "ny:1:2 new:0:1 york:1:1 city:1:1",
    "fast:1:1 quick:0:1 car:1:1",
    "a:1:1 b:2:1 c:1:1",
    "usa:1:4 united:0:1 states:1:1 of:1:1 america:1:1 rocks:1:1",
    "solo:1:1",
];

const SETTINGS: [&str; 4] = ["default", "nograph", "autophrase", "noposinc"];

const METHODS: [&str; 5] = [
    "bool-should",
    "bool-must",
    "phrase-0",
    "phrase-2",
    "msm-0.5",
];

/// `AnalysisTokensFixture.builder`.
fn builder(analyzer: &Arc<dyn Analyzer>, setting: &str) -> QueryBuilder {
    let mut b = QueryBuilder::new(Some(Arc::clone(analyzer)));
    match setting {
        "default" => {}
        "nograph" => b.set_enable_graph_queries(false),
        "autophrase" => b.set_auto_generate_multi_term_synonyms_phrase_query(true),
        "noposinc" => b.set_enable_position_increments(false),
        _ => unreachable!("{setting}"),
    }
    b
}

/// `AnalysisTokensFixture.create`.
fn create(b: &QueryBuilder, method: &str, text: &str) -> Result<Option<Arc<dyn Query>>> {
    match method {
        "bool-should" => b.create_boolean_query("body", text),
        "bool-must" => b.create_boolean_query_with_operator("body", text, Occur::MUST),
        "phrase-0" => b.create_phrase_query("body", text),
        "phrase-2" => b.create_phrase_query_with_slop("body", text, 2),
        "msm-0.5" => b.create_min_should_match_query("body", text, 0.5),
        _ => unreachable!("{method}"),
    }
}

fn occur_name(occur: Occur) -> &'static str {
    match occur {
        Occur::MUST => "MUST",
        Occur::FILTER => "FILTER",
        Occur::SHOULD => "SHOULD",
        Occur::MUST_NOT => "MUST_NOT",
    }
}

/// `AnalysisTokensFixture.tree`: the query's Java simple class name, with the
/// clause classes of boolean and boost queries. A query type the
/// `QueryBuilder` never builds in Lucene prints as `<unmapped>`, so it fails
/// the comparison.
fn tree(q: &dyn Query) -> String {
    let any = q.as_any();
    if let Some(bq) = any.downcast_ref::<BooleanQuery>() {
        let mut sb = format!("BooleanQuery(msm={}", bq.get_minimum_number_should_match());
        for c in bq.clauses() {
            write!(
                sb,
                " {}:{}",
                occur_name(c.occur()),
                tree(c.query().as_ref())
            )
            .unwrap();
        }
        sb.push(')');
        return sb;
    }
    if let Some(b) = any.downcast_ref::<BoostQuery>() {
        return format!("BoostQuery({})", tree(b.get_query().as_ref()));
    }
    if any.is::<TermQuery>() {
        "TermQuery".to_string()
    } else if any.is::<SynonymQuery>() {
        "SynonymQuery".to_string()
    } else if any.is::<PhraseQuery>() {
        "PhraseQuery".to_string()
    } else if any.is::<MultiPhraseQuery>() {
        "MultiPhraseQuery".to_string()
    } else {
        "<unmapped>".to_string()
    }
}

/// `AnalysisTokensFixture.queryBuilders`.
fn query_builders(out: &mut String) -> Result<()> {
    let analyzers: Vec<(&str, Arc<dyn Analyzer>, &[&str])> = vec![
        (
            "standard",
            Arc::new(StandardAnalyzer::new()),
            &STANDARD_TEXTS,
        ),
        (
            "standard-stop",
            Arc::new(StandardAnalyzer::with_stopwords(stop_set(false))),
            &STANDARD_TEXTS,
        ),
        ("graph", Arc::new(GraphAnalyzer::new()), &GRAPH_TEXTS),
    ];
    for (name, analyzer, texts) in &analyzers {
        for raw_text in texts.iter() {
            let text = unescape(raw_text);
            for setting in SETTINGS {
                let b = builder(analyzer, setting);
                for method in METHODS {
                    writeln!(
                        out,
                        "qb analyzer={name} setting={setting} method={method} text={}",
                        escape(&text)
                    )
                    .unwrap();
                    match create(&b, method, &text)? {
                        None => writeln!(out, "  query null").unwrap(),
                        Some(q) => writeln!(
                            out,
                            "  query class={} string={}",
                            tree(q.as_ref()),
                            escape(&query_to_string(q.as_ref()))
                        )
                        .unwrap(),
                    }
                }
            }
        }
    }
    Ok(())
}

/// Rucene's dump: the same lines `AnalysisTokensFixture dump` prints.
fn dump(perturbed: bool) -> String {
    let mut out = String::new();
    let corpus = corpus(perturbed);
    for (name, mut config) in stream_configs() {
        writeln!(out, "config {name}").unwrap();
        for (input, raw) in &corpus {
            let text = unescape(raw);
            writeln!(out, "input {input} length={}", text.encode_utf16().count()).unwrap();
            config(&text, &mut out).unwrap_or_else(|e| panic!("{name}/{input}: {e:?}"));
        }
    }
    query_builders(&mut out).unwrap_or_else(|e| panic!("QueryBuilder: {e:?}"));
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

/// Runs `AnalysisTokensFixture <mode>`: `(success, stdout, stderr)`.
fn run_java(mode: &str) -> (bool, String, String) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         AnalysisTokensFixture.java",
    );
    let output = Command::new("java")
        .arg("-cp")
        .arg(lucene_jar())
        .arg(fixture)
        .arg(mode)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The stderr lines of a Java run without the JVM's start-up warnings.
fn java_errors(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| {
            !(l.starts_with("WARNING:")
                || l.contains("VectorizationProvider lookup")
                || l.trim().is_empty())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// The first differing lines of two dumps, or `None` when they agree.
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
    let mut section = "";
    let mut context = "";
    for k in 0..l.len().max(r.len()) {
        let a = l.get(k).copied().unwrap_or("<end>");
        let b = r.get(k).copied().unwrap_or("<end>");
        if a.starts_with("config ") {
            section = a;
        }
        if a.starts_with("input ") || a.starts_with("qb ") {
            context = a;
        }
        if a != b {
            writeln!(
                out,
                "  line {} [{section} / {context}]\n    {left_name}: {a}\n    {right_name}: {b}",
                k + 1
            )
            .unwrap();
            shown += 1;
            if shown == 40 {
                out.push_str("  ...\n");
                break;
            }
        }
    }
    Some(out)
}

/// The `config` and `qb` header lines of a dump: its structure.
fn header_lines(dump: &str) -> Vec<&str> {
    dump.lines()
        .filter(|l| l.starts_with("config ") || l.starts_with("qb "))
        .collect()
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "panic".to_string())
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[test]
fn standard_tokenizer_stop_filter_and_query_builder_match_lucene() {
    let mut failures: Vec<String> = Vec::new();
    let mut report = String::new();

    let (ok, oracle, err) = run_java("dump");
    assert!(
        ok,
        "Lucene failed to dump the analysis:\n{oracle}\n{}",
        java_errors(&err)
    );
    let rucene = std::panic::catch_unwind(|| dump(false))
        .map_err(panic_message)
        .map_err(|e| failures.push(format!("Rucene dump panicked: {e}")))
        .ok();

    // The oracle must exercise what the comparison claims to cover.
    let configs = oracle.lines().filter(|l| l.starts_with("config ")).count();
    assert_eq!(
        configs,
        stream_configs().len(),
        "the oracle must report every configuration"
    );
    let mut lines = oracle.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with("input ") {
            assert!(
                lines.peek().is_some_and(|next| next.starts_with("  tok ")),
                "the oracle input produced no token: {line}"
            );
        }
    }
    for token_type in [
        "<ALPHANUM>",
        "<NUM>",
        "<SOUTHEAST_ASIAN>",
        "<IDEOGRAPHIC>",
        "<HIRAGANA>",
        "<KATAKANA>",
        "<HANGUL>",
        "<EMOJI>",
    ] {
        assert!(
            oracle.contains(&format!(" type={token_type} ")),
            "the oracle has no {token_type} token"
        );
    }
    let refill = oracle
        .lines()
        .find_map(|l| l.strip_prefix("input buffer_refill length="))
        .expect("the buffer_refill input")
        .parse::<usize>()
        .unwrap();
    assert!(refill > 4096, "buffer_refill is {refill} units long");
    assert!(
        oracle
            .lines()
            .any(|l| l.starts_with("  tok ") && !l.contains(" posInc=1 ")),
        "no token follows a removed or skipped position"
    );
    assert!(
        oracle
            .lines()
            .any(|l| l.starts_with("  end posInc=") && !l.starts_with("  end posInc=0 ")),
        "no end() carries trailing removed positions"
    );
    let qb_count = oracle.lines().filter(|l| l.starts_with("qb ")).count();
    assert_eq!(
        qb_count,
        (2 * STANDARD_TEXTS.len() + GRAPH_TEXTS.len()) * SETTINGS.len() * METHODS.len(),
        "the oracle must report every QueryBuilder call"
    );
    for class in [
        "TermQuery",
        "BooleanQuery(",
        "SynonymQuery",
        "PhraseQuery",
        "MultiPhraseQuery",
    ] {
        assert!(
            oracle.contains(class),
            "no QueryBuilder call of the oracle built a {class}"
        );
    }
    assert!(
        oracle.contains("  query null"),
        "no QueryBuilder call returned null"
    );
    writeln!(
        report,
        "== oracle dump ({} lines, {qb_count} QueryBuilder calls)",
        oracle.lines().count()
    )
    .unwrap();

    if let Some(rucene) = &rucene {
        match diff("Lucene", &oracle, "Rucene", rucene) {
            Some(d) => failures.push(d),
            None => writeln!(report, "== Lucene == Rucene").unwrap(),
        }
    }

    // Positive control: one more word in one input must change the dump the
    // comparison sees, with the same structure.
    let perturbed = std::panic::catch_unwind(|| dump(true))
        .unwrap_or_else(|e| panic!("Rucene perturbed dump panicked: {}", panic_message(e)));
    assert_eq!(
        header_lines(&perturbed),
        header_lines(&oracle),
        "the perturbed dump keeps the structure"
    );
    let control = diff("Lucene", &oracle, "Rucene perturbed", &perturbed);
    assert!(
        control.is_some(),
        "the comparison must fail on the perturbed corpus"
    );
    writeln!(
        report,
        "== positive control: Lucene != Rucene perturbed\n{}",
        control
            .unwrap()
            .lines()
            .take(4)
            .collect::<Vec<_>>()
            .join("\n")
    )
    .unwrap();

    println!("{report}");
    assert!(
        failures.is_empty(),
        "{} analysis failure(s):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
