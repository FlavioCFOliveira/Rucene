// Oracle of the analysis mocks of org.apache.lucene.tests.analysis (rmp #346) and of
// BaseTokenStreamTestCase (rmp #357): writes rucene-test-framework/analysis_mocks.tsv, one row per
// scenario, seed and step: "scenario<TAB>seed<TAB>step<TAB>value...". Every scenario that needs
// the test context runs inside RandomizedContext.runWithPrivateRandomness(new Randomness(seed,
// RandomSupplier.DEFAULT)), so its draws depend on the seed only. A token stream is dumped as its
// tokens (term, startOffset, endOffset, posInc, posLen, type, payload, flags, termFreq, keyword; '-'
// for an absent attribute; strings as hex UTF-16 units), then "end:" and the attributes after
// end(); an exception ends the dump with "!class<TAB>message".
package org.apache.lucene.tests.analysis;

import com.carrotsearch.randomizedtesting.OracleSupport;
import com.carrotsearch.randomizedtesting.RandomSupplier;
import com.carrotsearch.randomizedtesting.RandomizedContext;
import com.carrotsearch.randomizedtesting.Randomness;
import java.io.File;
import java.io.IOException;
import java.io.PrintWriter;
import java.io.Reader;
import java.io.StringReader;
import java.io.StringWriter;
import java.util.ArrayList;
import java.util.List;
import java.util.Random;
import java.util.concurrent.Callable;
import java.util.function.Function;
import java.util.function.Supplier;
import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.TokenFilter;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.Tokenizer;
import org.apache.lucene.analysis.standard.StandardTokenizer;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.analysis.tokenattributes.FlagsAttribute;
import org.apache.lucene.analysis.tokenattributes.KeywordAttribute;
import org.apache.lucene.analysis.tokenattributes.OffsetAttribute;
import org.apache.lucene.analysis.tokenattributes.PayloadAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionLengthAttribute;
import org.apache.lucene.analysis.tokenattributes.TermFrequencyAttribute;
import org.apache.lucene.analysis.tokenattributes.TermToBytesRefAttribute;
import org.apache.lucene.analysis.tokenattributes.TypeAttribute;
import org.apache.lucene.tests.util.TestUtil;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.automaton.CharacterRunAutomaton;
import org.apache.lucene.util.automaton.RegExp;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.notification.Failure;

import static org.apache.lucene.tests.analysis.BaseTokenStreamTestCase.*;

public final class AnalysisMocksOracle {
  private AnalysisMocksOracle() {}

  static final String SEED = "1DB1C0DE";
  static final List<String> ROWS = new ArrayList<>();
  static final long[] SEEDS = OracleSupport.seeds();

  /** The input corpus. */
  public static final String[] CORPUS = {
    "",
    "A bc defg hiJklmn opqrstuv wxy z ",
    "a-bc123 defg+hijklmn567opqrstuv78wxy_z ",
    "FooBarBAZ aFooBar",
    "x\uD801\uDC00y \uD834\uDD1E\uD834\uDD1E zz \uD801\uDC28",
    "abcdefghijklmnopqrstuvwxyz0123456789 short",
    "\u0130STANBUL \u03A3\u0391\u03A3 \u00C0\u00D6\u00F8\u4E00\u9FCC",
    "the quick brown a fox and it is",
    "  \t\r\n  leading whitespace",
    "small dogs and guinea pig running guinea dogs",
  };

  static CharacterRunAutomaton[] patterns() {
    return new CharacterRunAutomaton[] {
      MockTokenizer.WHITESPACE,
      MockTokenizer.KEYWORD,
      MockTokenizer.SIMPLE,
      new CharacterRunAutomaton(new RegExp("[A-Z][a-z]*").toAutomaton()),
      new CharacterRunAutomaton(new RegExp("..").toAutomaton()),
      new CharacterRunAutomaton(new RegExp(".").toAutomaton()),
    };
  }

  static void row(String scenario, long seed, String step, String value) {
    ROWS.add(scenario + "\t" + seed + "\t" + step + "\t" + value);
  }

  static String esc(String s) {
    return s.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t").replace("\r", "\\r");
  }

  static String err(Throwable t) {
    return OracleSupport.err(t);
  }

  static <T> T ctx(long seed, Callable<T> c) throws Exception {
    return RandomizedContext.current()
        .runWithPrivateRandomness(new Randomness(seed, RandomSupplier.DEFAULT), c);
  }

  static String hex(BytesRef b) {
    if (b == null) return "null";
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < b.length; i++) {
      sb.append(String.format("%02x", b.bytes[b.offset + i] & 0xff));
    }
    return sb.length() == 0 ? "~" : sb.toString();
  }

  static String tok(TokenStream ts) {
    CharTermAttribute term = ts.getAttribute(CharTermAttribute.class);
    OffsetAttribute off = ts.getAttribute(OffsetAttribute.class);
    PositionIncrementAttribute inc = ts.getAttribute(PositionIncrementAttribute.class);
    PositionLengthAttribute len = ts.getAttribute(PositionLengthAttribute.class);
    TypeAttribute type = ts.getAttribute(TypeAttribute.class);
    PayloadAttribute payload = ts.getAttribute(PayloadAttribute.class);
    FlagsAttribute flags = ts.getAttribute(FlagsAttribute.class);
    TermFrequencyAttribute freq = ts.getAttribute(TermFrequencyAttribute.class);
    KeywordAttribute kw = ts.getAttribute(KeywordAttribute.class);
    return (term == null ? "-" : OracleSupport.units(term.toString()))
        + ","
        + (off == null ? "-" : off.startOffset() + "," + off.endOffset())
        + ","
        + (inc == null ? "-" : Integer.toString(inc.getPositionIncrement()))
        + ","
        + (len == null ? "-" : Integer.toString(len.getPositionLength()))
        + ","
        + (type == null ? "-" : OracleSupport.units(type.type()))
        + ","
        + (payload == null ? "-" : hex(payload.getPayload()))
        + ","
        + (flags == null ? "-" : Integer.toString(flags.getFlags()))
        + ","
        + (freq == null ? "-" : Integer.toString(freq.getTermFrequency()))
        + ","
        + (kw == null ? "-" : Boolean.toString(kw.isKeyword()));
  }

  /** Consumes the stream, recording every token, end() and close(). */
  static String dump(TokenStream ts) {
    StringBuilder sb = new StringBuilder();
    try {
      ts.reset();
      while (ts.incrementToken()) {
        sb.append(tok(ts)).append(' ');
      }
      ts.end();
      sb.append("end:").append(tok(ts));
      ts.close();
    } catch (Throwable t) {
      sb.append('!').append(err(t));
    }
    return sb.toString();
  }

  /** The simple class name of the implementation of CharTermAttribute. */
  static String impl(TokenStream ts) {
    CharTermAttribute term = ts.getAttribute(CharTermAttribute.class);
    return term == null ? "-" : term.getClass().getSimpleName();
  }

  static Random random() {
    return RandomizedContext.current().getRandom();
  }

  interface Step {
    String run() throws Throwable;
  }

  static String outcome(Step s) {
    try {
      return "ok\t" + s.run();
    } catch (Throwable t) {
      return "err\t" + err(t);
    }
  }

  // ---------------------------------------------------------------------------
  // tok: MockTokenizer over every pattern and input
  // ---------------------------------------------------------------------------

  static void tokenizers() throws Exception {
    CharacterRunAutomaton[] p = patterns();
    for (int si = 0; si < SEEDS.length; si++) {
      long seed = SEEDS[si];
      for (int pi = 0; pi < p.length; pi++) {
        for (int c = 0; c < CORPUS.length; c++) {
          final boolean lc = (si + pi) % 2 == 1;
          final int max = c % 3 == 0 ? 5 : MockTokenizer.DEFAULT_MAX_TOKEN_LENGTH;
          final int fpi = pi;
          final int fc = c;
          String v =
              ctx(
                  seed,
                  () -> {
                    MockTokenizer t = new MockTokenizer(p[fpi], lc, max);
                    t.setReader(new StringReader(CORPUS[fc]));
                    String d = dump(t);
                    return impl(t) + "\t" + d + "\t" + random().nextLong();
                  });
          row("tok", seed, "p" + pi + ".lc" + lc + ".m" + max + ".c" + c, v);
        }
      }
    }
  }

  // ---------------------------------------------------------------------------
  // len: the token length limit over supplementary characters (UTF-16 units)
  // ---------------------------------------------------------------------------

  static void lengths() throws Exception {
    CharacterRunAutomaton[] p = {MockTokenizer.WHITESPACE, MockTokenizer.KEYWORD};
    for (int si = 0; si < 5; si++) {
      long seed = SEEDS[si];
      for (int pi = 0; pi < p.length; pi++) {
        for (int max = 1; max <= 5; max++) {
          final int fpi = pi;
          final int fmax = max;
          row(
              "len",
              seed,
              "p" + pi + ".m" + max,
              ctx(
                  seed,
                  () -> {
                    MockTokenizer t = new MockTokenizer(p[fpi], false, fmax);
                    t.setReader(new StringReader(CORPUS[4]));
                    return dump(t);
                  }));
        }
      }
    }
  }

  // ---------------------------------------------------------------------------
  // rd: MockReaderWrapper and MockCharFilter under MockTokenizer
  // ---------------------------------------------------------------------------

  static void readers() throws Exception {
    for (int si = 0; si < 5; si++) {
      long seed = SEEDS[si];
      for (int c = 0; c < CORPUS.length; c++) {
        final int fc = c;
        row(
            "rd",
            seed,
            "wrap.c" + c,
            ctx(
                seed,
                () -> {
                  Random r = new Random(seed);
                  MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
                  t.setReader(new MockReaderWrapper(r, new StringReader(CORPUS[fc])));
                  return dump(t) + "\t" + r.nextLong();
                }));
        for (int remainder : new int[] {0, 2, 7}) {
          row(
              "rd",
              seed,
              "cf" + remainder + ".c" + c,
              ctx(
                  seed,
                  () -> {
                    MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
                    t.setReader(new MockCharFilter(new StringReader(CORPUS[fc]), remainder));
                    return dump(t);
                  }));
        }
        for (int k : new int[] {0, 3, CORPUS[c].length()}) {
          row(
              "rd",
              seed,
              "evil" + k + ".c" + c,
              ctx(
                  seed,
                  () -> {
                    Random r = new Random(seed);
                    MockReaderWrapper w = new MockReaderWrapper(r, new StringReader(CORPUS[fc]));
                    w.throwExcAfterChar(k);
                    MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
                    t.setEnableChecks(false);
                    t.setReader(w);
                    return dump(t);
                  }));
        }
      }
    }
  }

  // ---------------------------------------------------------------------------
  // an: MockAnalyzer configurations
  // ---------------------------------------------------------------------------

  static Analyzer analyzer(int config, Random random) {
    switch (config) {
      case 0:
        return new MockAnalyzer(random);
      case 1:
        return new MockAnalyzer(
            random, MockTokenizer.SIMPLE, true, MockTokenFilter.ENGLISH_STOPSET);
      case 2:
        return new MockAnalyzer(
            random,
            MockTokenizer.WHITESPACE,
            true,
            new CharacterRunAutomaton(new RegExp(".{5,}").toAutomaton()));
      case 3:
        return new MockAnalyzer(random, MockTokenizer.KEYWORD, false);
      default:
        return new MockAnalyzer(random, term -> term.length() + 1);
    }
  }

  static void analyzers() throws Exception {
    int[] inputs = {1, 2, 4, 7};
    for (long seed : SEEDS) {
      for (int config = 0; config < 5; config++) {
        final int fconfig = config;
        ctx(
            seed,
            () -> {
              Analyzer a = analyzer(fconfig, random());
              for (int f = 0; f < 3; f++) {
                for (int c : inputs) {
                  TokenStream ts = a.tokenStream("f" + f, CORPUS[c]);
                  row("an", seed, fconfig + ".f" + f + ".c" + c, impl(ts) + "\t" + dump(ts));
                }
              }
              row("an", seed, fconfig + ".norm", hex(a.normalize("f0", CORPUS[1])));
              row("an", seed, fconfig + ".next", Long.toString(random().nextLong()));
              return null;
            });
      }
    }
  }

  // ---------------------------------------------------------------------------
  // fl: the filters over a whitespace MockTokenizer
  // ---------------------------------------------------------------------------

  static TokenStream ws(String text) throws IOException {
    MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
    t.setReader(new StringReader(text));
    return t;
  }

  interface Filter {
    TokenStream wrap(Random r, TokenStream in) throws IOException;
  }

  static void filters() throws Exception {
    String[] names = {
      "fixed0", "fixed1", "fixed5", "fixed11", "var", "hole", "graph", "look", "syn", "low", "stop",
      "cranky", "valid", "simplepayload"
    };
    Filter[] filters = {
      (r, in) -> new MockFixedLengthPayloadFilter(r, in, 0),
      (r, in) -> new MockFixedLengthPayloadFilter(r, in, 1),
      (r, in) -> new MockFixedLengthPayloadFilter(r, in, 5),
      (r, in) -> new MockFixedLengthPayloadFilter(r, in, 11),
      (r, in) -> new MockVariableLengthPayloadFilter(r, in),
      (r, in) -> new MockHoleInjectingTokenFilter(r, in),
      (r, in) -> new MockGraphTokenFilter(r, in),
      (r, in) -> new MockRandomLookaheadTokenFilter(r, in),
      (r, in) -> new MockSynonymFilter(in),
      (r, in) -> new MockLowerCaseFilter(in),
      (r, in) -> new MockTokenFilter(in, MockTokenFilter.ENGLISH_STOPSET),
      (r, in) -> new CrankyTokenFilter(in, r),
      (r, in) -> new ValidatingTokenFilter(new MockGraphTokenFilter(r, in), "v"),
      (r, in) -> new SimplePayloadFilter(in),
    };
    int[] inputs = {1, 3, 4, 5, 7, 9};
    for (long seed : SEEDS) {
      for (int fi = 0; fi < filters.length; fi++) {
        for (int c : inputs) {
          final int ffi = fi;
          row(
              "fl",
              seed,
              names[fi] + ".c" + c,
              ctx(
                  seed,
                  () -> {
                    Random r = new Random(seed);
                    TokenStream ts = filters[ffi].wrap(r, ws(CORPUS[c]));
                    // twice: the deterministic filters reproduce their tokens after reset
                    String first = dump(ts);
                    if (ts instanceof MockGraphTokenFilter || ts instanceof ValidatingTokenFilter) {
                      ((Tokenizer) unwrapTokenizer(ts)).setReader(new StringReader(CORPUS[c]));
                      first += "\t" + dump(ts);
                    } else {
                      first += "\t-";
                    }
                    return first + "\t" + r.nextLong() + "\t" + random().nextLong();
                  }));
        }
      }
    }
  }

  static TokenStream unwrapTokenizer(TokenStream ts) {
    while (ts instanceof TokenFilter) {
      ts = ((TokenFilter) ts).unwrap();
    }
    return ts;
  }

  // ---------------------------------------------------------------------------
  // fx: fixed streams, the payload and bytes analyzers, TokenStreamToDot
  // ---------------------------------------------------------------------------

  static void fixed() throws Exception {
    ctx(
        SEEDS[0],
        () -> {
          Token t1 = new Token("foo", 0, 3);
          Token t2 = new Token("bar", 0, 4, 7);
          t2.setFlags(3);
          t2.setPayload(new BytesRef(new byte[] {1, 2}));
          Token t3 = new Token("baz", 2, 8, 11, 2);
          t3.setType("t");
          row("fx", 0, "canned", impl(new CannedTokenStream(t1, t2, t3)) + "\t"
              + dump(new CannedTokenStream(t1, t2, t3)));
          row("fx", 0, "canned.final", dump(new CannedTokenStream(3, 20, t1, t3)));
          CannedBinaryTokenStream.BinaryToken b1 =
              new CannedBinaryTokenStream.BinaryToken(new BytesRef(new byte[] {(byte) 0xff, 0}));
          CannedBinaryTokenStream.BinaryToken b2 =
              new CannedBinaryTokenStream.BinaryToken(new BytesRef("x"), 0, 3);
          b2.startOffset = 4;
          b2.endOffset = 9;
          CannedBinaryTokenStream bin = new CannedBinaryTokenStream(b1, b2);
          TermToBytesRefAttribute bytes = bin.getAttribute(TermToBytesRefAttribute.class);
          StringBuilder sb = new StringBuilder();
          bin.reset();
          while (bin.incrementToken()) {
            OffsetAttribute o = bin.getAttribute(OffsetAttribute.class);
            sb.append(hex(bytes.getBytesRef()))
                .append(',')
                .append(o.startOffset())
                .append(',')
                .append(o.endOffset())
                .append(',')
                .append(bin.getAttribute(PositionIncrementAttribute.class).getPositionIncrement())
                .append(',')
                .append(bin.getAttribute(PositionLengthAttribute.class).getPositionLength())
                .append(' ');
          }
          row("fx", 0, "binary", sb.toString());
          for (int c : new int[] {1, 7}) {
            row("fx", 0, "payloadanalyzer.c" + c, dump(new MockPayloadAnalyzer().tokenStream("p", CORPUS[c])));
            TokenStream ts = new MockBytesAnalyzer().tokenStream("b", CORPUS[c == 1 ? 4 : c]);
            TermToBytesRefAttribute ba = ts.getAttribute(TermToBytesRefAttribute.class);
            StringBuilder s = new StringBuilder(impl(ts));
            ts.reset();
            while (ts.incrementToken()) {
              s.append(' ').append(hex(ba.getBytesRef()));
            }
            ts.end();
            ts.close();
            row("fx", 0, "bytes.c" + c, s.toString());
          }
          row("fx", 0, "synonymanalyzer", dump(new MockSynonymAnalyzer().tokenStream("s", CORPUS[9])));
          return null;
        });
    for (int si = 0; si < 5; si++) {
      long seed = SEEDS[si];
      for (int c : new int[] {1, 4, 9}) {
        row(
            "fx",
            seed,
            "dot.graph.c" + c,
            ctx(
                seed,
                () -> {
                  StringWriter sw = new StringWriter();
                  TokenStream ts = new MockGraphTokenFilter(new Random(seed), ws(CORPUS[c]));
                  new TokenStreamToDot(CORPUS[c], ts, new PrintWriter(sw)).toDot();
                  return esc(sw.toString());
                }));
        row(
            "fx",
            seed,
            "dot.nulltext.c" + c,
            ctx(
                seed,
                () -> {
                  StringWriter sw = new StringWriter();
                  TokenStream ts = new MockSynonymFilter(ws(CORPUS[c]));
                  new TokenStreamToDot(null, ts, new PrintWriter(sw)).toDot();
                  return esc(sw.toString());
                }));
      }
    }
  }

  // ---------------------------------------------------------------------------
  // mis: workflow violations and the assertion messages of BaseTokenStreamTestCase
  // ---------------------------------------------------------------------------

  /** A stream that never calls clearAttributes(). */
  static final class NoClearStream extends TokenStream {
    final CharTermAttribute term = addAttribute(CharTermAttribute.class);
    final OffsetAttribute off = addAttribute(OffsetAttribute.class);
    int upto;

    @Override
    public boolean incrementToken() {
      if (upto < 2) {
        term.setEmpty().append(upto == 0 ? "a" : "b");
        off.setOffset(upto * 2, upto * 2 + 1);
        upto++;
        return true;
      }
      return false;
    }

    @Override
    public void reset() throws IOException {
      super.reset();
      upto = 0;
    }
  }

  static void misuse() throws Exception {
    ctx(
        SEEDS[5],
        () -> {
          for (boolean checks : new boolean[] {true, false}) {
            String k = checks ? "checks" : "nochecks";
            row("mis", 0, k + ".noreset", outcome(() -> {
              MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
              t.setEnableChecks(checks);
              t.setReader(new StringReader("a b"));
              return Boolean.toString(t.incrementToken());
            }));
            row("mis", 0, k + ".doublereset", outcome(() -> {
              MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
              t.setEnableChecks(checks);
              t.setReader(new StringReader("a b"));
              t.reset();
              t.reset();
              return "-";
            }));
            row("mis", 0, k + ".closenoend", outcome(() -> {
              MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
              t.setEnableChecks(checks);
              t.setReader(new StringReader("a b"));
              t.reset();
              while (t.incrementToken()) {}
              t.close();
              return "-";
            }));
            row("mis", 0, k + ".earlyend", outcome(() -> {
              MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
              t.setEnableChecks(checks);
              t.setReader(new StringReader("a b"));
              t.reset();
              t.incrementToken();
              t.end();
              return tok(t);
            }));
            row("mis", 0, k + ".setreadertwice", outcome(() -> {
              MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
              t.setEnableChecks(checks);
              t.setReader(new StringReader("a b"));
              t.setReader(new StringReader("c"));
              return "-";
            }));
            row("mis", 0, k + ".noreader", outcome(() -> {
              MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
              t.setEnableChecks(checks);
              t.reset();
              return Boolean.toString(t.incrementToken());
            }));
            row("mis", 0, k + ".closenoreset", outcome(() -> {
              MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
              t.setEnableChecks(checks);
              t.setReader(new StringReader("a b"));
              t.close();
              return "-";
            }));
            row("mis", 0, k + ".nocloseinputs", outcome(() -> {
              MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
              t.setEnableChecks(checks);
              t.setReader(new StringReader("a b"));
              t.reset();
              while (t.incrementToken()) {}
              t.end();
              t.setReader(new StringReader("c"));
              return "-";
            }));
          }
          row("mis", 0, "an.twice", outcome(() -> {
            Analyzer a = new MockAnalyzer(random());
            TokenStream ts = a.tokenStream("f", "a b");
            ts.reset();
            while (ts.incrementToken()) {}
            ts.end();
            a.tokenStream("f", "c");
            return "-";
          }));
          row("mis", 0, "charfilter.remainder", outcome(() -> {
            new MockCharFilter(new StringReader("x"), 10);
            return "-";
          }));
          row("mis", 0, "fixed.length", outcome(() -> {
            new MockFixedLengthPayloadFilter(new Random(1), ws("a"), -1);
            return "-";
          }));
          row("mis", 0, "graph.noreset", outcome(() -> {
            TokenStream ts = new MockGraphTokenFilter(new Random(1), ws("a"));
            return Boolean.toString(ts.incrementToken());
          }));
          Analyzer a = new MockAnalyzer(random());
          row("mis", 0, "btstc.ok", outcome(() -> {
            assertAnalyzesTo(a, "foo bar", new String[] {"foo", "bar"}, new int[] {0, 4}, new int[] {3, 7});
            checkResetException(a, "abc");
            return "-";
          }));
          row("mis", 0, "btstc.term", outcome(() -> {
            assertAnalyzesTo(a, "foo bar", new String[] {"foo", "baz"});
            return "-";
          }));
          row("mis", 0, "btstc.term.long", outcome(() -> {
            assertAnalyzesTo(a, "the quick brown fox jumps over the lazy dog xxxxxxxxxxxxxxxxxxxxxxxxxx",
                new String[] {"the quick brown fox jumps over the lazy dog xxxxxxxxxxxxxxxxxxxxxxxxxx"});
            return "-";
          }));
          Analyzer kw = new MockAnalyzer(random(), MockTokenizer.KEYWORD, false);
          row("mis", 0, "btstc.term.context", outcome(() -> {
            assertAnalyzesTo(kw, "abcdefghijklmnopqrstuvwxyz0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ",
                new String[] {"abcdefghijklmnopqrstuvwxyz0123456789-ABCDEFGHIJKLMNOPQRSTUVWXYZ"});
            return "-";
          }));
          row("mis", 0, "btstc.extra", outcome(() -> {
            assertAnalyzesTo(a, "foo bar", new String[] {"foo"});
            return "-";
          }));
          row("mis", 0, "btstc.missing", outcome(() -> {
            assertAnalyzesTo(a, "foo bar", new String[] {"foo", "bar", "x"});
            return "-";
          }));
          row("mis", 0, "btstc.start", outcome(() -> {
            assertAnalyzesTo(a, "foo bar", new String[] {"foo", "bar"}, new int[] {1, 4}, new int[] {3, 7});
            return "-";
          }));
          row("mis", 0, "btstc.end", outcome(() -> {
            assertAnalyzesTo(a, "foo bar", new String[] {"foo", "bar"}, new int[] {0, 4}, new int[] {3, 8});
            return "-";
          }));
          row("mis", 0, "btstc.posinc", outcome(() -> {
            assertAnalyzesTo(a, "foo bar", new String[] {"foo", "bar"}, new int[] {1, 2});
            return "-";
          }));
          row("mis", 0, "btstc.type", outcome(() -> {
            assertAnalyzesTo(a, "foo bar", new String[] {"foo", "bar"}, new String[] {"word", "wordy"});
            return "-";
          }));
          row("mis", 0, "btstc.finaloffset", outcome(() -> {
            assertTokenStreamContents(a.tokenStream("f", "foo"), new String[] {"foo"}, null, null, null, null, null, 99);
            return "-";
          }));
          row("mis", 0, "btstc.finalposinc", outcome(() -> {
            assertTokenStreamContents(a.tokenStream("f", "foo"), new String[] {"foo"}, null, null, null, null, null, 3, 7, null, true, null);
            return "-";
          }));
          row("mis", 0, "btstc.noclear", outcome(() -> {
            assertTokenStreamContents(new NoClearStream(), new String[] {"a", "b"});
            return "-";
          }));
          row("mis", 0, "btstc.resetcheck", outcome(() -> {
            Analyzer canned = new Analyzer() {
              @Override
              protected TokenStreamComponents createComponents(String fieldName) {
                Tokenizer t = new Tokenizer() {
                  final CharTermAttribute term = addAttribute(CharTermAttribute.class);
                  boolean done;
                  @Override
                  public boolean incrementToken() {
                    clearAttributes();
                    if (done) return false;
                    done = true;
                    term.append("x");
                    return true;
                  }
                  @Override
                  public void reset() throws IOException {
                    super.reset();
                    done = false;
                  }
                };
                return new TokenStreamComponents(t);
              }
            };
            checkResetException(canned, "abc");
            return "-";
          }));
          row("mis", 0, "btstc.tostring", outcome(() -> esc(BaseTokenStreamTestCase.toString(new MockSynonymAnalyzer(), CORPUS[9]))));
          row("mis", 0, "btstc.escape", outcome(() -> BaseTokenStreamTestCase.escape("a\"b\\c\nd\re\u00e9" + CORPUS[4])));
          row("mis", 0, "btstc.number", outcome(() -> {
            TokenStream ts = a.tokenStream("f", "a b c");
            assertStreamHasNumberOfTokens(ts, 2);
            return "-";
          }));
          return null;
        });
  }

  // ---------------------------------------------------------------------------
  // crd: checkRandomData draw parity
  // ---------------------------------------------------------------------------

  static final ThreadLocal<List<String>> SEEN = new ThreadLocal<>();

  static final class Recording extends TokenFilter {
    final CharTermAttribute term = addAttribute(CharTermAttribute.class);

    Recording(TokenStream in) {
      super(in);
    }

    @Override
    public boolean incrementToken() throws IOException {
      if (input.incrementToken()) {
        List<String> seen = SEEN.get();
        if (seen != null) seen.add(term.toString());
        return true;
      }
      return false;
    }
  }

  static void checkRandomDataParity() throws Exception {
    for (long seed : SEEDS) {
      for (boolean simple : new boolean[] {false, true}) {
        String v =
            ctx(
                seed,
                () -> {
                  Analyzer a =
                      new Analyzer() {
                        @Override
                        protected TokenStreamComponents createComponents(String fieldName) {
                          Tokenizer t = new MockTokenizer(MockTokenizer.KEYWORD, false, Integer.MAX_VALUE);
                          return new TokenStreamComponents(t, new Recording(t));
                        }
                      };
                  List<String> seen = new ArrayList<>();
                  SEEN.set(seen);
                  Random r = new Random(seed);
                  String o;
                  try {
                    checkRandomData(r, a, 4, 20, simple);
                    o = "ok";
                  } catch (Throwable t) {
                    o = "err\t" + err(t);
                  } finally {
                    SEEN.remove();
                  }
                  StringBuilder sb = new StringBuilder();
                  for (String s : seen) {
                    sb.append(OracleSupport.units(s)).append(' ');
                  }
                  return o + "\t" + seen.size() + "\t" + sb + "\t" + r.nextLong() + "\t" + random().nextLong();
                });
        row("crd", seed, "simple" + simple, v);
      }
      // the random analysis strings themselves
      Random r = new Random(seed);
      StringBuilder sb = new StringBuilder();
      for (int i = 0; i < 6; i++) {
        sb.append(OracleSupport.units(TestUtil.randomAnalysisString(r, 20, i % 2 == 1))).append(' ');
      }
      row("crd", seed, "strings", sb + "\t" + r.nextLong());
    }
  }

  // ---------------------------------------------------------------------------
  // core: the core behaviours the mocks depend on
  // ---------------------------------------------------------------------------

  static void core() throws Exception {
    ctx(
        SEEDS[6],
        () -> {
          StandardTokenizer st = new StandardTokenizer();
          st.setReader(new StringReader("hello world"));
          row("core", 0, "standard.end", dump(st));
          MockTokenizer t = new MockTokenizer(MockTokenizer.WHITESPACE, false);
          TokenStream outer = new MockLowerCaseFilter(t);
          FlagsAttribute flags = outer.addAttribute(FlagsAttribute.class);
          flags.setFlags(5);
          t.clearAttributes();
          row("core", 0, "shared.clear", Integer.toString(flags.getFlags()));
          flags.setFlags(6);
          t.setReader(new StringReader("A"));
          outer.reset();
          outer.incrementToken();
          String during = tok(outer);
          flags.setFlags(9);
          outer.incrementToken();
          outer.end();
          row("core", 0, "shared.end", during + " " + tok(outer));
          outer.close();
          return null;
        });
  }

  public static void scenarios() throws Exception {
    tokenizers();
    lengths();
    readers();
    analyzers();
    filters();
    fixed();
    misuse();
    checkRandomDataParity();
    core();
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    ROWS.clear();
    System.setProperty("tests.seed", SEED);
    Result r = new JUnitCore().run(TestAnalysisMocksScenarios.class);
    System.clearProperty("tests.seed");
    try (PrintWriter w = new PrintWriter(new File(out, "analysis_mocks.tsv"), "UTF-8")) {
      w.println("# scenario\tseed\tstep\tvalue...");
      for (Failure f : r.getFailures()) {
        w.println("# failure " + f.toString().replace("\n", "\\n"));
      }
      for (String row : ROWS) {
        w.println(row);
      }
    }
  }
}
