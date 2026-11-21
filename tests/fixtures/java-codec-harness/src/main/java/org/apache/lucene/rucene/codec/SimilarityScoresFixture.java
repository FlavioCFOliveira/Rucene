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
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Supplier;
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
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.Explanation;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.PhraseQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.similarities.AfterEffectB;
import org.apache.lucene.search.similarities.AfterEffectL;
import org.apache.lucene.search.similarities.AxiomaticF1EXP;
import org.apache.lucene.search.similarities.AxiomaticF1LOG;
import org.apache.lucene.search.similarities.AxiomaticF2EXP;
import org.apache.lucene.search.similarities.AxiomaticF2LOG;
import org.apache.lucene.search.similarities.AxiomaticF3EXP;
import org.apache.lucene.search.similarities.AxiomaticF3LOG;
import org.apache.lucene.search.similarities.BM25Similarity;
import org.apache.lucene.search.similarities.BasicModelG;
import org.apache.lucene.search.similarities.BasicModelIF;
import org.apache.lucene.search.similarities.BasicModelIn;
import org.apache.lucene.search.similarities.BasicModelIne;
import org.apache.lucene.search.similarities.BooleanSimilarity;
import org.apache.lucene.search.similarities.ClassicSimilarity;
import org.apache.lucene.search.similarities.DFISimilarity;
import org.apache.lucene.search.similarities.DFRSimilarity;
import org.apache.lucene.search.similarities.DistributionLL;
import org.apache.lucene.search.similarities.DistributionSPL;
import org.apache.lucene.search.similarities.IBSimilarity;
import org.apache.lucene.search.similarities.IndependenceChiSquared;
import org.apache.lucene.search.similarities.IndependenceSaturated;
import org.apache.lucene.search.similarities.IndependenceStandardized;
import org.apache.lucene.search.similarities.IndriDirichletSimilarity;
import org.apache.lucene.search.similarities.LMDirichletSimilarity;
import org.apache.lucene.search.similarities.LMJelinekMercerSimilarity;
import org.apache.lucene.search.similarities.LambdaDF;
import org.apache.lucene.search.similarities.LambdaTTF;
import org.apache.lucene.search.similarities.MultiSimilarity;
import org.apache.lucene.search.similarities.Normalization;
import org.apache.lucene.search.similarities.NormalizationH1;
import org.apache.lucene.search.similarities.NormalizationH2;
import org.apache.lucene.search.similarities.NormalizationH3;
import org.apache.lucene.search.similarities.NormalizationZ;
import org.apache.lucene.search.similarities.PerFieldSimilarityWrapper;
import org.apache.lucene.search.similarities.RawTFSimilarity;
import org.apache.lucene.search.similarities.Similarity;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Oracle of {@code tests/portability/similarity_scores.rs} (rmp task #404).
 *
 * <p>The scoring corpus is defined here and in the Rust test by the same integer formulas (it is
 * shared with {@code MultiTermQueryFixture} and {@code ScoringQueriesFixture}). For every
 * similarity of {@link #SIMILARITIES} the corpus is indexed with that similarity set on the writer
 * (norms depend on it) into {@code <root>/<name>}, and searched with the same similarity set on
 * the searcher. Modes:
 *
 * <ul>
 *   <li>{@code write <root>}: writes one index per similarity under {@code root}.
 *   <li>{@code query <root>}: runs the query set on every index and prints, per hit, the score
 *       bits and the {@code explain()} text.
 * </ul>
 */
public final class SimilarityScoresFixture {

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

  private SimilarityScoresFixture() {}

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
  static void write(Directory dir, Similarity similarity) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    config.setSimilarity(similarity);
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
  // Similarities
  // ---------------------------------------------------------------------------------------------

  /** Scores {@code body} with BM25(1.5, 0.5) and every other field with ClassicSimilarity. */
  static final class PerFieldSim extends PerFieldSimilarityWrapper {
    private final Similarity body = new BM25Similarity(1.5f, 0.5f);
    private final Similarity other = new ClassicSimilarity();

    @Override
    public Similarity get(String name) {
      return name.equals("body") ? body : other;
    }
  }

  /** The similarities under test, by the directory name both sides use. */
  static Map<String, Supplier<Similarity>> similarities() {
    Map<String, Supplier<Similarity>> s = new LinkedHashMap<>();
    s.put("classic", ClassicSimilarity::new);
    s.put("bm25-default", BM25Similarity::new);
    s.put("bm25-1.5-0.5", () -> new BM25Similarity(1.5f, 0.5f));
    s.put("boolean", BooleanSimilarity::new);
    s.put(
        "dfr-G-B-H2",
        () -> new DFRSimilarity(new BasicModelG(), new AfterEffectB(), new NormalizationH2()));
    s.put(
        "dfr-In-L-H1",
        () -> new DFRSimilarity(new BasicModelIn(), new AfterEffectL(), new NormalizationH1()));
    s.put(
        "dfr-IF-B-H3",
        () -> new DFRSimilarity(new BasicModelIF(), new AfterEffectB(), new NormalizationH3()));
    s.put(
        "dfr-Ine-L-Z",
        () -> new DFRSimilarity(new BasicModelIne(), new AfterEffectL(), new NormalizationZ()));
    s.put(
        "dfr-G-L-none",
        () ->
            new DFRSimilarity(
                new BasicModelG(), new AfterEffectL(), new Normalization.NoNormalization()));
    s.put(
        "ib-LL-DF-H2",
        () -> new IBSimilarity(new DistributionLL(), new LambdaDF(), new NormalizationH2()));
    s.put(
        "ib-SPL-TTF-H3",
        () -> new IBSimilarity(new DistributionSPL(), new LambdaTTF(), new NormalizationH3()));
    s.put(
        "ib-LL-TTF-Z",
        () -> new IBSimilarity(new DistributionLL(), new LambdaTTF(), new NormalizationZ()));
    s.put(
        "ib-SPL-DF-H1",
        () -> new IBSimilarity(new DistributionSPL(), new LambdaDF(), new NormalizationH1()));
    s.put("lm-dirichlet-default", LMDirichletSimilarity::new);
    s.put("lm-dirichlet-500", () -> new LMDirichletSimilarity(500f));
    s.put("lm-jm-0.7", () -> new LMJelinekMercerSimilarity(0.7f));
    s.put("lm-jm-0.1", () -> new LMJelinekMercerSimilarity(0.1f));
    s.put("indri-default", IndriDirichletSimilarity::new);
    s.put("indri-1000", () -> new IndriDirichletSimilarity(1000f));
    s.put("axiomatic-F1EXP", AxiomaticF1EXP::new);
    s.put("axiomatic-F1LOG", AxiomaticF1LOG::new);
    s.put("axiomatic-F2EXP", AxiomaticF2EXP::new);
    s.put("axiomatic-F2LOG", AxiomaticF2LOG::new);
    s.put("axiomatic-F3EXP", () -> new AxiomaticF3EXP(0.25f, 3));
    s.put("axiomatic-F3LOG", () -> new AxiomaticF3LOG(0.25f, 3));
    s.put("dfi-standardized", () -> new DFISimilarity(new IndependenceStandardized()));
    s.put("dfi-saturated", () -> new DFISimilarity(new IndependenceSaturated()));
    s.put("dfi-chisquared", () -> new DFISimilarity(new IndependenceChiSquared()));
    s.put("rawtf", RawTFSimilarity::new);
    s.put(
        "multi-bm25-classic",
        () -> new MultiSimilarity(new Similarity[] {new BM25Similarity(), new ClassicSimilarity()}));
    s.put("perfield-bm25-classic", PerFieldSim::new);
    return s;
  }

  // ---------------------------------------------------------------------------------------------
  // Query set
  // ---------------------------------------------------------------------------------------------

  /** Hits printed per query. */
  static final int TOP_N = 5;

  /** Hits whose explanation is printed per query. */
  static final int EXPLAINED = 3;

  static String bits(float f) {
    return String.format("%08x", Float.floatToIntBits(f));
  }

  static TermQuery tq(String field, String text) {
    return new TermQuery(new Term(field, text));
  }

  /** The query set, by name, in the order it is printed. */
  static Map<String, Query> queries() {
    Map<String, Query> q = new LinkedHashMap<>();
    q.put("term body:w7", tq("body", "w7"));
    q.put("term title:w13", tq("title", "w13"));
    BooleanQuery.Builder disj = new BooleanQuery.Builder();
    disj.add(tq("body", "w3"), BooleanClause.Occur.SHOULD);
    disj.add(tq("body", "w11"), BooleanClause.Occur.SHOULD);
    disj.add(tq("title", "w3"), BooleanClause.Occur.SHOULD);
    q.put("bool body:w3 body:w11 title:w3", disj.build());
    BooleanQuery.Builder conj = new BooleanQuery.Builder();
    conj.add(tq("body", "w1"), BooleanClause.Occur.MUST);
    conj.add(tq("body", "w2"), BooleanClause.Occur.SHOULD);
    q.put("bool +body:w1 body:w2", conj.build());
    q.put("phrase body:\"w5 w22\"~1", new PhraseQuery(1, "body", "w5", "w22"));
    return q;
  }

  static void query(Path root) throws IOException {
    for (Map.Entry<String, Supplier<Similarity>> e : similarities().entrySet()) {
      System.out.println("similarity " + e.getKey());
      try (Directory dir = FSDirectory.open(root.resolve(e.getKey()));
          DirectoryReader reader = DirectoryReader.open(dir)) {
        IndexSearcher searcher = new IndexSearcher(reader);
        searcher.setSimilarity(e.getValue().get());
        StoredFields stored = searcher.storedFields();
        System.out.println("reader maxDoc=" + reader.maxDoc() + " numDocs=" + reader.numDocs());
        for (Map.Entry<String, Query> qe : queries().entrySet()) {
          TopDocs top = searcher.search(qe.getValue(), TOP_N);
          System.out.println(
              "query "
                  + qe.getKey()
                  + " total="
                  + top.totalHits.value()
                  + " "
                  + top.totalHits.relation());
          for (int k = 0; k < top.scoreDocs.length; k++) {
            ScoreDoc hit = top.scoreDocs[k];
            System.out.println(
                "  hit doc="
                    + hit.doc
                    + " score="
                    + bits(hit.score)
                    + " id="
                    + stored.document(hit.doc).get("id"));
            if (k < EXPLAINED) {
              Explanation explanation = searcher.explain(qe.getValue(), hit.doc);
              System.out.println("    explain value=" + bits(explanation.getValue().floatValue()));
              for (String line : explanation.toString().split("\n")) {
                System.out.println("    | " + line);
              }
            }
          }
        }
      }
    }
  }

  static void write(Path root) throws IOException {
    for (Map.Entry<String, Supplier<Similarity>> e : similarities().entrySet()) {
      Path path = root.resolve(e.getKey());
      Files.createDirectories(path);
      try (Directory dir = FSDirectory.open(path)) {
        write(dir, e.getValue().get());
      }
    }
  }

  public static void main(String[] args) throws Exception {
    Path root = Paths.get(args[1]);
    switch (args[0]) {
      case "write" -> write(root);
      case "query" -> query(root);
      default -> throw new IllegalArgumentException(args[0]);
    }
  }
}
