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
import org.apache.lucene.search.BlendedTermQuery;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.BoostQuery;
import org.apache.lucene.search.CombinedFieldQuery;
import org.apache.lucene.search.ConstantScoreQuery;
import org.apache.lucene.search.DisjunctionMaxQuery;
import org.apache.lucene.search.FieldDoc;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.MultiPhraseQuery;
import org.apache.lucene.search.NGramPhraseQuery;
import org.apache.lucene.search.PhraseQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.search.SynonymQuery;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.TopFieldDocs;
import org.apache.lucene.search.WildcardQuery;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Oracle of {@code tests/portability/scoring_queries.rs} (rmp task #407).
 *
 * <p>The scoring corpus is the one of {@code SimilarityScoresFixture}, defined here and in the
 * Rust test by the same integer formulas, indexed with the default similarity. Modes:
 *
 * <ul>
 *   <li>{@code write <dir>}: writes the corpus.
 *   <li>{@code query <dir>}: runs the positional and compound queries and prints their hits with
 *       score bits, then {@code IndexSearcher.count}, {@code searchAfter} paging by score and by a
 *       field sort, and {@code TopDocs.merge} over per-leaf searchers.
 * </ul>
 */
public final class ScoringQueriesFixture {

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

  private ScoringQueriesFixture() {}

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

  /** Hits per page of the paging sections. */
  static final int PAGE = 7;

  /** Pages of the paging sections. */
  static final int PAGES = 3;

  static String bits(float f) {
    return String.format("%08x", Float.floatToIntBits(f));
  }

  static Term t(String field, String text) {
    return new Term(field, text);
  }

  static TermQuery tq(String field, String text) {
    return new TermQuery(t(field, text));
  }

  static String sortValue(FieldDoc fd) {
    Object v = fd.fields[0];
    if (v == null) {
      return "null";
    } else if (v instanceof BytesRef b) {
      return "bytes:" + b.utf8ToString();
    }
    return v.getClass().getSimpleName() + ":" + v;
  }

  static void printHits(String header, TopDocs top, StoredFields stored) throws IOException {
    System.out.println(
        header + " total=" + top.totalHits.value() + " " + top.totalHits.relation());
    for (ScoreDoc hit : top.scoreDocs) {
      StringBuilder line = new StringBuilder();
      line.append("  hit doc=").append(hit.doc).append(" score=").append(bits(hit.score));
      if (hit instanceof FieldDoc fd) {
        line.append(" sort=").append(sortValue(fd));
      }
      line.append(" id=").append(stored.document(hit.doc).get("id"));
      System.out.println(line);
    }
  }

  /** The scored queries, by name, in the order they are printed. */
  static Map<String, Query> queries() {
    Map<String, Query> q = new LinkedHashMap<>();
    q.put("phrase body:\"w17 w0\"~2", new PhraseQuery(2, "body", "w17", "w0"));
    q.put("phrase body:\"w3 w37\"~3", new PhraseQuery(3, "body", "w3", "w37"));
    q.put("phrase body:\"w12 w6\"~1", new PhraseQuery(1, "body", "w12", "w6"));
    q.put("phrase body:\"w21 w21\"~1", new PhraseQuery(1, "body", "w21", "w21"));
    q.put("phrase body:\"w21 w21\"~2", new PhraseQuery(2, "body", "w21", "w21"));
    q.put("phrase body:\"w9 w26 w9\"~2", new PhraseQuery(2, "body", "w9", "w26", "w9"));
    MultiPhraseQuery.Builder mp = new MultiPhraseQuery.Builder();
    mp.add(new Term[] {t("body", "w0"), t("body", "w3")});
    mp.add(new Term[] {t("body", "w17"), t("body", "w20")});
    q.put("multiphrase body:\"(w0|w3) (w17|w20)\"", mp.build());
    MultiPhraseQuery.Builder mps = new MultiPhraseQuery.Builder();
    mps.add(new Term[] {t("body", "w17"), t("body", "w20")});
    mps.add(new Term[] {t("body", "w0"), t("body", "w3")});
    mps.setSlop(2);
    q.put("multiphrase body:\"(w17|w20) (w0|w3)\"~2", mps.build());
    q.put(
        "ngram2 body:\"w12 w29 w6\"",
        new NGramPhraseQuery(2, new PhraseQuery("body", "w12", "w29", "w6")));
    q.put(
        "synonym body:{w3 w13^0.5 w23^0.25}",
        new SynonymQuery.Builder("body")
            .addTerm(t("body", "w3"))
            .addTerm(t("body", "w13"), 0.5f)
            .addTerm(t("body", "w23"), 0.25f)
            .build());
    q.put(
        "combined w7 body^1 title^2",
        new CombinedFieldQuery.Builder("w7").addField("body", 1f).addField("title", 2f).build());
    q.put(
        "blended boolean body:w7 title:w7^2 body:w8",
        new BlendedTermQuery.Builder()
            .setRewriteMethod(BlendedTermQuery.BOOLEAN_REWRITE)
            .add(t("body", "w7"))
            .add(t("title", "w7"), 2f)
            .add(t("body", "w8"))
            .build());
    q.put(
        "blended dismax0.1 body:w7 title:w7^2 body:w8",
        new BlendedTermQuery.Builder()
            .setRewriteMethod(new BlendedTermQuery.DisjunctionMaxRewrite(0.1f))
            .add(t("body", "w7"))
            .add(t("title", "w7"), 2f)
            .add(t("body", "w8"))
            .build());
    q.put(
        "dismax (body:w3 | title:w3 | body:w13)~0.3",
        new DisjunctionMaxQuery(
            List.of(tq("body", "w3"), tq("title", "w3"), tq("body", "w13")), 0.3f));
    BooleanQuery.Builder msm = new BooleanQuery.Builder();
    for (String w : new String[] {"w1", "w2", "w3", "w4"}) {
      msm.add(tq("body", w), BooleanClause.Occur.SHOULD);
    }
    msm.setMinimumNumberShouldMatch(2);
    q.put("bool body:(w1 w2 w3 w4) msm=2", msm.build());
    q.put("boost body:w5^2.5", new BoostQuery(tq("body", "w5"), 2.5f));
    q.put("constant body:w6", new ConstantScoreQuery(tq("body", "w6")));
    q.put(
        "boost constant(body:w6)^3", new BoostQuery(new ConstantScoreQuery(tq("body", "w6")), 3f));
    return q;
  }

  /** The counted queries, by name, in the order they are printed. */
  static Map<String, Query> countQueries() {
    Map<String, Query> q = new LinkedHashMap<>();
    q.put("term body:w7", tq("body", "w7"));
    q.put("phrase body:\"w0 w17\"", new PhraseQuery("body", "w0", "w17"));
    q.put("matchall", MatchAllDocsQuery.INSTANCE);
    q.put("wildcard body:w1*", new WildcardQuery(t("body", "w1*")));
    q.put("bool body:(w1 w2 w3 w4) msm=2", queries().get("bool body:(w1 w2 w3 w4) msm=2"));
    q.put(
        "dismax (body:w3 | title:w3 | body:w13)~0.3",
        queries().get("dismax (body:w3 | title:w3 | body:w13)~0.3"));
    q.put(
        "synonym body:{w3 w13^0.5 w23^0.25}", queries().get("synonym body:{w3 w13^0.5 w23^0.25}"));
    return q;
  }

  /** The query the paging and merge sections run. */
  static Query pagingQuery() {
    BooleanQuery.Builder b = new BooleanQuery.Builder();
    for (String w : new String[] {"w1", "w2", "w3"}) {
      b.add(tq("body", w), BooleanClause.Occur.SHOULD);
    }
    return b.build();
  }

  static Sort rankSort() {
    return new Sort(new SortField("rank", SortField.Type.LONG));
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

      for (Map.Entry<String, Query> q : queries().entrySet()) {
        printHits("query " + q.getKey(), searcher.search(q.getValue(), TOP_N), stored);
      }

      for (Map.Entry<String, Query> q : countQueries().entrySet()) {
        System.out.println("count " + q.getKey() + " = " + searcher.count(q.getValue()));
      }

      Query paging = pagingQuery();
      ScoreDoc after = null;
      for (int page = 0; page < PAGES; page++) {
        TopDocs top = searcher.searchAfter(after, paging, PAGE);
        printHits("page score " + page, top, stored);
        after = top.scoreDocs[top.scoreDocs.length - 1];
      }
      FieldDoc fieldAfter = null;
      for (int page = 0; page < PAGES; page++) {
        TopDocs top = searcher.searchAfter(fieldAfter, paging, PAGE, rankSort());
        printHits("page rank " + page, top, stored);
        fieldAfter = (FieldDoc) top.scoreDocs[top.scoreDocs.length - 1];
      }
      fieldAfter = null;
      for (int page = 0; page < PAGES; page++) {
        TopFieldDocs top = searcher.searchAfter(fieldAfter, paging, PAGE, rankSort(), true);
        printHits("page rank+scores " + page, top, stored);
        fieldAfter = (FieldDoc) top.scoreDocs[top.scoreDocs.length - 1];
      }

      List<LeafReaderContext> leaves = reader.leaves();
      IndexSearcher[] shards = new IndexSearcher[leaves.size()];
      TopDocs[] scoreHits = new TopDocs[leaves.size()];
      TopFieldDocs[] sortHits = new TopFieldDocs[leaves.size()];
      for (int s = 0; s < shards.length; s++) {
        shards[s] = new IndexSearcher(leaves.get(s).reader());
        scoreHits[s] = shards[s].search(paging, TOP_N);
        sortHits[s] = shards[s].search(paging, TOP_N, rankSort());
        // TopDocs.merge requires every hit's shardIndex to be set (or all unset); it does not
        // set them itself, and printMerged resolves each hit through its shard.
        for (ScoreDoc hit : scoreHits[s].scoreDocs) {
          hit.shardIndex = s;
        }
        for (ScoreDoc hit : sortHits[s].scoreDocs) {
          hit.shardIndex = s;
        }
      }
      printMerged("merge score", TopDocs.merge(TOP_N, scoreHits), shards);
      printMerged("merge rank", TopDocs.merge(rankSort(), TOP_N, sortHits), shards);
    }
  }

  static void printMerged(String header, TopDocs merged, IndexSearcher[] shards)
      throws IOException {
    System.out.println(
        header + " total=" + merged.totalHits.value() + " " + merged.totalHits.relation());
    for (ScoreDoc hit : merged.scoreDocs) {
      StringBuilder line = new StringBuilder();
      line.append("  hit shard=")
          .append(hit.shardIndex)
          .append(" doc=")
          .append(hit.doc)
          .append(" score=")
          .append(bits(hit.score));
      if (hit instanceof FieldDoc fd) {
        line.append(" sort=").append(sortValue(fd));
      }
      line.append(" id=").append(shards[hit.shardIndex].storedFields().document(hit.doc).get("id"));
      System.out.println(line);
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
