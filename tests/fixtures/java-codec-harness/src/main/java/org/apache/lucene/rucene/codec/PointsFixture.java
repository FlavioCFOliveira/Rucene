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
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.TreeSet;

import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.BinaryPoint;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.PointValues;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Writes a single-segment Apache Lucene Core 10.5.0 index whose only content is
 * point values, so that the resulting {@code .kdd}, {@code .kdi} and
 * {@code .kdm} files depend only on {@code PointValuesWriter}, on
 * {@code Lucene90PointsWriter} and on {@code BKDWriter}.
 *
 * <p>Every case is a fixed, algorithmically generated table of documents and
 * values that the Rust portability test mirrors exactly, so no analyzer takes
 * part: a byte difference can only come from the point-values writer or from
 * the points codec. The order in which each field is first seen fixes the field
 * numbers, and the order the indexing chain's field-hash table flushes in fixes
 * the order of the per-field entries inside the {@code .kdm}.
 *
 * <p>The cases span the shape dimensions of the format:
 *
 * <ul>
 *   <li>{@code int1d} — one 4-byte, one-dimensional field over 1300 documents,
 *       so the BKD tree has several leaves, with heavily repeated values;
 *   <li>{@code multi1d} — several values per document and documents with none,
 *       so {@code numPoints} and {@code numDocs} genuinely differ;
 *   <li>{@code long1d} — 8 bytes per dimension, spanning negative and positive
 *       values so the sortable encoding is exercised;
 *   <li>{@code bin1d} — 16 bytes per dimension, which is
 *       {@code PointValues.MAX_NUM_BYTES};
 *   <li>{@code small} — a handful of documents, so the whole tree is one leaf;
 *   <li>{@code mixed} — five one-dimensional fields in one segment, named so
 *       that the indexing chain's field-hash order is neither the registration
 *       order nor the field-number order;
 *   <li>{@code nd2} — two indexed dimensions over enough documents for several
 *       leaves. See {@link #main} for what this case can and cannot prove.
 * </ul>
 *
 * <p>Command-line usage: {@code java ... PointsFixture <dir> <case>}.
 */
public final class PointsFixture {

  private PointsFixture() {}

  /** One field of one document: the packed dimensions of a single point. */
  record Point(String field, int[] ints, long[] longs, byte[] binary, int indexDims) {
    static Point ints(String field, int... values) {
      return new Point(field, values, null, null, values.length);
    }

    static Point longs(String field, long... values) {
      return new Point(field, null, values, null, values.length);
    }

    static Point binary(String field, byte[] value) {
      return new Point(field, null, null, value, 1);
    }

    /** An int point whose leading {@code indexDims} dimensions are indexed. */
    static Point selective(String field, int indexDims, int... values) {
      return new Point(field, values, null, null, indexDims);
    }
  }

  /** The documents of one case, each a list of points. */
  static List<List<Point>> documents(String shape) {
    List<List<Point>> docs = new ArrayList<>();
    switch (shape) {
      case "int1d" -> {
        // 1300 points is more than two default leaves of 512, and the values
        // repeat heavily so that leaves share long common prefixes.
        for (int doc = 0; doc < 1300; doc++) {
          docs.add(List.of(Point.ints("p", (doc * 7919) % 1000)));
        }
      }
      case "multi1d" -> {
        // Every third document carries no point at all, and the others carry
        // one to three, so numPoints > numDocs and the field is sparse.
        for (int doc = 0; doc < 400; doc++) {
          if (doc % 3 == 0) {
            docs.add(List.of());
            continue;
          }
          List<Point> points = new ArrayList<>();
          for (int k = 0; k <= doc % 3; k++) {
            points.add(Point.ints("m", doc * 10 + k));
          }
          docs.add(points);
        }
      }
      case "long1d" -> {
        for (int doc = 0; doc < 700; doc++) {
          long value = ((long) doc * 1_000_000_007L) % 4_000_000_000L - 2_000_000_000L;
          docs.add(List.of(Point.longs("l", value)));
        }
      }
      case "bin1d" -> {
        for (int doc = 0; doc < 300; doc++) {
          docs.add(List.of(Point.binary("b", binary16(doc))));
        }
      }
      case "small" -> {
        for (int doc = 0; doc < 7; doc++) {
          docs.add(List.of(Point.ints("s", 7 - doc)));
        }
      }
      case "mixed" -> {
        // The five names are the ones whose field-hash order is already pinned
        // by the doc-values suite: the table flushes them 3, 0, 1, 2, 4, which
        // is neither registration nor field-number order. Each field has its
        // own presence pattern so the field numbers are fixed by first use.
        String[] names = {"mnum", "mbin", "msort", "msnum", "mss"};
        for (int doc = 0; doc < 600; doc++) {
          List<Point> points = new ArrayList<>();
          for (int f = 0; f < names.length; f++) {
            if (doc % (f + 1) == 0) {
              points.add(Point.ints(names[f], (doc * (f * 31 + 7)) % 997));
            }
          }
          docs.add(points);
        }
      }
      case "nd2" -> {
        for (int doc = 0; doc < 900; doc++) {
          docs.add(List.of(Point.ints("g", (doc * 7919) % 601, (doc * 104729) % 397)));
        }
      }
      case "ndmulti" -> {
        // Two points per document over two indexed dimensions, with so few
        // distinct values that many pairs collide. `sortByDim` compares the
        // sorted dimension, then the *unindexed* data dimensions — of which
        // there are none here — then the doc ID, so two points of the same
        // document that agree on the sorted dimension tie outright, while
        // still differing in the other indexed dimension that the leaf writes.
        // Which of them is written first is decided by `IntroSorter`, and no
        // single-valued case can express that.
        for (int doc = 0; doc < 400; doc++) {
          docs.add(
              List.of(
                  Point.ints("m2", (doc * 7919) % 3, (doc * 104729) % 3),
                  Point.ints("m2", (doc * 7919) % 3, ((doc * 104729) + 1) % 3)));
        }
      }
      case "nddeep" -> {
        // Three indexed dimensions over more than sixteen leaves, so the
        // recursion reaches a node whose parent splits sum to a multiple of
        // SPLITS_BEFORE_EXACT_BOUNDS. That is the only place `build`
        // recomputes the exact bounds of a node, and the only shape in which
        // omitting the recomputation changes which dimension is split.
        // The three dimensions carry the same value. Correlation is not the
        // trigger, only the cheapest way to reach it: what the recomputation
        // needs is a node at split depth four or more, under more than two
        // indexed dimensions, whose inherited bounds are loose enough to
        // change what `split` picks. Correlated dimensions make the bounds
        // loose at once, so this shape fires at 12000 points. Independent
        // dimensions were measured NOT to fire up to 20000 points when drawn
        // uniformly over 31 bits, but they do fire at 70000 when drawn into a
        // narrow range — depth reaches the same place, six times more
        // expensively. That is why this fixture is correlated.
        for (int doc = 0; doc < 12000; doc++) {
          int v = (doc * 7919) % 1009;
          docs.add(List.of(Point.ints("d3", v, v, v)));
        }
      }
      case "ndsplit" -> {
        // 513 points over two indexed dimensions and a leaf size of 512: two
        // leaves, so exactly one partition, and the leaf that follows it starts
        // at whatever point the *selection* left there. Values are spread wide
        // enough that no dimension is near-constant, which is what makes the
        // per-dimension byte cardinalities close enough for the choice of
        // sorted dimension to hinge on that one point.
        //
        // This is the shape that diverged until `RadixSelector` was ported: a
        // full sort leaves the smallest point first, Lucene's selection leaves
        // an arbitrary one, and the two then compress the leaf on different
        // dimensions.
        long state = 1L;
        for (int doc = 0; doc < 513; doc++) {
          int[] values = new int[2];
          for (int dim = 0; dim < 2; dim++) {
            state ^= state >>> 12;
            state ^= state << 25;
            state ^= state >>> 27;
            values[dim] = (int) ((state * 0x2545F4914F6CDD1DL) >>> 33);
          }
          docs.add(List.of(Point.ints("s2", values)));
        }
      }
      case "ndsel" -> {
        // Three data dimensions of which only the first is indexed. The BKD
        // tree splits on dimension 0 alone, but every leaf still stores all
        // three, and the two unindexed ones take part in the tie-break that
        // orders points inside a leaf. No other case here has
        // `numDims != numIndexDims`.
        for (int doc = 0; doc < 400; doc++) {
          docs.add(
              List.of(
                  Point.selective(
                      "j", 1, (doc * 7919) % 211, (doc * 104729) % 17, (doc * 2654435) % 5)));
        }
      }
      case "ndsort" -> {
        // Built so that the leaf's sorted dimension depends on whether the
        // first point of the leaf is counted when the per-dimension byte
        // cardinalities are measured. `BKDWriter.build` over a MutablePointTree
        // counts from `from + 1` (`BKDWriter.java:1688`), so document 0's
        // dimension-0 byte — which no other document carries — does not count:
        // dimensions 0 and 1 then tie on three distinct bytes and dimension 0
        // wins. Counting from `from` instead gives dimension 0 four and picks
        // dimension 1, which writes a different compressed dimension and a
        // different point order.
        //
        // Every value stays inside one byte, so each dimension's common prefix
        // is three of its four bytes and the byte that is counted is the value
        // itself. Dimension 2 never varies, so it has a full common prefix and
        // is not a candidate at all.
        for (int doc = 0; doc < 200; doc++) {
          int d0 = doc == 0 ? 200 : 1 + (doc % 3);
          int d1 = doc == 0 ? 1 : 1 + ((doc * 7) % 3);
          docs.add(List.of(Point.ints("k", d0, d1, 7)));
        }
      }
      case "ndleaf" -> {
        // Three indexed dimensions over fewer documents than a leaf holds, so
        // the whole field is one leaf that is also the root. That is where the
        // sorted-dimension choice is made with no partition above it, and it is
        // the shape the multi-leaf cases cannot express.
        for (int doc = 0; doc < 300; doc++) {
          docs.add(
              List.of(
                  Point.ints(
                      "h", (doc * 7919) % 53, (doc * 104729) % 29, (doc * 2654435) % 11)));
        }
      }
      default -> throw new IllegalArgumentException("unknown shape: " + shape);
    }
    return docs;
  }

  /** A deterministic 16-byte value for {@code bin1d}. */
  static byte[] binary16(int doc) {
    byte[] value = new byte[16];
    long high = (long) doc * 0x9E3779B97F4A7C15L;
    long low = (long) (doc % 17) * 0xC2B2AE3D27D4EB4FL;
    for (int i = 0; i < 8; i++) {
      value[i] = (byte) (high >>> (56 - 8 * i));
      value[8 + i] = (byte) (low >>> (56 - 8 * i));
    }
    return value;
  }

  /**
   * The inclusive range each case queries, as packed lower and upper bounds.
   * Chosen to accept a proper subset of the corpus so that the traversal has
   * to prune, which is what makes the comparison test the tree and not just
   * the leaves.
   */
  static byte[][] range(String shape) {
    return switch (shape) {
      case "int1d" -> new byte[][] {packInts(250), packInts(750)};
      case "multi1d" -> new byte[][] {packInts(1000), packInts(2500)};
      case "long1d" -> new byte[][] {packLongs(-500_000_000L), packLongs(500_000_000L)};
      case "bin1d" -> new byte[][] {binary16(0), binary16(299)};
      case "small" -> new byte[][] {packInts(3), packInts(6)};
      case "mixed" -> new byte[][] {packInts(100), packInts(400)};
      case "nd2" -> new byte[][] {packInts(100, 50), packInts(500, 300)};
      case "ndleaf" -> new byte[][] {packInts(5, 3, 2), packInts(40, 25, 9)};
      case "ndsort" -> new byte[][] {packInts(1, 1, 7), packInts(2, 2, 7)};
      // Only dimension 0 is indexed, so the range has one dimension.
      case "ndsel" -> new byte[][] {packInts(40), packInts(160)};
      case "ndmulti" -> new byte[][] {packInts(0, 0), packInts(1, 1)};
      case "nddeep" -> new byte[][] {packInts(200, 200, 200), packInts(800, 800, 800)};
      case "ndsplit" ->
          new byte[][] {packInts(200_000_000, 200_000_000), packInts(800_000_000, 800_000_000)};
      default -> throw new IllegalArgumentException("unknown shape: " + shape);
    };
  }

  static byte[] packInts(int... values) {
    byte[] packed = new byte[values.length * Integer.BYTES];
    for (int dim = 0; dim < values.length; dim++) {
      IntPoint.encodeDimension(values[dim], packed, dim * Integer.BYTES);
    }
    return packed;
  }

  static byte[] packLongs(long... values) {
    byte[] packed = new byte[values.length * Long.BYTES];
    for (int dim = 0; dim < values.length; dim++) {
      LongPoint.encodeDimension(values[dim], packed, dim * Long.BYTES);
    }
    return packed;
  }

  public static void main(String[] args) throws IOException {
    if (args.length != 2) {
      System.err.println("Usage: PointsFixture <dir> <case>");
      System.exit(1);
    }
    Path dirPath = Paths.get(args[0]);
    String shape = args[1];
    List<List<Point>> documents = documents(shape);

    try (FSDirectory dir = FSDirectory.open(dirPath)) {
      IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
      config.setCodec(new Lucene104Codec());
      config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      config.setUseCompoundFile(false);
      // One segment, flushed once: the byte comparison needs a single, fully
      // written segment and no merge to reorder anything.
      // The RAM buffer keeps its 16 MB default: at least one of the two
      // triggers must stay enabled, and no case here comes close to it.
      config.setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH);

      try (IndexWriter writer = new IndexWriter(dir, config)) {
        for (List<Point> points : documents) {
          Document document = new Document();
          for (Point point : points) {
            if (point.ints() != null && point.indexDims() < point.ints().length) {
              // Selective indexing: Lucene has no ready-made point class for
              // it, so the field type is built by hand exactly as `IntPoint`
              // builds its own, but with a smaller index dimension count.
              FieldType type = new FieldType();
              type.setDimensions(point.ints().length, point.indexDims(), Integer.BYTES);
              type.freeze();
              document.add(
                  new Field(point.field(), new BytesRef(packInts(point.ints())), type));
            } else if (point.ints() != null) {
              document.add(new IntPoint(point.field(), point.ints()));
            } else if (point.longs() != null) {
              document.add(new LongPoint(point.field(), point.longs()));
            } else {
              document.add(new BinaryPoint(point.field(), point.binary()));
            }
          }
          writer.addDocument(document);
        }
        writer.commit();
      }

      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      if (infos.size() != 1) {
        throw new IllegalStateException("expected exactly one segment, got " + infos.size());
      }
      SegmentCommitInfo commit = infos.info(0);
      System.out.println("fixture=PointsFixture");
      System.out.println("case=" + shape);
      System.out.println("segment=" + commit.info.name);
      System.out.println("segment_id=" + IndexingChainFixture.hex(commit.info.getId()));
      System.out.println("max_doc=" + commit.info.maxDoc());
      System.out.println("compound=" + commit.info.getUseCompoundFile());

      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        if (reader.leaves().size() != 1) {
          throw new IllegalStateException("expected one leaf, got " + reader.leaves().size());
        }
        LeafReaderContext leaf = reader.leaves().get(0);
        for (FieldInfo fi : leaf.reader().getFieldInfos()) {
          System.out.println(
              "fieldinfo="
                  + fi.number
                  + " "
                  + fi.name
                  + " dims="
                  + fi.getPointDimensionCount()
                  + " indexDims="
                  + fi.getPointIndexDimensionCount()
                  + " bytes="
                  + fi.getPointNumBytes());
        }
        byte[][] range = range(shape);
        for (FieldInfo fi : leaf.reader().getFieldInfos()) {
          if (fi.getPointDimensionCount() == 0) {
            continue;
          }
          PointValues values = leaf.reader().getPointValues(fi.name);
          dump(fi.name, values);
          dumpRange(fi.name, values, range[0], range[1]);
        }
      }
      System.out.println("read_ok=true");
    }
  }

  /**
   * Prints the statistics and every {@code (docID, packedValue)} pair of one
   * field, in the order {@code intersect} visits them.
   *
   * <p>Visit order is tree order, so two indexes that hold the same points in a
   * different tree produce different — but equally correct — line orders. The
   * Rust side compares the lines as a sequence where the trees must be
   * identical and as a multiset where they need not be.
   */
  static void dump(String field, PointValues values) throws IOException {
    System.out.println(
        "pointstats field="
            + field
            + " size="
            + values.size()
            + " docCount="
            + values.getDocCount()
            + " numDims="
            + values.getNumDimensions()
            + " numIndexDims="
            + values.getNumIndexDimensions()
            + " bytesPerDim="
            + values.getBytesPerDimension()
            + " min="
            + IndexingChainFixture.hex(values.getMinPackedValue())
            + " max="
            + IndexingChainFixture.hex(values.getMaxPackedValue()));
    values.intersect(
        new PointValues.IntersectVisitor() {
          @Override
          public void visit(int docID) {
            throw new IllegalStateException("a CELL_CROSSES_QUERY visitor never gets bare doc IDs");
          }

          @Override
          public void visit(int docID, byte[] packedValue) {
            System.out.println(
                "point field="
                    + field
                    + " doc="
                    + docID
                    + " value="
                    + IndexingChainFixture.hex(packedValue));
          }

          @Override
          public PointValues.Relation compare(byte[] minPackedValue, byte[] maxPackedValue) {
            return PointValues.Relation.CELL_CROSSES_QUERY;
          }
        });
  }

  /**
   * Prints the sorted, deduplicated doc IDs whose point falls inside the
   * inclusive box {@code [lower, upper]}, found through a real pruning
   * traversal.
   *
   * <p>Unlike {@link #dump}, this visitor implements {@code compare} properly,
   * so subtrees outside the box are skipped and subtrees inside it are accepted
   * without looking at the values. That is what exercises the tree rather than
   * only its leaves, and the result is order-independent, so it compares
   * cleanly across two different tree layouts holding the same points.
   */
  static void dumpRange(String field, PointValues values, byte[] lower, byte[] upper)
      throws IOException {
    int numIndexDims = values.getNumIndexDimensions();
    int bytesPerDim = values.getBytesPerDimension();
    TreeSet<Integer> accepted = new TreeSet<>();
    values.intersect(
        new PointValues.IntersectVisitor() {
          @Override
          public void visit(int docID) {
            accepted.add(docID);
          }

          @Override
          public void visit(int docID, byte[] packedValue) {
            for (int dim = 0; dim < numIndexDims; dim++) {
              int offset = dim * bytesPerDim;
              if (compareDim(packedValue, lower, offset, bytesPerDim) < 0
                  || compareDim(packedValue, upper, offset, bytesPerDim) > 0) {
                return;
              }
            }
            accepted.add(docID);
          }

          @Override
          public PointValues.Relation compare(byte[] minPackedValue, byte[] maxPackedValue) {
            boolean crosses = false;
            for (int dim = 0; dim < numIndexDims; dim++) {
              int offset = dim * bytesPerDim;
              if (compareDim(maxPackedValue, lower, offset, bytesPerDim) < 0
                  || compareDim(minPackedValue, upper, offset, bytesPerDim) > 0) {
                return PointValues.Relation.CELL_OUTSIDE_QUERY;
              }
              crosses |=
                  compareDim(minPackedValue, lower, offset, bytesPerDim) < 0
                      || compareDim(maxPackedValue, upper, offset, bytesPerDim) > 0;
            }
            return crosses
                ? PointValues.Relation.CELL_CROSSES_QUERY
                : PointValues.Relation.CELL_INSIDE_QUERY;
          }
        });
    StringBuilder line = new StringBuilder("range field=").append(field).append(" docs=");
    boolean first = true;
    for (int doc : accepted) {
      if (!first) {
        line.append(',');
      }
      line.append(doc);
      first = false;
    }
    System.out.println(line);
  }

  /** Unsigned comparison of one dimension of {@code a} against {@code b}. */
  static int compareDim(byte[] a, byte[] b, int offset, int bytesPerDim) {
    return java.util.Arrays.compareUnsigned(
        a, offset, offset + bytesPerDim, b, offset, offset + bytesPerDim);
  }
}
