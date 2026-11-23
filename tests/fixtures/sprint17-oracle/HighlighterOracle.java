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
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.BooleanClause.Occur;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.PhraseQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.highlight.Highlighter;
import org.apache.lucene.search.highlight.QueryScorer;
import org.apache.lucene.search.highlight.SimpleFragmenter;
import org.apache.lucene.search.matchhighlight.FieldValueHighlighters;
import org.apache.lucene.search.matchhighlight.MatchHighlighter;
import org.apache.lucene.search.matchhighlight.PassageFormatter;
import org.apache.lucene.search.uhighlight.UnifiedHighlighter;
import org.apache.lucene.search.vectorhighlight.FastVectorHighlighter;
import org.apache.lucene.search.vectorhighlight.FieldQuery;
import org.apache.lucene.store.ByteBuffersDirectory;

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
      case "fragments" -> fragments(r, out);
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

  /**
   * The fragments and passages of the four highlighters on a seeded corpus: twelve seeded texts in
   * one index (field {@code f}, stored, with term vectors, positions and offsets, and postings
   * offsets), then eight seeded queries (a term, an adjacent pair of a text as a phrase, or two
   * terms as SHOULD clauses), each searched for its top twelve documents and highlighted by {@code
   * Highlighter} ({@code QueryScorer}, {@code SimpleFragmenter(20)}, three fragments), {@code
   * FastVectorHighlighter} (fragment size 30, three fragments), {@code UnifiedHighlighter} (two
   * passages) and {@code MatchHighlighter} ({@code highlighted(60, 2, PassageFormatter("...", ">",
   * "<"))}).
   */
  static void fragments(Random r, PrintStream out) throws Exception {
    StandardAnalyzer analyzer = new StandardAnalyzer();
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    FieldType ft = new FieldType(TextField.TYPE_STORED);
    ft.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    ft.setStoreTermVectors(true);
    ft.setStoreTermVectorPositions(true);
    ft.setStoreTermVectorOffsets(true);
    ft.freeze();
    int numDocs = 12;
    String[] texts = new String[numDocs];
    try (IndexWriter w = new IndexWriter(dir, new IndexWriterConfig(analyzer))) {
      for (int d = 0; d < numDocs; d++) {
        texts[d] = Corpus.text(r, 20 + r.nextInt(60));
        Document doc = new Document();
        doc.add(new Field("f", texts[d], ft));
        w.addDocument(doc);
      }
    }
    try (DirectoryReader reader = DirectoryReader.open(dir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      for (int q = 0; q < 8; q++) {
        Query query;
        switch (r.nextInt(3)) {
          case 0 -> query = new TermQuery(new Term("f", Corpus.word(r)));
          case 1 -> {
            String[] words = texts[r.nextInt(numDocs)].split(" ");
            int at = r.nextInt(words.length - 1);
            query = new PhraseQuery("f", words[at], words[at + 1]);
          }
          default -> {
            BooleanQuery.Builder b = new BooleanQuery.Builder();
            b.add(new TermQuery(new Term("f", Corpus.word(r))), Occur.SHOULD);
            b.add(new TermQuery(new Term("f", Corpus.word(r))), Occur.SHOULD);
            query = b.build();
          }
        }
        TopDocs top = searcher.search(query, numDocs);
        out.println("query " + q + " " + escape(query.toString()) + " " + top.scoreDocs.length);
        FastVectorHighlighter fvh = new FastVectorHighlighter();
        FieldQuery fieldQuery = fvh.getFieldQuery(query, reader);
        for (ScoreDoc sd : top.scoreDocs) {
          String text = texts[sd.doc];
          Highlighter highlighter = new Highlighter(new QueryScorer(query, "f"));
          highlighter.setTextFragmenter(new SimpleFragmenter(20));
          String[] hl = highlighter.getBestFragments(analyzer, "f", text, 3);
          out.println("doc " + sd.doc + " " + bits(sd.score));
          for (String fragment : hl) {
            out.println("hl " + escape(fragment));
          }
          String[] vh = fvh.getBestFragments(fieldQuery, reader, sd.doc, "f", 30, 3);
          for (String fragment : vh) {
            out.println("fvh " + escape(fragment));
          }
        }
        UnifiedHighlighter uh = UnifiedHighlighter.builder(searcher, analyzer).build();
        String[] passages = uh.highlight("f", query, top, 2);
        for (int i = 0; i < passages.length; i++) {
          out.println("uh " + top.scoreDocs[i].doc + " " + escape(passages[i]));
        }
        MatchHighlighter mh =
            new MatchHighlighter(searcher, analyzer)
                .appendFieldHighlighter(
                    FieldValueHighlighters.highlighted(
                        60, 2, new PassageFormatter("...", ">", "<"), fld -> true));
        mh.highlight(top, query)
            .forEach(
                dh -> {
                  out.println("mh " + dh.docId);
                  dh.fields.forEach(
                      (field, values) -> {
                        for (String value : values) {
                          out.println("mh " + field + " " + escape(value));
                        }
                      });
                });
      }
    }
  }
}
