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
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Function;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.search.AutomatonQuery;
import org.apache.lucene.search.FuzzyQuery;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.MultiTermQuery;
import org.apache.lucene.search.PrefixQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.RegexpQuery;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TermInSetQuery;
import org.apache.lucene.search.TermRangeQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.WildcardQuery;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.automaton.Automata;
import org.apache.lucene.util.automaton.Automaton;
import org.apache.lucene.util.automaton.CompiledAutomaton;
import org.apache.lucene.util.automaton.Operations;
import org.apache.lucene.util.automaton.RegExp;

/**
 * Oracle of {@code tests/portability/multi_term_queries.rs} (rmp task #405).
 *
 * <p>The scoring corpus is the one of {@code SimilarityScoresFixture}, defined here and in the
 * Rust test by the same integer formulas, indexed with the default similarity. Modes:
 *
 * <ul>
 *   <li>{@code write <dir>}: writes the corpus.
 *   <li>{@code query <dir>}: runs every multi-term query under every rewrite method Lucene allows
 *       for its field and prints the hits with their score bits and the total hits, then dumps the
 *       terms {@code Terms.intersect(CompiledAutomaton, BytesRef)} returns for a few automata.
 * </ul>
 */
public final class MultiTermQueryFixture {

  // ---------------------------------------------------------------------------------------------
  // Scoring corpus (shared by the three scoring fixtures)
  // ---------------------------------------------------------------------------------------------

  /** Documents of each batch, in the order they are added; a commit closes each batch. */
  static final int[][] BATCHES = {{0, 100}, {100, 200}, {200, 300}};

  /** Keyword vocabulary of the {@code kw} field. */
  static final String[] WORDS = {
    "apple", "apply", "ample", "maple", "apricot", "banana", "bandana", "cabana", "cherry", "sherry",
    "berry", "merry", "date", "data", "gate", "grape", "graph", "form", "from", "zebra"
  };

  /** Tokenized text with frequencies, positions and norms. */
  static final FieldType TEXT_TYPE = new FieldType();

  static {
    TEXT_TYPE.setTokenized(true);
    TEXT_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    TEXT_TYPE.freeze();
  }

  private MultiTermQueryFixture() {}

  static String id(int i) {
    return String.format("id-%04d", i);
  }

  /** {@code i % 9 + 2} words; every fourth word repeats the word two positions before it. */
  static String body(int i) {
    List<String> words = new ArrayList<>();
    for (int j = 0; j < i % 9 + 2; j++) {
      words.add(j % 4 == 3 ? words.get(j - 2) : "w" + ((i * 31 + j * 17) % 40));
    }
    return String.join(" ", words);
  }

  static String title(int i) {
    List<String> words = new ArrayList<>();
    for (int j = 0; j < i % 3 + 1; j++) {
      words.add("w" + ((i * 7 + j * 11) % 40));
    }
    return String.join(" ", words);
  }

  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", id(i), Field.Store.YES));
    d.add(new Field("body", body(i), TEXT_TYPE));
    d.add(new Field("title", title(i), TEXT_TYPE));
    for (String kw : new String[] {WORDS[i % 20], WORDS[(i * 7 + 3) % 20]}) {
      d.add(new StringField("kw", kw, Field.Store.NO));
      d.add(new SortedSetDocValuesField("kw", new BytesRef(kw)));
    }
    d.add(new NumericDocValuesField("rank", (i * 37) % 101));
    return d;
  }

  /** Three commits, deletions in the first and the last segment. */
  static void write(Directory dir) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    try (IndexWriter w = new IndexWriter(dir, config)) {
      for (int b = 0; b < BATCHES.length; b++) {
        for (int i = BATCHES[b][0]; i < BATCHES[b][1]; i++) {
          w.addDocument(doc(i));
        }
        if (b == 0) {
          w.deleteDocuments(new Term("id", id(7)));
          w.deleteDocuments(new Term("id", id(42)));
        } else if (b == 2) {
          w.deleteDocuments(new Term("id", id(250)));
        }
        w.commit();
      }
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Query set
  // ---------------------------------------------------------------------------------------------

  /** Hits printed per query. */
  static final int TOP_N = 10;

  static String bits(float f) {
    return String.format("%08x", Float.floatToIntBits(f));
  }

  static BytesRef br(String s) {
    return new BytesRef(s);
  }

  static Automaton regexp(String s) {
    return Operations.determinize(
        new RegExp(s).toAutomaton(), Operations.DEFAULT_DETERMINIZE_WORK_LIMIT);
  }

  /** The rewrite methods every field accepts, by name. */
  static Map<String, MultiTermQuery.RewriteMethod> textRewrites() {
    Map<String, MultiTermQuery.RewriteMethod> r = new LinkedHashMap<>();
    r.put("blended", MultiTermQuery.CONSTANT_SCORE_BLENDED_REWRITE);
    r.put("constant", MultiTermQuery.CONSTANT_SCORE_REWRITE);
    r.put("scoring-boolean", MultiTermQuery.SCORING_BOOLEAN_REWRITE);
    r.put("top-terms-3", new MultiTermQuery.TopTermsScoringBooleanQueryRewrite(3));
    return r;
  }

  /** {@link #textRewrites} plus the doc-values rewrite, for {@code kw} (SORTED_SET doc values). */
  static Map<String, MultiTermQuery.RewriteMethod> keywordRewrites() {
    Map<String, MultiTermQuery.RewriteMethod> r = textRewrites();
    r.put("doc-values", MultiTermQuery.DOC_VALUES_REWRITE);
    return r;
  }

  static Query wildcard(String field, String text, MultiTermQuery.RewriteMethod rewrite) {
    return new WildcardQuery(
        new Term(field, text), Operations.DEFAULT_DETERMINIZE_WORK_LIMIT, rewrite);
  }

  static Query regexpQuery(String field, String text, MultiTermQuery.RewriteMethod rewrite) {
    return new RegexpQuery(
        new Term(field, text),
        RegExp.ALL,
        0,
        RegexpQuery.DEFAULT_PROVIDER,
        Operations.DEFAULT_DETERMINIZE_WORK_LIMIT,
        rewrite,
        true);
  }

  static Query fuzzy(
      String field,
      String text,
      int maxEdits,
      boolean transpositions,
      int prefixLength,
      MultiTermQuery.RewriteMethod rewrite) {
    return new FuzzyQuery(
        new Term(field, text),
        maxEdits,
        prefixLength,
        FuzzyQuery.defaultMaxExpansions,
        transpositions,
        rewrite);
  }

  /** The keyword queries, by name, in the order they are printed. */
  static Map<String, Function<MultiTermQuery.RewriteMethod, Query>> keywordQueries() {
    Map<String, Function<MultiTermQuery.RewriteMethod, Query>> q = new LinkedHashMap<>();
    q.put("wildcard kw:ap*e", r -> wildcard("kw", "ap*e", r));
    q.put("wildcard kw:?erry", r -> wildcard("kw", "?erry", r));
    q.put("prefix kw:ba", r -> new PrefixQuery(new Term("kw", "ba"), r));
    q.put("regexp kw:[bc]a.*a", r -> regexpQuery("kw", "[bc]a.*a", r));
    for (int maxEdits = 0; maxEdits <= 2; maxEdits++) {
      for (boolean transpositions : new boolean[] {true, false}) {
        for (int prefixLength = 0; prefixLength <= 1; prefixLength++) {
          final int e = maxEdits;
          final boolean t = transpositions;
          final int p = prefixLength;
          q.put(
              "fuzzy kw:form maxEdits=" + e + " transpositions=" + t + " prefixLength=" + p,
              r -> fuzzy("kw", "form", e, t, p, r));
        }
      }
    }
    q.put("fuzzy kw:apple maxEdits=1", r -> fuzzy("kw", "apple", 1, true, 0, r));
    q.put("fuzzy kw:apple maxEdits=2", r -> fuzzy("kw", "apple", 2, true, 0, r));
    q.put(
        "range kw:[banana TO date]",
        r -> new TermRangeQuery("kw", br("banana"), br("date"), true, true, r));
    q.put(
        "range kw:{banana TO date}",
        r -> new TermRangeQuery("kw", br("banana"), br("date"), false, false, r));
    q.put(
        "range kw:{* TO cherry}", r -> new TermRangeQuery("kw", null, br("cherry"), false, false, r));
    q.put("range kw:{grape TO *]", r -> new TermRangeQuery("kw", br("grape"), null, false, true, r));
    q.put(
        "terms kw:{apple berry zebra nothere}",
        r ->
            new TermInSetQuery(
                r, "kw", List.of(br("apple"), br("berry"), br("zebra"), br("nothere"))));
    q.put(
        "automaton kw:(apple|cherry|g.*)",
        r -> new AutomatonQuery(new Term("kw", ""), regexp("(apple|cherry|g.*)"), false, r));
    q.put(
        "automaton kw:binary [c TO g}",
        r ->
            new AutomatonQuery(
                new Term("kw", ""),
                Automata.makeBinaryInterval(br("c"), true, br("g"), false),
                true,
                r));
    return q;
  }

  /** The text queries, by name, in the order they are printed. */
  static Map<String, Function<MultiTermQuery.RewriteMethod, Query>> textQueries() {
    Map<String, Function<MultiTermQuery.RewriteMethod, Query>> q = new LinkedHashMap<>();
    q.put("wildcard body:w1*", r -> wildcard("body", "w1*", r));
    q.put("prefix body:w2", r -> new PrefixQuery(new Term("body", "w2"), r));
    q.put("regexp body:w[1-3]?[05]", r -> regexpQuery("body", "w[1-3]?[05]", r));
    q.put(
        "fuzzy body:w12 maxEdits=1 transpositions=true prefixLength=1",
        r -> fuzzy("body", "w12", 1, true, 1, r));
    q.put(
        "fuzzy body:w21 maxEdits=1 transpositions=false prefixLength=0",
        r -> fuzzy("body", "w21", 1, false, 0, r));
    q.put(
        "range body:[w20 TO w29]",
        r -> new TermRangeQuery("body", br("w20"), br("w29"), true, true, r));
    q.put("terms body:{w3 w33}", r -> new TermInSetQuery(r, "body", List.of(br("w3"), br("w33"))));
    return q;
  }

  /** The automata whose {@code Terms.intersect} output is dumped: name, field, start term. */
  static List<Object[]> intersections() {
    List<Object[]> l = new ArrayList<>();
    l.add(new Object[] {"kw a.*e", "kw", regexp("a.*e"), null});
    l.add(new Object[] {"kw a.*e after ample", "kw", regexp("a.*e"), br("ample")});
    l.add(
        new Object[] {
          "kw wildcard ?erry",
          "kw",
          WildcardQuery.toAutomaton(new Term("kw", "?erry"), Operations.DEFAULT_DETERMINIZE_WORK_LIMIT),
          null
        });
    l.add(new Object[] {"body w[1-3]?[05]", "body", regexp("w[1-3]?[05]"), null});
    l.add(new Object[] {"body w1.* after w15", "body", regexp("w1.*"), br("w15")});
    return l;
  }

  static void printQuery(
      IndexSearcher searcher, StoredFields stored, String name, String rewrite, Query query)
      throws IOException {
    TopDocs top = searcher.search(query, TOP_N);
    System.out.println(
        "query "
            + name
            + " rewrite="
            + rewrite
            + " total="
            + top.totalHits.value()
            + " "
            + top.totalHits.relation());
    for (ScoreDoc hit : top.scoreDocs) {
      System.out.println(
          "  hit doc="
              + hit.doc
              + " score="
              + bits(hit.score)
              + " id="
              + stored.document(hit.doc).get("id"));
    }
  }

  static void query(Directory dir) throws IOException {
    try (DirectoryReader reader = DirectoryReader.open(dir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      StoredFields stored = searcher.storedFields();
      System.out.println("reader maxDoc=" + reader.maxDoc() + " numDocs=" + reader.numDocs());
      for (LeafReaderContext ctx : reader.leaves()) {
        System.out.println(
            "leaf ord="
                + ctx.ord
                + " docBase="
                + ctx.docBase
                + " maxDoc="
                + ctx.reader().maxDoc()
                + " numDocs="
                + ctx.reader().numDocs());
      }
      for (Map.Entry<String, Function<MultiTermQuery.RewriteMethod, Query>> q :
          keywordQueries().entrySet()) {
        for (Map.Entry<String, MultiTermQuery.RewriteMethod> r : keywordRewrites().entrySet()) {
          printQuery(searcher, stored, q.getKey(), r.getKey(), q.getValue().apply(r.getValue()));
        }
      }
      for (Map.Entry<String, Function<MultiTermQuery.RewriteMethod, Query>> q :
          textQueries().entrySet()) {
        for (Map.Entry<String, MultiTermQuery.RewriteMethod> r : textRewrites().entrySet()) {
          printQuery(searcher, stored, q.getKey(), r.getKey(), q.getValue().apply(r.getValue()));
        }
      }
      for (Object[] in : intersections()) {
        CompiledAutomaton compiled = new CompiledAutomaton((Automaton) in[2]);
        for (LeafReaderContext ctx : reader.leaves()) {
          StringBuilder line = new StringBuilder();
          line.append("intersect ").append(in[0]).append(" leaf=").append(ctx.ord).append(':');
          Terms terms = ctx.reader().terms((String) in[1]);
          TermsEnum te = terms.intersect(compiled, (BytesRef) in[3]);
          for (BytesRef t = te.next(); t != null; t = te.next()) {
            line.append(' ').append(t.utf8ToString()).append('=').append(te.docFreq());
          }
          System.out.println(line);
        }
      }
    }
  }

  public static void main(String[] args) throws Exception {
    try (Directory dir = FSDirectory.open(Paths.get(args[1]))) {
      switch (args[0]) {
        case "write" -> write(dir);
        case "query" -> query(dir);
        default -> throw new IllegalArgumentException(args[0]);
      }
    }
  }
}
