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
import org.apache.lucene.codecs.hnsw.DefaultFlatVectorScorer;
import org.apache.lucene.codecs.hnsw.FlatVectorScorerUtil;
import org.apache.lucene.codecs.hnsw.FlatVectorsScorer;
import org.apache.lucene.codecs.hnsw.ScalarQuantizedVectorScorer;
import org.apache.lucene.codecs.KnnVectorsFormat;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsFormat;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.ByteVectorValues;
import org.apache.lucene.index.FloatVectorValues;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.DocAndFloatFeatureBuffer;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.VectorScorer;
import org.apache.lucene.document.KnnByteVectorField;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.KnnByteVectorQuery;
import org.apache.lucene.search.KnnFloatVectorQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.store.ByteBuffersDataOutput;
import org.apache.lucene.store.ByteBuffersIndexInput;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.store.MMapDirectory;
import org.apache.lucene.util.hnsw.RandomVectorScorer;
import org.apache.lucene.util.hnsw.UpdateableRandomVectorScorer;
import org.apache.lucene.util.quantization.LegacyQuantizedByteVectorValues;
import org.apache.lucene.util.quantization.ScalarQuantizer;

/**
 * Reference tool for the kNN scorers of Apache Lucene Core 10.5.0, judged
 * against by {@code tests/knn_scorers.rs}.
 *
 * <ul>
 *   <li>{@code exact <dir>} writes six documents as two committed segments of
 *       three with the default codec, too few for any HNSW graph, so every
 *       {@code KnnFloatVectorQuery} and {@code KnnByteVectorQuery} below runs
 *       the exact search through {@code FloatVectorValues.scorer} and {@code
 *       ByteVectorValues.scorer}. Field {@code s} is sparse: only the even
 *       documents have it. It prints {@code hit <query> <doc> <score bits>}.
 *   <li>{@code legacy} scores in-memory {@code LegacyQuantizedByteVectorValues}
 *       through {@code FlatVectorScorerUtil.getLucene99ScalarQuantizedVectorsScorer()},
 *       the production entry point of the Lucene 9.9 quantized read path, and
 *       through {@code new ScalarQuantizedVectorScorer(DefaultFlatVectorScorer.INSTANCE)},
 *       and prints {@code <scorer> query|supplier <bits> <similarity> <ord>
 *       <doc> <score bits>}, where {@code <scorer>} is {@code lucene99} or
 *       {@code hnsw}.
 * </ul>
 */
public final class KnnScorerFixture {

  private KnnScorerFixture() {}

  public static void main(String[] args) throws IOException {
    switch (args[0]) {
      case "exact" -> exact(args[1]);
      case "empty" -> empty(args[1]);
      case "sq" -> sq(args[1]);
      case "legacy" -> legacy();
      case "legacy-mmap" -> legacyMmap(args[1]);
      default -> throw new IllegalArgumentException("unknown case " + args[0]);
    }
  }

  // ---------------------------------------------------------------------------
  // exact
  // ---------------------------------------------------------------------------

  private static void exact(String dir) throws IOException {
    try (FSDirectory directory = FSDirectory.open(Paths.get(dir))) {
      IndexWriterConfig config = new IndexWriterConfig().setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(directory, config)) {
        for (int i = 0; i < 6; i++) {
          writer.addDocument(document(i));
          if (i % 3 == 2) {
            writer.commit();
          }
        }
      }
      try (DirectoryReader reader = DirectoryReader.open(directory)) {
        if (reader.leaves().size() != 2) {
          throw new IllegalStateException("expected two segments, got " + reader.leaves().size());
        }
        IndexSearcher searcher = new IndexSearcher(reader);
        search(searcher, "f3", new KnnFloatVectorQuery("f", new float[] {0f, 1f}, 3), 3);
        search(searcher, "f10", new KnnFloatVectorQuery("f", new float[] {0f, 1f}, 10), 10);
        search(searcher, "c3", new KnnFloatVectorQuery("c", new float[] {1f, 1f}, 3), 3);
        search(searcher, "m4", new KnnFloatVectorQuery("m", new float[] {0.5f, 1f}, 4), 4);
        search(searcher, "s3", new KnnFloatVectorQuery("s", new float[] {0f, 1f}, 3), 3);
        search(searcher, "b3", new KnnByteVectorQuery("b", new byte[] {0, 1}, 3), 3);
        search(searcher, "bd3", new KnnByteVectorQuery("bd", new byte[] {1, 2}, 3), 3);
        // A filter whose cost per segment is at most perLeafTopK forces
        // AbstractKnnVectorQuery.exactSearch, which scores through
        // VectorScorer.bulk.
        Query even = new TermQuery(new Term("tag", "even"));
        Query odd = new TermQuery(new Term("tag", "odd"));
        search(searcher, "f3e", new KnnFloatVectorQuery("f", new float[] {0f, 1f}, 3, even), 3);
        search(searcher, "c3o", new KnnFloatVectorQuery("c", new float[] {1f, 1f}, 3, odd), 3);
        search(searcher, "m4o", new KnnFloatVectorQuery("m", new float[] {0.5f, 1f}, 4, odd), 4);
        search(searcher, "s3e", new KnnFloatVectorQuery("s", new float[] {0f, 1f}, 3, even), 3);
        search(searcher, "b3e", new KnnByteVectorQuery("b", new byte[] {0, 1}, 3, even), 3);
        search(searcher, "bd3o", new KnnByteVectorQuery("bd", new byte[] {1, 2}, 3, odd), 3);
        for (LeafReaderContext leaf : reader.leaves()) {
          dumpFloat(leaf, "f", new float[] {0f, 1f});
          dumpFloat(leaf, "s", new float[] {0f, 1f});
          dumpByte(leaf, "b", new byte[] {0, 1});
          dumpByte(leaf, "bd", new byte[] {1, 2});
        }
      }
    }
  }

  /** Prints what {@code values.scorer(query)} answers, document by document and in bulk. */
  static void dumpFloat(LeafReaderContext leaf, String field, float[] query) throws IOException {
    FloatVectorValues values = leaf.reader().getFloatVectorValues(field);
    dump(leaf.ord, field, values == null ? null : values.scorer(query), values == null ? null : values.scorer(query));
  }

  static void dumpByte(LeafReaderContext leaf, String field, byte[] query) throws IOException {
    ByteVectorValues values = leaf.reader().getByteVectorValues(field);
    dump(leaf.ord, field, values == null ? null : values.scorer(query), values == null ? null : values.scorer(query));
  }

  private static void dump(int leaf, String field, VectorScorer one, VectorScorer bulk)
      throws IOException {
    if (one == null) {
      System.out.println("noscorer " + leaf + " " + field);
      return;
    }
    DocIdSetIterator it = one.iterator();
    for (int doc = it.nextDoc(); doc != DocIdSetIterator.NO_MORE_DOCS; doc = it.nextDoc()) {
      System.out.println("score " + leaf + " " + field + " " + doc + " " + bits(one.score()));
    }
    VectorScorer.Bulk b = bulk.bulk(null);
    DocAndFloatFeatureBuffer buffer = new DocAndFloatFeatureBuffer();
    while (true) {
      float max = b.nextDocsAndScores(DocIdSetIterator.NO_MORE_DOCS, null, buffer);
      if (buffer.size == 0) {
        break;
      }
      System.out.println("bulkmax " + leaf + " " + field + " " + bits(max));
      for (int i = 0; i < buffer.size; i++) {
        System.out.println(
            "bulk " + leaf + " " + field + " " + buffer.docs[i] + " " + bits(buffer.features[i]));
      }
    }
  }

  static String bits(float f) {
    return Integer.toHexString(Float.floatToRawIntBits(f));
  }

  // ---------------------------------------------------------------------------
  // empty: a segment whose field infos list a vector field with no vector
  // ---------------------------------------------------------------------------

  private static void empty(String dir) throws IOException {
    try (FSDirectory directory = FSDirectory.open(Paths.get(dir))) {
      try (IndexWriter writer = new IndexWriter(directory, new IndexWriterConfig())) {
        for (int i = 0; i < 4; i++) {
          Document doc = new Document();
          doc.add(new StringField("id", "doc-" + i, Field.Store.NO));
          doc.add(new KnnFloatVectorField("f", new float[] {i, 1f}));
          if (i == 0) {
            doc.add(new KnnFloatVectorField("e", new float[] {1f, 1f}));
          }
          writer.addDocument(doc);
        }
        writer.commit();
        writer.deleteDocuments(new Term("id", "doc-0"));
        writer.forceMerge(1);
      }
      try (DirectoryReader reader = DirectoryReader.open(directory)) {
        LeafReaderContext leaf = reader.leaves().get(0);
        System.out.println(
            "leaves " + reader.leaves().size() + " maxDoc " + reader.maxDoc() + " hasE "
                + (leaf.reader().getFieldInfos().fieldInfo("e") != null));
        FloatVectorValues values = leaf.reader().getFloatVectorValues("e");
        System.out.println("esize " + (values == null ? "null" : values.size()));
        IndexSearcher searcher = new IndexSearcher(reader);
        Query all = new TermQuery(new Term("id", "doc-1"));
        search(searcher, "e1", new KnnFloatVectorQuery("e", new float[] {1f, 1f}, 3, all), 3);
        dumpFloat(leaf, "e", new float[] {1f, 1f});
      }
    }
  }

  // ---------------------------------------------------------------------------
  // sq: the Lucene 10.4 scalar-quantized format, dense and sparse
  // ---------------------------------------------------------------------------

  static final int SQ_DOCS = 40;

  /** Whether document {@code d} has field {@code q}: an irregular sparse pattern. */
  static boolean sqHasQ(int d) {
    return (d * 7) % 5 < 3;
  }

  private static void sq(String dir) throws IOException {
    try (FSDirectory directory = FSDirectory.open(Paths.get(dir))) {
      IndexWriterConfig config =
          new IndexWriterConfig()
              .setMergePolicy(NoMergePolicy.INSTANCE)
              .setCodec(
                  new Lucene104Codec() {
                    @Override
                    public KnnVectorsFormat getKnnVectorsFormatForField(String field) {
                      return new Lucene104ScalarQuantizedVectorsFormat();
                    }
                  });
      try (IndexWriter writer = new IndexWriter(directory, config)) {
        for (int d = 0; d < SQ_DOCS; d++) {
          Document doc = new Document();
          doc.add(new StringField("id", "doc-" + d, Field.Store.NO));
          doc.add(new StringField("tag", d % 2 == 0 ? "even" : "odd", Field.Store.NO));
          doc.add(new IntPoint("rank", d));
          doc.add(new KnnFloatVectorField("qd", new float[] {d * 0.5f, 3f - d * 0.25f, 1f}));
          if (sqHasQ(d)) {
            doc.add(new KnnFloatVectorField("q", new float[] {d, 0.5f * d, 2f}));
          }
          writer.addDocument(doc);
        }
        writer.commit();
        writer.deleteDocuments(new Term("id", "doc-21"), new Term("id", "doc-30"));
        writer.commit();
      }
      try (DirectoryReader reader = DirectoryReader.open(directory)) {
        IndexSearcher searcher = new IndexSearcher(reader);
        float[] target = {20f, 10f, 2f};
        search(searcher, "q5", new KnnFloatVectorQuery("q", target, 5), 5);
        search(
            searcher, "q5e", new KnnFloatVectorQuery("q", target, 5, new TermQuery(new Term("tag", "even"))), 5);
        search(
            searcher, "q5r", new KnnFloatVectorQuery("q", target, 5, IntPoint.newRangeQuery("rank", 18, 23)), 5);
        search(
            searcher, "qd4r", new KnnFloatVectorQuery("qd", new float[] {5f, 1f, 1f}, 4, IntPoint.newRangeQuery("rank", 8, 11)), 4);
        for (LeafReaderContext leaf : reader.leaves()) {
          dumpFloat(leaf, "q", target);
          dumpFloat(leaf, "qd", new float[] {5f, 1f, 1f});
        }
      }
    }
  }

  /** The fields of document {@code i}; mirrored by {@code document} in the Rust test. */
  static Document document(int i) {
    Document doc = new Document();
    doc.add(new StringField("tag", i % 2 == 0 ? "even" : "odd", Field.Store.NO));
    doc.add(new KnnFloatVectorField("f", new float[] {i, 1f + 0.5f * i}));
    doc.add(
        new KnnFloatVectorField(
            "c", new float[] {1f + i, 2f}, VectorSimilarityFunction.COSINE));
    doc.add(
        new KnnFloatVectorField(
            "m", new float[] {0.25f * i, 1f - 0.1f * i}, VectorSimilarityFunction.MAXIMUM_INNER_PRODUCT));
    if (i % 2 == 0) {
      doc.add(new KnnFloatVectorField("s", new float[] {i, 2f - i}));
    }
    doc.add(new KnnByteVectorField("b", new byte[] {(byte) i, (byte) (2 * i + 1)}));
    doc.add(
        new KnnByteVectorField(
            "bd", new byte[] {(byte) (3 - i), (byte) (i + 1)}, VectorSimilarityFunction.DOT_PRODUCT));
    return doc;
  }

  private static void search(IndexSearcher searcher, String name, Query query, int n)
      throws IOException {
    TopDocs topDocs = searcher.search(query, n);
    System.out.println("total " + name + " " + topDocs.totalHits.value());
    for (ScoreDoc hit : topDocs.scoreDocs) {
      System.out.println(
          "hit " + name + " " + hit.doc + " " + Integer.toHexString(Float.floatToRawIntBits(hit.score)));
    }
  }

  // ---------------------------------------------------------------------------
  // legacy
  // ---------------------------------------------------------------------------

  static final int DIMENSION = 4;
  static final int SIZE = 5;

  /** Component {@code j} of stored vector {@code ord}; mirrored in the Rust test. */
  static float component(int ord, int j) {
    return ((ord * 7 + j * 3) % 9) / 2f - 1f;
  }

  static final float[] QUERY = {0.75f, -0.5f, 1.25f, 0.25f};

  private static void legacy() throws IOException {
    legacy("lucene99", FlatVectorScorerUtil.getLucene99ScalarQuantizedVectorsScorer());
    legacy("hnsw", new ScalarQuantizedVectorScorer(DefaultFlatVectorScorer.INSTANCE));
  }

  private static void legacy(String name, FlatVectorsScorer scorer) throws IOException {
    for (byte bits : new byte[] {7, 4}) {
      ScalarQuantizer quantizer = new ScalarQuantizer(-1f, 3f, bits);
      for (VectorSimilarityFunction similarity : VectorSimilarityFunction.values()) {
        byte[][] vectors = new byte[SIZE][DIMENSION];
        float[] offsets = new float[SIZE];
        for (int ord = 0; ord < SIZE; ord++) {
          float[] raw = new float[DIMENSION];
          for (int j = 0; j < DIMENSION; j++) {
            raw[j] = component(ord, j);
          }
          offsets[ord] = quantizer.quantize(raw, vectors[ord], similarity);
        }
        score(name, "", bits, similarity, scorer, new MemoryValues(quantizer, vectors, offsets, null));
        if (bits == 4) {
          // Four-bit components packed two to a byte behind a slice: the case
          // Lucene99ScalarQuantizedVectorScorer.dotProductFactory routes to
          // CompressedInt4DotProduct.
          ByteBuffersDataOutput out = new ByteBuffersDataOutput();
          for (int ord = 0; ord < SIZE; ord++) {
            byte[] packed = new byte[(DIMENSION + 1) >> 1];
            for (int i = 0; i < packed.length; i++) {
              packed[i] = (byte) ((vectors[ord][i] << 4) | vectors[ord][packed.length + i]);
            }
            out.writeBytes(packed, packed.length);
            out.writeInt(Float.floatToIntBits(offsets[ord]));
          }
          IndexInput slice = new ByteBuffersIndexInput(out.toDataInput(), "compressed");
          score(name, "c", bits, similarity, scorer, new MemoryValues(quantizer, vectors, offsets, slice));
        }
      }
    }
  }

  /**
   * Scores the same values with their nodes in a memory-mapped file, so that the
   * slice is a {@code MemorySegmentAccessInput}. Run with {@code --add-modules
   * jdk.incubator.vector}, {@code FlatVectorScorerUtil} then answers {@code
   * Lucene99MemorySegmentScalarQuantizedVectorScorer}, which scores straight from
   * the mapped nodes.
   */
  private static void legacyMmap(String dir) throws IOException {
    FlatVectorsScorer scorer = FlatVectorScorerUtil.getLucene99ScalarQuantizedVectorsScorer();
    System.out.println("scorer " + scorer);
    int file = 0;
    try (MMapDirectory directory = new MMapDirectory(Paths.get(dir))) {
      for (byte bits : new byte[] {7, 4}) {
        ScalarQuantizer quantizer = new ScalarQuantizer(-1f, 3f, bits);
        for (VectorSimilarityFunction similarity : VectorSimilarityFunction.values()) {
          byte[][] vectors = new byte[SIZE][DIMENSION];
          float[] offsets = new float[SIZE];
          for (int ord = 0; ord < SIZE; ord++) {
            float[] raw = new float[DIMENSION];
            for (int j = 0; j < DIMENSION; j++) {
              raw[j] = component(ord, j);
            }
            offsets[ord] = quantizer.quantize(raw, vectors[ord], similarity);
          }
          for (boolean packed : bits == 4 ? new boolean[] {false, true} : new boolean[] {false}) {
            String name = "nodes" + file++;
            try (IndexOutput out = directory.createOutput(name, IOContext.DEFAULT)) {
              for (int ord = 0; ord < SIZE; ord++) {
                if (packed) {
                  byte[] p = new byte[(DIMENSION + 1) >> 1];
                  for (int i = 0; i < p.length; i++) {
                    p[i] = (byte) ((vectors[ord][i] << 4) | vectors[ord][p.length + i]);
                  }
                  out.writeBytes(p, p.length);
                } else {
                  out.writeBytes(vectors[ord], DIMENSION);
                }
                out.writeInt(Float.floatToIntBits(offsets[ord]));
              }
            }
            IndexInput slice = directory.openInput(name, IOContext.DEFAULT);
            MemoryValues values = new MemoryValues(quantizer, vectors, offsets, slice);
            values.packed = packed;
            score("lucene99", packed ? "c" : "", bits, similarity, scorer, values);
            slice.close();
          }
        }
      }
    }
  }

  private static void score(
      String name,
      String prefix,
      byte bits,
      VectorSimilarityFunction similarity,
      FlatVectorsScorer scorer,
      LegacyQuantizedByteVectorValues values)
      throws IOException {
    RandomVectorScorer query = scorer.getRandomVectorScorer(similarity, values, QUERY);
    for (int ord = 0; ord < SIZE; ord++) {
      print(name, prefix + "query", bits, similarity, ord, query.ordToDoc(ord), query.score(ord));
    }
    UpdateableRandomVectorScorer supplied =
        scorer.getRandomVectorScorerSupplier(similarity, values).scorer();
    supplied.setScoringOrdinal(0);
    for (int ord = 0; ord < SIZE; ord++) {
      print(name, prefix + "supplier", bits, similarity, ord, supplied.ordToDoc(ord), supplied.score(ord));
    }
  }

  private static void print(
      String name,
      String kind,
      byte bits,
      VectorSimilarityFunction similarity,
      int ord,
      int doc,
      float score) {
    System.out.println(
        name
            + " "
            + kind
            + " "
            + bits
            + " "
            + similarity
            + " "
            + ord
            + " "
            + doc
            + " "
            + Integer.toHexString(Float.floatToRawIntBits(score)));
  }

  /** Quantized vectors held in memory; ordinal {@code ord} belongs to document {@code 2 * ord + 1}. */
  private static final class MemoryValues extends LegacyQuantizedByteVectorValues {
    private final ScalarQuantizer quantizer;
    private final byte[][] vectors;
    private final float[] offsets;
    private final IndexInput slice;
    private Boolean packed;

    MemoryValues(ScalarQuantizer quantizer, byte[][] vectors, float[] offsets, IndexInput slice) {
      this.quantizer = quantizer;
      this.vectors = vectors;
      this.offsets = offsets;
      this.slice = slice;
    }

    @Override
    public IndexInput getSlice() {
      return slice;
    }

    @Override
    public int getVectorByteLength() {
      boolean isPacked = packed == null ? slice != null : packed;
      return isPacked ? (DIMENSION + 1) >> 1 : DIMENSION;
    }

    @Override
    public ScalarQuantizer getScalarQuantizer() {
      return quantizer;
    }

    @Override
    public float getScoreCorrectionConstant(int ord) {
      return offsets[ord];
    }

    @Override
    public byte[] vectorValue(int ord) {
      return vectors[ord];
    }

    @Override
    public int dimension() {
      return DIMENSION;
    }

    @Override
    public int size() {
      return SIZE;
    }

    @Override
    public int ordToDoc(int ord) {
      return 2 * ord + 1;
    }
  }
}
