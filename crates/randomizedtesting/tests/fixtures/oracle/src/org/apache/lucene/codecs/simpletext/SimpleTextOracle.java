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

package org.apache.lucene.codecs.simpletext;

import java.io.File;
import java.io.IOException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.stream.Stream;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.analysis.tokenattributes.OffsetAttribute;
import org.apache.lucene.analysis.tokenattributes.PayloadAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.codecs.PostingsFormat;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.BinaryPoint;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.DoublePoint;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.FloatPoint;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.KnnByteVectorField;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.TieredMergePolicy;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.NIOFSDirectory;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.codecs.MutablePointTree;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.PointValues;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.search.SortedNumericSortField;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.util.BytesRefBuilder;
import org.apache.lucene.util.Version;
import org.apache.lucene.util.bkd.BKDConfig;
import java.util.Map;

/**
 * Oracle of {@code crates/rucene-codecs/tests/simpletext_oracle.rs}: writes indexes with {@code
 * SimpleTextCodec} from fixed documents and operations that the Rust test reproduces with the same
 * formulas, and records into {@code <out>/rucene-codecs/}:
 *
 * <ul>
 *   <li>{@code simpletext_files.tsv}: every file of every index, {@code case, name, length, FNV-1a
 *       64 digest, content} (bytes outside printable ASCII escaped as {@code \xHH}, backslash as
 *       {@code \\});
 *   <li>{@code simpletext_reads.tsv}: what Lucene reads back ({@link SimpleTextReadOracle});
 *   <li>{@code simpletext_errors.tsv}: the exception a corrupted copy of an index raises on open,
 *       {@code case, class, message} with the directory masked as {@code $D};
 *   <li>{@code simpletext_spi.tsv}: the service lookups.
 * </ul>
 */
public final class SimpleTextOracle {

  /** Documents of each batch of the corpus, in the order they are added. */
  static final int[][] BATCHES = {{0, 120}, {120, 240}, {240, 320}, {320, 360}};

  static final FieldType BODY_TYPE = new FieldType();
  static final FieldType TV_TYPE = new FieldType();
  static final FieldType DOCS_TYPE = new FieldType();
  static final FieldType FREQS_TYPE = new FieldType();
  static final FieldType TVP_TYPE = new FieldType();
  static final FieldType TVO_TYPE = new FieldType();
  static final FieldType PDATA_TYPE = new FieldType();

  static {
    BODY_TYPE.setTokenized(true);
    BODY_TYPE.setStored(true);
    BODY_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    BODY_TYPE.freeze();

    TV_TYPE.setTokenized(true);
    TV_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    TV_TYPE.setStoreTermVectors(true);
    TV_TYPE.setStoreTermVectorPositions(true);
    TV_TYPE.setStoreTermVectorOffsets(true);
    TV_TYPE.setStoreTermVectorPayloads(true);
    TV_TYPE.freeze();

    DOCS_TYPE.setTokenized(true);
    DOCS_TYPE.setIndexOptions(IndexOptions.DOCS);
    DOCS_TYPE.freeze();

    FREQS_TYPE.setTokenized(true);
    FREQS_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS);
    FREQS_TYPE.freeze();

    TVP_TYPE.setTokenized(true);
    TVP_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    TVP_TYPE.setStoreTermVectors(true);
    TVP_TYPE.setStoreTermVectorPositions(true);
    TVP_TYPE.freeze();

    TVO_TYPE.setTokenized(true);
    TVO_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS);
    TVO_TYPE.setStoreTermVectors(true);
    TVO_TYPE.setStoreTermVectorOffsets(true);
    TVO_TYPE.freeze();

    PDATA_TYPE.setDimensions(3, 2, 4);
    PDATA_TYPE.freeze();
  }

  private SimpleTextOracle() {}

  // -------------------------------------------------------------------------------------------
  // The corpus (the documents and operations of IndexInteropFixture)
  // -------------------------------------------------------------------------------------------

  static String id(int i) {
    return String.format("id-%04d", i);
  }

  static String body(int i) {
    StringBuilder sb = new StringBuilder();
    int words = i % 7 + 1;
    for (int j = 0; j < words; j++) {
      if (j > 0) {
        sb.append(' ');
      }
      sb.append('w').append((i * 31 + j * 17) % 50);
    }
    return sb.toString();
  }

  record Tok(String term, int start, int end, byte[] payload) {}

  static List<Tok> tvTokens(int i) {
    List<Tok> tokens = new ArrayList<>();
    for (int k = 0; k < i % 4 + 1; k++) {
      byte[] payload = (i + k) % 3 == 0 ? null : new byte[] {(byte) (i + k), (byte) k};
      tokens.add(new Tok("t" + ((i + k * 3) % 9), k * 4, k * 4 + 2, payload));
    }
    return tokens;
  }

  static float[] floatVector(int i) {
    return new float[] {
      ((i * 3) % 17 - 8) / 2f, ((i * 5) % 13 - 6) / 4f, (i % 7) / 2f, ((i * 11) % 19) / 8f
    };
  }

  static byte[] byteVector(int i) {
    return new byte[] {
      (byte) ((i * 3) % 50), (byte) ((i * 7) % 60 - 30), (byte) (i % 20), (byte) ((i * 13) % 40 - 20)
    };
  }

  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", id(i), Field.Store.YES));
    d.add(new Field("body", body(i), BODY_TYPE));
    d.add(new Field("tv", new ScriptedTokenStream(tvTokens(i)), TV_TYPE));
    d.add(new StoredField("s_int", i * 3 - 100));
    d.add(new StoredField("s_long", i * 1_000_000_007L));
    d.add(new StoredField("s_float", i / 4f));
    d.add(new StoredField("s_double", i / 8d));
    d.add(new StoredField("s_bytes", new BytesRef(new byte[] {(byte) i, (byte) (i >> 8), 7})));
    d.add(new StoredField("s_str", "stored-" + i));
    if (i % 11 != 0) {
      d.add(new NumericDocValuesField("dv_num", (i * 37) % 101 - 50));
    }
    if (i % 5 != 0) {
      d.add(new BinaryDocValuesField("dv_bin", new BytesRef("bin-" + (i * 7 % 13))));
    }
    if (i % 6 != 0) {
      d.add(new SortedDocValuesField("dv_sorted", new BytesRef("s" + (i * 13 % 29))));
    }
    d.add(new SortedNumericDocValuesField("dv_snum", i % 10));
    d.add(new SortedNumericDocValuesField("dv_snum", (i * 3) % 17));
    d.add(new SortedSetDocValuesField("dv_sset", new BytesRef("a" + (i % 5))));
    d.add(new SortedSetDocValuesField("dv_sset", new BytesRef("b" + (i % 3))));
    d.add(new IntPoint("p_int", (i * 7) % 200));
    d.add(new LongPoint("p_long", i * 100_000L - 5_000_000L));
    d.add(new FloatPoint("p_float", i * 0.5f - 40f));
    d.add(new DoublePoint("p_double", i * 0.25 - 10));
    d.add(new IntPoint("p_2d", i % 20, i / 20));
    if (i % 9 != 0) {
      d.add(new KnnFloatVectorField("vf", floatVector(i), VectorSimilarityFunction.EUCLIDEAN));
    }
    d.add(new KnnByteVectorField("vb", byteVector(i), VectorSimilarityFunction.EUCLIDEAN));
    return d;
  }

  static void addBatch(IndexWriter w, int batch) throws IOException {
    for (int i = BATCHES[batch][0]; i < BATCHES[batch][1]; i++) {
      w.addDocument(doc(i));
    }
  }

  /** The corpus operations; {@code simpletext_oracle.rs::write_corpus} performs the same. */
  static void writeCorpus(Directory dir, boolean compound) throws IOException {
    writeCorpus(dir, compound, new SimpleTextCodec());
  }

  /** {@link #writeCorpus(Directory, boolean)} with {@code codec} ({@link PostingsFormatOracle}). */
  public static void writeCorpus(Directory dir, boolean compound, Codec codec) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setCodec(codec);
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    if (!compound) {
      TieredMergePolicy policy = new TieredMergePolicy();
      policy.setNoCFSRatio(0.0);
      config.setMergePolicy(policy);
      config.setUseCompoundFile(false);
    }
    try (IndexWriter w = new IndexWriter(dir, config)) {
      addBatch(w, 0);
      w.commit();
      w.deleteDocuments(new Term("id", id(3)));
      w.deleteDocuments(new Term("id", id(17)));
      w.deleteDocuments(IntPoint.newRangeQuery("p_int", 50, 60));
      addBatch(w, 1);
      w.updateNumericDocValue(new Term("id", id(5)), "dv_num", 777);
      w.updateBinaryDocValue(new Term("id", id(130)), "dv_bin", new BytesRef("updated-130"));
      w.commit();
      addBatch(w, 2);
      w.commit();
      w.forceMerge(2);
      addBatch(w, 3);
      w.deleteDocuments(new Term("id", id(250)));
      w.deleteDocuments(IntPoint.newRangeQuery("p_int", 190, 192));
      w.updateNumericDocValue(new Term("id", id(300)), "dv_num", -999);
      w.updateBinaryDocValue(new Term("id", id(10)), "dv_bin", new BytesRef("updated-10"));
      w.commit();
      w.deleteDocuments(IntPoint.newRangeQuery("p_2d", new int[] {0, 3}, new int[] {19, 4}));
      w.forceMergeDeletes();
      w.commit();
    }
  }

  // -------------------------------------------------------------------------------------------
  // The edge documents: every value kind at its limits
  // -------------------------------------------------------------------------------------------

  static final int EDGE_DOCS = 12;

  static final int[] INTS = {Integer.MIN_VALUE, Integer.MAX_VALUE, 0, -1, 7};
  static final long[] LONGS = {Long.MIN_VALUE, Long.MAX_VALUE, 0, -1, 1_000_000_007L};
  static final float[] FLOATS = {
    Float.NaN,
    -0.0f,
    0.0f,
    Float.POSITIVE_INFINITY,
    Float.NEGATIVE_INFINITY,
    Float.MAX_VALUE,
    1.0E-30f,
    123456.78f,
    1.0E7f,
    0.001f,
    9.999999E-4f,
    9999999.0f
  };
  static final double[] DOUBLES = {
    Double.NaN,
    -0.0,
    0.0,
    Double.POSITIVE_INFINITY,
    Double.NEGATIVE_INFINITY,
    Double.MAX_VALUE,
    1.0E-300,
    123456.789,
    1.0E7,
    0.001,
    9.99999999999E-4,
    9999999.999
  };
  static final long[] NUMS = {Long.MIN_VALUE, Long.MAX_VALUE, -1, 0, 42, Long.MIN_VALUE + 1};
  static final float[][] UNIT = {
    {1f, 0f, 0f}, {0f, 1f, 0f}, {0f, 0f, 1f}, {0.6f, 0.8f, 0f}, {0f, -0.6f, 0.8f}, {-1f, 0f, 0f}
  };

  static String edgeText(int i) {
    if (i == 0) {
      return "";
    }
    if (i == 1) {
      return "héllo wörld 日本";
    }
    return "edge" + i + " common words" + (i % 3 == 0 ? " common" : "");
  }

  static String edgeStr(int i) {
    return switch (i) {
      case 0 -> "";
      case 1 -> "naïve-日本-😀";
      case 2 -> "line\nbreak\\slash";
      default -> "s" + (i % 4);
    };
  }

  static String edgeStoredStr(int i) {
    return switch (i) {
      case 0 -> "";
      case 1 -> "tab\there\nnew\\line ünï😀";
      default -> "v" + i;
    };
  }

  static byte[] edgeBytes(int i) {
    return switch (i) {
      case 0 -> new byte[0];
      case 1 -> new byte[] {0x0a, 0x5c, (byte) 0xff, 0x00};
      default -> new byte[] {(byte) i, 0x0a};
    };
  }

  static float[] euclideanVector(int i) {
    float[] u = UNIT[i % 6];
    return new float[] {u[0] * (i + 1) / 4f, u[1] == 0f && i == 0 ? -0.0f : u[1], u[2] + 2.5E-5f};
  }

  static byte[] edgeByteVector(int i, int salt) {
    return new byte[] {(byte) (i * 3 - 5 + salt), (byte) (-i), (byte) ((i * i + salt) % 100)};
  }

  static Document edgeDoc(int i) {
    Document d = new Document();
    d.add(new StringField("e_id", "e" + i, Field.Store.YES));
    d.add(new TextField("e_text", edgeText(i), Field.Store.NO));
    d.add(new StringField("e_str", edgeStr(i), Field.Store.YES));
    d.add(new Field("e_docs", "d" + (i % 3) + " shared", DOCS_TYPE));
    d.add(new Field("e_freqs", "f" + (i % 2) + " f" + (i % 2) + " g", FREQS_TYPE));
    d.add(new Field("e_tvp", "tv" + (i % 3) + " pos", TVP_TYPE));
    d.add(new Field("e_tvo", "off" + (i % 2) + " set", TVO_TYPE));
    d.add(new StoredField("e_int", INTS[i % INTS.length]));
    d.add(new StoredField("e_long", LONGS[i % LONGS.length]));
    d.add(new StoredField("e_float", FLOATS[i % FLOATS.length]));
    d.add(new StoredField("e_double", DOUBLES[i % DOUBLES.length]));
    d.add(new StoredField("e_bytes", new BytesRef(edgeBytes(i))));
    d.add(new StoredField("e_sstr", edgeStoredStr(i)));
    if (i % 7 != 3) {
      d.add(NumericDocValuesField.indexedField("e_num", NUMS[i % NUMS.length]));
    }
    d.add(new NumericDocValuesField("e_num2", i * 3 - 5));
    d.add(new NumericDocValuesField("e_sortkey", (i * 7) % 12));
    if (i != 3) {
      byte[] bin = i == 0 ? new byte[0] : i == 1 ? new byte[] {0x0a, 0x5c} : ("bin" + i).getBytes(StandardCharsets.UTF_8);
      d.add(new BinaryDocValuesField("e_bin", new BytesRef(bin)));
    }
    if (i != 2) {
      d.add(SortedDocValuesField.indexedField("e_sorted", new BytesRef("ü" + (i % 3))));
    }
    if (i != 5) {
      d.add(SortedSetDocValuesField.indexedField("e_sset", new BytesRef("x" + (i % 4))));
      d.add(SortedSetDocValuesField.indexedField("e_sset", new BytesRef("y" + (i % 2))));
      d.add(SortedSetDocValuesField.indexedField("e_sset", new BytesRef("x" + (i % 4))));
    }
    if (i != 6) {
      long first = i % 4 == 0 ? Long.MIN_VALUE : i % 4 == 1 ? Long.MAX_VALUE : i;
      d.add(SortedNumericDocValuesField.indexedField("e_snum", first));
      d.add(SortedNumericDocValuesField.indexedField("e_snum", -i));
    }
    if (i == 7) {
      d.add(new NumericDocValuesField("e_soft", 1));
    }
    d.add(new BinaryPoint("e_bp1", new byte[] {(byte) (i * 20)}));
    d.add(
        new BinaryPoint(
            "e_bp3",
            new byte[] {(byte) i, (byte) (i * 2), (byte) (255 - i)},
            new byte[] {7, (byte) i, 0}));
    byte[] b16 = new byte[16];
    for (int k = 0; k < 16; k++) {
      b16[k] = (byte) (i * 17 + k * 31);
    }
    d.add(new BinaryPoint("e_bp16", b16));
    d.add(new IntPoint("e_int3", i, -i, i * 1000));
    d.add(new LongPoint("e_lp", LONGS[i % LONGS.length]));
    byte[] pdata = new byte[12];
    for (int k = 0; k < 12; k++) {
      pdata[k] = (byte) (i * 5 + k);
    }
    d.add(new BinaryPoint("e_pdata", pdata, PDATA_TYPE));
    if (i != 4) {
      d.add(
          new KnnFloatVectorField("e_vf_euc", euclideanVector(i), VectorSimilarityFunction.EUCLIDEAN));
    }
    d.add(new KnnFloatVectorField("e_vf_dot", UNIT[i % 6], VectorSimilarityFunction.DOT_PRODUCT));
    d.add(new KnnFloatVectorField("e_vf_cos", UNIT[(i + 1) % 6], VectorSimilarityFunction.COSINE));
    d.add(
        new KnnFloatVectorField(
            "e_vf_mip", UNIT[(i + 2) % 6], VectorSimilarityFunction.MAXIMUM_INNER_PRODUCT));
    d.add(new KnnByteVectorField("e_vb_euc", edgeByteVector(i, 0), VectorSimilarityFunction.EUCLIDEAN));
    d.add(new KnnByteVectorField("e_vb_dot", edgeByteVector(i, 1), VectorSimilarityFunction.DOT_PRODUCT));
    d.add(new KnnByteVectorField("e_vb_cos", edgeByteVector(i, 2), VectorSimilarityFunction.COSINE));
    d.add(
        new KnnByteVectorField(
            "e_vb_mip", edgeByteVector(i, 3), VectorSimilarityFunction.MAXIMUM_INNER_PRODUCT));
    return d;
  }

  /** The edge operations; {@code simpletext_oracle.rs::write_edge} performs the same. */
  public static void writeEdge(Directory dir) throws IOException {
    writeEdge(dir, new SimpleTextCodec());
  }

  /** {@link #writeEdge(Directory)} with {@code codec} ({@link PostingsFormatOracle}). */
  public static void writeEdge(Directory dir, Codec codec) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setCodec(codec);
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    config.setUseCompoundFile(false);
    config.setSoftDeletesField("e_soft");
    config.setIndexSort(new Sort(new SortField("e_sortkey", SortField.Type.LONG)));
    try (IndexWriter w = new IndexWriter(dir, config)) {
      for (int i = 0; i < EDGE_DOCS; i++) {
        w.addDocument(edgeDoc(i));
      }
      w.deleteDocuments(new Term("e_id", "e9"));
      w.commit();
    }
  }


  // -------------------------------------------------------------------------------------------
  // Format-level files: a pinned segment info and field infos, and SimpleTextBKDWriter's
  // writeField (1-D and N-D) and add/finish paths through an offline spill
  // -------------------------------------------------------------------------------------------

  static final byte[] SEGMENT_ID = {
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78,
    (byte) 0x87, (byte) 0x96, (byte) 0xa5, (byte) 0xb4, (byte) 0xc3, (byte) 0xd2, (byte) 0xe1,
    (byte) 0xf0
  };

  /** A mutable point tree over arrays, the shape of PointValuesWriter's. */
  static final class ArrayTree extends MutablePointTree {
    final byte[][] values;
    final int[] docs;
    final byte[][] savedValues;
    final int[] savedDocs;

    ArrayTree(byte[][] values, int[] docs) {
      this.values = values;
      this.docs = docs;
      this.savedValues = new byte[values.length][];
      this.savedDocs = new int[docs.length];
    }

    @Override
    public void getValue(int i, BytesRef packedValue) {
      packedValue.bytes = values[i];
      packedValue.offset = 0;
      packedValue.length = values[i].length;
    }

    @Override
    public byte getByteAt(int i, int k) {
      return values[i][k];
    }

    @Override
    public int getDocID(int i) {
      return docs[i];
    }

    @Override
    public void swap(int i, int j) {
      byte[] v = values[i];
      values[i] = values[j];
      values[j] = v;
      int d = docs[i];
      docs[i] = docs[j];
      docs[j] = d;
    }

    @Override
    public void save(int i, int j) {
      savedValues[j] = values[i];
      savedDocs[j] = docs[i];
    }

    @Override
    public void restore(int i, int j) {
      for (int k = i; k < j; k++) {
        values[k] = savedValues[k];
        docs[k] = savedDocs[k];
      }
    }

    @Override
    public long size() {
      return values.length;
    }

    @Override
    public void visitDocValues(PointValues.IntersectVisitor visitor) throws IOException {
      for (int i = 0; i < values.length; i++) {
        visitor.visit(docs[i], values[i]);
      }
    }
  }

  /** {@code formatValue(n, i)}: the packed value of point {@code i} of the {@code n}-D case. */
  static byte[] formatValue(int dims, int i) {
    if (dims == 1) {
      int v = (i * 7919) % 100003;
      return new byte[] {(byte) (v >>> 24), (byte) (v >>> 16), (byte) (v >>> 8), (byte) v};
    }
    int[] d = {(i * 31) % 997, (i * 17) % 1009, (i * 13) % 251};
    byte[] b = new byte[6];
    for (int k = 0; k < 3; k++) {
      b[2 * k] = (byte) (d[k] >>> 8);
      b[2 * k + 1] = (byte) d[k];
    }
    return b;
  }

  static BKDConfig formatConfig(int dims) {
    return dims == 1 ? new BKDConfig(1, 1, 4, 512) : new BKDConfig(3, 2, 2, 512);
  }

  static void writeBkd(Directory dir, String file, int dims, int n, double maxMB, boolean mutable)
      throws IOException {
    try (IndexOutput out = dir.createOutput(file, IOContext.DEFAULT)) {
      SimpleTextBKDWriter w =
          new SimpleTextBKDWriter(n, dir, "_9", formatConfig(dims), maxMB, n);
      long fp;
      if (mutable) {
        byte[][] values = new byte[n][];
        int[] docs = new int[n];
        for (int i = 0; i < n; i++) {
          values[i] = formatValue(dims, i);
          docs[i] = (i * 37) % n;
        }
        fp = w.writeField(out, "f", new ArrayTree(values, docs));
      } else {
        for (int i = 0; i < n; i++) {
          w.add(formatValue(dims, i), (i * 37) % n);
        }
        fp = w.finish(out);
      }
      w.close();
      SimpleTextUtil.write(out, "fp " + fp, new BytesRefBuilder());
      SimpleTextUtil.writeNewline(out);
    }
  }

  static FieldInfo fieldInfo(
      String name, int number, IndexOptions io, boolean tv, boolean omitNorms, boolean payloads,
      DocValuesType dv, DocValuesSkipIndexType skip, long dvGen, int pd, int pi, int pb, int vd,
      VectorEncoding ve, VectorSimilarityFunction vs, boolean soft, boolean parent) {
    return new FieldInfo(
        name, number, tv, omitNorms, payloads, io, dv, skip, dvGen, Map.of(), pd, pi, pb, vd, ve,
        vs, soft, parent);
  }

  /** The format-level files; {@code simpletext_oracle.rs::write_formats} writes the same. */
  static void writeFormats(Path path) throws IOException {
    try (Directory dir = new NIOFSDirectory(path)) {
      Codec codec = new SimpleTextCodec();
      SegmentInfo si =
          new SegmentInfo(
              dir,
              Version.LATEST,
              Version.fromBits(9, 12, 0),
              "_9",
              7,
              true,
              false,
              codec,
              Map.of("source", "flush"),
              SEGMENT_ID,
              Map.of(),
              new Sort(
                  new SortField("s", SortField.Type.STRING, true),
                  new SortedNumericSortField("n", SortField.Type.LONG)));
      si.putAttribute("b", "2");
      si.putAttribute("a", "1");
      si.putAttribute("SimpleText.key", "line\nvalue");
      si.setFiles(List.of("_9.inf", "_9.fld", "_9.pst", "_9.dat", "_9.tvc", "_9.len", "_9.vec"));
      codec.segmentInfoFormat().write(dir, si, IOContext.DEFAULT);

      FieldInfo all =
          fieldInfo("f_all", 0, IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS, true, false,
              true, DocValuesType.SORTED_SET, DocValuesSkipIndexType.RANGE, 3, 3, 2, 4, 8,
              VectorEncoding.BYTE, VectorSimilarityFunction.COSINE, false, false);
      all.putAttribute("k2", "v2");
      all.putAttribute("k1", "v1");
      all.putAttribute("PerFieldPostingsFormat.format", "SimpleText");
      FieldInfos infos =
          new FieldInfos(
              new FieldInfo[] {
                all,
                fieldInfo("f_custom", 1, IndexOptions.DOCS_AND_CUSTOM_FREQS, false, true, false,
                    DocValuesType.NUMERIC, DocValuesSkipIndexType.NONE, -1, 0, 0, 0, 0,
                    VectorEncoding.FLOAT32, VectorSimilarityFunction.EUCLIDEAN, false, false),
                fieldInfo("f_soft", 2, IndexOptions.NONE, false, false, false,
                    DocValuesType.NUMERIC, DocValuesSkipIndexType.NONE, -1, 0, 0, 0, 0,
                    VectorEncoding.FLOAT32, VectorSimilarityFunction.EUCLIDEAN, true, false),
                fieldInfo("f_parent", 3, IndexOptions.NONE, false, false, false,
                    DocValuesType.NUMERIC, DocValuesSkipIndexType.NONE, -1, 0, 0, 0, 0,
                    VectorEncoding.FLOAT32, VectorSimilarityFunction.EUCLIDEAN, false, true),
                fieldInfo("f_mip", 4, IndexOptions.NONE, false, false, false,
                    DocValuesType.NONE, DocValuesSkipIndexType.NONE, -1, 0, 0, 0, 16,
                    VectorEncoding.FLOAT32, VectorSimilarityFunction.MAXIMUM_INNER_PRODUCT, false,
                    false),
                fieldInfo("f_dot", 5, IndexOptions.DOCS, false, false, false,
                    DocValuesType.BINARY, DocValuesSkipIndexType.NONE, 9, 0, 0, 0, 2,
                    VectorEncoding.FLOAT32, VectorSimilarityFunction.DOT_PRODUCT, false, false)
              });
      codec.fieldInfosFormat().write(dir, si, "", infos, IOContext.DEFAULT);
      codec.fieldInfosFormat().write(dir, si, "3", infos, IOContext.DEFAULT);

      writeBkd(dir, "_9.bkd1m", 1, 1200, 16.0, true);
      writeBkd(dir, "_9.bkdnm", 3, 1100, 16.0, true);
      writeBkd(dir, "_9.bkd1o", 1, 3000, 0.01, false);
      writeBkd(dir, "_9.bkdno", 3, 2500, 0.02, false);
    }
  }

  // -------------------------------------------------------------------------------------------
  // Records
  // -------------------------------------------------------------------------------------------

  /** FNV-1a 64 over {@code bytes}. */
  static long fnv(byte[] bytes) {
    long h = 0xcbf29ce484222325L;
    for (byte b : bytes) {
      h ^= b & 0xff;
      h *= 0x100000001b3L;
    }
    return h;
  }

  /** Printable ASCII as is, backslash as {@code \\}, every other byte as {@code \xHH}. */
  static String escape(byte[] bytes) {
    StringBuilder sb = new StringBuilder();
    for (byte b : bytes) {
      int v = b & 0xff;
      if (v == '\\') {
        sb.append("\\\\");
      } else if (v >= 0x20 && v < 0x7f) {
        sb.append((char) v);
      } else {
        sb.append(String.format("\\x%02x", v));
      }
    }
    return sb.toString();
  }

  public static void files(String name, Path path, PrintWriter w) throws IOException {
    List<Path> files;
    try (Stream<Path> s = Files.list(path)) {
      files = s.sorted().toList();
    }
    for (Path f : files) {
      String file = f.getFileName().toString();
      if (file.equals("write.lock")) {
        continue;
      }
      byte[] bytes = Files.readAllBytes(f);
      w.println(
          name + "\t" + file + "\t" + bytes.length + "\t" + Long.toHexString(fnv(bytes)) + "\t"
              + escape(bytes));
    }
  }

  public static void reads(String name, Path path, PrintWriter w) throws IOException {
    for (String line : SimpleTextReadOracle.dump(path)) {
      w.println(name + "\t" + escape(line.getBytes(StandardCharsets.UTF_8)));
    }
  }

  static void copy(Path from, Path to) throws IOException {
    Files.createDirectories(to);
    try (Stream<Path> s = Files.list(from)) {
      for (Path f : s.toList()) {
        Files.copy(f, to.resolve(f.getFileName()));
      }
    }
  }

  /** The files of the first segment of the latest commit with the extension {@code ext}. */
  static Path firstSegmentFile(Path dir, String ext) throws IOException {
    try (Directory d = new NIOFSDirectory(dir)) {
      SegmentInfos infos = SegmentInfos.readLatestCommit(d);
      String segment = infos.info(0).info.name;
      return dir.resolve(segment + "." + ext);
    }
  }

  static byte[] replaceLast(byte[] bytes, String from, String to) {
    String s = new String(bytes, StandardCharsets.ISO_8859_1);
    int at = s.lastIndexOf(from);
    if (at < 0) {
      throw new IllegalStateException("no " + from);
    }
    return (s.substring(0, at) + to + s.substring(at + from.length()))
        .getBytes(StandardCharsets.ISO_8859_1);
  }

  /** Opens the corrupted copy and records the exception. */
  static void corrupt(
      String name, Path source, Path work, String ext, java.util.function.UnaryOperator<byte[]> edit,
      PrintWriter w)
      throws IOException {
    Path dir = work.resolve(name);
    copy(source, dir);
    String id;
    try (Directory d = new NIOFSDirectory(dir)) {
      id = org.apache.lucene.util.StringHelper.idToString(
          SegmentInfos.readLatestCommit(d).info(0).info.getId());
    }
    Path file = firstSegmentFile(dir, ext);
    Files.write(file, edit.apply(Files.readAllBytes(file)));
    String real = dir.toRealPath().toString();
    try (Directory d = new NIOFSDirectory(dir);
        DirectoryReader r = DirectoryReader.open(d)) {
      w.println(name + "\tnone\t-");
    } catch (Throwable t) {
      w.println(
          name + "\t" + t.getClass().getName() + "\t"
              + String.valueOf(t.getMessage())
                  .replace(real, "$D")
                  .replace(id, "$ID")
                  .replace('\n', ' '));
    }
  }

  /** {@code replaceLast} for the first occurrence. */
  static byte[] replaceFirst(byte[] bytes, String from, String to) {
    String s = new String(bytes, StandardCharsets.ISO_8859_1);
    int at = s.indexOf(from);
    if (at < 0) {
      throw new IllegalStateException("no " + from);
    }
    return (s.substring(0, at) + to + s.substring(at + from.length()))
        .getBytes(StandardCharsets.ISO_8859_1);
  }

  /**
   * Rewrites the last {@code checksum} line as {@code SimpleTextUtil.writeChecksum} would for the
   * edited bytes before it: the CRC-32 of everything before the line, {@code %020d}.
   */
  static byte[] fixChecksum(byte[] bytes) {
    String s = new String(bytes, StandardCharsets.ISO_8859_1);
    int at = s.lastIndexOf("checksum ");
    java.util.zip.CRC32 crc = new java.util.zip.CRC32();
    crc.update(bytes, 0, at);
    return (s.substring(0, at)
            + "checksum "
            + String.format(java.util.Locale.ROOT, "%020d", crc.getValue())
            + "\n")
        .getBytes(StandardCharsets.ISO_8859_1);
  }

  /** An operation on the first leaf of an index, returning what it read. */
  interface LeafOp {
    String run(Path dir, org.apache.lucene.index.LeafReader leaf) throws Exception;
  }

  /** The {@code SegmentReadState} of {@code leaf}, for the format readers it opens directly. */
  static org.apache.lucene.index.SegmentReadState readState(
      Directory d, org.apache.lucene.index.LeafReader leaf) {
    var sr = (org.apache.lucene.index.SegmentReader) leaf;
    return new org.apache.lucene.index.SegmentReadState(
        d, sr.getSegmentInfo().info, sr.getFieldInfos(), IOContext.DEFAULT);
  }

  static String masked(Throwable t, String real, String id) {
    return t.getClass().getName()
        + "\t"
        + String.valueOf(t.getMessage()).replace(real, "$D").replace(id, "$ID").replace('\n', ' ');
  }

  /**
   * Edits {@code file} of a copy of {@code source} ({@code ext} of the first segment, or a literal
   * file name starting with {@code _}; {@code null} for none), opens it, runs {@code op} on the
   * first leaf and records {@code case, class, message, cause class, cause message} of the
   * exception, or {@code case, none, result}.
   */
  static void unchecked(
      String name, Path source, Path work, String file,
      java.util.function.UnaryOperator<byte[]> edit, LeafOp op, PrintWriter w)
      throws IOException {
    Path dir = work.resolve("unchecked-" + name);
    copy(source, dir);
    String id;
    try (Directory d = new NIOFSDirectory(dir)) {
      id = org.apache.lucene.util.StringHelper.idToString(
          SegmentInfos.readLatestCommit(d).info(0).info.getId());
    }
    if (file != null) {
      Path f = file.startsWith("_") ? dir.resolve(file) : firstSegmentFile(dir, file);
      Files.write(f, edit.apply(Files.readAllBytes(f)));
    }
    String real = dir.toRealPath().toString();
    try (Directory d = new NIOFSDirectory(dir);
        DirectoryReader r = DirectoryReader.open(d)) {
      w.println(name + "\tnone\t" + op.run(dir, r.leaves().get(0).reader()));
    } catch (Throwable t) {
      Throwable c = t.getCause();
      w.println(name + "\t" + masked(t, real, id) + "\t" + (c == null ? "-\t-" : masked(c, real, id)));
    }
  }

  /** The visitor of the direct stored-fields reads: it reads every field and keeps nothing. */
  static final class AllFields extends org.apache.lucene.index.StoredFieldVisitor {
    @Override
    public Status needsField(FieldInfo fieldInfo) {
      return Status.YES;
    }
  }

  /** The unchecked exceptions of the SimpleText readers, on copies of {@code nocfs}. */
  static void uncheckedCases(Path nocfs, Path work, PrintWriter w) throws IOException {
    unchecked("dv-num-exhausted", nocfs, work, null, null, (p, leaf) -> {
      var v = leaf.getNumericDocValues("dv_num");
      while (v.nextDoc() != org.apache.lucene.search.DocIdSetIterator.NO_MORE_DOCS) {}
      return String.valueOf(v.longValue());
    }, w);
    unchecked("dv-bin-exhausted", nocfs, work, null, null, (p, leaf) -> {
      var v = leaf.getBinaryDocValues("dv_bin");
      while (v.nextDoc() != org.apache.lucene.search.DocIdSetIterator.NO_MORE_DOCS) {}
      return String.valueOf(v.binaryValue());
    }, w);
    unchecked("dv-sorted-ord-neg", nocfs, work, null, null,
        (p, leaf) -> String.valueOf(leaf.getSortedDocValues("dv_sorted").lookupOrd(-1)), w);
    unchecked("dv-sorted-ord-count", nocfs, work, null, null, (p, leaf) -> {
      var v = leaf.getSortedDocValues("dv_sorted");
      return String.valueOf(v.lookupOrd(v.getValueCount()));
    }, w);
    unchecked("dv-sset-ord-neg", nocfs, work, null, null,
        (p, leaf) -> String.valueOf(leaf.getSortedSetDocValues("dv_sset").lookupOrd(-1)), w);
    unchecked("dv-sset-ord-count", nocfs, work, null, null, (p, leaf) -> {
      var v = leaf.getSortedSetDocValues("dv_sset");
      return String.valueOf(v.lookupOrd(v.getValueCount()));
    }, w);
    unchecked("dv-snum-past", nocfs, work, null, null, (p, leaf) -> {
      var v = leaf.getSortedNumericDocValues("dv_snum");
      v.nextDoc();
      for (int i = 0; i < v.docValueCount(); i++) {
        v.nextValue();
      }
      return String.valueOf(v.nextValue());
    }, w);
    unchecked("dv-sset-past", nocfs, work, null, null, (p, leaf) -> {
      var v = leaf.getSortedSetDocValues("dv_sset");
      v.nextDoc();
      for (int i = 0; i < v.docValueCount(); i++) {
        v.nextOrd();
      }
      return String.valueOf(v.nextOrd());
    }, w);
    unchecked("dv-num-corrupt", nocfs, work, "dat",
        b -> replaceFirst(b, "  pattern 0000\n1042\n", "  pattern 0000\nx042\n"), (p, leaf) -> {
          var v = leaf.getNumericDocValues("dv_num");
          v.nextDoc();
          return String.valueOf(v.longValue());
        }, w);
    unchecked("dv-bin-corrupt", nocfs, work, "dat",
        b -> replaceFirst(b, "length 19\n", "length x9\n"), (p, leaf) -> {
          var v = leaf.getBinaryDocValues("dv_bin");
          v.nextDoc();
          return String.valueOf(v.binaryValue());
        }, w);
    unchecked("fld-doc-past", nocfs, work, null, null, (p, leaf) -> {
      ((org.apache.lucene.index.CodecReader) leaf).getFieldsReader()
          .document(leaf.maxDoc(), new AllFields());
      return "read";
    }, w);
    unchecked("fld-doc-neg", nocfs, work, null, null, (p, leaf) -> {
      ((org.apache.lucene.index.CodecReader) leaf).getFieldsReader().document(-1, new AllFields());
      return "read";
    }, w);
    unchecked("fld-extra-doc", nocfs, work, "fld", b -> replaceFirst(b, "doc 0\n", "doc 0\ndoc 0\n"),
        (p, leaf) -> "opened", w);
    unchecked("fld-unknown-type", nocfs, work, "fld",
        b -> fixChecksum(replaceFirst(b, "    type string\n", "    type strinx\n")), (p, leaf) -> {
          ((org.apache.lucene.index.CodecReader) leaf).getFieldsReader().document(0, new AllFields());
          return "read";
        }, w);
    unchecked("fld-unknown-field", nocfs, work, "fld",
        b -> fixChecksum(replaceFirst(b, "  field 0\n", "  field 99\n")),
        (p, leaf) -> String.valueOf(leaf.storedFields().document(0)), w);
    unchecked("tvc-doc-past", nocfs, work, null, null, (p, leaf) -> String.valueOf(
        ((org.apache.lucene.index.CodecReader) leaf).getTermVectorsReader().get(leaf.maxDoc())), w);
    unchecked("tvc-extra-doc", nocfs, work, "tvc", b -> replaceFirst(b, "doc 0\n", "doc 0\ndoc 0\n"),
        (p, leaf) -> "opened", w);
    unchecked("tvc-term-unpositioned", nocfs, work, null, null, (p, leaf) -> String.valueOf(
        ((org.apache.lucene.index.CodecReader) leaf).getTermVectorsReader().get(0).terms("tv")
            .iterator().term()), w);
    unchecked("tvc-ttf-unpositioned", nocfs, work, null, null, (p, leaf) -> String.valueOf(
        ((org.apache.lucene.index.CodecReader) leaf).getTermVectorsReader().get(0).terms("tv")
            .iterator().totalTermFreq()), w);
    unchecked("pst-term-unpositioned", nocfs, work, null, null,
        (p, leaf) -> String.valueOf(leaf.terms("body").iterator().term()), w);
    unchecked("pst-term-exhausted", nocfs, work, null, null, (p, leaf) -> {
      var te = leaf.terms("body").iterator();
      while (te.next() != null) {}
      return String.valueOf(te.term());
    }, w);
    unchecked("pst-doc-out", nocfs, work, "pst",
        b -> fixChecksum(replaceFirst(b, "    doc 3\n", "    doc 99999\n")),
        (p, leaf) -> String.valueOf(leaf.terms("body").getDocCount()), w);
    unchecked("pst-doc-neg", nocfs, work, "pst",
        b -> fixChecksum(replaceFirst(b, "    doc 3\n", "    doc -1\n")),
        (p, leaf) -> String.valueOf(leaf.terms("body").getDocCount()), w);
    unchecked("pst-doc-ghost", nocfs, work, "pst",
        b -> fixChecksum(replaceFirst(b, "    doc 3\n", "    doc 191\n")),
        (p, leaf) -> String.valueOf(leaf.terms("body").getDocCount()), w);
    unchecked("pst-field-unknown", nocfs, work, "pst",
        b -> fixChecksum(replaceFirst(b, "field tv\n", "field tx\n")), (p, leaf) -> {
          try (Directory d = new NIOFSDirectory(p);
              var fields = new SimpleTextFieldsReader(readState(d, leaf))) {
            return String.valueOf(fields.terms("tx"));
          }
        }, w);
    unchecked("pst-field-unknown-iterator", nocfs, work, "pst",
        b -> fixChecksum(replaceFirst(b, "field tv\n", "field tx\n")), (p, leaf) -> {
          try (Directory d = new NIOFSDirectory(p);
              var fields = new SimpleTextFieldsReader(readState(d, leaf))) {
            return String.valueOf(fields.terms("tx").iterator());
          }
        }, w);
    unchecked("pst-field-unknown-has-freqs", nocfs, work, "pst",
        b -> fixChecksum(replaceFirst(b, "field tv\n", "field tx\n")), (p, leaf) -> {
          try (Directory d = new NIOFSDirectory(p);
              var fields = new SimpleTextFieldsReader(readState(d, leaf))) {
            return String.valueOf(fields.terms("tx").hasFreqs());
          }
        }, w);
    unchecked("pst-field-unknown-has-offsets", nocfs, work, "pst",
        b -> fixChecksum(replaceFirst(b, "field tv\n", "field tx\n")), (p, leaf) -> {
          try (Directory d = new NIOFSDirectory(p);
              var fields = new SimpleTextFieldsReader(readState(d, leaf))) {
            return String.valueOf(fields.terms("tx").hasOffsets());
          }
        }, w);
    unchecked("pst-field-unknown-has-positions", nocfs, work, "pst",
        b -> fixChecksum(replaceFirst(b, "field tv\n", "field tx\n")), (p, leaf) -> {
          try (Directory d = new NIOFSDirectory(p);
              var fields = new SimpleTextFieldsReader(readState(d, leaf))) {
            return String.valueOf(fields.terms("tx").hasPositions());
          }
        }, w);
    unchecked("pst-field-unknown-has-payloads", nocfs, work, "pst",
        b -> fixChecksum(replaceFirst(b, "field tv\n", "field tx\n")), (p, leaf) -> {
          try (Directory d = new NIOFSDirectory(p);
              var fields = new SimpleTextFieldsReader(readState(d, leaf))) {
            return String.valueOf(fields.terms("tx").hasPayloads());
          }
        }, w);
    unchecked("gri-field-unknown", nocfs, work, "gri",
        b -> fixChecksum(replaceFirst(b, "field-name vb\n", "field-name vz\n")),
        (p, leaf) -> "opened", w);
    unchecked("vec-short-line", nocfs, work, "vec",
        b -> replaceFirst(b, "[20, -30, 0, -20]\n", "[\n"), (p, leaf) -> {
          var v = leaf.getByteVectorValues("vb");
          return Arrays.toString(v.vectorValue(0));
        }, w);
    unchecked("knn-search-unknown", nocfs, work, null, null, (p, leaf) -> {
      try (Directory d = new NIOFSDirectory(p);
          var vectors = new SimpleTextKnnVectorsReader(readState(d, leaf))) {
        vectors.search("nope", new float[] {1, 2, 3, 4},
            new org.apache.lucene.search.TopKnnCollector(3, Integer.MAX_VALUE),
            org.apache.lucene.search.AcceptDocs.fromLiveDocs(null, leaf.maxDoc()));
        return "searched";
      }
    }, w);
    unchecked("knn-search-bytes-unknown", nocfs, work, null, null, (p, leaf) -> {
      try (Directory d = new NIOFSDirectory(p);
          var vectors = new SimpleTextKnnVectorsReader(readState(d, leaf))) {
        vectors.search("nope", new byte[] {1, 2, 3, 4},
            new org.apache.lucene.search.TopKnnCollector(3, Integer.MAX_VALUE),
            org.apache.lucene.search.AcceptDocs.fromLiveDocs(null, leaf.maxDoc()));
        return "searched";
      }
    }, w);
    unchecked("liv-negative", nocfs, work, "_1_1.liv",
        b -> replaceFirst(b, "  doc 0\n", "  doc -1\n"), (p, leaf) -> "opened", w);
    unchecked("fld-clone-closed", nocfs, work, null, null, (p, leaf) -> {
      try (Directory d = new NIOFSDirectory(p)) {
        var state = readState(d, leaf);
        var reader = new SimpleTextStoredFieldsReader(d, state.segmentInfo, state.fieldInfos,
            IOContext.DEFAULT);
        reader.close();
        reader.clone();
        return "cloned";
      }
    }, w);
    unchecked("tvc-clone-closed", nocfs, work, null, null, (p, leaf) -> {
      try (Directory d = new NIOFSDirectory(p)) {
        var reader = new SimpleTextTermVectorsReader(d, readState(d, leaf).segmentInfo,
            IOContext.DEFAULT);
        reader.close();
        reader.clone();
        return "cloned";
      }
    }, w);
  }

  static String lastDigitBumped(byte[] bytes) {
    String s = new String(bytes, StandardCharsets.ISO_8859_1);
    int at = s.length() - 2; // the last digit, before the final newline
    char c = s.charAt(at);
    char bumped = (char) ('0' + ((c - '0' + 1) % 10));
    return s.substring(0, at) + bumped + s.substring(at + 1);
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-codecs");
    out.mkdirs();
    Path work = Files.createTempDirectory("simpletext-oracle");
    Path nocfs = work.resolve("nocfs");
    Path cfs = work.resolve("cfs");
    Path edge = work.resolve("edge");
    try (Directory d = new NIOFSDirectory(nocfs)) {
      writeCorpus(d, false);
    }
    try (Directory d = new NIOFSDirectory(cfs)) {
      writeCorpus(d, true);
    }
    try (Directory d = new NIOFSDirectory(edge)) {
      writeEdge(d);
    }
    Path formats = work.resolve("formats");
    writeFormats(formats);
    try (PrintWriter w = new PrintWriter(new File(out, "simpletext_files.tsv"), "UTF-8")) {
      w.println("# case\tfile\tlength\tfnv1a64\tcontent (\\xHH, \\\\)");
      files("nocfs", nocfs, w);
      files("cfs", cfs, w);
      files("edge", edge, w);
      files("formats", formats, w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "simpletext_reads.tsv"), "UTF-8")) {
      w.println("# case\tline of SimpleTextReadOracle.dump (\\xHH, \\\\)");
      reads("nocfs", nocfs, w);
      reads("cfs", cfs, w);
      reads("edge", edge, w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "simpletext_errors.tsv"), "UTF-8")) {
      w.println("# case\tclass\tmessage ($D = the index directory)");
      corrupt(
          "pst-checksum", nocfs, work, "pst",
          b -> lastDigitBumped(b).getBytes(StandardCharsets.ISO_8859_1), w);
      corrupt(
          "fld-trailing", nocfs, work, "fld",
          b -> {
            byte[] c = Arrays.copyOf(b, b.length + 1);
            c[b.length] = 'x';
            return c;
          },
          w);
      corrupt("inf-word", nocfs, work, "inf", b -> replaceLast(b, "checksum ", "chksum   "), w);
      corrupt("si-id", nocfs, work, "si", b -> replaceLast(b, "    id [", "    id [0 "), w);
      corrupt("si-version", nocfs, work, "si", b -> replaceLast(b, "    version 1", "    version x"), w);
      corrupt("dat-checksum", nocfs, work, "dat",
          b -> lastDigitBumped(b).getBytes(StandardCharsets.ISO_8859_1), w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "simpletext_unchecked.tsv"), "UTF-8")) {
      w.println(
          "# case\tclass\tmessage\tcause class\tcause message ($D = the index directory),"
              + " or case\tnone\tresult");
      uncheckedCases(nocfs, work, w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "simpletext_spi.tsv"), "UTF-8")) {
      w.println("# lookup\tresult");
      w.println("Codec.forName(SimpleText)\t" + Codec.forName("SimpleText").getName());
      w.println(
          "Codec.forName(SimpleText).getClass()\t" + Codec.forName("SimpleText").getClass().getName());
      w.println("postingsFormat\t" + Codec.forName("SimpleText").postingsFormat().getName());
      w.println("docValuesFormat\t" + Codec.forName("SimpleText").docValuesFormat().getName());
      w.println(
          "knnVectorsFormat\t" + Codec.forName("SimpleText").knnVectorsFormat().getName());
      try {
        PostingsFormat.forName("SimpleText");
        w.println("PostingsFormat.forName(SimpleText)\tfound");
      } catch (IllegalArgumentException e) {
        w.println("PostingsFormat.forName(SimpleText)\t" + e.getClass().getName());
      }
    }
    org.apache.lucene.util.IOUtils.rm(work);
  }

  /** A token stream that emits scripted tokens with offsets and payloads. */
  static final class ScriptedTokenStream extends TokenStream {
    private final List<Tok> tokens;
    private final CharTermAttribute termAtt = addAttribute(CharTermAttribute.class);
    private final OffsetAttribute offsetAtt = addAttribute(OffsetAttribute.class);
    private final PositionIncrementAttribute posIncrAtt =
        addAttribute(PositionIncrementAttribute.class);
    private final PayloadAttribute payloadAtt = addAttribute(PayloadAttribute.class);
    private int upto;
    private int finalOffset;

    ScriptedTokenStream(List<Tok> tokens) {
      this.tokens = tokens;
    }

    @Override
    public boolean incrementToken() {
      if (upto == tokens.size()) {
        return false;
      }
      clearAttributes();
      Tok token = tokens.get(upto++);
      termAtt.append(token.term());
      posIncrAtt.setPositionIncrement(1);
      offsetAtt.setOffset(token.start(), token.end());
      payloadAtt.setPayload(token.payload() == null ? null : new BytesRef(token.payload()));
      finalOffset = token.end();
      return true;
    }

    @Override
    public void reset() throws IOException {
      super.reset();
      upto = 0;
      finalOffset = 0;
    }

    @Override
    public void end() throws IOException {
      super.end();
      offsetAtt.setOffset(finalOffset, finalOffset);
    }
  }
}
