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

import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.Random;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.highlight.Highlighter;
import org.apache.lucene.search.highlight.QueryScorer;
import org.apache.lucene.search.highlight.SimpleFragmenter;

/**
 * The Java side of the sprint-17 oracle harness for {@code lucene/highlighter} (rmp #628,
 * {@code tests/sprint17_oracle/mod.rs}): {@code main(seed, scenario, input, args...)} prints the
 * dump of the scenario, which Rucene's test computes from the same seed.
 */
public class HighlighterOracle {
  /** The seeded generators of the harness (`tests/sprint17_oracle/mod.rs`, `corpus`). */
  static final class Corpus {
    static final String[] VOCABULARY = {
      "the", "of", "and", "to", "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
      "hotel", "india", "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec",
      "romeo", "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu"
    };

    static String word(Random r) {
      return VOCABULARY[r.nextInt(VOCABULARY.length)];
    }

    static String text(Random r, int n) {
      StringBuilder sb = new StringBuilder();
      for (int i = 0; i < n; i++) {
        if (i > 0) sb.append(' ');
        sb.append(word(r));
      }
      return sb.toString();
    }
  }

  /** Printable ASCII except the backslash as is, every other UTF-16 unit as a u-escape. */
  static String escape(String s) {
    if (s == null) return "null";
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (c >= 0x20 && c <= 0x7e && c != '\\') sb.append(c);
      else sb.append(String.format("\\u%04X", (int) c));
    }
    return sb.toString();
  }

  static String bits(float f) {
    return Integer.toHexString(Float.floatToRawIntBits(f));
  }

  static String bits(double d) {
    return Long.toHexString(Double.doubleToRawLongBits(d));
  }

  static String input(String path) throws IOException {
    return path.equals("-") ? null : Files.readString(Path.of(path), StandardCharsets.UTF_8);
  }

  public static void main(String[] args) throws Exception {
    long seed = Long.parseLong(args[0]);
    String scenario = args[1];
    String input = input(args[2]);
    String[] rest = Arrays.copyOfRange(args, 3, args.length);
    PrintStream out =
        new PrintStream(new FileOutputStream(FileDescriptor.out), false, StandardCharsets.UTF_8);
    run(scenario, new Random(seed), input, rest, out);
    out.flush();
  }

  static IllegalArgumentException unknown(String scenario) {
    return new IllegalArgumentException("unknown scenario: " + scenario);
  }

  static void run(String scenario, Random r, String input, String[] args, PrintStream out)
      throws Exception {
    switch (scenario) {
      case "smoke" -> smoke(r, out);
      default -> throw unknown(scenario);
    }
  }

  /**
   * Six seeded texts, each highlighted for a seeded term with {@code QueryScorer} and a {@code
   * SimpleFragmenter(20)}: the best three fragments.
   */
  static void smoke(Random r, PrintStream out) throws Exception {
    StandardAnalyzer analyzer = new StandardAnalyzer();
    for (int doc = 0; doc < 6; doc++) {
      String text = Corpus.text(r, 20 + r.nextInt(60));
      String word = Corpus.word(r);
      Highlighter highlighter =
          new Highlighter(new QueryScorer(new TermQuery(new Term("f", word))));
      highlighter.setTextFragmenter(new SimpleFragmenter(20));
      String[] fragments = highlighter.getBestFragments(analyzer, "f", text, 3);
      out.println("doc " + doc + " " + word + " " + fragments.length);
      for (String fragment : fragments) {
        out.println("frag " + escape(fragment));
      }
    }
  }
}
