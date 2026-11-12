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

import java.nio.file.Path;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.codecs.CodecUtil;
import org.apache.lucene.codecs.KnnFieldVectorsWriter;
import org.apache.lucene.codecs.KnnVectorsReader;
import org.apache.lucene.codecs.KnnVectorsWriter;
import org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsFormat;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.SegmentReadState;
import org.apache.lucene.index.SegmentWriteState;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.BooleanQuery;
import org.apache.lucene.search.PhraseQuery;
import org.apache.lucene.search.QueryVisitor;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.store.ByteArrayDataInput;
import org.apache.lucene.store.ByteBuffersDataOutput;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.MMapDirectory;
import org.apache.lucene.store.RandomAccessInput;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.Constants;
import org.apache.lucene.util.InfoStream;
import org.apache.lucene.util.IntsRefBuilder;
import org.apache.lucene.util.SloppyMath;
import org.apache.lucene.util.Version;
import org.apache.lucene.util.VectorUtil;
import org.apache.lucene.util.fst.FST;
import org.apache.lucene.util.fst.FSTCompiler;
import org.apache.lucene.util.fst.PositiveIntOutputs;
import org.apache.lucene.util.fst.Util;

/**
 * Prints what Lucene 10.5.0 does for the behaviours the sprint 7 census gaps closed, one case per
 * argument, so the Rust tests can compare their own output line by line.
 *
 * <p>Command-line usage: {@code java -cp lucene-core-10.5.0.jar CensusGapFixture.java <case>
 * [args]}.
 */
public final class CensusGapFixture {

  private CensusGapFixture() {}

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "hnswRam" -> hnswRam();
      case "fst" -> fst();
      case "termCollector" -> termCollector();
      case "fma" -> fma();
      case "constants" -> constants();
      case "exitable" -> exitable();
      case "sloppy" -> sloppy();
      case "mmap" -> mmap(Path.of(args[1]));
      default -> throw new IllegalArgumentException(args[0]);
    }
  }

  /** {@code Throwable.toString()} of whatever {@code body} throws, or {@code ok}. */
  private static String outcome(ThrowingRunnable body) {
    try {
      body.run();
      return "ok";
    } catch (Throwable t) {
      return t.toString();
    }
  }

  private interface ThrowingRunnable {
    void run() throws Exception;
  }

  /** The vector of document {@code i} in {@code hnswRam}. */
  static float[] vector(int i) {
    return new float[] {
      (i * 37 % 101) / 7f, (i * 53 % 97) / 3f, (i % 13) * 0.5f, i * 0.25f,
    };
  }

  /** G-005, G-016, G-406: writer and reader {@code ramBytesUsed()} of the HNSW format. */
  static void hnswRam() throws Exception {
    int dim = 4;
    int n = 1000;
    try (Directory dir = new ByteBuffersDirectory()) {
      SegmentInfo si =
          new SegmentInfo(
              dir,
              Version.LATEST,
              Version.LATEST,
              "_0",
              n,
              false,
              false,
              Codec.getDefault(),
              Map.of(),
              new byte[16],
              Map.of(),
              null);
      FieldInfo fi =
          new FieldInfo(
              "vec",
              0,
              false,
              false,
              false,
              IndexOptions.NONE,
              DocValuesType.NONE,
              DocValuesSkipIndexType.NONE,
              -1,
              Map.of(),
              0,
              0,
              0,
              dim,
              VectorEncoding.FLOAT32,
              VectorSimilarityFunction.EUCLIDEAN,
              false,
              false);
      FieldInfos fis = new FieldInfos(new FieldInfo[] {fi});
      SegmentWriteState ws =
          new SegmentWriteState(InfoStream.NO_OUTPUT, dir, si, fis, null, IOContext.DEFAULT);
      Lucene99HnswVectorsFormat format = new Lucene99HnswVectorsFormat();
      KnnVectorsWriter w = format.fieldsWriter(ws);
      System.out.println("writer=" + w.ramBytesUsed());
      @SuppressWarnings("unchecked")
      KnnFieldVectorsWriter<float[]> fw = (KnnFieldVectorsWriter<float[]>) w.addField(fi);
      System.out.println("field=" + w.ramBytesUsed());
      for (int i = 0; i < n; i++) {
        fw.addValue(i, vector(i));
        int added = i + 1;
        if (added == 1 || added == 647 || added == 648 || added == 649 || added == 800
            || added == 1000) {
          System.out.println("added" + added + "=" + w.ramBytesUsed());
        }
      }
      w.flush(n, null);
      System.out.println("flushed=" + w.ramBytesUsed());
      w.finish();
      w.close();
      try (KnnVectorsReader r =
          format.fieldsReader(new SegmentReadState(dir, si, fis, IOContext.DEFAULT))) {
        System.out.println(
            "reader=" + ((org.apache.lucene.codecs.lucene99.Lucene99HnswVectorsReader) r).ramBytesUsed());
      }
    }
  }

  /** G-413, G-414, G-415: FST size, toString, arc toString and metadata failures. */
  static void fst() throws Exception {
    FSTCompiler<Long> compiler =
        new FSTCompiler.Builder<>(FST.INPUT_TYPE.BYTE1, PositiveIntOutputs.getSingleton()).build();
    IntsRefBuilder scratch = new IntsRefBuilder();
    String[] terms = {"ab", "ac", "b"};
    for (int i = 0; i < terms.length; i++) {
      compiler.add(Util.toIntsRef(new BytesRef(terms[i]), scratch), (long) (i + 5));
    }
    FST<Long> fst = FST.fromFSTReader(compiler.compile(), compiler.getFSTReader());
    System.out.println("ram=" + fst.ramBytesUsed());
    System.out.println("toString=" + fst);
    FST.Arc<Long> first = fst.getFirstArc(new FST.Arc<>());
    System.out.println("first=" + first);
    FST.Arc<Long> target = fst.readFirstTargetArc(first, new FST.Arc<>(), fst.getBytesReader());
    System.out.println("target=" + target);

    // An empty-string output whose byte count is negative.
    ByteBuffersDataOutput negative = new ByteBuffersDataOutput();
    CodecUtil.writeHeader(negative, "FST", FST.VERSION_CURRENT);
    negative.writeByte((byte) 1);
    negative.writeVInt(-1);
    negative.writeByte((byte) 0);
    negative.writeVLong(0);
    negative.writeVLong(0);
    byte[] negativeBytes = negative.toArrayCopy();
    System.out.println(
        "negativeEmpty="
            + outcome(
                () ->
                    FST.readMetadata(
                        new ByteArrayDataInput(negativeBytes), PositiveIntOutputs.getSingleton())));

    // An input type byte outside 0..2.
    ByteBuffersDataOutput badType = new ByteBuffersDataOutput();
    CodecUtil.writeHeader(badType, "FST", FST.VERSION_CURRENT);
    badType.writeByte((byte) 0);
    badType.writeByte((byte) -1);
    byte[] badTypeBytes = badType.toArrayCopy();
    try {
      FST.readMetadata(new ByteArrayDataInput(badTypeBytes), PositiveIntOutputs.getSingleton());
      System.out.println("badType=ok");
    } catch (org.apache.lucene.index.CorruptIndexException e) {
      System.out.println("badType=" + e.getOriginalMessage());
    }
  }

  /** G-022: {@code QueryVisitor.termCollector} over a {@code java.util.HashSet}. */
  static void termCollector() {
    BooleanQuery.Builder builder = new BooleanQuery.Builder();
    String[][] terms = {
      {"body", "zebra"}, {"title", "apple"}, {"body", "mango"}, {"id", "42"}, {"body", "kiwi"},
      {"title", "pear"}, {"tag", "x"}, {"body", "banana"}, {"title", "zebra"},
    };
    for (String[] t : terms) {
      builder.add(new TermQuery(new Term(t[0], t[1])), BooleanClause.Occur.SHOULD);
    }
    builder.add(new PhraseQuery("body", "quick", "brown", "fox"), BooleanClause.Occur.MUST);
    Set<Term> collected = new HashSet<>();
    builder.build().visit(QueryVisitor.termCollector(collected));
    for (Term t : collected) {
      System.out.println(t.field() + ":" + t.text());
    }
  }

  /** The vectors {@code fma} scores. */
  static float[] fmaVector(int seed, int length) {
    float[] v = new float[length];
    for (int i = 0; i < length; i++) {
      v[i] = (float) Math.sin(seed * 31 + i * 7.3) * 1000.5f + i / 3f;
    }
    return v;
  }

  /** G-031: {@code Constants.HAS_FAST_SCALAR_FMA} and the float scores it decides. */
  static void fma() {
    System.out.println("HAS_FAST_SCALAR_FMA=" + Constants.HAS_FAST_SCALAR_FMA);
    for (int length : new int[] {3, 37, 64}) {
      float[] a = fmaVector(1, length);
      float[] b = fmaVector(2, length);
      System.out.println(
          "len="
              + length
              + " dot="
              + Integer.toHexString(Float.floatToRawIntBits(VectorUtil.dotProduct(a, b)))
              + " square="
              + Integer.toHexString(Float.floatToRawIntBits(VectorUtil.squareDistance(a, b)))
              + " cosine="
              + Integer.toHexString(Float.floatToRawIntBits(VectorUtil.cosine(a, b))));
    }
  }

  /**
   * G-031: the {@code Constants} fields read from HotSpot VM options ({@code Constants.java:89-106})
   * and the FMA choices they decide, followed by the {@code fma} case.
   */
  static void constants() throws Exception {
    for (String name :
        new String[] {
          "HAS_FMA", "HAS_AVX2", "HAS_SVE", "HAS_SSE4A", "MAX_VECTOR_SIZE",
          "HAS_FAST_VECTOR_FMA", "HAS_FAST_SCALAR_FMA"
        }) {
      java.lang.reflect.Field field = Constants.class.getDeclaredField(name);
      field.setAccessible(true);
      System.out.println(name + "=" + field.get(null));
    }
    fma();
  }

  /**
   * G-028: the {@code ExitingReaderException} messages of an {@code ExitableDirectoryReader}
   * whose timeout has fired, one per wrapped object ({@code ExitableDirectoryReader.java:439-914}).
   */
  static void exitable() throws Exception {
    Directory dir = new ByteBuffersDirectory();
    try (org.apache.lucene.index.IndexWriter w =
        new org.apache.lucene.index.IndexWriter(
            dir, new org.apache.lucene.index.IndexWriterConfig())) {
      for (int i = 0; i < 3; i++) {
        org.apache.lucene.document.Document d = new org.apache.lucene.document.Document();
        d.add(
            new org.apache.lucene.document.StringField(
                "s", "t" + i, org.apache.lucene.document.Field.Store.NO));
        d.add(new org.apache.lucene.document.NumericDocValuesField("n", i));
        d.add(new org.apache.lucene.document.SortedDocValuesField("sd", new BytesRef("v" + i)));
        d.add(new org.apache.lucene.document.BinaryDocValuesField("b", new BytesRef("v" + i)));
        d.add(new org.apache.lucene.document.SortedNumericDocValuesField("sn", i));
        d.add(new org.apache.lucene.document.SortedSetDocValuesField("ss", new BytesRef("v" + i)));
        d.add(new org.apache.lucene.document.IntPoint("p", i));
        d.add(new org.apache.lucene.document.KnnFloatVectorField("fv", new float[] {i, 1}));
        d.add(new org.apache.lucene.document.KnnByteVectorField("bv", new byte[] {(byte) i, 1}));
        w.addDocument(d);
      }
      w.forceMerge(1);
    }
    org.apache.lucene.index.QueryTimeout timeout =
        new org.apache.lucene.index.QueryTimeout() {
          @Override
          public boolean shouldExit() {
            return true;
          }

          @Override
          public String toString() {
            return "T";
          }
        };
    org.apache.lucene.index.QueryTimeout never =
        new org.apache.lucene.index.QueryTimeout() {
          @Override
          public boolean shouldExit() {
            return false;
          }

          @Override
          public String toString() {
            return "N";
          }
        };
    java.util.concurrent.atomic.AtomicBoolean fire = new java.util.concurrent.atomic.AtomicBoolean();
    org.apache.lucene.index.QueryTimeout switchable =
        new org.apache.lucene.index.QueryTimeout() {
          @Override
          public boolean shouldExit() {
            return fire.get();
          }

          @Override
          public String toString() {
            return "S";
          }
        };
    try (org.apache.lucene.index.DirectoryReader reader =
            new org.apache.lucene.index.ExitableDirectoryReader(
                org.apache.lucene.index.DirectoryReader.open(dir), timeout);
        org.apache.lucene.index.DirectoryReader raw =
            org.apache.lucene.index.DirectoryReader.open(dir)) {
      org.apache.lucene.index.LeafReader leaf = reader.leaves().get(0).reader();
      for (ThrowingRunnable call : exitableCalls(leaf)) {
        System.out.println(outcome(call));
      }
      org.apache.lucene.index.LeafReader rawLeaf = raw.leaves().get(0).reader();
      // The Thread.interrupted() branch, with a timeout that never fires.
      org.apache.lucene.index.LeafReader calm =
          new org.apache.lucene.index.ExitableDirectoryReader.ExitableFilterAtomicReader(
              rawLeaf, never);
      for (ThrowingRunnable call : exitableCalls(calm)) {
        Thread.currentThread().interrupt();
        System.out.println(outcome(call));
        Thread.interrupted();
      }
      // An exitable leaf over an exitable leaf: the messages name the inner wrappers.
      org.apache.lucene.index.LeafReader nested =
          new org.apache.lucene.index.ExitableDirectoryReader.ExitableFilterAtomicReader(
              calm, timeout);
      for (ThrowingRunnable call : exitableCalls(nested).subList(0, 9)) {
        System.out.println(outcome(call));
      }
      // A sorted view: the messages name the SortingCodecReader values.
      org.apache.lucene.index.LeafReader sorted =
          new org.apache.lucene.index.ExitableDirectoryReader.ExitableFilterAtomicReader(
              org.apache.lucene.index.SortingCodecReader.wrap(
                  (org.apache.lucene.index.CodecReader) rawLeaf,
                  new org.apache.lucene.search.Sort(
                      new org.apache.lucene.search.SortField(
                          "n", org.apache.lucene.search.SortField.Type.LONG, true))),
              timeout);
      java.util.List<ThrowingRunnable> sortedCalls = exitableCalls(sorted);
      for (int i : new int[] {0, 1, 2, 3, 4, 6, 7, 8}) {
        System.out.println(outcome(sortedCalls.get(i)));
      }
      // ExitablePointTree: the timeout names the point values, the interrupt the tree.
      org.apache.lucene.index.PointValues.PointTree tree =
          new org.apache.lucene.index.ExitableDirectoryReader.ExitableFilterAtomicReader(
                  rawLeaf, switchable)
              .getPointValues("p")
              .getPointTree();
      fire.set(true);
      System.out.println(outcome(tree::size));
      fire.set(false);
      Thread.currentThread().interrupt();
      System.out.println(outcome(tree::size));
      Thread.interrupted();
      // ExitableIntersectVisitor: the visitor interrupts its own thread.
      System.out.println(
          outcome(
              () ->
                  calm.getPointValues("p").getPointTree().visitDocValues(new InterruptingVisitor())));
      Thread.interrupted();
    }
  }

  /** The ten calls of {@link #exitable()}, each reaching one {@code ExitableDirectoryReader} check. */
  static java.util.List<ThrowingRunnable> exitableCalls(org.apache.lucene.index.LeafReader leaf) {
    return java.util.List.of(
        () -> leaf.getNumericDocValues("n").nextDoc(),
        () -> leaf.getSortedDocValues("sd").nextDoc(),
        () -> leaf.getBinaryDocValues("b").nextDoc(),
        () -> leaf.getSortedNumericDocValues("sn").nextDoc(),
        () -> leaf.getSortedSetDocValues("ss").nextDoc(),
        () -> leaf.terms("s").iterator().next(),
        () -> leaf.getPointValues("p"),
        () -> leaf.getFloatVectorValues("fv").iterator().nextDoc(),
        () -> leaf.getByteVectorValues("bv").iterator().nextDoc(),
        () ->
            leaf.searchNearestVectors(
                "fv",
                new float[] {1, 1},
                2,
                org.apache.lucene.search.AcceptDocs.fromLiveDocs(null, 3),
                100));
  }

  /** A visitor that interrupts its thread on its first callback. */
  static final class InterruptingVisitor
      implements org.apache.lucene.index.PointValues.IntersectVisitor {
    private boolean interrupted;

    private void interruptOnce() {
      if (interrupted == false) {
        interrupted = true;
        Thread.currentThread().interrupt();
      }
    }

    @Override
    public void visit(int docID) {
      interruptOnce();
    }

    @Override
    public void visit(int docID, byte[] packedValue) {
      interruptOnce();
    }

    @Override
    public org.apache.lucene.index.PointValues.Relation compare(
        byte[] minPackedValue, byte[] maxPackedValue) {
      interruptOnce();
      return org.apache.lucene.index.PointValues.Relation.CELL_CROSSES_QUERY;
    }

    @Override
    public void grow(int count) {
      interruptOnce();
    }
  }

  /** G-032: {@code SloppyMath} results, seeded from {@code StrictMath}. */
  static void sloppy() {
    long bits = 0x9E3779B97F4A7C15L;
    for (int i = 0; i < 2000; i++) {
      bits = bits * 6364136223846793005L + 1442695040888963407L;
      double x = ((bits >>> 11) * 0x1.0p-53 - 0.5) * 40;
      double y = (bits >>> 11) * 0x1.0p-53 * 2 - 1;
      double lat = y * 90;
      double lon = ((bits >>> 7) % 360000) / 1000.0 - 180;
      System.out.println(
          Long.toHexString(Double.doubleToRawLongBits(SloppyMath.cos(x)))
              + " "
              + Long.toHexString(Double.doubleToRawLongBits(SloppyMath.sin(x)))
              + " "
              + Long.toHexString(Double.doubleToRawLongBits(SloppyMath.asin(y)))
              + " "
              + Long.toHexString(
                  Double.doubleToRawLongBits(SloppyMath.haversinMeters(lat, lon, -lat / 2, lon / 3))));
    }
  }

  /** G-429, G-430: {@code MemorySegmentIndexInput} failures on a 100-byte file. */
  static void mmap(Path dirPath) throws Exception {
    try (MMapDirectory dir = new MMapDirectory(dirPath)) {
      IndexInput in = dir.openInput("f.bin", IOContext.DEFAULT);
      System.out.println("readBytes=" + outcome(() -> in.readBytes(new byte[8], 5, 10)));
      System.out.println("readBytesNegative=" + outcome(() -> in.readBytes(new byte[8], -1, 2)));
      System.out.println(
          "readBytesPositional="
              + outcome(
                  () -> {
                    RandomAccessInput slice = in.randomAccessSlice(0, in.length());
                    slice.readBytes(3L, new byte[8], 5, 10);
                  }));
      System.out.println("pointer=" + in.getFilePointer());
      in.close();
      System.out.println("closedPointer=" + outcome(in::getFilePointer));
    }
  }
}
