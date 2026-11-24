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
import org.apache.lucene.index.memory.MemoryIndex;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.PhraseQuery;
import org.apache.lucene.search.PrefixQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.similarities.ClassicSimilarity;

/**
 * The Java side of the sprint-17 oracle harness for {@code lucene/memory} (rmp #628,
 * {@code tests/sprint17_oracle/mod.rs}): {@code main(seed, scenario, input, args...)} prints the
 * dump of the scenario, which Rucene's test computes from the same seed.
 */
public class MemoryOracle {
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
      case "search" -> search(r, out);
      default -> throw unknown(scenario);
    }
  }

  /**
   * Eight seeded texts, each in its own {@code MemoryIndex} (offsets drawn), searched with six
   * term queries and one phrase query: the bits of every score, then {@code toStringDebug()}.
   */
  static void smoke(Random r, PrintStream out) throws Exception {
    StandardAnalyzer analyzer = new StandardAnalyzer();
    for (int doc = 0; doc < 8; doc++) {
      String text = Corpus.text(r, 5 + r.nextInt(20));
      MemoryIndex index = new MemoryIndex(r.nextBoolean());
      index.addField("body", text, analyzer);
      out.println("doc " + doc + " " + text);
      for (int q = 0; q < 6; q++) {
        String word = Corpus.word(r);
        float score = index.search(new TermQuery(new Term("body", word)));
        out.println("term " + word + " " + bits(score));
      }
      String w1 = Corpus.word(r);
      String w2 = Corpus.word(r);
      float score = index.search(new PhraseQuery("body", w1, w2));
      out.println("phrase " + w1 + " " + w2 + " " + bits(score));
      out.print(index.toStringDebug());
      out.println();
    }
  }

  /**
   * MemoryIndex search parity (rmp #629): twelve seeded two-field documents ({@code title}, {@code
   * body}; offsets drawn; {@code ClassicSimilarity} drawn), each searched with eight queries (term,
   * phrase, sloppy phrase, boolean, prefix): the bits of every score, then {@code
   * toStringDebug()}.
   */
  static void search(Random r, PrintStream out) throws Exception {
    StandardAnalyzer analyzer = new StandardAnalyzer();
    for (int doc = 0; doc < 12; doc++) {
      String title = Corpus.text(r, 3 + r.nextInt(8));
      String body = Corpus.text(r, 10 + r.nextInt(50));
      MemoryIndex index = new MemoryIndex(r.nextBoolean());
      boolean classic = r.nextBoolean();
      if (classic) {
        index.setSimilarity(new ClassicSimilarity());
      }
      index.addField("title", title, analyzer);
      index.addField("body", body, analyzer);
      out.println("doc " + doc + " classic=" + classic);
      out.println("title " + title);
      out.println("body " + body);
      for (int q = 0; q < 8; q++) {
        Query query = query(r);
        out.println("query " + query + " " + bits(index.search(query)));
      }
      out.print(index.toStringDebug());
      out.println();
    }
  }

  static Query query(Random r) {
    switch (r.nextInt(5)) {
      case 0:
        return new TermQuery(new Term(r.nextBoolean() ? "title" : "body", Corpus.word(r)));
      case 1:
        return new PhraseQuery("body", Corpus.word(r), Corpus.word(r));
      case 2:
        return new PhraseQuery(r.nextInt(4), "body", Corpus.word(r), Corpus.word(r));
      case 3:
        {
          BooleanQuery.Builder b = new BooleanQuery.Builder();
          b.add(new TermQuery(new Term("title", Corpus.word(r))), BooleanClause.Occur.SHOULD);
          b.add(new TermQuery(new Term("body", Corpus.word(r))), BooleanClause.Occur.SHOULD);
          b.add(
              new TermQuery(new Term("body", Corpus.word(r))),
              r.nextBoolean() ? BooleanClause.Occur.MUST : BooleanClause.Occur.MUST_NOT);
          return b.build();
        }
      default:
        {
          String w = Corpus.word(r);
          return new PrefixQuery(new Term("body", w.substring(0, 1 + r.nextInt(w.length()))));
        }
    }
  }
}
