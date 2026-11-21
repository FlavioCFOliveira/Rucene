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
import java.util.Arrays;
import java.util.Base64;
import java.util.List;

import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.PointValues;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.Version;

/**
 * Captures the observable behaviour of {@link PointValues} on a real BKD index:
 * the tree geometry, the full call trace of {@code intersect}, and the two
 * estimators.
 *
 * <p>The trace is produced by a visitor that overrides only the single-document
 * callbacks ({@code visit(int)}, {@code visit(int, byte[])}), {@code compare}
 * and {@code grow}. The bulk callbacks keep their interface defaults, which fan
 * out to the single-document ones. That matters: {@code DocIdsWriter} picks
 * between {@code visit(IntsRef)}, {@code visit(DocIdSetIterator)} and
 * {@code visit(int)} according to how the doc IDs of a leaf were encoded, so
 * only the fanned-out trace is a property of the traversal algorithm rather
 * than of the doc-ID codec.
 *
 * <p>Usage:
 * <pre>
 *   mvn -q -f tests/fixtures/java-codec-harness/pom.xml \
 *       exec:java -Dexec.mainClass=org.apache.lucene.rucene.codec.PointTreeFixture \
 *       -Dexec.args="/tmp/rucene-point-fixtures ONE_LEAF_1D"
 * </pre>
 * where CASE is ONE_LEAF_1D, ONE_LEAF_2D, MULTI_VALUED_1D, MULTI_LEAF_1D or
 * MULTI_LEAF_2D.
 */
public final class PointTreeFixture {

  private PointTreeFixture() {}

  private static final String FIELD = "p";

  public static void main(String[] args) throws IOException {
    if (args.length != 2) {
      System.err.println("Usage: PointTreeFixture <output-dir> <case>");
      System.err.println("Cases: ONE_LEAF_1D, ONE_LEAF_2D, MULTI_VALUED_1D, MULTI_LEAF_1D, MULTI_LEAF_2D");
      System.exit(1);
    }
    Path outputDir = Paths.get(args[0]);
    String testCase = args[1];
    Files.createDirectories(outputDir);

    IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
    config.setCodec(new Lucene104Codec());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    config.setUseCompoundFile(false);

    try (FSDirectory dir = FSDirectory.open(outputDir)) {
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        writeDocuments(writer, testCase);
        writer.commit();
      }

      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        if (reader.leaves().size() != 1) {
          throw new IllegalStateException("expected a single segment, got " + reader.leaves().size());
        }
        LeafReader leaf = reader.leaves().get(0).reader();
        PointValues values = leaf.getPointValues(FIELD);
        if (values == null) {
          throw new IllegalStateException("no point values for field " + FIELD);
        }

        System.out.println("fixture=PointTreeFixture");
        System.out.println("version=" + Version.LATEST);
        System.out.println("case=" + testCase);
        System.out.println("field=" + FIELD);
        System.out.println("num_dims=" + values.getNumDimensions());
        System.out.println("num_index_dims=" + values.getNumIndexDimensions());
        System.out.println("bytes_per_dim=" + values.getBytesPerDimension());
        System.out.println("size=" + values.size());
        System.out.println("doc_count=" + values.getDocCount());
        System.out.println("min_packed=" + b64(values.getMinPackedValue()));
        System.out.println("max_packed=" + b64(values.getMaxPackedValue()));

        emitTreeWalk(values);
        emitLeafContents(values);
        for (Query query : queries(testCase, values.getBytesPerDimension())) {
          emitQuery(values, query);
        }
      }
    }
  }

  // ---------------------------------------------------------------------------
  // Corpus
  // ---------------------------------------------------------------------------

  private static void writeDocuments(IndexWriter writer, String testCase) throws IOException {
    switch (testCase) {
      case "ONE_LEAF_1D" -> {
        // Deliberately unsorted on insertion and with a duplicated value, so
        // the visit order (increasing value, ties by increasing doc ID) is
        // observable and differs from the doc order. The count is odd on
        // purpose: `estimatePointCount` rounds a crossing leaf up with
        // `(size + 1) / 2`, and an even size would hide the rounding.
        int[] points = {40, 10, 30, 20, 30, 60, 50, 70, 15};
        for (int value : points) {
          Document doc = new Document();
          doc.add(new IntPoint(FIELD, value));
          writer.addDocument(doc);
        }
      }
      case "MULTI_VALUED_1D" -> {
        // Three points per document, so size() > getDocCount() and
        // estimateDocCount takes its urn-problem branch instead of returning
        // the point estimate unchanged.
        int[][] perDoc = {{5, 45, 85}, {15, 55, 95}, {25, 65, 105}, {35, 75, 115}, {40, 80, 120}};
        for (int[] values : perDoc) {
          Document doc = new Document();
          for (int value : values) {
            doc.add(new IntPoint(FIELD, value));
          }
          writer.addDocument(doc);
        }
      }
      case "ONE_LEAF_2D" -> {
        int[][] points = {{10, 90}, {50, 20}, {30, 30}, {70, 10}, {20, 60}, {60, 80}};
        for (int[] point : points) {
          Document doc = new Document();
          doc.add(new IntPoint(FIELD, point[0], point[1]));
          writer.addDocument(doc);
        }
      }
      case "MULTI_LEAF_1D" -> {
        // Well beyond BKDConfig.DEFAULT_MAX_POINTS_IN_LEAF_NODE (512), so the
        // tree has several levels.
        for (int i = 0; i < 2000; i++) {
          Document doc = new Document();
          doc.add(new IntPoint(FIELD, (i * 7919) % 4001));
          writer.addDocument(doc);
        }
      }
      case "MULTI_LEAF_2D" -> {
        // A 2D tree large enough to build several levels, exercising the
        // multi-dimensional BKD cursor (per-level split dimensions, leaf-bound
        // refinement `compare`).
        for (int i = 0; i < 2000; i++) {
          Document doc = new Document();
          doc.add(new IntPoint(FIELD, (i * 7919) % 4001, (i * 5003) % 4001));
          writer.addDocument(doc);
        }
      }
      default -> throw new IllegalArgumentException("unknown case: " + testCase);
    }
  }

  /** A range query over every index dimension. */
  private record Query(String name, byte[] min, byte[] max) {}

  private static List<Query> queries(String testCase, int bytesPerDim) {
    List<Query> queries = new ArrayList<>();
    switch (testCase) {
      case "ONE_LEAF_1D", "MULTI_LEAF_1D", "MULTI_VALUED_1D" -> {
        queries.add(range1d("all", Integer.MIN_VALUE, Integer.MAX_VALUE, bytesPerDim));
        queries.add(range1d("none", 100000, 200000, bytesPerDim));
        queries.add(range1d("partial", 25, 55, bytesPerDim));
        queries.add(range1d("single", 30, 30, bytesPerDim));
        queries.add(range1d("prefix", Integer.MIN_VALUE, 35, bytesPerDim));
      }
      case "ONE_LEAF_2D" -> {
        queries.add(range2d("all", Integer.MIN_VALUE, Integer.MAX_VALUE, Integer.MIN_VALUE,
            Integer.MAX_VALUE, bytesPerDim));
        queries.add(range2d("none", 1000, 2000, 1000, 2000, bytesPerDim));
        queries.add(range2d("box", 15, 55, 15, 65, bytesPerDim));
      }
      case "MULTI_LEAF_2D" -> {
        // The corpus spans 0..4001 in both dimensions; these three ranges cover
        // CELL_INSIDE_QUERY (all), CELL_OUTSIDE_QUERY (none) and
        // CELL_CROSSES_QUERY (box) at the root.
        queries.add(range2d("all", Integer.MIN_VALUE, Integer.MAX_VALUE, Integer.MIN_VALUE,
            Integer.MAX_VALUE, bytesPerDim));
        queries.add(range2d("none", 10000, 20000, 10000, 20000, bytesPerDim));
        queries.add(range2d("box", 0, 2000, 0, 2000, bytesPerDim));
      }
      default -> throw new IllegalArgumentException("unknown case: " + testCase);
    }
    return queries;
  }

  private static Query range1d(String name, int lower, int upper, int bytesPerDim) {
    byte[] min = new byte[bytesPerDim];
    byte[] max = new byte[bytesPerDim];
    IntPoint.encodeDimension(lower, min, 0);
    IntPoint.encodeDimension(upper, max, 0);
    return new Query(name, min, max);
  }

  private static Query range2d(
      String name, int lowerX, int upperX, int lowerY, int upperY, int bytesPerDim) {
    byte[] min = new byte[2 * bytesPerDim];
    byte[] max = new byte[2 * bytesPerDim];
    IntPoint.encodeDimension(lowerX, min, 0);
    IntPoint.encodeDimension(upperX, max, 0);
    IntPoint.encodeDimension(lowerY, min, bytesPerDim);
    IntPoint.encodeDimension(upperY, max, bytesPerDim);
    return new Query(name, min, max);
  }

  // ---------------------------------------------------------------------------
  // Emission
  // ---------------------------------------------------------------------------

  private static void emitTreeWalk(PointValues values) throws IOException {
    List<String> lines = new ArrayList<>();
    walk(values.getPointTree(), "0", lines);
    System.out.println("node_count=" + lines.size());
    lines.forEach(System.out::println);
  }

  private static void walk(PointValues.PointTree tree, String path, List<String> out)
      throws IOException {
    out.add(
        "node path="
            + path
            + " min="
            + b64(tree.getMinPackedValue())
            + " max="
            + b64(tree.getMaxPackedValue())
            + " size="
            + tree.size());
    if (tree.moveToChild()) {
      int child = 0;
      do {
        walk(tree, path + "." + child, out);
        child++;
      } while (tree.moveToSibling());
      tree.moveToParent();
    }
  }

  /**
   * Prints every stored point in traversal order, by forcing the traversal to
   * treat every cell as crossing.
   */
  private static void emitLeafContents(PointValues values) throws IOException {
    TracingVisitor scan =
        new TracingVisitor(null, null, values.getNumIndexDimensions(), values.getBytesPerDimension());
    scan.forceCrosses = true;
    values.intersect(scan);
    for (String point : scan.points) {
      System.out.println("point " + point);
    }
  }

  private static void emitQuery(PointValues values, Query query) throws IOException {
    System.out.println(
        "query name=" + query.name() + " min=" + b64(query.min()) + " max=" + b64(query.max()));

    TracingVisitor visitor =
        new TracingVisitor(
            query.min(), query.max(), values.getNumIndexDimensions(), values.getBytesPerDimension());
    values.intersect(visitor);
    for (String entry : visitor.trace) {
      System.out.println("trace " + query.name() + " " + entry);
    }
    System.out.println(
        "accepted " + query.name() + " " + visitor.accepted.stream().map(String::valueOf)
            .reduce((a, b) -> a + "," + b).orElse(""));

    TracingVisitor estimator =
        new TracingVisitor(
            query.min(), query.max(), values.getNumIndexDimensions(), values.getBytesPerDimension());
    long pointCount = values.estimatePointCount(estimator);
    long docCount = values.estimateDocCount(estimator);
    System.out.println(
        "estimate " + query.name() + " point_count=" + pointCount + " doc_count=" + docCount);
  }

  // ---------------------------------------------------------------------------
  // Visitor
  // ---------------------------------------------------------------------------

  /**
   * Records every callback the traversal makes.
   *
   * <p>Only the single-document callbacks are overridden; the bulk ones keep
   * their interface defaults so the recorded trace does not depend on the
   * doc-ID encoding of a leaf.
   */
  private static final class TracingVisitor implements PointValues.IntersectVisitor {
    private final byte[] queryMin;
    private final byte[] queryMax;
    private final int numIndexDims;
    private final int bytesPerDim;
    final List<String> trace = new ArrayList<>();
    final List<Integer> accepted = new ArrayList<>();
    final List<String> points = new ArrayList<>();
    boolean forceCrosses;

    TracingVisitor(byte[] queryMin, byte[] queryMax, int numIndexDims, int bytesPerDim) {
      this.queryMin = queryMin;
      this.queryMax = queryMax;
      this.numIndexDims = numIndexDims;
      this.bytesPerDim = bytesPerDim;
    }

    @Override
    public void visit(int docID) {
      trace.add("visit " + docID);
      accepted.add(docID);
    }

    @Override
    public void visit(int docID, byte[] packedValue) {
      trace.add("visitv " + docID + " " + b64(packedValue));
      points.add("doc=" + docID + " value=" + b64(packedValue));
      if (matches(packedValue)) {
        accepted.add(docID);
      }
    }

    @Override
    public void grow(int count) {
      trace.add("grow " + count);
    }

    @Override
    public PointValues.Relation compare(byte[] minPackedValue, byte[] maxPackedValue) {
      PointValues.Relation relation = relate(minPackedValue, maxPackedValue);
      trace.add("compare " + b64(minPackedValue) + " " + b64(maxPackedValue) + " " + relation);
      return relation;
    }

    private PointValues.Relation relate(byte[] cellMin, byte[] cellMax) {
      if (forceCrosses) {
        return PointValues.Relation.CELL_CROSSES_QUERY;
      }
      boolean inside = true;
      for (int dim = 0; dim < numIndexDims; dim++) {
        int offset = dim * bytesPerDim;
        if (compareDim(cellMax, offset, queryMin, offset) < 0
            || compareDim(cellMin, offset, queryMax, offset) > 0) {
          return PointValues.Relation.CELL_OUTSIDE_QUERY;
        }
        if (compareDim(cellMin, offset, queryMin, offset) < 0
            || compareDim(cellMax, offset, queryMax, offset) > 0) {
          inside = false;
        }
      }
      return inside
          ? PointValues.Relation.CELL_INSIDE_QUERY
          : PointValues.Relation.CELL_CROSSES_QUERY;
    }

    private boolean matches(byte[] packedValue) {
      if (forceCrosses) {
        return false;
      }
      for (int dim = 0; dim < numIndexDims; dim++) {
        int offset = dim * bytesPerDim;
        if (compareDim(packedValue, offset, queryMin, offset) < 0
            || compareDim(packedValue, offset, queryMax, offset) > 0) {
          return false;
        }
      }
      return true;
    }

    private int compareDim(byte[] a, int aOffset, byte[] b, int bOffset) {
      return Arrays.compareUnsigned(
          a, aOffset, aOffset + bytesPerDim, b, bOffset, bOffset + bytesPerDim);
    }
  }

  private static String b64(byte[] value) {
    return Base64.getEncoder().encodeToString(value);
  }
}
