//! Parity of the analysis mocks of `org.apache.lucene.tests.analysis` (rmp
//! #346) and of `BaseTokenStreamTestCase` (rmp #357) with Lucene 10.5.0:
//! replays every scenario of `AnalysisMocksOracle.java` (the oracle harness
//! of `crates/randomizedtesting/tests/fixtures/oracle/`) and compares the rows
//! of `tests/fixtures/analysis_mocks.tsv`.

mod oracle_support;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use oracle_support::{assert_same, rows};
use randomizedtesting::jdk::exception::{QuietPanics, Throwable};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::{Class, Constructor, Field, Method, Value};
use randomizedtesting::jdk::system;
use randomizedtesting::junit::{ClassRule, JUnitCore, Request, Test};
use randomizedtesting::{default_supplier, RandomizedContext, Randomness};
use rucene::analysis::{
    new_standard_tokenizer, Analyzer, AnalyzerState, GlobalReuseStrategy, StandardAnalyzer,
    StringCharReader, TokenFilter, TokenFilterLogic, TokenStream, TokenStreamComponents, Tokenizer,
    TokenizerLogic,
};
use rucene::error::Result;
use rucene::util::attribute::AttributeSource;
use rucene::util::automaton::CharacterRunAutomaton;
use rucene::util::automaton::RegExp;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::{
    char_term_id, flags_id, keyword_id, offset_id, payload_id, pos_inc_id, pos_len_id,
    term_freq_id, term_to_bytes_ref_id, type_id, Att,
};
use rucene_test_framework::analysis::base_token_stream_test_case::java_class_and_message;
use rucene_test_framework::analysis::mock_token_filter::ENGLISH_STOPSET;
use rucene_test_framework::analysis::mock_tokenizer::{
    DEFAULT_MAX_TOKEN_LENGTH, KEYWORD, SIMPLE, WHITESPACE,
};
use rucene_test_framework::analysis::{
    components, shared, sink, tokenizer_components, BaseTokenStreamTestCase as B, BinaryToken,
    CannedBinaryTokenStream, CannedTokenStream, CrankyTokenFilter, MockAnalyzer, MockBytesAnalyzer,
    MockCharFilter, MockFixedLengthPayloadFilter, MockGraphTokenFilter,
    MockHoleInjectingTokenFilter, MockLowerCaseFilter, MockPayloadAnalyzer,
    MockRandomLookaheadTokenFilter, MockReaderWrapper, MockSynonymAnalyzer, MockSynonymFilter,
    MockTokenFilter, MockTokenizer, MockUTF16TermAttributeImpl, MockVariableLengthPayloadFilter,
    SimplePayloadFilter, Token, TokenStreamToDot, ValidatingTokenFilter,
};
use rucene_test_framework::util::lucene_test_case::{LuceneTestCase, CLASS_NAME_RULE};
use rucene_test_framework::util::TestUtil;

const SEED: &str = "1DB1C0DE";

static ROWS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// `OracleSupport.seeds()`: edge values, then ten draws of
/// `SplittableRandom(20261103)`.
fn seeds() -> Vec<i64> {
    let mut s = vec![
        0,
        1,
        -1,
        i64::MIN,
        i64::MAX,
        42,
        0x5DEECE66D,
        0xDEADBEEF,
        0x123456789ABCDEF,
        -0x123456789ABCDEF,
    ];
    // SplittableRandom(20261103).nextLong(), ten times.
    const GOLDEN_GAMMA: u64 = 0x9e3779b97f4a7c15;
    let mut seed: u64 = 20261103;
    let mix64 = |mut z: u64| {
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    };
    for _ in 0..10 {
        seed = seed.wrapping_add(GOLDEN_GAMMA);
        s.push(mix64(seed) as i64);
    }
    s
}

const CORPUS: [&str; 10] = [
    "",
    "A bc defg hiJklmn opqrstuv wxy z ",
    "a-bc123 defg+hijklmn567opqrstuv78wxy_z ",
    "FooBarBAZ aFooBar",
    "x\u{10400}y \u{1D11E}\u{1D11E} zz \u{10428}",
    "abcdefghijklmnopqrstuvwxyz0123456789 short",
    "\u{130}STANBUL \u{3A3}\u{391}\u{3A3} \u{C0}\u{D6}\u{F8}\u{4E00}\u{9FCC}",
    "the quick brown a fox and it is",
    "  \t\r\n  leading whitespace",
    "small dogs and guinea pig running guinea dogs",
];

fn len16(s: &str) -> i32 {
    s.encode_utf16().count() as i32
}

fn automaton(regexp: &str) -> Arc<CharacterRunAutomaton> {
    Arc::new(
        CharacterRunAutomaton::new(RegExp::new(regexp).unwrap().to_automaton().unwrap()).unwrap(),
    )
}

fn patterns() -> Vec<Arc<CharacterRunAutomaton>> {
    vec![
        Arc::clone(&WHITESPACE),
        Arc::clone(&KEYWORD),
        Arc::clone(&SIMPLE),
        automaton("[A-Z][a-z]*"),
        automaton(".."),
        automaton("."),
    ]
}

fn row(scenario: &str, seed: i64, step: &str, value: String) {
    ROWS.lock()
        .unwrap()
        .push(format!("{scenario}\t{seed}\t{step}\t{value}"));
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

fn units(s: &str) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.encode_utf16()
        .map(|u| format!("{u:04x}"))
        .collect::<Vec<_>>()
        .join(".")
}

fn class_message(class: &str, message: &str) -> String {
    format!(
        "{class}\t{}",
        message.replace('\n', "\\n").replace('\t', "\\t")
    )
}

fn err(e: &rucene::error::LuceneError) -> String {
    let (class, message) = java_class_and_message(e);
    class_message(&class, &message)
}

fn panic_err(payload: Box<dyn std::any::Any + Send>) -> String {
    let t = Throwable::from_panic(payload);
    class_message(t.class_name(), t.message().unwrap_or("<null>"))
}

fn ctx<T>(seed: i64, c: impl FnOnce() -> T) -> T {
    RandomizedContext::current()
        .run_with_private_randomness(Randomness::new(seed, default_supplier(), vec![]), c)
}

fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

fn hex(b: Option<&BytesRef>) -> String {
    match b {
        None => "null".to_string(),
        Some(b) => {
            let s: String = b.bytes[b.offset..b.offset + b.length]
                .iter()
                .map(|x| format!("{x:02x}"))
                .collect();
            if s.is_empty() {
                "~".to_string()
            } else {
                s
            }
        }
    }
}

fn tok(source: &AttributeSource) -> String {
    let get = |id| Att::get(source, id);
    let term = get(char_term_id()).map_or("-".to_string(), |a| units(&a.term_string()));
    let off = get(offset_id()).map_or("-".to_string(), |a| {
        let o = a.offset();
        format!("{},{}", o.start_offset(), o.end_offset())
    });
    let inc = get(pos_inc_id()).map_or("-".to_string(), |a| {
        a.pos_inc().get_position_increment().to_string()
    });
    let len = get(pos_len_id()).map_or("-".to_string(), |a| {
        a.pos_len().get_position_length().to_string()
    });
    let ty = get(type_id()).map_or("-".to_string(), |a| units(a.type_att().type_value()));
    let payload = get(payload_id()).map_or("-".to_string(), |a| hex(a.payload().get_payload()));
    let flags = get(flags_id()).map_or("-".to_string(), |a| a.flags().get_flags().to_string());
    let freq = get(term_freq_id()).map_or("-".to_string(), |a| {
        a.term_freq().get_term_frequency().to_string()
    });
    let kw = get(keyword_id()).map_or("-".to_string(), |a| a.keyword().is_keyword().to_string());
    format!("{term},{off},{inc},{len},{ty},{payload},{flags},{freq},{kw}")
}

/// Consumes the stream, recording every token, end() and close().
fn dump(ts: &mut dyn TokenStream) -> String {
    let mut sb = String::new();
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        ts.reset()?;
        while ts.increment_token()? {
            sb.push_str(&tok(ts.attribute_source()));
            sb.push(' ');
        }
        ts.end()?;
        sb.push_str("end:");
        sb.push_str(&tok(ts.attribute_source()));
        ts.close()
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            sb.push('!');
            sb.push_str(&err(&e));
        }
        Err(payload) => {
            sb.push('!');
            sb.push_str(&panic_err(payload));
        }
    }
    sb
}

fn dump_rc(ts: &Rc<RefCell<dyn TokenStream>>) -> String {
    dump(&mut *ts.borrow_mut())
}

/// The Java simple class name of the implementation of `CharTermAttribute`.
fn impl_name(source: &AttributeSource) -> String {
    match Att::get(source, char_term_id()) {
        None => "-".to_string(),
        Some(a) => {
            let i = a.implementation();
            let any = i.as_any();
            if any.is::<rucene::analysis::PackedTokenAttributeImpl>() {
                "PackedTokenAttributeImpl"
            } else if any.is::<Token>() {
                "Token"
            } else if any.is::<rucene::analysis::CharTermAttributeImpl>() {
                "CharTermAttributeImpl"
            } else if any.is::<MockUTF16TermAttributeImpl>() {
                "MockUTF16TermAttributeImpl"
            } else {
                "?"
            }
            .to_string()
        }
    }
}

fn outcome(s: impl FnOnce() -> Result<String>) -> String {
    match catch_unwind(AssertUnwindSafe(s)) {
        Ok(Ok(v)) => format!("ok\t{v}"),
        Ok(Err(e)) => format!("err\t{}", err(&e)),
        Err(payload) => format!("err\t{}", panic_err(payload)),
    }
}

fn reader(text: &str) -> Box<dyn rucene::analysis::CharReader> {
    Box::new(StringCharReader::new(text))
}

// ---------------------------------------------------------------------------
// tok
// ---------------------------------------------------------------------------

fn tokenizers() {
    let p = patterns();
    for (si, &seed) in seeds().iter().enumerate() {
        for (pi, pattern) in p.iter().enumerate() {
            for (c, text) in CORPUS.iter().enumerate() {
                let lc = (si + pi) % 2 == 1;
                let max = if c % 3 == 0 {
                    5
                } else {
                    DEFAULT_MAX_TOKEN_LENGTH
                };
                let v = ctx(seed, || {
                    let mut t =
                        MockTokenizer::with_max_token_length(Arc::clone(pattern), lc, max).unwrap();
                    t.set_reader(reader(text)).unwrap();
                    let d = dump(&mut t);
                    format!(
                        "{}\t{}\t{}",
                        impl_name(t.attribute_source()),
                        d,
                        random().next_long()
                    )
                });
                row("tok", seed, &format!("p{pi}.lc{lc}.m{max}.c{c}"), v);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// len
// ---------------------------------------------------------------------------

fn lengths() {
    let p = [Arc::clone(&WHITESPACE), Arc::clone(&KEYWORD)];
    for &seed in &seeds()[..5] {
        for (pi, pattern) in p.iter().enumerate() {
            for max in 1..=5 {
                row(
                    "len",
                    seed,
                    &format!("p{pi}.m{max}"),
                    ctx(seed, || {
                        let mut t =
                            MockTokenizer::with_max_token_length(Arc::clone(pattern), false, max)
                                .unwrap();
                        t.set_reader(reader(CORPUS[4])).unwrap();
                        dump(&mut t)
                    }),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// rd
// ---------------------------------------------------------------------------

fn readers() {
    for &seed in &seeds()[..5] {
        for (c, text) in CORPUS.iter().enumerate() {
            row(
                "rd",
                seed,
                &format!("wrap.c{c}"),
                ctx(seed, || {
                    let r: Arc<dyn Random> = Arc::new(JdkRandom::new(seed));
                    let mut t = MockTokenizer::new(Arc::clone(&WHITESPACE), false).unwrap();
                    t.set_reader(Box::new(MockReaderWrapper::new(
                        Arc::clone(&r),
                        reader(text),
                    )))
                    .unwrap();
                    format!("{}\t{}", dump(&mut t), r.next_long())
                }),
            );
            for remainder in [0, 2, 7] {
                row(
                    "rd",
                    seed,
                    &format!("cf{remainder}.c{c}"),
                    ctx(seed, || {
                        let mut t = MockTokenizer::new(Arc::clone(&WHITESPACE), false).unwrap();
                        t.set_reader(Box::new(
                            MockCharFilter::new(reader(text), remainder).unwrap(),
                        ))
                        .unwrap();
                        dump(&mut t)
                    }),
                );
            }
            for k in [0, 3, len16(text)] {
                row(
                    "rd",
                    seed,
                    &format!("evil{k}.c{c}"),
                    ctx(seed, || {
                        let r: Arc<dyn Random> = Arc::new(JdkRandom::new(seed));
                        let mut w = MockReaderWrapper::new(r, reader(text));
                        w.throw_exc_after_char(k);
                        let mut t = MockTokenizer::new(Arc::clone(&WHITESPACE), false).unwrap();
                        t.set_enable_checks(false);
                        t.set_reader(Box::new(w)).unwrap();
                        dump(&mut t)
                    }),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// an
// ---------------------------------------------------------------------------

fn analyzer(config: i32, random: &dyn Random) -> MockAnalyzer {
    match config {
        0 => MockAnalyzer::new(random),
        1 => MockAnalyzer::with_filter(
            random,
            Arc::clone(&SIMPLE),
            true,
            Arc::clone(&ENGLISH_STOPSET),
        ),
        2 => MockAnalyzer::with_filter(random, Arc::clone(&WHITESPACE), true, automaton(".{5,}")),
        3 => MockAnalyzer::with_automaton(random, Arc::clone(&KEYWORD), false),
        _ => MockAnalyzer::with_term_freq(random, Arc::new(|term: &str| len16(term) + 1)),
    }
}

fn analyzers() {
    let inputs = [1, 2, 4, 7];
    for &seed in &seeds() {
        for config in 0..5 {
            ctx(seed, || {
                let a = analyzer(config, &*random());
                for f in 0..3 {
                    for c in inputs {
                        let ts = a
                            .token_stream_from_str(&format!("f{f}"), CORPUS[c])
                            .unwrap();
                        let name = impl_name(ts.borrow().attribute_source());
                        row(
                            "an",
                            seed,
                            &format!("{config}.f{f}.c{c}"),
                            format!("{}\t{}", name, dump_rc(&ts)),
                        );
                    }
                }
                row(
                    "an",
                    seed,
                    &format!("{config}.norm"),
                    hex(Some(&a.normalize("f0", CORPUS[1]).unwrap())),
                );
                row(
                    "an",
                    seed,
                    &format!("{config}.next"),
                    random().next_long().to_string(),
                );
            });
        }
    }
}

// ---------------------------------------------------------------------------
// fl
// ---------------------------------------------------------------------------

fn ws(text: &str) -> Result<Rc<RefCell<MockTokenizer>>> {
    let mut t = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    t.set_reader(reader(text))?;
    Ok(Rc::new(RefCell::new(t)))
}

type Wrap = fn(&Arc<dyn Random>, Box<dyn TokenStream>) -> Result<Box<dyn TokenStream>>;

fn filters() {
    let filters: Vec<(&str, Wrap)> = vec![
        ("fixed0", |r, i| {
            Ok(Box::new(MockFixedLengthPayloadFilter::new(
                Arc::clone(r),
                i,
                0,
            )?))
        }),
        ("fixed1", |r, i| {
            Ok(Box::new(MockFixedLengthPayloadFilter::new(
                Arc::clone(r),
                i,
                1,
            )?))
        }),
        ("fixed5", |r, i| {
            Ok(Box::new(MockFixedLengthPayloadFilter::new(
                Arc::clone(r),
                i,
                5,
            )?))
        }),
        ("fixed11", |r, i| {
            Ok(Box::new(MockFixedLengthPayloadFilter::new(
                Arc::clone(r),
                i,
                11,
            )?))
        }),
        ("var", |r, i| {
            Ok(Box::new(MockVariableLengthPayloadFilter::new(
                Arc::clone(r),
                i,
            )?))
        }),
        ("hole", |r, i| {
            Ok(Box::new(MockHoleInjectingTokenFilter::new(&**r, i)?))
        }),
        ("graph", |r, i| {
            Ok(Box::new(MockGraphTokenFilter::new(&**r, i)?))
        }),
        ("look", |r, i| {
            Ok(Box::new(MockRandomLookaheadTokenFilter::new(&**r, i)?))
        }),
        ("syn", |_, i| Ok(Box::new(MockSynonymFilter::new(i)?))),
        ("low", |_, i| Ok(Box::new(MockLowerCaseFilter::new(i)?))),
        ("stop", |_, i| {
            Ok(Box::new(MockTokenFilter::new(
                i,
                Arc::clone(&ENGLISH_STOPSET),
            )?))
        }),
        ("cranky", |r, i| {
            Ok(Box::new(CrankyTokenFilter::new(i, Arc::clone(r))))
        }),
        ("valid", |r, i| {
            Ok(Box::new(ValidatingTokenFilter::new(
                Box::new(MockGraphTokenFilter::new(&**r, i)?),
                "v",
            )))
        }),
        ("simplepayload", |_, i| {
            Ok(Box::new(SimplePayloadFilter::new(i)?))
        }),
    ];
    let inputs = [1, 3, 4, 5, 7, 9];
    for &seed in &seeds() {
        for (name, wrap) in &filters {
            for c in inputs {
                let v = ctx(seed, || {
                    let r: Arc<dyn Random> = Arc::new(JdkRandom::new(seed));
                    let t = ws(CORPUS[c]).unwrap();
                    let mut ts = wrap(&r, shared(&t)).unwrap();
                    let mut first = dump(&mut *ts);
                    if *name == "graph" || *name == "valid" {
                        t.borrow_mut().set_reader(reader(CORPUS[c])).unwrap();
                        first.push('\t');
                        first.push_str(&dump(&mut *ts));
                    } else {
                        first.push_str("\t-");
                    }
                    format!("{}\t{}\t{}", first, r.next_long(), random().next_long())
                });
                row("fl", seed, &format!("{name}.c{c}"), v);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// fx
// ---------------------------------------------------------------------------

fn fixed() {
    let s = seeds();
    ctx(s[0], || {
        let t1 = Token::with_offsets("foo", 0, 3);
        let mut t2 = Token::with_pos_inc("bar", 0, 4, 7);
        rucene::analysis::tokenattributes::FlagsAttribute::set_flags(&mut t2, 3);
        rucene::analysis::tokenattributes::PayloadAttribute::set_payload(
            &mut t2,
            Some(BytesRef::new(vec![1, 2])),
        );
        let mut t3 = Token::with_pos_len("baz", 2, 8, 11, 2);
        rucene::analysis::tokenattributes::TypeAttribute::set_type(&mut t3, "t".to_string());
        let canned = CannedTokenStream::new(vec![t1.clone(), t2.clone(), t3.clone()]);
        let name = impl_name(canned.attribute_source());
        row(
            "fx",
            0,
            "canned",
            format!(
                "{}\t{}",
                name,
                dump(&mut CannedTokenStream::new(vec![
                    t1.clone(),
                    t2.clone(),
                    t3.clone()
                ]))
            ),
        );
        row(
            "fx",
            0,
            "canned.final",
            dump(&mut CannedTokenStream::with_final(3, 20, vec![t1, t3])),
        );
        let b1 = BinaryToken::new(BytesRef::new(vec![0xff, 0]));
        let mut b2 = BinaryToken::with_positions(BytesRef::new(b"x".to_vec()), 0, 3);
        b2.start_offset = 4;
        b2.end_offset = 9;
        let mut bin = CannedBinaryTokenStream::new(vec![b1, b2]);
        let bytes = Att::get(bin.attribute_source(), term_to_bytes_ref_id()).unwrap();
        let mut sb = String::new();
        bin.reset().unwrap();
        while bin.increment_token().unwrap() {
            let src = bin.attribute_source();
            let o = Att::get(src, offset_id()).unwrap();
            let (so, eo) = {
                let o = o.offset();
                (o.start_offset(), o.end_offset())
            };
            sb.push_str(&format!(
                "{},{},{},{},{} ",
                hex(Some(&bytes.term_to_bytes_ref().get_bytes_ref())),
                so,
                eo,
                Att::get(src, pos_inc_id())
                    .unwrap()
                    .pos_inc()
                    .get_position_increment(),
                Att::get(src, pos_len_id())
                    .unwrap()
                    .pos_len()
                    .get_position_length()
            ));
        }
        row("fx", 0, "binary", sb);
        for c in [1usize, 7] {
            let pa = MockPayloadAnalyzer::new();
            row(
                "fx",
                0,
                &format!("payloadanalyzer.c{c}"),
                dump_rc(&pa.token_stream_from_str("p", CORPUS[c]).unwrap()),
            );
            let ba = MockBytesAnalyzer::new();
            let ts = ba
                .token_stream_from_str("b", CORPUS[if c == 1 { 4 } else { c }])
                .unwrap();
            let mut ts = ts.borrow_mut();
            let bytes = Att::get(ts.attribute_source(), term_to_bytes_ref_id()).unwrap();
            let mut s = impl_name(ts.attribute_source());
            ts.reset().unwrap();
            while ts.increment_token().unwrap() {
                s.push(' ');
                s.push_str(&hex(Some(&bytes.term_to_bytes_ref().get_bytes_ref())));
            }
            ts.end().unwrap();
            ts.close().unwrap();
            row("fx", 0, &format!("bytes.c{c}"), s);
        }
        let sa = MockSynonymAnalyzer::new();
        row(
            "fx",
            0,
            "synonymanalyzer",
            dump_rc(&sa.token_stream_from_str("s", CORPUS[9]).unwrap()),
        );
    });
    for &seed in &s[..5] {
        for c in [1usize, 4, 9] {
            row(
                "fx",
                seed,
                &format!("dot.graph.c{c}"),
                ctx(seed, || {
                    let t = ws(CORPUS[c]).unwrap();
                    let mut ts =
                        MockGraphTokenFilter::new(&JdkRandom::new(seed), shared(&t)).unwrap();
                    let mut dot = TokenStreamToDot::new(Some(CORPUS[c]), &mut ts).unwrap();
                    dot.to_dot().unwrap();
                    esc(&dot.out)
                }),
            );
            row(
                "fx",
                seed,
                &format!("dot.nulltext.c{c}"),
                ctx(seed, || {
                    let t = ws(CORPUS[c]).unwrap();
                    let mut ts = MockSynonymFilter::new(shared(&t)).unwrap();
                    let mut dot = TokenStreamToDot::new(None, &mut ts).unwrap();
                    dot.to_dot().unwrap();
                    esc(&dot.out)
                }),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// mis
// ---------------------------------------------------------------------------

/// A stream that never calls clearAttributes().
#[derive(Debug)]
struct NoClearStream {
    source: AttributeSource,
    term: Att,
    off: Att,
    upto: i32,
}

impl NoClearStream {
    fn new() -> NoClearStream {
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        let term = Att::add(&mut source, char_term_id()).unwrap();
        let off = Att::add(&mut source, offset_id()).unwrap();
        NoClearStream {
            source,
            term,
            off,
            upto: 0,
        }
    }
}

impl TokenStream for NoClearStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.upto < 2 {
            self.term.set_term(if self.upto == 0 { "a" } else { "b" });
            self.off
                .offset_mut()
                .set_offset(self.upto * 2, self.upto * 2 + 1);
            self.upto += 1;
            return Ok(true);
        }
        Ok(false)
    }
    fn reset(&mut self) -> Result<()> {
        self.upto = 0;
        Ok(())
    }
    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }
    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// The anonymous tokenizer of the `btstc.resetcheck` scenario.
#[derive(Debug)]
struct OneX {
    term: Att,
    done: bool,
}

impl TokenizerLogic for OneX {
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        _reader: &mut dyn rucene::analysis::CharReader,
    ) -> Result<bool> {
        source.clear_attributes();
        if self.done {
            return Ok(false);
        }
        self.done = true;
        self.term.char_term_mut().append_string("x");
        Ok(true)
    }

    fn reset(
        &mut self,
        _source: &mut AttributeSource,
        _reader: &mut dyn rucene::analysis::CharReader,
    ) -> Result<()> {
        self.done = false;
        Ok(())
    }
}

#[derive(Debug)]
struct CannedAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for CannedAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.analysis.AnalysisMocksOracle$1"
    }
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let mut source =
            AttributeSource::new_with_factory(rucene::analysis::default_token_attribute_factory());
        let term = Att::add(&mut source, char_term_id())?;
        let t = Tokenizer::with_source(source, OneX { term, done: false });
        Ok(tokenizer_components(Rc::new(RefCell::new(t))))
    }
}

fn mock(checks: bool) -> Result<MockTokenizer> {
    let mut t = MockTokenizer::new(Arc::clone(&WHITESPACE), false)?;
    t.set_enable_checks(checks);
    Ok(t)
}

fn misuse() {
    ctx(seeds()[5], || {
        for checks in [true, false] {
            let k = if checks { "checks" } else { "nochecks" };
            row(
                "mis",
                0,
                &format!("{k}.noreset"),
                outcome(|| {
                    let mut t = mock(checks)?;
                    t.set_reader(reader("a b"))?;
                    Ok(t.increment_token()?.to_string())
                }),
            );
            row(
                "mis",
                0,
                &format!("{k}.doublereset"),
                outcome(|| {
                    let mut t = mock(checks)?;
                    t.set_reader(reader("a b"))?;
                    t.reset()?;
                    t.reset()?;
                    Ok("-".to_string())
                }),
            );
            row(
                "mis",
                0,
                &format!("{k}.closenoend"),
                outcome(|| {
                    let mut t = mock(checks)?;
                    t.set_reader(reader("a b"))?;
                    t.reset()?;
                    while t.increment_token()? {}
                    t.close()?;
                    Ok("-".to_string())
                }),
            );
            row(
                "mis",
                0,
                &format!("{k}.earlyend"),
                outcome(|| {
                    let mut t = mock(checks)?;
                    t.set_reader(reader("a b"))?;
                    t.reset()?;
                    t.increment_token()?;
                    t.end()?;
                    Ok(tok(t.attribute_source()))
                }),
            );
            row(
                "mis",
                0,
                &format!("{k}.setreadertwice"),
                outcome(|| {
                    let mut t = mock(checks)?;
                    t.set_reader(reader("a b"))?;
                    t.set_reader(reader("c"))?;
                    Ok("-".to_string())
                }),
            );
            row(
                "mis",
                0,
                &format!("{k}.noreader"),
                outcome(|| {
                    let mut t = mock(checks)?;
                    t.reset()?;
                    Ok(t.increment_token()?.to_string())
                }),
            );
            row(
                "mis",
                0,
                &format!("{k}.closenoreset"),
                outcome(|| {
                    let mut t = mock(checks)?;
                    t.set_reader(reader("a b"))?;
                    t.close()?;
                    Ok("-".to_string())
                }),
            );
            row(
                "mis",
                0,
                &format!("{k}.nocloseinputs"),
                outcome(|| {
                    let mut t = mock(checks)?;
                    t.set_reader(reader("a b"))?;
                    t.reset()?;
                    while t.increment_token()? {}
                    t.end()?;
                    t.set_reader(reader("c"))?;
                    Ok("-".to_string())
                }),
            );
        }
        row(
            "mis",
            0,
            "an.twice",
            outcome(|| {
                let a = MockAnalyzer::new(&*random());
                let ts = a.token_stream_from_str("f", "a b")?;
                {
                    let mut ts = ts.borrow_mut();
                    ts.reset()?;
                    while ts.increment_token()? {}
                    ts.end()?;
                }
                a.token_stream_from_str("f", "c")?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "charfilter.remainder",
            outcome(|| {
                MockCharFilter::new(reader("x"), 10)?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "fixed.length",
            outcome(|| {
                let t = ws("a")?;
                MockFixedLengthPayloadFilter::new(Arc::new(JdkRandom::new(1)), shared(&t), -1)?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "graph.noreset",
            outcome(|| {
                let t = ws("a")?;
                let mut ts = MockGraphTokenFilter::new(&JdkRandom::new(1), shared(&t))?;
                Ok(ts.increment_token()?.to_string())
            }),
        );
        let a = MockAnalyzer::new(&*random());
        row(
            "mis",
            0,
            "btstc.ok",
            outcome(|| {
                B::assert_analyzes_to_offsets(&a, "foo bar", &["foo", "bar"], &[0, 4], &[3, 7])?;
                B::check_reset_exception(&a, "abc")?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.term",
            outcome(|| {
                B::assert_analyzes_to(&a, "foo bar", &["foo", "baz"])?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.term.long",
            outcome(|| {
                B::assert_analyzes_to(
                    &a,
                    "the quick brown fox jumps over the lazy dog xxxxxxxxxxxxxxxxxxxxxxxxxx",
                    &["the quick brown fox jumps over the lazy dog xxxxxxxxxxxxxxxxxxxxxxxxxx"],
                )?;
                Ok("-".to_string())
            }),
        );
        let kw = MockAnalyzer::with_automaton(&*random(), Arc::clone(&KEYWORD), false);
        row(
            "mis",
            0,
            "btstc.term.context",
            outcome(|| {
                B::assert_analyzes_to(
                    &kw,
                    "abcdefghijklmnopqrstuvwxyz0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ",
                    &["abcdefghijklmnopqrstuvwxyz0123456789-ABCDEFGHIJKLMNOPQRSTUVWXYZ"],
                )?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.extra",
            outcome(|| {
                B::assert_analyzes_to(&a, "foo bar", &["foo"])?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.missing",
            outcome(|| {
                B::assert_analyzes_to(&a, "foo bar", &["foo", "bar", "x"])?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.start",
            outcome(|| {
                B::assert_analyzes_to_offsets(&a, "foo bar", &["foo", "bar"], &[1, 4], &[3, 7])?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.end",
            outcome(|| {
                B::assert_analyzes_to_offsets(&a, "foo bar", &["foo", "bar"], &[0, 4], &[3, 8])?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.posinc",
            outcome(|| {
                B::assert_analyzes_to_pos_increments(&a, "foo bar", &["foo", "bar"], &[1, 2])?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.type",
            outcome(|| {
                B::assert_analyzes_to_types_only(
                    &a,
                    "foo bar",
                    &["foo", "bar"],
                    &["word", "wordy"],
                )?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.finaloffset",
            outcome(|| {
                let ts = a.token_stream_from_str("f", "foo")?;
                let mut ts = ts.borrow_mut();
                B::assert_token_stream_contents_final_offset(
                    &mut *ts,
                    &["foo"],
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(99),
                )?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.finalposinc",
            outcome(|| {
                let ts = a.token_stream_from_str("f", "foo")?;
                let mut ts = ts.borrow_mut();
                B::assert_token_stream_contents_payloads(
                    &mut *ts,
                    &["foo"],
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(3),
                    Some(7),
                    None,
                    true,
                    None,
                )?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.noclear",
            outcome(|| {
                B::assert_token_stream_contents(&mut NoClearStream::new(), &["a", "b"])?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.resetcheck",
            outcome(|| {
                let canned = CannedAnalyzer {
                    state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
                };
                B::check_reset_exception(&canned, "abc")?;
                Ok("-".to_string())
            }),
        );
        row(
            "mis",
            0,
            "btstc.tostring",
            outcome(|| Ok(esc(&B::to_string(&MockSynonymAnalyzer::new(), CORPUS[9])?))),
        );
        row(
            "mis",
            0,
            "btstc.escape",
            outcome(|| Ok(B::escape(&format!("a\"b\\c\nd\re\u{e9}{}", CORPUS[4])))),
        );
        row(
            "mis",
            0,
            "btstc.number",
            outcome(|| {
                let ts = a.token_stream_from_str("f", "a b c")?;
                let mut ts = ts.borrow_mut();
                B::assert_stream_has_number_of_tokens(&mut *ts, 2)?;
                Ok("-".to_string())
            }),
        );
    });
}

// ---------------------------------------------------------------------------
// crd
// ---------------------------------------------------------------------------

thread_local! {
    static SEEN: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

#[derive(Debug)]
struct Recording {
    term: Att,
}

impl TokenFilterLogic for Recording {
    fn increment_token(
        &mut self,
        _source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        if input.increment_token()? {
            let term = self.term.term_string();
            SEEN.with(|s| {
                if let Some(seen) = s.borrow_mut().as_mut() {
                    seen.push(term);
                }
            });
            return Ok(true);
        }
        Ok(false)
    }
}

#[derive(Debug)]
struct RecordingAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for RecordingAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.analysis.AnalysisMocksOracle$2"
    }
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let t = Rc::new(RefCell::new(MockTokenizer::with_max_token_length(
            Arc::clone(&KEYWORD),
            false,
            i32::MAX,
        )?));
        let mut input = shared(&t);
        let term = Att::add(input.attribute_source_mut(), char_term_id())?;
        let filter = TokenFilter::new(input, Recording { term });
        Ok(components(t, sink(filter)))
    }
}

fn check_random_data_parity() {
    for &seed in &seeds() {
        for simple in [false, true] {
            let v = ctx(seed, || {
                let a: Arc<dyn Analyzer> = Arc::new(RecordingAnalyzer {
                    state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
                });
                SEEN.with(|s| *s.borrow_mut() = Some(Vec::new()));
                let r = JdkRandom::new(seed);
                let o = match catch_unwind(AssertUnwindSafe(|| {
                    B::check_random_data_max_word_length_simple(&r, &a, 4, 20, simple)
                })) {
                    Ok(Ok(())) => "ok".to_string(),
                    Ok(Err(e)) => format!("err\t{}", err(&e)),
                    Err(payload) => format!("err\t{}", panic_err(payload)),
                };
                let seen = SEEN.with(|s| s.borrow_mut().take()).unwrap_or_default();
                let mut sb = String::new();
                for s in &seen {
                    sb.push_str(&units(s));
                    sb.push(' ');
                }
                format!(
                    "{}\t{}\t{}\t{}\t{}",
                    o,
                    seen.len(),
                    sb,
                    r.next_long(),
                    random().next_long()
                )
            });
            row("crd", seed, &format!("simple{simple}"), v);
        }
        // the random analysis strings themselves
        let r = JdkRandom::new(seed);
        let mut sb = String::new();
        for i in 0..6 {
            sb.push_str(&units(&TestUtil::random_analysis_string(
                &r,
                20,
                i % 2 == 1,
            )));
            sb.push(' ');
        }
        row("crd", seed, "strings", format!("{}\t{}", sb, r.next_long()));
    }
}

// ---------------------------------------------------------------------------
// core
// ---------------------------------------------------------------------------

fn core() {
    ctx(seeds()[6], || {
        let mut st = new_standard_tokenizer();
        st.set_reader(reader("hello world")).unwrap();
        row("core", 0, "standard.end", dump(&mut st));
        let t = Rc::new(RefCell::new(
            MockTokenizer::new(Arc::clone(&WHITESPACE), false).unwrap(),
        ));
        let mut outer = MockLowerCaseFilter::new(shared(&t)).unwrap();
        let flags = Att::add(outer.attribute_source_mut(), flags_id()).unwrap();
        flags.flags_mut().set_flags(5);
        t.borrow().attribute_source().clear_attributes();
        row(
            "core",
            0,
            "shared.clear",
            flags.flags().get_flags().to_string(),
        );
        flags.flags_mut().set_flags(6);
        t.borrow_mut().set_reader(reader("A")).unwrap();
        outer.reset().unwrap();
        outer.increment_token().unwrap();
        let during = tok(outer.attribute_source());
        flags.flags_mut().set_flags(9);
        outer.increment_token().unwrap();
        outer.end().unwrap();
        row(
            "core",
            0,
            "shared.end",
            format!("{} {}", during, tok(outer.attribute_source())),
        );
        outer.close().unwrap();
    });
}

// ---------------------------------------------------------------------------
// u16
// ---------------------------------------------------------------------------

const U16: [&str; 13] = [
    CORPUS[4],
    CORPUS[6],
    "\u{29B05}\u{8271}\u{935F}\u{43F9}\u{612F}\u{701B}",
    "\u{1F4A9} \u{1F4A9}\u{1F4A9}",
    "poo\u{1F4A9}poo",
    "\u{1F4A9}\u{4E2D}\u{570B}\u{1F4A9}",
    "\u{1F469}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F469}",
    "\u{1F468}\u{1F3FC}\u{200D}\u{2695}\u{FE0F}",
    "\u{1F1FA}\u{1F1F8}\u{1F1FA}\u{1F1F8}",
    "12345678\u{10300}",
    "ab cd toolong xy z",
    CORPUS[1],
    CORPUS[8],
];

/// The reader of `u16` case `r`: plain, or behind a `MockCharFilter` of
/// remainder 0, 2, 7.
fn u16_reader(r: usize, text: &str) -> Box<dyn rucene::analysis::CharReader> {
    if r == 0 {
        reader(text)
    } else {
        Box::new(MockCharFilter::new(reader(text), [0, 0, 2, 7][r]).unwrap())
    }
}

fn utf16() {
    let maxes = [255, 5, 2, 1];
    for (c, text) in U16.iter().enumerate() {
        for r in 0..4 {
            for max in maxes {
                row(
                    "u16",
                    0,
                    &format!("std.c{c}.r{r}.m{max}"),
                    ctx(seeds()[0], || {
                        let mut st = new_standard_tokenizer();
                        st.logic_mut().set_max_token_length(max).unwrap();
                        st.set_reader(u16_reader(r, text)).unwrap();
                        dump(&mut st)
                    }),
                );
            }
            for (pi, pattern) in [&WHITESPACE, &SIMPLE, &KEYWORD].into_iter().enumerate() {
                row(
                    "u16",
                    0,
                    &format!("mock.c{c}.r{r}.p{pi}"),
                    ctx(seeds()[0], || {
                        let mut t =
                            MockTokenizer::with_max_token_length(Arc::clone(pattern), false, 3)
                                .unwrap();
                        t.set_reader(u16_reader(r, text)).unwrap();
                        dump(&mut t)
                    }),
                );
            }
        }
        for max in [255, 3] {
            row(
                "u16",
                0,
                &format!("sa.c{c}.m{max}"),
                ctx(seeds()[0], || {
                    let mut a = StandardAnalyzer::new();
                    a.set_max_token_length(max);
                    let first = dump_rc(&a.token_stream_from_str("f", text).unwrap());
                    let twice = format!("{text} {text}");
                    let second = dump_rc(&a.token_stream_from_str("f", &twice).unwrap());
                    format!("{first}\t{second}")
                }),
            );
        }
        for seed in [seeds()[0], seeds()[3], seeds()[12]] {
            row(
                "u16",
                seed,
                &format!("ma.c{c}"),
                ctx(seed, || {
                    let a = MockAnalyzer::new(&*random());
                    let ts = a.token_stream_from_str("f", text).unwrap();
                    let name = impl_name(ts.borrow().attribute_source());
                    format!("{}\t{}", name, dump_rc(&ts))
                }),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// fac
// ---------------------------------------------------------------------------

fn factory(f: usize) -> Arc<dyn rucene::util::attribute::AttributeFactory> {
    match f {
        0 => rucene::analysis::default_token_attribute_factory(),
        1 => Token::token_attribute_factory(),
        _ => rucene::analysis::default_attribute_factory(),
    }
}

fn chain(t: Box<dyn TokenStream>, syn: bool) -> Result<Box<dyn TokenStream>> {
    let stops = rucene::analysis::CharArraySet::from_iter(["the", "a", "and", "is", "it"], false);
    let ts: Box<dyn TokenStream> = Box::new(rucene::analysis::new_stop_filter(
        Box::new(rucene::analysis::new_lower_case_filter(t)?),
        stops,
    )?);
    Ok(if syn {
        Box::new(MockSynonymFilter::new(ts)?)
    } else {
        ts
    })
}

fn standard(f: usize, text: &str) -> Result<rucene::analysis::StandardTokenizer> {
    let mut t = rucene::analysis::new_standard_tokenizer_with_factory(factory(f))?;
    t.set_reader(reader(text))?;
    Ok(t)
}

/// Consumes the stream up to `end()`, without closing it.
fn pass(ts: &mut dyn TokenStream) -> Result<String> {
    let mut sb = String::new();
    ts.reset()?;
    while ts.increment_token()? {
        sb.push_str(&tok(ts.attribute_source()));
        sb.push(' ');
    }
    ts.end()?;
    sb.push_str("end:");
    sb.push_str(&tok(ts.attribute_source()));
    Ok(sb)
}

/// `Paths.walk()`: every base token, then each of its graphs.
fn walk(g: &mut rucene::analysis::graph_token_filter::GraphTokenFilter) -> Result<String> {
    g.attribute_source_mut()
        .add_attribute_interface::<dyn rucene::analysis::CharTermAttribute>()?;
    let term = |g: &rucene::analysis::graph_token_filter::GraphTokenFilter| {
        let t = g
            .attribute_source()
            .get_attribute_interface::<dyn rucene::analysis::CharTermAttribute>()
            .unwrap();
        units(&t.buffer()[..t.length()].iter().collect::<String>())
    };
    let mut sb = String::new();
    g.reset()?;
    while g.increment_base_token()? {
        sb.push('[');
        sb.push_str(&term(g));
        loop {
            sb.push_str(" {");
            while g.increment_graph_token()? {
                sb.push_str(&term(g));
                sb.push(',');
            }
            sb.push('}');
            if !g.increment_graph()? {
                break;
            }
        }
        sb.push_str("] ");
    }
    g.end()?;
    sb.push_str(&format!(
        "end:{} {}",
        tok(g.attribute_source()),
        g.get_trailing_positions()
    ));
    g.close()?;
    Ok(sb)
}

#[derive(Debug)]
struct FacAnalyzer {
    state: AnalyzerState,
    f: usize,
}

impl Analyzer for FacAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.tests.analysis.AnalysisMocksOracle$2"
    }
    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let t = Rc::new(RefCell::new(
            rucene::analysis::new_standard_tokenizer_with_factory(factory(self.f))?,
        ));
        let ts = chain(shared(&t), true)?;
        let ts: Rc<RefCell<dyn TokenStream>> = Rc::new(RefCell::new(BoxedStream(ts)));
        Ok(components(t, ts))
    }
}

/// A boxed stream as a sized `TokenStream`.
#[derive(Debug)]
struct BoxedStream(Box<dyn TokenStream>);

impl TokenStream for BoxedStream {
    fn increment_token(&mut self) -> Result<bool> {
        self.0.increment_token()
    }
    fn end(&mut self) -> Result<()> {
        self.0.end()
    }
    fn reset(&mut self) -> Result<()> {
        self.0.reset()
    }
    fn close(&mut self) -> Result<()> {
        self.0.close()
    }
    fn attribute_source(&self) -> &AttributeSource {
        self.0.attribute_source()
    }
    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.0.attribute_source_mut()
    }
}

fn query_string(q: Result<Option<Arc<dyn rucene::search::Query>>>) -> String {
    match q.unwrap() {
        Some(q) => q.to_query_string(""),
        None => "null".to_string(),
    }
}

fn factories() {
    for f in 0..3 {
        for c in [1, 4, 7, 9] {
            let text = CORPUS[c];
            let p = format!("f{f}.c{c}.");
            row(
                "fac",
                0,
                &format!("{p}std"),
                ctx(seeds()[0], || {
                    let mut t = standard(f, text).unwrap();
                    format!("{}\t{}", impl_name(t.attribute_source()), dump(&mut t))
                }),
            );
            row(
                "fac",
                0,
                &format!("{p}lcstop"),
                ctx(seeds()[0], || {
                    dump(&mut *chain(Box::new(standard(f, text).unwrap()), false).unwrap())
                }),
            );
            row(
                "fac",
                0,
                &format!("{p}syn"),
                ctx(seeds()[0], || {
                    dump(&mut *chain(Box::new(standard(f, text).unwrap()), true).unwrap())
                }),
            );
            row(
                "fac",
                0,
                &format!("{p}cache"),
                ctx(seeds()[0], || {
                    let mut cf = rucene::analysis::CachingTokenFilter::new(
                        chain(Box::new(standard(f, text).unwrap()), true).unwrap(),
                    );
                    let first = pass(&mut cf).unwrap();
                    let second = pass(&mut cf).unwrap();
                    cf.close().unwrap();
                    format!("{first}\t{second}")
                }),
            );
            row(
                "fac",
                0,
                &format!("{p}graph"),
                ctx(seeds()[0], || {
                    let mut g = rucene::analysis::graph_token_filter::GraphTokenFilter::new(
                        chain(Box::new(standard(f, text).unwrap()), true).unwrap(),
                    )
                    .unwrap();
                    walk(&mut g).unwrap()
                }),
            );
            row(
                "fac",
                0,
                &format!("{p}qb"),
                ctx(seeds()[0], || {
                    use rucene::util::query_builder::{QueryBuilder, QueryBuilderOps};
                    let a: Arc<dyn Analyzer> = Arc::new(FacAnalyzer {
                        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
                        f,
                    });
                    let qb = QueryBuilder::new(a);
                    format!(
                        "{}\t{}\t{}",
                        query_string(qb.create_boolean_query("f", text)),
                        query_string(qb.create_phrase_query("f", text)),
                        query_string(qb.create_boolean_query_with_operator(
                            "f",
                            text,
                            rucene::search::Occur::MUST
                        ))
                    )
                }),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// stdtok
// ---------------------------------------------------------------------------

/// `AnalysisMocksOracle.STD`: the inputs of Lucene's `TestStandardAnalyzer`
/// plus odd regional indicators and supplementary digits (generated from the
/// Java source).
fn std_inputs() -> Vec<String> {
    vec![
        "\u{E01}\u{E32}\u{E23}\u{E17}\u{E35}\u{E48}\u{E44}\u{E14}\u{E49}\u{E15}\u{E49}\u{E2D}\u{E07}\u{E41}\u{E2A}\u{E14}\u{E07}\u{E27}\u{E48}\u{E32}\u{E07}\u{E32}\u{E19}\u{E14}\u{E35}. \u{E41}\u{E25}\u{E49}\u{E27}\u{E40}\u{E18}\u{E2D}\u{E08}\u{E30}\u{E44}\u{E1B}\u{E44}\u{E2B}\u{E19}? \u{E51}\u{E52}\u{E53}\u{E54}".to_string(),
        "\u{EAA}\u{EB2}\u{E97}\u{EB2}\u{EA5}\u{EB0}\u{E99}\u{EB0}\u{EA5}\u{EB1}\u{E94} \u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E97}\u{EB4}\u{E9B}\u{EB0}\u{EC4}\u{E95} \u{E9B}\u{EB0}\u{E8A}\u{EB2}\u{E8A}\u{EBB}\u{E99}\u{EA5}\u{EB2}\u{EA7}".to_string(),
        "\u{C548}\u{B155}\u{D558}\u{C138}\u{C694} \u{D55C}\u{AE00}\u{C785}\u{B2C8}\u{B2E4} \u{D6C8}\u{BBFC}\u{C815}\u{C74C}".to_string(),
        "\u{4EEE}\u{540D}\u{9063}\u{3044} \u{30AB}\u{30BF}\u{30AB}\u{30CA}".to_string(),
        "\u{3056} \u{30B6} \u{58F9}\u{3099} \u{C544}\u{3099}".to_string(),
        "21.35 R2D2 C3PO 216.239.63.104 David has 5000 bones".to_string(),
        "A:B A::B 1.2 A.B 1..2 A..B 1,2 1,,2 A.:B 1,.2 A:B_A::B 1.2_1..2 C_A.:B 3_1.,2".to_string(),
        "\u{1F4A9} \u{1F4A9}\u{1F4A9} poo\u{1F4A9}poo \u{1F4A9}\u{4E2D}\u{570B}\u{1F4A9}".to_string(),
        "\u{1F469}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F469} \u{1F468}\u{1F3FC}\u{200D}\u{2695}\u{FE0F}".to_string(),
        "\u{1F1FA}\u{1F1F8}\u{1F1FA}\u{1F1F8}\u{1F1FA} x\u{1F1FA}\u{1F1F8}y".to_string(),
        "#\u{FE0F}\u{20E3} 3\u{FE0F}\u{20E3} #\u{FE0E} 3\u{FE0E} \u{2B55}\u{FE0E} \u{2B55}\u{FE0E}\u{200D}\u{2B55}\u{FE0E}".to_string(),
        "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}".to_string(),
        "\u{1D7CE}\u{1D7CF} 12\u{1D7D0} \u{104A0}\u{104A1}.5 \u{661}\u{662}\u{663} x\u{104A2}".to_string(),
        format!("x {} {}b", "b".repeat(255), "b".repeat(255)),
    ]
}

/// `TestStandardAnalyzer`'s `SpoonFeedMaxCharsReaderWrapper`: at most
/// `max_chars` characters per read.
#[derive(Debug)]
struct SpoonFeed {
    input: Box<dyn rucene::analysis::CharReader>,
    max_chars: usize,
}

impl rucene::analysis::CharReader for SpoonFeed {
    fn read(&mut self, buf: &mut [char]) -> Result<usize> {
        let n = self.max_chars.min(buf.len());
        self.input.read(&mut buf[..n])
    }

    fn close(&mut self) -> Result<()> {
        self.input.close()
    }
}

/// The reader of `stdtok` case `r`: plain, `MockCharFilter` 0/2/7,
/// spoon-feed 1/2/9.
fn std_reader(r: usize, text: &str) -> Box<dyn rucene::analysis::CharReader> {
    match r {
        0 => reader(text),
        1..=3 => Box::new(MockCharFilter::new(reader(text), [0, 0, 2, 7][r]).unwrap()),
        _ => Box::new(SpoonFeed {
            input: reader(text),
            max_chars: [1, 2, 9][r - 4],
        }),
    }
}

/// `dumpResizing`: consumes the stream, calling `setMaxTokenLength(max)`
/// after token `after`.
fn dump_resizing(st: &mut rucene::analysis::StandardTokenizer, after: i32, max: i32) -> String {
    let mut sb = String::new();
    let result = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
        st.reset()?;
        let mut n = 0;
        while st.increment_token()? {
            sb.push_str(&tok(st.attribute_source()));
            sb.push(' ');
            n += 1;
            if n == after {
                st.logic_mut().set_max_token_length(max)?;
            }
        }
        st.end()?;
        sb.push_str("end:");
        sb.push_str(&tok(st.attribute_source()));
        st.close()
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            sb.push('!');
            sb.push_str(&err(&e));
        }
        Err(payload) => {
            sb.push('!');
            sb.push_str(&panic_err(payload));
        }
    }
    sb
}

fn standard_tokenizer() {
    let inputs = std_inputs();
    for (c, text) in inputs.iter().enumerate() {
        for r in 0..7 {
            for m in [1, 2, 3, 255] {
                row(
                    "stdtok",
                    0,
                    &format!("c{c}.r{r}.m{m}"),
                    ctx(seeds()[0], || {
                        let mut st = new_standard_tokenizer();
                        st.logic_mut().set_max_token_length(m).unwrap();
                        st.set_reader(std_reader(r, text)).unwrap();
                        dump(&mut st)
                    }),
                );
            }
        }
        // setBufferSize between two streams of one tokenizer
        for (a, b) in [(255, 2), (2, 255), (1, 3), (3, 1), (2, 2)] {
            row(
                "stdtok",
                0,
                &format!("c{c}.reuse{a}-{b}"),
                ctx(seeds()[0], || {
                    let mut st = new_standard_tokenizer();
                    st.logic_mut().set_max_token_length(a).unwrap();
                    st.set_reader(std_reader(2, text)).unwrap();
                    let first = dump(&mut st);
                    st.logic_mut().set_max_token_length(b).unwrap();
                    st.set_reader(std_reader(2, text)).unwrap();
                    format!("{first}\t{}", dump(&mut st))
                }),
            );
        }
        // setBufferSize in the middle of a stream, after the first and the
        // second token
        for after in 1..=2 {
            for (a, b) in [(3, 255), (255, 3), (2, 1), (1, 2), (255, 1)] {
                row(
                    "stdtok",
                    0,
                    &format!("c{c}.mid{after}.{a}-{b}"),
                    ctx(seeds()[0], || {
                        let mut st = new_standard_tokenizer();
                        st.logic_mut().set_max_token_length(a).unwrap();
                        st.set_reader(std_reader(0, text)).unwrap();
                        dump_resizing(&mut st, after, b)
                    }),
                );
            }
        }
    }
}

fn scenarios() {
    tokenizers();
    lengths();
    readers();
    analyzers();
    filters();
    fixed();
    misuse();
    check_random_data_parity();
    core();
    utf16();
    factories();
    standard_tokenizer();
}

#[test]
fn analysis_mocks_match_lucene() {
    let class = Class::builder("org.apache.lucene.tests.analysis.TestAnalysisMocksScenarios")
        .constructor(Constructor::no_args(|| ()))
        .field(
            Field::new_static("classNameRule", "org.junit.rules.TestRule", || {
                Value::TestRule(
                    Arc::clone(&CLASS_NAME_RULE) as Arc<dyn randomizedtesting::junit::TestRule>
                )
            })
            .annotate(ClassRule),
        )
        .method(
            Method::new("testScenarios")
                .annotate(Test::new())
                .body_any(scenarios),
        )
        .build();
    let class = Arc::new(class);
    ROWS.lock().unwrap().clear();
    system::set_property("tests.seed", SEED);
    let quiet = QuietPanics::new();
    let result = JUnitCore::new().run(&Request::a_class(&class));
    drop(quiet);
    system::clear_property("tests.seed");
    if let Some(f) = result.failures().first() {
        panic!("suite failure: {}", f.exception());
    }

    let key = |r: &[String]| format!("{}\t{}\t{}", r[0], r[1], r[2]);
    let mut expected = BTreeMap::new();
    for r in rows("analysis_mocks.tsv") {
        expected.insert(key(&r), r[3..].join("\t"));
    }
    let mut actual = BTreeMap::new();
    for line in ROWS.lock().unwrap().iter() {
        let r: Vec<String> = line.split('\t').map(str::to_string).collect();
        actual.insert(key(&r), r[3..].join("\t"));
    }
    assert_same("analysis mocks", &expected, &actual);
}
