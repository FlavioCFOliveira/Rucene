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

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.analysis.tokenattributes.OffsetAttribute;
import org.apache.lucene.analysis.tokenattributes.PayloadAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute;
import org.apache.lucene.document.BinaryDocValuesField;
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
import org.apache.lucene.index.BinaryDocValues;
import org.apache.lucene.index.CheckIndex;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.Fields;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.IndexableField;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.ReaderUtil;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.SortedDocValues;
import org.apache.lucene.index.SortedNumericDocValues;
import org.apache.lucene.index.SortedSetDocValues;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.index.TieredMergePolicy;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.FieldDoc;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.KnnByteVectorQuery;
import org.apache.lucene.search.KnnFloatVectorQuery;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.PhraseQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Oracle of the bidirectional index acceptance test {@code tests/portability/index_interop.rs}.
 *
 * <p>The corpus, the operations and the query set are defined here and in the Rust test by the
 * same integer formulas, so each side builds the same index from the same definition. Modes:
 *
 * <ul>
 *   <li>{@code write <dir>}: writes the corpus with the default codec.
 *   <li>{@code forced-deletes <dir>}: writes the forced-deletes corpus of {@link
 *       #writeForcedDeletes}.
 *   <li>{@code check <dir>}: runs {@code CheckIndex} at {@code Level.MAX_VALUE} (checksums,
 *       integrity and slow checks) and prints whether the index is clean.
 *   <li>{@code query <dir>}: runs the fixed query set and prints the results.
 *   <li>{@code files <dir>}: prints the latest commit's segments and their files with sizes.
 * </ul>
 */
public final class IndexInteropFixture {

  /** Documents of each batch, in the order they are added. */
  static final int[][] BATCHES = {{0, 120}, {120, 240}, {240, 320}, {320, 360}};

  /** Text field with positions and offsets, stored, with norms. */
  static final FieldType BODY_TYPE = new FieldType();

  /** Token-stream field with positions, offsets and payloads in postings and term vectors. */
  static final FieldType TV_TYPE = new FieldType();

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
  }

  private IndexInteropFixture() {}

  public static void main(String[] args) throws Exception {
    Directory dir = FSDirectory.open(Paths.get(args[1]));
    switch (args[0]) {
      case "write" -> write(dir);
      case "forced-deletes" -> writeForcedDeletes(dir);
      case "check" -> check(dir);
      case "query" -> query(dir);
      case "files" -> files(dir);
      default -> throw new IllegalArgumentException(args[0]);
    }
    dir.close();
  }

  // ---------------------------------------------------------------------------------------------
  // Corpus
  // ---------------------------------------------------------------------------------------------

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

  /** The operations; {@code index_interop.rs::write_corpus} performs the same, in order. */
  static void write(Directory dir) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
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
      // Documents 60-99 (p_2d second dimension 3 or 4): more than
      // forceMergeDeletesPctAllowed (10%) of the merged segment holding them, and none of the
      // documents the query set reports.
      w.deleteDocuments(IntPoint.newRangeQuery("p_2d", new int[] {0, 3}, new int[] {19, 4}));
      w.forceMergeDeletes();
      w.commit();
    }
  }

  /**
   * Ten segments of 20 documents; the even ones get 3 deletions each (15%: over {@code
   * forceMergeDeletesPctAllowed}, under the natural {@code deletesPctAllowed}), then {@code
   * forceMergeDeletes()}. The floor segment size is below every segment, the tier is wide and
   * {@code targetSearchConcurrency} is 2, so the merges depend on {@code findForcedDeletesMerges}
   * passing the document budget of the whole index, {@code getMaxAllowedDocs(infos.totalMaxDoc(),
   * totalDelCount)}, to {@code doFindMerges}; {@code index_interop.rs} does the same.
   */
  static void writeForcedDeletes(Directory dir) throws IOException {
    TieredMergePolicy policy = new TieredMergePolicy();
    policy.setFloorSegmentMB(0.001);
    policy.setSegmentsPerTier(50);
    policy.setTargetSearchConcurrency(2);
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    config.setMergePolicy(policy);
    try (IndexWriter w = new IndexWriter(dir, config)) {
      for (int s = 0; s < 10; s++) {
        for (int i = s * 20; i < s * 20 + 20; i++) {
          w.addDocument(doc(i));
        }
        w.commit();
      }
      for (int s = 0; s < 10; s += 2) {
        for (int k = 0; k < 3; k++) {
          w.deleteDocuments(new Term("id", id(s * 20 + k * 7)));
        }
      }
      w.commit();
      w.forceMergeDeletes();
      w.commit();
    }
    files(dir);
  }

  // ---------------------------------------------------------------------------------------------
  // CheckIndex
  // ---------------------------------------------------------------------------------------------

  static void check(Directory dir) throws IOException {
    ByteArrayOutputStream log = new ByteArrayOutputStream();
    CheckIndex.Status status;
    try (CheckIndex checker = new CheckIndex(dir)) {
      checker.setLevel(CheckIndex.Level.MAX_VALUE);
      checker.setInfoStream(new PrintStream(log, true, StandardCharsets.UTF_8), true);
      status = checker.checkIndex();
    }
    System.err.print(log.toString(StandardCharsets.UTF_8));
    System.out.println("clean=" + status.clean);
    System.out.println("numSegments=" + status.numSegments);
    System.out.println("numBadSegments=" + status.numBadSegments);
    System.out.println("totLoseDocCount=" + status.totLoseDocCount);
    for (CheckIndex.Status.SegmentInfoStatus seg : status.segmentInfos) {
      System.out.println(
          "segment "
              + seg.name
              + " maxDoc="
              + seg.maxDoc
              + " error="
              + (seg.error == null ? "none" : seg.error.toString().replace('\n', ' ')));
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Query set
  // ---------------------------------------------------------------------------------------------

  static String hex(float f) {
    return String.format("%08x", Float.floatToRawIntBits(f));
  }

  static String hex(double d) {
    return String.format("%016x", Double.doubleToRawLongBits(d));
  }

  static String hex(BytesRef b) {
    StringBuilder sb = new StringBuilder();
    for (int k = b.offset; k < b.offset + b.length; k++) {
      sb.append(String.format("%02x", b.bytes[k] & 0xff));
    }
    return sb.toString();
  }

  static Term term(String field, String text) {
    return new Term(field, text);
  }

  static String storedId(StoredFields stored, int doc) throws IOException {
    return stored.document(doc).get("id");
  }

  static void printTop(String name, TopDocs top, StoredFields stored) throws IOException {
    System.out.println(
        "query " + name + " total=" + top.totalHits.value() + " " + top.totalHits.relation());
    for (ScoreDoc hit : top.scoreDocs) {
      StringBuilder line = new StringBuilder();
      line.append("  hit doc=").append(hit.doc).append(" score=").append(hex(hit.score));
      if (hit instanceof FieldDoc fd) {
        line.append(" sort=");
        Object v = fd.fields[0];
        if (v == null) {
          line.append("null");
        } else if (v instanceof BytesRef b) {
          line.append("bytes:").append(b.utf8ToString());
        } else {
          line.append(v.getClass().getSimpleName()).append(':').append(v);
        }
      }
      line.append(" id=").append(storedId(stored, hit.doc));
      System.out.println(line);
    }
  }

  static void printStored(StoredFields stored, int doc) throws IOException {
    for (IndexableField f : stored.document(doc)) {
      StringBuilder line = new StringBuilder("  stored ").append(f.name()).append(' ');
      Number n = f.numericValue();
      if (n != null) {
        if (n instanceof Integer || n instanceof Long) {
          line.append(n.getClass().getSimpleName()).append(':').append(n);
        } else if (n instanceof Float x) {
          line.append("Float:").append(hex(x));
        } else {
          line.append("Double:").append(hex(n.doubleValue()));
        }
      } else if (f.binaryValue() != null) {
        line.append("bytes:").append(hex(f.binaryValue()));
      } else {
        line.append("string:").append(f.stringValue());
      }
      System.out.println(line);
    }
  }

  static void printDocValues(List<LeafReaderContext> leaves, int doc) throws IOException {
    LeafReaderContext ctx = leaves.get(ReaderUtil.subIndex(doc, leaves));
    LeafReader leaf = ctx.reader();
    int local = doc - ctx.docBase;
    NumericDocValues num = leaf.getNumericDocValues("dv_num");
    System.out.println(
        "  dv dv_num "
            + (num != null && num.advanceExact(local) ? Long.toString(num.longValue()) : "none"));
    BinaryDocValues bin = leaf.getBinaryDocValues("dv_bin");
    System.out.println(
        "  dv dv_bin "
            + (bin != null && bin.advanceExact(local) ? bin.binaryValue().utf8ToString() : "none"));
    SortedDocValues sorted = leaf.getSortedDocValues("dv_sorted");
    System.out.println(
        "  dv dv_sorted "
            + (sorted != null && sorted.advanceExact(local)
                ? sorted.ordValue() + ":" + sorted.lookupOrd(sorted.ordValue()).utf8ToString()
                : "none"));
    SortedNumericDocValues snum = leaf.getSortedNumericDocValues("dv_snum");
    StringBuilder line = new StringBuilder("  dv dv_snum");
    if (snum != null && snum.advanceExact(local)) {
      for (int k = 0; k < snum.docValueCount(); k++) {
        line.append(' ').append(snum.nextValue());
      }
    } else {
      line.append(" none");
    }
    System.out.println(line);
    SortedSetDocValues sset = leaf.getSortedSetDocValues("dv_sset");
    line = new StringBuilder("  dv dv_sset");
    if (sset != null && sset.advanceExact(local)) {
      for (int k = 0; k < sset.docValueCount(); k++) {
        long ord = sset.nextOrd();
        line.append(' ').append(ord).append(':').append(sset.lookupOrd(ord).utf8ToString());
      }
    } else {
      line.append(" none");
    }
    System.out.println(line);
  }

  static void printTermVectors(DirectoryReader reader, int doc) throws IOException {
    Fields fields = reader.termVectors().get(doc);
    if (fields == null) {
      System.out.println("  tv none");
      return;
    }
    for (String field : fields) {
      Terms terms = fields.terms(field);
      System.out.println(
          "  tv field "
              + field
              + " size="
              + terms.size()
              + " positions="
              + terms.hasPositions()
              + " offsets="
              + terms.hasOffsets()
              + " payloads="
              + terms.hasPayloads());
      TermsEnum te = terms.iterator();
      BytesRef text;
      while ((text = te.next()) != null) {
        StringBuilder line = new StringBuilder("   term ").append(text.utf8ToString());
        line.append(" freq=").append(te.totalTermFreq());
        PostingsEnum pe = te.postings(null, PostingsEnum.ALL);
        pe.nextDoc();
        for (int k = 0; k < pe.freq(); k++) {
          int pos = pe.nextPosition();
          BytesRef payload = pe.getPayload();
          line.append(" [")
              .append(pos)
              .append(',')
              .append(pe.startOffset())
              .append(',')
              .append(pe.endOffset())
              .append(',')
              .append(payload == null ? "-" : hex(payload))
              .append(']');
        }
        System.out.println(line);
      }
    }
  }

  /** The terms whose postings are dumped with {@code PostingsEnum.ALL}. */
  static final String[][] POSTINGS_TERMS = {
    {"body", "w7"}, {"body", "w22"}, {"tv", "t1"}, {"tv", "t4"}
  };

  /**
   * Every leaf's postings of {@code field:text} with {@code PostingsEnum.ALL}, deleted documents
   * included: the global doc, the freq, then each occurrence's position, offsets and payload.
   */
  static void printPostings(List<LeafReaderContext> leaves, String field, String text)
      throws IOException {
    System.out.println("postings " + field + ":" + text);
    for (LeafReaderContext ctx : leaves) {
      Terms terms = ctx.reader().terms(field);
      if (terms == null) {
        continue;
      }
      TermsEnum te = terms.iterator();
      if (te.seekExact(new BytesRef(text)) == false) {
        continue;
      }
      PostingsEnum pe = te.postings(null, PostingsEnum.ALL);
      for (int d = pe.nextDoc(); d != PostingsEnum.NO_MORE_DOCS; d = pe.nextDoc()) {
        StringBuilder line = new StringBuilder("  doc=").append(ctx.docBase + d);
        line.append(" freq=").append(pe.freq());
        for (int k = 0; k < pe.freq(); k++) {
          int pos = pe.nextPosition();
          BytesRef payload = pe.getPayload();
          line.append(" [")
              .append(pos)
              .append(',')
              .append(pe.startOffset())
              .append(',')
              .append(pe.endOffset())
              .append(',')
              .append(payload == null ? "-" : hex(payload))
              .append(']');
        }
        System.out.println(line);
      }
    }
  }

  static void query(Directory dir) throws IOException {
    try (DirectoryReader reader = DirectoryReader.open(dir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      StoredFields stored = searcher.storedFields();
      List<LeafReaderContext> leaves = reader.leaves();
      System.out.println("reader maxDoc=" + reader.maxDoc() + " numDocs=" + reader.numDocs());
      for (LeafReaderContext ctx : leaves) {
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

      TopDocs termTop = searcher.search(new TermQuery(term("body", "w7")), 10);
      printTop("term body:w7", termTop, stored);

      BooleanQuery.Builder bool = new BooleanQuery.Builder();
      bool.add(new TermQuery(term("body", "w3")), BooleanClause.Occur.MUST);
      bool.add(new TermQuery(term("body", "w10")), BooleanClause.Occur.SHOULD);
      bool.add(new TermQuery(term("body", "w20")), BooleanClause.Occur.MUST_NOT);
      printTop("bool +w3 w10 -w20", searcher.search(bool.build(), 10), stored);

      BooleanQuery.Builder disj = new BooleanQuery.Builder();
      disj.add(new TermQuery(term("body", "w1")), BooleanClause.Occur.SHOULD);
      disj.add(new TermQuery(term("body", "w2")), BooleanClause.Occur.SHOULD);
      disj.add(new TermQuery(term("body", "w4")), BooleanClause.Occur.SHOULD);
      printTop("bool w1 w2 w4", searcher.search(disj.build(), 10), stored);

      printTop(
          "phrase body:\"w5 w22\"", searcher.search(new PhraseQuery("body", "w5", "w22"), 10), stored);

      printTop("point p_int [10,60]", searcher.search(IntPoint.newRangeQuery("p_int", 10, 60), 20), stored);
      printTop(
          "point p_long [0,8000000]",
          searcher.search(LongPoint.newRangeQuery("p_long", 0L, 8_000_000L), 20),
          stored);
      printTop(
          "point p_float [-10,25.5]",
          searcher.search(FloatPoint.newRangeQuery("p_float", -10f, 25.5f), 20),
          stored);
      printTop(
          "point p_double [0,40]",
          searcher.search(DoublePoint.newRangeQuery("p_double", 0d, 40d), 20),
          stored);
      printTop(
          "point p_2d [3,2]-[10,10]",
          searcher.search(IntPoint.newRangeQuery("p_2d", new int[] {3, 2}, new int[] {10, 10}), 20),
          stored);

      printTop(
          "sort dv_num LONG",
          searcher.search(
              MatchAllDocsQuery.INSTANCE, 20, new Sort(new SortField("dv_num", SortField.Type.LONG))),
          stored);
      printTop(
          "sort dv_sorted STRING",
          searcher.search(
              MatchAllDocsQuery.INSTANCE,
              20,
              new Sort(new SortField("dv_sorted", SortField.Type.STRING))),
          stored);

      printTop(
          "knn vf",
          searcher.search(
              new KnnFloatVectorQuery("vf", new float[] {0.5f, -0.25f, 1f, 1.125f}, 10), 10),
          stored);
      printTop(
          "knn vb",
          searcher.search(new KnnByteVectorQuery("vb", new byte[] {10, -5, 7, 3}, 10), 10),
          stored);

      for (String[] ft : POSTINGS_TERMS) {
        printPostings(leaves, ft[0], ft[1]);
      }

      for (int k = 0; k < Math.min(3, termTop.scoreDocs.length); k++) {
        int doc = termTop.scoreDocs[k].doc;
        System.out.println("hitdoc " + doc);
        printStored(stored, doc);
        printDocValues(leaves, doc);
      }

      for (String target : new String[] {id(42), id(331), id(10), id(130), id(5), id(300)}) {
        TopDocs found = searcher.search(new TermQuery(term("id", target)), 1);
        if (found.scoreDocs.length == 0) {
          System.out.println("doc " + target + " missing");
          continue;
        }
        int doc = found.scoreDocs[0].doc;
        System.out.println("doc " + target + " = " + doc);
        printStored(stored, doc);
        printDocValues(leaves, doc);
        printTermVectors(reader, doc);
      }
    }
  }

  // ---------------------------------------------------------------------------------------------
  // File listing
  // ---------------------------------------------------------------------------------------------

  static void files(Directory dir) throws IOException {
    SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
    String segmentsFile = infos.getSegmentsFileName();
    System.out.println("commit " + segmentsFile + " " + dir.fileLength(segmentsFile));
    for (SegmentCommitInfo sci : infos) {
      System.out.println(
          "segment "
              + sci.info.name
              + " maxDoc="
              + sci.info.maxDoc()
              + " delCount="
              + sci.getDelCount()
              + " delGen="
              + sci.getDelGen()
              + " dvGen="
              + sci.getDocValuesGen()
              + " fieldInfosGen="
              + sci.getFieldInfosGen()
              + " compound="
              + sci.info.getUseCompoundFile()
              + " codec="
              + sci.info.getCodec().getName());
      List<String> names = new ArrayList<>(sci.files());
      names.sort(null);
      for (String name : names) {
        System.out.println("  file " + name + " " + dir.fileLength(name));
      }
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Token stream
  // ---------------------------------------------------------------------------------------------

  /** Emits the tokens of {@link #tvTokens}, bypassing analysis. */
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
