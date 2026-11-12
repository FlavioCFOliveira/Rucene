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
import java.util.List;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.KnnByteVectorField;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.QueryTimeout;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.AcceptDocs;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.ByteVectorSimilarityQuery;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.FloatVectorSimilarityQuery;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.KnnByteVectorQuery;
import org.apache.lucene.search.KnnFloatVectorQuery;
import org.apache.lucene.search.PatienceKnnVectorQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.RescoreTopNQuery;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.SeededKnnVectorQuery;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.knn.KnnCollectorManager;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.Constants;

/**
 * Oracle of {@code tests/portability/knn_variants.rs} (rmp task #411).
 *
 * <p>The corpus is defined here and in the Rust test by the same integer formulas: three commits
 * of 800, 800 and 300 documents (the first two segments are large enough for an HNSW graph of
 * both vector fields, 720 float and 686 byte vectors each, the last is not), a float vector field {@code fvec} (EUCLIDEAN) and a byte vector field {@code bvec}
 * (DOT_PRODUCT), each missing from some documents, keyword fields for seeds and filters, and
 * deletions in the first and the last segment. Modes:
 *
 * <ul>
 *   <li>{@code write <root>}: writes the corpus under {@code root}.
 *   <li>{@code query <root>}: runs the query set and prints, per hit, the doc id, the score bits
 *       and the id, after the total hits value and relation.
 *   <li>{@code probe <root>}: runs the exact-search fallback queries through subclasses that
 *       record every {@code approximateSearch} and {@code exactSearch} call, proving the corpus
 *       drives Lucene into both fallback paths.
 * </ul>
 *
 * <p>The JVM must run with {@code -Dlucene.useScalarFMA=false} and without the incubator vector
 * module, so the float scores do not depend on the CPU; the first {@code query} line records it.
 */
public final class KnnVariantsFixture {

  private KnnVariantsFixture() {}

  // ---------------------------------------------------------------------------------------------
  // Corpus
  // ---------------------------------------------------------------------------------------------

  /** Documents of each batch, in the order they are added; a commit closes each batch. */
  static final int[][] BATCHES = {{0, 800}, {800, 1600}, {1600, 1900}};

  static final int DIM = 8;

  /** Hits printed per query. */
  static final int TOP_N = 20;

  static String id(int i) {
    return String.format("id-%04d", i);
  }

  /** Quarter-integer components in [-31.25, 31.25]: every product and sum is exact. */
  static float[] fvec(int i) {
    float[] v = new float[DIM];
    for (int j = 0; j < DIM; j++) {
      v[j] = ((i * i * 7 + i * 13 + j * j * 5 + i * j * 3) % 251 - 125) / 4.0f;
    }
    return v;
  }

  static byte[] bvec(int i) {
    byte[] v = new byte[DIM];
    for (int j = 0; j < DIM; j++) {
      v[j] = (byte) ((i * i * 3 + j * 11 + i * j * 7) % 256 - 128);
    }
    return v;
  }

  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", id(i), Field.Store.YES));
    d.add(new StringField("tag", "t" + (i % 10), Field.Store.NO));
    d.add(new StringField("g", "g" + (i % 20), Field.Store.NO));
    if (i % 97 == 0) {
      d.add(new StringField("rare", "yes", Field.Store.NO));
    }
    if (i % 10 != 9) {
      d.add(new KnnFloatVectorField("fvec", fvec(i), VectorSimilarityFunction.EUCLIDEAN));
    }
    if (i % 7 != 6) {
      d.add(new KnnByteVectorField("bvec", bvec(i), VectorSimilarityFunction.DOT_PRODUCT));
    }
    return d;
  }

  static void write(String root) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig();
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    try (Directory dir = FSDirectory.open(Paths.get(root));
        IndexWriter w = new IndexWriter(dir, config)) {
      for (int b = 0; b < BATCHES.length; b++) {
        for (int i = BATCHES[b][0]; i < BATCHES[b][1]; i++) {
          w.addDocument(doc(i));
        }
        if (b == 0) {
          w.deleteDocuments(new Term("id", id(7)));
          w.deleteDocuments(new Term("id", id(42)));
          w.deleteDocuments(new Term("id", id(100)));
        } else if (b == 2) {
          w.deleteDocuments(new Term("id", id(1650)));
        }
        w.commit();
      }
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Query set
  // ---------------------------------------------------------------------------------------------

  /** A float target that is no document's vector. */
  static final float[] TARGET_X = {0.5f, -1.25f, 3f, 7.75f, -6f, 2.5f, 0f, -0.25f};

  /** The fallback filter: {@code tag:t0 tag:t1 tag:t2}, as optional clauses. */
  static Query tags012() {
    BooleanQuery.Builder b = new BooleanQuery.Builder();
    for (String t : new String[] {"t0", "t1", "t2"}) {
      b.add(tq("tag", t), BooleanClause.Occur.SHOULD);
    }
    return b.build();
  }

  static TermQuery tq(String field, String text) {
    return new TermQuery(new Term(field, text));
  }

  static List<Object[]> queries() {
    float[] ta = fvec(123);
    float[] tb = fvec(1233);
    float[] tc = fvec(1777);
    byte[] ba = bvec(123);
    byte[] bb = bvec(1233);
    List<Object[]> q = new ArrayList<>();
    q.add(new Object[] {"knn-float k=10 A", new KnnFloatVectorQuery("fvec", ta, 10)});
    q.add(new Object[] {"knn-float k=10 X", new KnnFloatVectorQuery("fvec", TARGET_X, 10)});
    q.add(new Object[] {"knn-byte k=10 A", new KnnByteVectorQuery("bvec", ba, 10)});
    q.add(
        new Object[] {
          "seeded-float k=10 X seed=tag:t3",
          SeededKnnVectorQuery.fromFloatQuery(
              new KnnFloatVectorQuery("fvec", TARGET_X, 10), tq("tag", "t3"))
        });
    q.add(
        new Object[] {
          "seeded-float k=5 A filter=g:g3 seed=tag:t7",
          SeededKnnVectorQuery.fromFloatQuery(
              new KnnFloatVectorQuery("fvec", ta, 5, tq("g", "g3")), tq("tag", "t7"))
        });
    q.add(
        new Object[] {
          "seeded-byte k=10 B seed=tag:t1",
          SeededKnnVectorQuery.fromByteQuery(
              new KnnByteVectorQuery("bvec", bb, 10), tq("tag", "t1"))
        });
    q.add(
        new Object[] {
          "patience-float default k=10 X",
          PatienceKnnVectorQuery.fromFloatQuery(new KnnFloatVectorQuery("fvec", TARGET_X, 10))
        });
    q.add(
        new Object[] {
          "patience-float 0.9/5 k=10 A",
          PatienceKnnVectorQuery.fromFloatQuery(new KnnFloatVectorQuery("fvec", ta, 10), 0.9, 5)
        });
    q.add(
        new Object[] {
          "patience-byte default k=10 A",
          PatienceKnnVectorQuery.fromByteQuery(new KnnByteVectorQuery("bvec", ba, 10))
        });
    q.add(
        new Object[] {
          "patience-byte 0.99/10 k=20 B",
          PatienceKnnVectorQuery.fromByteQuery(new KnnByteVectorQuery("bvec", bb, 20), 0.99, 10)
        });
    q.add(
        new Object[] {
          "patience-seeded default k=10 X seed=tag:t5",
          PatienceKnnVectorQuery.fromSeededQuery(
              SeededKnnVectorQuery.fromFloatQuery(
                  new KnnFloatVectorQuery("fvec", TARGET_X, 10), tq("tag", "t5")))
        });
    q.add(
        new Object[] {"fsim A result=0.5", new FloatVectorSimilarityQuery("fvec", ta, 0.5f)});
    q.add(
        new Object[] {
          "fsim A result=0.002 decay=0",
          new FloatVectorSimilarityQuery("fvec", ta, 0.002f, 0f, null)
        });
    q.add(
        new Object[] {
          "fsim A result=0.002 decay=1",
          new FloatVectorSimilarityQuery("fvec", ta, 0.002f, 1f, null)
        });
    q.add(
        new Object[] {
          "fsim A result=0.002 filter=g:g3",
          new FloatVectorSimilarityQuery("fvec", ta, 0.002f, tq("g", "g3"))
        });
    q.add(
        new Object[] {"fsim C result=0.01", new FloatVectorSimilarityQuery("fvec", tc, 0.01f)});
    q.add(
        new Object[] {"fsim B result=0.002", new FloatVectorSimilarityQuery("fvec", tb, 0.002f)});
    q.add(
        new Object[] {"bsim A result=0.55", new ByteVectorSimilarityQuery("bvec", ba, 0.55f)});
    q.add(
        new Object[] {
          "bsim A result=0.55 decay=1",
          new ByteVectorSimilarityQuery("bvec", ba, 0.55f, 1f, null)
        });
    q.add(
        new Object[] {
          "bsim B result=0.55 filter=tag:t3",
          new ByteVectorSimilarityQuery("bvec", bb, 0.55f, tq("tag", "t3"))
        });
    q.add(
        new Object[] {
          "rescore knn-float k=20 X n=5",
          RescoreTopNQuery.createFullPrecisionRescorerQuery(
              new KnnFloatVectorQuery("fvec", TARGET_X, 20), TARGET_X, "fvec", 5)
        });
    q.add(
        new Object[] {
          "rescore tag:t4 A n=7",
          RescoreTopNQuery.createFullPrecisionRescorerQuery(tq("tag", "t4"), ta, "fvec", 7)
        });
    q.addAll(fallbackQueries(false));
    return q;
  }

  /**
   * The exact-search fallback queries. {@code rare:yes} matches at most 9 documents of a segment,
   * below every segment's per-leaf top k, so the leaf short-circuits to exact search. {@code
   * tag:t0 tag:t1 tag:t2} matches 240 documents of each large segment: above the number of nodes
   * the HNSW search is expected to visit for the per-leaf top k (29 for k = 10, {@code
   * HnswGraphSearcher.expectedVisitedNodes}), so the reader searches the graph instead of scoring
   * every accepted vector, with a visit limit of the filter cost plus one; the graph search reaches
   * that limit and the leaf falls back to exact search. With {@code probing}, the queries are
   * instances of {@link ProbeFloat} and {@link ProbeByte}.
   */
  static List<Object[]> fallbackQueries(boolean probing) {
    float[] ta = fvec(123);
    byte[] ba = bvec(123);
    List<Object[]> q = new ArrayList<>();
    q.add(
        new Object[] {
          "exact-short knn-float k=10 A filter=rare:yes",
          probing
              ? new ProbeFloat("fvec", ta, 10, tq("rare", "yes"))
              : new KnnFloatVectorQuery("fvec", ta, 10, tq("rare", "yes"))
        });
    q.add(
        new Object[] {
          "exact-fallback knn-float k=10 X filter=tag:t0|t1|t2",
          probing
              ? new ProbeFloat("fvec", TARGET_X, 10, tags012())
              : new KnnFloatVectorQuery("fvec", TARGET_X, 10, tags012())
        });
    q.add(
        new Object[] {
          "exact-fallback knn-byte k=10 A filter=tag:t0|t1|t2",
          probing
              ? new ProbeByte("bvec", ba, 10, tags012())
              : new KnnByteVectorQuery("bvec", ba, 10, tags012())
        });
    return q;
  }

  /** The calls the probes record, in order. */
  static final List<String> PROBE_LOG = new ArrayList<>();

  /** A {@link KnnFloatVectorQuery} that records its leaf searches. */
  static final class ProbeFloat extends KnnFloatVectorQuery {
    ProbeFloat(String field, float[] target, int k, Query filter) {
      super(field, target, k, filter);
    }

    @Override
    protected TopDocs approximateSearch(
        LeafReaderContext context,
        AcceptDocs acceptDocs,
        int visitedLimit,
        KnnCollectorManager knnCollectorManager)
        throws IOException {
      TopDocs results =
          super.approximateSearch(context, acceptDocs, visitedLimit, knnCollectorManager);
      PROBE_LOG.add(approximateLine(context, visitedLimit, results));
      return results;
    }

    @Override
    protected TopDocs exactSearch(
        LeafReaderContext context, DocIdSetIterator acceptIterator, QueryTimeout queryTimeout)
        throws IOException {
      PROBE_LOG.add("leaf=" + context.ord + " exact");
      return super.exactSearch(context, acceptIterator, queryTimeout);
    }
  }

  /** A {@link KnnByteVectorQuery} that records its leaf searches. */
  static final class ProbeByte extends KnnByteVectorQuery {
    ProbeByte(String field, byte[] target, int k, Query filter) {
      super(field, target, k, filter);
    }

    @Override
    protected TopDocs approximateSearch(
        LeafReaderContext context,
        AcceptDocs acceptDocs,
        int visitedLimit,
        KnnCollectorManager knnCollectorManager)
        throws IOException {
      TopDocs results =
          super.approximateSearch(context, acceptDocs, visitedLimit, knnCollectorManager);
      PROBE_LOG.add(approximateLine(context, visitedLimit, results));
      return results;
    }

    @Override
    protected TopDocs exactSearch(
        LeafReaderContext context, DocIdSetIterator acceptIterator, QueryTimeout queryTimeout)
        throws IOException {
      PROBE_LOG.add("leaf=" + context.ord + " exact");
      return super.exactSearch(context, acceptIterator, queryTimeout);
    }
  }

  static String approximateLine(LeafReaderContext context, int visitedLimit, TopDocs results) {
    return "leaf="
        + context.ord
        + " approximate visitedLimit="
        + visitedLimit
        + " hits="
        + results.scoreDocs.length
        + " relation="
        + results.totalHits.relation();
  }

  // ---------------------------------------------------------------------------------------------
  // Printing
  // ---------------------------------------------------------------------------------------------

  static String bits(float f) {
    return String.format("%08x", Float.floatToIntBits(f));
  }

  static void printHits(StoredFields stored, String header, TopDocs top) throws IOException {
    System.out.println(header + " total=" + top.totalHits.value() + " " + top.totalHits.relation());
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

  static void printReader(DirectoryReader reader) {
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
  }

  static void query(String root) throws IOException {
    System.out.println(
        "env hasFastScalarFma="
            + Constants.HAS_FAST_SCALAR_FMA
            + " incubatorVector="
            + ModuleLayer.boot().findModule("jdk.incubator.vector").isPresent());
    try (Directory dir = FSDirectory.open(Paths.get(root));
        DirectoryReader reader = DirectoryReader.open(dir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      StoredFields stored = searcher.storedFields();
      printReader(reader);
      for (Object[] q : queries()) {
        printHits(stored, "query " + q[0], searcher.search((Query) q[1], TOP_N));
      }
    }
  }

  static void probe(String root) throws IOException {
    try (Directory dir = FSDirectory.open(Paths.get(root));
        DirectoryReader reader = DirectoryReader.open(dir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      for (Object[] q : fallbackQueries(true)) {
        PROBE_LOG.clear();
        searcher.search((Query) q[1], TOP_N);
        for (String line : PROBE_LOG) {
          System.out.println("probe " + q[0] + " | " + line);
        }
      }
    }
  }

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "write" -> write(args[1]);
      case "query" -> query(args[1]);
      case "probe" -> probe(args[1]);
      default -> throw new IllegalArgumentException(args[0]);
    }
  }
}
