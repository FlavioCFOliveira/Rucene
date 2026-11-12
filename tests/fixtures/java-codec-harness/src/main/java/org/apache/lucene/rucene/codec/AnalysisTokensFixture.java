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

package org.apache.lucene.rucene.codec;

import java.io.IOException;
import java.io.StringReader;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.CharArraySet;
import org.apache.lucene.analysis.StopFilter;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.Tokenizer;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.analysis.standard.StandardTokenizer;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.analysis.tokenattributes.OffsetAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionLengthAttribute;
import org.apache.lucene.analysis.tokenattributes.TypeAttribute;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.BoostQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.util.QueryBuilder;

/**
 * Oracle of {@code tests/portability/analysis_tokens.rs} (rmp task #410).
 *
 * <p>Prints, for every input of {@link #corpus()} and every token-stream configuration of {@link
 * #streamConfigs()}, each token's text (UTF-16 code units, escaped), type, position increment,
 * position length and offsets, then the attributes after {@code end()}. It then prints, for every
 * analyzer, text, {@link QueryBuilder} setting and creation method, the class tree and the {@code
 * toString()} of the built query. The Rust test builds the same dump with Rucene and compares it
 * line by line. The corpus text is written with {@code [U+XXXX]} escapes, decoded by {@link
 * #unescape(String)}, so both sides hold the same source text. Mode: {@code dump}.
 */
public final class AnalysisTokensFixture {

  private AnalysisTokensFixture() {}

  // ---------------------------------------------------------------------------------------------
  // Corpus
  // ---------------------------------------------------------------------------------------------

  /** Replaces every {@code [U+XXXX]} with the code point it names. */
  static String unescape(String s) {
    StringBuilder out = new StringBuilder();
    int i = 0;
    while (i < s.length()) {
      if (s.startsWith("[U+", i)) {
        int close = s.indexOf(']', i);
        out.appendCodePoint(Integer.parseInt(s.substring(i + 3, close), 16));
        i = close + 1;
      } else {
        out.append(s.charAt(i));
        i++;
      }
    }
    return out.toString();
  }

  /** Tokens longer than 255 and than 5 characters, around short ones. */
  static String longTokens() {
    return "a".repeat(300)
        + " bcdefghijkl 12345678901234 short "
        + "[U+4E00][U+4E01] "
        + "x".repeat(255)
        + " "
        + "y".repeat(256)
        + " end";
  }

  /** More than 4096 UTF-16 units, so the scanner refills its buffer mid-token. */
  static String bufferRefill() {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < 900; i++) {
      if (i % 97 == 0) {
        sb.append("[U+20000]");
      }
      sb.append('w').append(i);
      sb.append(i % 13 == 0 ? ", " : " ");
      if (i == 700) {
        sb.append("z".repeat(300)).append(' ');
      }
    }
    return sb.toString();
  }

  /** The word-break corpus: name and escaped text, in order. */
  static List<String[]> corpus() {
    List<String[]> c = new ArrayList<>();
    c.add(
        new String[] {
          "latin",
          "The quick-brown fox's 3rd jump, isn't it? O'Neil's e-mail: john.doe@example.com;"
              + " visit https://lucene.apache.org/core/10_5_0/ now! U.S.A. AT&T C++ C# wi-fi"
        });
    c.add(
        new String[] {
          "numbers",
          "1,234.56 3.14159 1-800-555-0199 2026-11-11 10:30 1e10 0x1F 12,34 $100 50% v2.0.1 3rd 4th"
        });
    c.add(new String[] {"stopwords", "The of a the quick and to the brown it is a fox the a"});
    c.add(
        new String[] {
          "accents",
          "caf[U+00E9] na[U+00EF]ve [U+00C5]ngstr[U+00F6]m STRASSE stra[U+00DF]e [U+0130]stanbul"
              + " [U+0391][U+0392][U+0393] [U+0414][U+0416]"
        });
    c.add(
        new String[] {
          "cjk",
          "[U+4E2D][U+6587][U+5206][U+8BCD] Lucene[U+662F][U+4E00][U+4E2A][U+5E93] [U+3007][U+3005]"
        });
    c.add(
        new String[] {
          "hiragana",
          "[U+3072][U+3089][U+304C][U+306A][U+306E][U+30C6][U+30B9][U+30C8]"
              + " [U+3053][U+3093][U+306B][U+3061][U+306F]"
        });
    c.add(
        new String[] {
          "katakana",
          "[U+30AB][U+30BF][U+30AB][U+30CA] [U+30EB][U+30FC][U+30BB][U+30F3]"
              + " [U+FF76][U+FF80][U+FF76][U+FF85] [U+30A2][U+30C3][U+30D7][U+30EB]123"
        });
    c.add(
        new String[] {
          "hangul",
          "[U+D55C][U+AD6D][U+C5B4] [U+D14C][U+C2A4][U+D2B8][U+C785][U+B2C8][U+B2E4]"
              + " [U+1100][U+1161][U+11A8]"
        });
    c.add(
        new String[] {
          "southeast_asian",
          "[U+0E20][U+0E32][U+0E29][U+0E32][U+0E44][U+0E17][U+0E22]"
              + " [U+0E9E][U+0EB2][U+0EAA][U+0EB2][U+0EA5][U+0EB2][U+0EA7]"
              + " [U+1019][U+103C][U+1014][U+103A][U+1019][U+102C] [U+1780][U+1781]"
        });
    c.add(
        new String[] {
          "emoji",
          "I [U+2764][U+FE0F] Lucene [U+1F600][U+1F600] family"
              + " [U+1F468][U+200D][U+1F469][U+200D][U+1F467] flag"
              + " [U+1F1F5][U+1F1F9][U+1F1FA][U+1F1F8]"
              + " skin [U+1F44D][U+1F3FD] keycap 1[U+FE0F][U+20E3] #[U+FE0F][U+20E3] [U+00A9]"
              + " [U+261D][U+1F3FB] [U+1F3F4][U+E0067][U+E0062][U+E0065][U+E006E][U+E0067][U+E007F]"
              + " a[U+1F600]b"
        });
    c.add(
        new String[] {
          "supplementary",
          "[U+20000][U+20001] [U+1D400][U+1D401]bold [U+10400][U+10401]deseret [U+1F1E6]"
              + " x[U+10428]y"
        });
    c.add(new String[] {"long_tokens", longTokens()});
    c.add(new String[] {"buffer_refill", bufferRefill()});
    return c;
  }

  /** The stop words of the analyzer with a custom stop set (already lower case). */
  static CharArraySet stopSet(boolean ignoreCase) {
    return new CharArraySet(
        List.of("the", "a", "an", "and", "of", "to", "is", "it", "isn't"), ignoreCase);
  }

  // ---------------------------------------------------------------------------------------------
  // Token dumps
  // ---------------------------------------------------------------------------------------------

  /** Printable ASCII except the backslash as is, every other UTF-16 unit as {@code \\uXXXX}. */
  static String escape(String s) {
    StringBuilder out = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char ch = s.charAt(i);
      if (ch >= 0x20 && ch <= 0x7e && ch != '\\') {
        out.append(ch);
      } else {
        out.append(String.format("\\u%04X", (int) ch));
      }
    }
    return out.toString();
  }

  /** Consumes {@code ts} through the full workflow and prints every token and the end state. */
  static void consume(TokenStream ts) throws IOException {
    CharTermAttribute term = ts.addAttribute(CharTermAttribute.class);
    TypeAttribute type = ts.addAttribute(TypeAttribute.class);
    PositionIncrementAttribute posInc = ts.addAttribute(PositionIncrementAttribute.class);
    PositionLengthAttribute posLen = ts.addAttribute(PositionLengthAttribute.class);
    OffsetAttribute offset = ts.addAttribute(OffsetAttribute.class);
    ts.reset();
    while (ts.incrementToken()) {
      System.out.println(
          "  tok term="
              + escape(term.toString())
              + " type="
              + type.type()
              + " posInc="
              + posInc.getPositionIncrement()
              + " posLen="
              + posLen.getPositionLength()
              + " offset="
              + offset.startOffset()
              + "-"
              + offset.endOffset());
    }
    ts.end();
    System.out.println(
        "  end posInc="
            + posInc.getPositionIncrement()
            + " posLen="
            + posLen.getPositionLength()
            + " offset="
            + offset.startOffset()
            + "-"
            + offset.endOffset());
    ts.close();
  }

  /** A token-stream configuration: consumes one text with its own reused stream. */
  interface StreamConfig {
    void run(String text) throws IOException;
  }

  /** The token-stream configurations, in order; each instance is reused across the corpus. */
  static List<Object[]> streamConfigs() {
    List<Object[]> configs = new ArrayList<>();
    StandardTokenizer plain = new StandardTokenizer();
    configs.add(
        new Object[] {
          "tokenizer",
          (StreamConfig)
              text -> {
                plain.setReader(new StringReader(text));
                consume(plain);
              }
        });
    StandardTokenizer max5 = new StandardTokenizer();
    max5.setMaxTokenLength(5);
    configs.add(
        new Object[] {
          "tokenizer-max5",
          (StreamConfig)
              text -> {
                max5.setReader(new StringReader(text));
                consume(max5);
              }
        });
    StandardTokenizer max255 = new StandardTokenizer();
    max255.setMaxTokenLength(255);
    configs.add(
        new Object[] {
          "tokenizer-max255",
          (StreamConfig)
              text -> {
                max255.setReader(new StringReader(text));
                consume(max255);
              }
        });
    StandardAnalyzer analyzer = new StandardAnalyzer();
    configs.add(
        new Object[] {
          "analyzer", (StreamConfig) text -> consume(analyzer.tokenStream("body", text))
        });
    StandardAnalyzer analyzerMax5 = new StandardAnalyzer();
    analyzerMax5.setMaxTokenLength(5);
    configs.add(
        new Object[] {
          "analyzer-max5", (StreamConfig) text -> consume(analyzerMax5.tokenStream("body", text))
        });
    StandardAnalyzer analyzerStop = new StandardAnalyzer(stopSet(false));
    configs.add(
        new Object[] {
          "analyzer-stop", (StreamConfig) text -> consume(analyzerStop.tokenStream("body", text))
        });
    StandardTokenizer stopSource = new StandardTokenizer();
    StopFilter stopFilter = new StopFilter(stopSource, stopSet(true));
    configs.add(
        new Object[] {
          "stopfilter",
          (StreamConfig)
              text -> {
                stopSource.setReader(new StringReader(text));
                consume(stopFilter);
              }
        });
    return configs;
  }

  // ---------------------------------------------------------------------------------------------
  // QueryBuilder
  // ---------------------------------------------------------------------------------------------

  /** One token of a {@link GraphTokenizer} input. */
  static final class GraphToken {
    final String text;
    final int posInc;
    final int posLen;
    final int start;

    GraphToken(String text, int posInc, int posLen, int start) {
      this.text = text;
      this.posInc = posInc;
      this.posLen = posLen;
      this.start = start;
    }
  }

  /**
   * Emits the tokens its input spells out as space-separated {@code text:posInc:posLen} items, so
   * a query text can describe a token graph: the core stand-in for the test framework's {@code
   * CannedTokenStream}. Offsets are the item's start and that plus the token text's length.
   */
  static final class GraphTokenizer extends Tokenizer {
    private final CharTermAttribute termAtt = addAttribute(CharTermAttribute.class);
    private final PositionIncrementAttribute posIncAtt =
        addAttribute(PositionIncrementAttribute.class);
    private final PositionLengthAttribute posLenAtt = addAttribute(PositionLengthAttribute.class);
    private final OffsetAttribute offsetAtt = addAttribute(OffsetAttribute.class);
    private final List<GraphToken> tokens = new ArrayList<>();
    private int upto;
    private int length;

    @Override
    public void reset() throws IOException {
      super.reset();
      StringBuilder sb = new StringBuilder();
      char[] buffer = new char[64];
      int n;
      while ((n = input.read(buffer)) != -1) {
        sb.append(buffer, 0, n);
      }
      String s = sb.toString();
      length = s.length();
      tokens.clear();
      upto = 0;
      int start = 0;
      while (start < s.length()) {
        int end = s.indexOf(' ', start);
        if (end < 0) {
          end = s.length();
        }
        if (end > start) {
          String[] parts = s.substring(start, end).split(":");
          tokens.add(
              new GraphToken(
                  parts[0], Integer.parseInt(parts[1]), Integer.parseInt(parts[2]), start));
        }
        start = end + 1;
      }
    }

    @Override
    public boolean incrementToken() {
      if (upto == tokens.size()) {
        return false;
      }
      clearAttributes();
      GraphToken t = tokens.get(upto++);
      termAtt.setEmpty().append(t.text);
      posIncAtt.setPositionIncrement(t.posInc);
      posLenAtt.setPositionLength(t.posLen);
      offsetAtt.setOffset(correctOffset(t.start), correctOffset(t.start + t.text.length()));
      return true;
    }

    @Override
    public void end() throws IOException {
      super.end();
      offsetAtt.setOffset(correctOffset(length), correctOffset(length));
    }
  }

  /** An analyzer over a {@link GraphTokenizer}. */
  static final class GraphAnalyzer extends Analyzer {
    @Override
    protected TokenStreamComponents createComponents(String fieldName) {
      return new TokenStreamComponents(new GraphTokenizer());
    }
  }

  /** Texts given to the two StandardAnalyzer-based builders. */
  static final String[] STANDARD_TEXTS = {
    "quick brown fox",
    "the quick brown fox of the year",
    "Lucene",
    "wi-fi network",
    "[U+4E2D][U+6587][U+5206][U+8BCD]",
    "it is the",
    ""
  };

  /** Texts given to the graph builder: {@code text:posInc:posLen} items. */
  static final String[] GRAPH_TEXTS = {
    "wifi:1:2 wi:0:1 fi:1:1 network:1:1",
    "ny:1:2 new:0:1 york:1:1 city:1:1",
    "fast:1:1 quick:0:1 car:1:1",
    "a:1:1 b:2:1 c:1:1",
    "usa:1:4 united:0:1 states:1:1 of:1:1 america:1:1 rocks:1:1",
    "solo:1:1"
  };

  static final String[] SETTINGS = {"default", "nograph", "autophrase", "noposinc"};

  static final String[] METHODS = {"bool-should", "bool-must", "phrase-0", "phrase-2", "msm-0.5"};

  static QueryBuilder builder(Analyzer analyzer, String setting) {
    QueryBuilder b = new QueryBuilder(analyzer);
    switch (setting) {
      case "default" -> {}
      case "nograph" -> b.setEnableGraphQueries(false);
      case "autophrase" -> b.setAutoGenerateMultiTermSynonymsPhraseQuery(true);
      case "noposinc" -> b.setEnablePositionIncrements(false);
      default -> throw new IllegalArgumentException(setting);
    }
    return b;
  }

  static Query create(QueryBuilder b, String method, String text) {
    return switch (method) {
      case "bool-should" -> b.createBooleanQuery("body", text);
      case "bool-must" -> b.createBooleanQuery("body", text, BooleanClause.Occur.MUST);
      case "phrase-0" -> b.createPhraseQuery("body", text);
      case "phrase-2" -> b.createPhraseQuery("body", text, 2);
      case "msm-0.5" -> b.createMinShouldMatchQuery("body", text, 0.5f);
      default -> throw new IllegalArgumentException(method);
    };
  }

  /** The query's class, with the clause classes of boolean and boost queries. */
  static String tree(Query q) {
    if (q instanceof BooleanQuery bq) {
      StringBuilder sb = new StringBuilder("BooleanQuery(msm=");
      sb.append(bq.getMinimumNumberShouldMatch());
      for (BooleanClause c : bq.clauses()) {
        sb.append(' ').append(c.occur().name()).append(':').append(tree(c.query()));
      }
      return sb.append(')').toString();
    }
    if (q instanceof BoostQuery b) {
      return "BoostQuery(" + tree(b.getQuery()) + ")";
    }
    return q.getClass().getSimpleName();
  }

  static void queryBuilders() {
    Object[][] analyzers = {
      {"standard", new StandardAnalyzer(), STANDARD_TEXTS},
      {"standard-stop", new StandardAnalyzer(stopSet(false)), STANDARD_TEXTS},
      {"graph", new GraphAnalyzer(), GRAPH_TEXTS}
    };
    for (Object[] a : analyzers) {
      for (String rawText : (String[]) a[2]) {
        String text = unescape(rawText);
        for (String setting : SETTINGS) {
          QueryBuilder b = builder((Analyzer) a[1], setting);
          for (String method : METHODS) {
            System.out.println(
                "qb analyzer="
                    + a[0]
                    + " setting="
                    + setting
                    + " method="
                    + method
                    + " text="
                    + escape(text));
            Query q = create(b, method, text);
            if (q == null) {
              System.out.println("  query null");
            } else {
              System.out.println("  query class=" + tree(q) + " string=" + escape(q.toString()));
            }
          }
        }
      }
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Entry point
  // ---------------------------------------------------------------------------------------------

  static void dump() throws IOException {
    List<String[]> corpus = corpus();
    for (Object[] config : streamConfigs()) {
      System.out.println("config " + config[0]);
      for (String[] input : corpus) {
        String text = unescape(input[1]);
        System.out.println("input " + input[0] + " length=" + text.length());
        ((StreamConfig) config[1]).run(text);
      }
    }
    queryBuilders();
  }

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "dump" -> dump();
      default -> throw new IllegalArgumentException(args[0]);
    }
  }
}
