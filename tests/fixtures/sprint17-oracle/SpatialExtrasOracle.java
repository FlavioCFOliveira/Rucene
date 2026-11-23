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

import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Calendar;
import java.util.Collections;
import java.util.List;
import java.util.Random;
import org.apache.lucene.analysis.tokenattributes.BytesTermAttribute;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.search.DoubleValues;
import org.apache.lucene.search.DoubleValuesSource;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.spatial.SpatialStrategy;
import org.apache.lucene.spatial.bbox.BBoxStrategy;
import org.apache.lucene.spatial.composite.CompositeSpatialStrategy;
import org.apache.lucene.spatial.prefix.HeatmapFacetCounter;
import org.apache.lucene.spatial.prefix.NumberRangePrefixTreeStrategy;
import org.apache.lucene.spatial.prefix.PrefixTreeStrategy;
import org.apache.lucene.spatial.prefix.RecursivePrefixTreeStrategy;
import org.apache.lucene.spatial.prefix.TermQueryPrefixTreeStrategy;
import org.apache.lucene.spatial.prefix.tree.DateRangePrefixTree;
import org.apache.lucene.spatial.prefix.tree.NumberRangePrefixTree.NRShape;
import org.apache.lucene.spatial.prefix.tree.NumberRangePrefixTree.UnitNRShape;
import org.apache.lucene.spatial.prefix.tree.PackedQuadPrefixTree;
import org.apache.lucene.spatial.prefix.tree.S2PrefixTree;
import org.apache.lucene.spatial.query.SpatialArgs;
import org.apache.lucene.spatial.query.SpatialOperation;
import org.apache.lucene.spatial.serialized.SerializedDVStrategy;
import org.apache.lucene.spatial.spatial4j.Geo3dSpatialContextFactory;
import org.apache.lucene.spatial.vector.PointVectorStrategy;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.util.BytesRef;
import org.locationtech.spatial4j.io.LegacyShapeWriter;
import org.locationtech.spatial4j.shape.Shape;
import org.locationtech.spatial4j.shape.ShapeFactory;
import java.math.RoundingMode;
import java.text.NumberFormat;
import org.apache.lucene.spatial.prefix.tree.Cell;
import org.apache.lucene.spatial.prefix.tree.CellIterator;
import org.apache.lucene.spatial.prefix.tree.GeohashPrefixTree;
import org.apache.lucene.spatial.prefix.tree.QuadPrefixTree;
import org.apache.lucene.spatial.prefix.tree.SpatialPrefixTree;
import org.locationtech.spatial4j.context.SpatialContext;
import org.locationtech.spatial4j.shape.Point;

/**
 * The Java side of the sprint-17 oracle harness for {@code lucene/spatial-extras} (rmp #628,
 * {@code tests/sprint17_oracle/mod.rs}): {@code main(seed, scenario, input, args...)} prints the
 * dump of the scenario, which Rucene's test computes from the same seed.
 */
public class SpatialExtrasOracle {
  /** The seeded generators of the harness (`tests/sprint17_oracle/mod.rs`, `corpus`). */
  static final class Corpus {
    static final String[] VOCABULARY = {
      "the", "of", "and", "to", "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
      "hotel", "india", "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec",
      "romeo", "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu"
    };

    static String word(Random r) {
      return VOCABULARY[r.nextInt(VOCABULARY.length)];
    }

    static String text(Random r, int n) {
      StringBuilder sb = new StringBuilder();
      for (int i = 0; i < n; i++) {
        if (i > 0) sb.append(' ');
        sb.append(word(r));
      }
      return sb.toString();
    }
  }

  /** Printable ASCII except the backslash as is, every other UTF-16 unit as a u-escape. */
  static String escape(String s) {
    if (s == null) return "null";
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (c >= 0x20 && c <= 0x7e && c != '\\') sb.append(c);
      else sb.append(String.format("\\u%04X", (int) c));
    }
    return sb.toString();
  }

  static String bits(float f) {
    return Integer.toHexString(Float.floatToRawIntBits(f));
  }

  static String bits(double d) {
    return Long.toHexString(Double.doubleToRawLongBits(d));
  }

  static String input(String path) throws IOException {
    return path.equals("-") ? null : Files.readString(Path.of(path), StandardCharsets.UTF_8);
  }

  public static void main(String[] args) throws Exception {
    long seed = Long.parseLong(args[0]);
    String scenario = args[1];
    String input = input(args[2]);
    String[] rest = Arrays.copyOfRange(args, 3, args.length);
    PrintStream out =
        new PrintStream(new FileOutputStream(FileDescriptor.out), false, StandardCharsets.UTF_8);
    run(scenario, new Random(seed), input, rest, out);
    out.flush();
  }

  static IllegalArgumentException unknown(String scenario) {
    return new IllegalArgumentException("unknown scenario: " + scenario);
  }

  static void run(String scenario, Random r, String input, String[] args, PrintStream out)
      throws Exception {
    switch (scenario) {
      case "smoke" -> smoke(r, out);
      case "terms" -> terms(r, out);
      case "nr" -> nr(r, out);
      case "queries" -> queries(r, out);
      case "heatmap" -> heatmap(r, out);
      case "wkt" -> wkt(r, out);
      case "decimalFormat" -> decimalFormat(r, out);
      default -> throw unknown(scenario);
    }
  }

  /**
   * Ten seeded points of {@code SpatialContext.GEO}, each walked down a {@code
   * GeohashPrefixTree(6)} and a {@code QuadPrefixTree(8)} with {@code getTreeCellIterator}: the
   * level and token of every cell.
   */
  @SuppressWarnings("deprecation") // SpatialContext.makePoint, as Lucene's tests call it
  static void smoke(Random r, PrintStream out) throws Exception {
    SpatialContext ctx = SpatialContext.GEO;
    SpatialPrefixTree[] trees = {new GeohashPrefixTree(ctx, 6), new QuadPrefixTree(ctx, 8)};
    for (int i = 0; i < 10; i++) {
      double x = r.nextDouble() * 360 - 180;
      double y = r.nextDouble() * 180 - 90;
      Point p = ctx.makePoint(x, y);
      out.println("point " + bits(x) + " " + bits(y));
      for (SpatialPrefixTree tree : trees) {
        CellIterator it = tree.getTreeCellIterator(p, tree.getMaxLevels());
        StringBuilder sb = new StringBuilder("tree " + tree.getMaxLevels());
        while (it.hasNext()) {
          Cell cell = it.next();
          sb.append(' ').append(cell.getLevel()).append(':')
              .append(escape(cell.getTokenBytesNoLeaf(null).utf8ToString()));
        }
        out.println(sb);
      }
    }
  }

  /** {@code hex} of the bytes of {@code b}. */
  static String hex(BytesRef b) {
    StringBuilder sb = new StringBuilder();
    for (int i = b.offset; i < b.offset + b.length; i++) {
      sb.append(String.format("%02x", b.bytes[i] & 0xff));
    }
    return sb.toString();
  }

  /** The terms {@code strategy} indexes for {@code shape}, hex, space-separated. */
  static String indexedTerms(PrefixTreeStrategy strategy, Shape shape) throws IOException {
    PrefixTreeStrategy.ShapeTokenStream ts = strategy.tokenStream();
    ts.setShape(shape);
    BytesTermAttribute att = ts.getAttribute(BytesTermAttribute.class);
    StringBuilder sb = new StringBuilder();
    ts.reset();
    int n = 0;
    while (ts.incrementToken()) {
      sb.append(' ').append(hex(att.getBytesRef()));
      n++;
    }
    ts.end();
    ts.close();
    return n + sb.toString();
  }

  /** A seeded point, rectangle or circle of {@code ctx} (bits of its parameters in {@code desc}). */
  static Shape randomShape(Random r, SpatialContext ctx, StringBuilder desc) {
    ShapeFactory sf = ctx.getShapeFactory();
    int kind = r.nextInt(3);
    double x = r.nextDouble() * 360 - 180;
    double y = r.nextDouble() * 180 - 90;
    switch (kind) {
      case 0 -> {
        desc.append("point ").append(bits(x)).append(' ').append(bits(y));
        return sf.pointXY(x, y);
      }
      case 1 -> {
        double w = r.nextDouble() * 40;
        double h = r.nextDouble() * 20;
        double maxX = x + w > 180 ? x + w - 360 : x + w;
        double maxY = Math.min(90, y + h);
        desc.append("rect ").append(bits(x)).append(' ').append(bits(maxX)).append(' ')
            .append(bits(y)).append(' ').append(bits(maxY));
        return sf.rect(x, maxX, y, maxY);
      }
      default -> {
        double d = r.nextDouble() * 10;
        desc.append("circle ").append(bits(x)).append(' ').append(bits(y)).append(' ').append(bits(d));
        return sf.circle(x, y, d);
      }
    }
  }

  /**
   * The terms RecursivePrefixTreeStrategy (random distErrPct, prune flag, pointsOnly for points)
   * and TermQueryPrefixTreeStrategy index for seeded shapes, over quad, geohash, packed-quad and
   * S2 trees (index compatibility of the cell bytes).
   */
  static void terms(Random r, PrintStream out) throws Exception {
    SpatialContext geo = SpatialContext.GEO;
    SpatialContext geo3d = new Geo3dSpatialContextFactory().newSpatialContext();
    for (int t = 0; t < 6; t++) {
      SpatialContext ctx;
      SpatialPrefixTree grid;
      String name;
      switch (t) {
        case 0 -> { ctx = geo; grid = new QuadPrefixTree(ctx, 8); name = "quad8"; }
        case 1 -> { ctx = geo; grid = new GeohashPrefixTree(ctx, 4); name = "geohash4"; }
        case 2 -> {
          ctx = geo;
          PackedQuadPrefixTree pq = new PackedQuadPrefixTree(ctx, 10);
          pq.setPruneLeafyBranches(r.nextBoolean());
          grid = pq;
          name = "packedquad10";
        }
        default -> {
          ctx = geo3d;
          int arity = t - 2;
          grid = new S2PrefixTree(ctx, 5 - arity, arity);
          name = "s2-" + arity;
        }
      }
      for (int i = 0; i < 8; i++) {
        StringBuilder desc = new StringBuilder();
        Shape shape = randomShape(r, ctx, desc);
        double distErrPct = r.nextBoolean() ? 0.025 : 0.15;
        boolean prune = r.nextBoolean();
        RecursivePrefixTreeStrategy rpt = new RecursivePrefixTreeStrategy(grid, "rpt");
        rpt.setDistErrPct(distErrPct);
        rpt.setPruneLeafyBranches(prune);
        if (shape instanceof Point && r.nextBoolean()) {
          rpt.setPointsOnly(true);
        }
        TermQueryPrefixTreeStrategy tq = new TermQueryPrefixTreeStrategy(grid, "tq");
        tq.setDistErrPct(distErrPct);
        out.println(name + " " + desc + " " + bits(distErrPct) + " " + prune);
        out.println("rpt " + indexedTerms(rpt, shape));
        out.println("tq " + indexedTerms(tq, shape));
      }
    }
  }

  /** A seeded calendar of {@code tree} within ten thousand years of 1970, truncated at a field. */
  static Calendar randomCal(Random r, DateRangePrefixTree tree) {
    Calendar cal = tree.newCal();
    cal.setTimeInMillis(r.nextLong() % 315_569_520_000_000L);
    int[] fields = {
      Calendar.YEAR, Calendar.MONTH, Calendar.DAY_OF_MONTH, Calendar.HOUR_OF_DAY,
      Calendar.MINUTE, Calendar.SECOND, Calendar.MILLISECOND
    };
    tree.clearFieldsAfter(cal, fields[r.nextInt(fields.length)]);
    return cal;
  }

  /**
   * NumberRangePrefixTreeStrategy over DateRangePrefixTree: seeded unit and range shapes, their
   * toString, parse round trip and indexed terms (index compatibility of the number-range and
   * date-range cell bytes).
   */
  static void nr(Random r, PrintStream out) throws Exception {
    for (Calendar template :
        new Calendar[] {DateRangePrefixTree.DEFAULT_CAL, DateRangePrefixTree.JAVA_UTIL_TIME_COMPAT_CAL}) {
      DateRangePrefixTree tree = new DateRangePrefixTree(template);
      NumberRangePrefixTreeStrategy strategy = new NumberRangePrefixTreeStrategy(tree, "dateRange");
      for (int i = 0; i < 25; i++) {
        Calendar c1 = randomCal(r, tree);
        UnitNRShape s1 = tree.toShape(c1);
        NRShape shape = s1;
        if (r.nextBoolean()) {
          Calendar c2 = randomCal(r, tree);
          UnitNRShape s2 = tree.toShape(c2);
          try {
            shape = c1.compareTo(c2) < 0 ? tree.toRangeShape(s1, s2) : tree.toRangeShape(s2, s1);
          } catch (IllegalArgumentException e) {
            out.println("iae " + escape(e.getMessage()));
          }
        }
        String str = shape.toString();
        out.println("shape " + escape(str) + " cal " + tree.toString(c1));
        out.println("parsed " + escape(tree.parseShape(str).toString()));
        strategy.setPointsOnly(false);
        out.println("terms " + indexedTerms(strategy, shape));
      }
    }
  }

  /** The id-sorted matches of {@code query}. */
  static String matches(IndexSearcher searcher, Query query) throws IOException {
    TopDocs docs = searcher.search(query, 1000);
    List<String> ids = new ArrayList<>();
    for (ScoreDoc sd : docs.scoreDocs) {
      ids.add(searcher.storedFields().document(sd.doc).get("id"));
    }
    Collections.sort(ids);
    return docs.totalHits.value() + " " + ids;
  }

  /**
   * Seeded documents indexed with each strategy (RPT quad/geohash/S2, TermQuery, BBox,
   * PointVector, SerializedDV, Composite), then every operation with seeded query shapes: the
   * matching ids or the exception; and the distance values of a seeded point per document.
   */
  static void queries(Random r, PrintStream out) throws Exception {
    SpatialContext geo = SpatialContext.GEO;
    for (int s = 0; s < 8; s++) {
      SpatialContext ctx = geo;
      SpatialStrategy strategy;
      boolean pointsOnly = false;
      switch (s) {
        case 0 -> strategy = new RecursivePrefixTreeStrategy(new QuadPrefixTree(ctx, 6), "rpt_quad");
        case 1 -> strategy = new RecursivePrefixTreeStrategy(new GeohashPrefixTree(ctx, 3), "rpt_geohash");
        case 2 -> strategy = new TermQueryPrefixTreeStrategy(new GeohashPrefixTree(ctx, 3), "tq_geohash");
        case 3 -> strategy = BBoxStrategy.newInstance(ctx, "bbox");
        case 4 -> {
          strategy = PointVectorStrategy.newInstance(ctx, "pv");
          pointsOnly = true;
        }
        case 5 -> strategy = new SerializedDVStrategy(ctx, "sdv");
        case 6 -> {
          RecursivePrefixTreeStrategy rpt =
              new RecursivePrefixTreeStrategy(new GeohashPrefixTree(ctx, 3), "comp_rpt");
          strategy = new CompositeSpatialStrategy("comp", rpt, new SerializedDVStrategy(ctx, "comp_sdv"));
        }
        default -> {
          ctx = new Geo3dSpatialContextFactory().newSpatialContext();
          strategy = new RecursivePrefixTreeStrategy(new S2PrefixTree(ctx, 4, 1), "rpt_s2");
        }
      }
      // (its toString() carries identity hash codes, which differ run to run)
      out.println("strategy " + strategy.getFieldName());
      ByteBuffersDirectory dir = new ByteBuffersDirectory();
      IndexWriter w = new IndexWriter(dir, new IndexWriterConfig());
      for (int d = 0; d < 12; d++) {
        StringBuilder desc = new StringBuilder();
        Shape shape;
        if (pointsOnly) {
          double x = r.nextDouble() * 360 - 180;
          double y = r.nextDouble() * 180 - 90;
          desc.append("point ").append(bits(x)).append(' ').append(bits(y));
          shape = ctx.getShapeFactory().pointXY(x, y);
        } else {
          shape = randomShape(r, ctx, desc);
        }
        Document doc = new Document();
        doc.add(new StringField("id", String.format("%02d", d), Field.Store.YES));
        for (Field f : strategy.createIndexableFields(shape)) {
          doc.add(f);
        }
        w.addDocument(doc);
        out.println("doc " + d + " " + desc);
      }
      w.close();
      DirectoryReader reader = DirectoryReader.open(dir);
      IndexSearcher searcher = new IndexSearcher(reader);
      for (SpatialOperation op : SpatialOperation.values()) {
        for (int q = 0; q < 3; q++) {
          StringBuilder desc = new StringBuilder();
          Shape queryShape = randomShape(r, ctx, desc);
          String result;
          try {
            result = matches(searcher, strategy.makeQuery(new SpatialArgs(op, queryShape)));
          } catch (UnsupportedOperationException e) {
            result = "unsupported " + e.getClass().getName();
          }
          out.println(op + " " + desc + " -> " + result);
        }
      }
      double px = r.nextDouble() * 360 - 180;
      double py = r.nextDouble() * 180 - 90;
      StringBuilder dist = new StringBuilder("distance " + bits(px) + " " + bits(py));
      try {
        DoubleValuesSource vs = strategy.makeDistanceValueSource(ctx.getShapeFactory().pointXY(px, py));
        for (LeafReaderContext leaf : reader.leaves()) {
          DoubleValues v = vs.getValues(leaf, null);
          for (int doc = 0; doc < leaf.reader().maxDoc(); doc++) {
            dist.append(' ').append(v.advanceExact(doc) ? bits(v.doubleValue()) : "none");
          }
        }
      } catch (UnsupportedOperationException e) {
        dist.append(" unsupported");
      }
      out.println(dist);
      reader.close();
    }
  }

  /** RPT quad heatmaps over seeded points and rectangles, at several levels and regions. */
  static void heatmap(Random r, PrintStream out) throws Exception {
    SpatialContext ctx = SpatialContext.GEO;
    RecursivePrefixTreeStrategy strategy =
        new RecursivePrefixTreeStrategy(new QuadPrefixTree(ctx, 6), "heat");
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    IndexWriter w = new IndexWriter(dir, new IndexWriterConfig());
    for (int d = 0; d < 30; d++) {
      StringBuilder desc = new StringBuilder();
      Shape shape = randomShape(r, ctx, desc);
      Document doc = new Document();
      for (Field f : strategy.createIndexableFields(shape)) {
        doc.add(f);
      }
      w.addDocument(doc);
      out.println("doc " + desc);
    }
    w.close();
    DirectoryReader reader = DirectoryReader.open(dir);
    IndexSearcher searcher = new IndexSearcher(reader);
    for (int i = 0; i < 6; i++) {
      Shape region;
      if (i < 3) {
        region = null;
      } else {
        double x = r.nextDouble() * 360 - 180;
        double y = r.nextDouble() * 160 - 80;
        double w2 = r.nextDouble() * 100;
        double maxX = x + w2 > 180 ? x + w2 - 360 : x + w2;
        region = ctx.getShapeFactory().rect(x, maxX, y, Math.min(90, y + r.nextDouble() * 30));
      }
      int level = 1 + (i % 3) + (i < 3 ? 0 : 1);
      HeatmapFacetCounter.Heatmap h =
          HeatmapFacetCounter.calcFacets(strategy, searcher.getTopReaderContext(), null, region, level, 100_000);
      out.println("heatmap " + level + " " + (region == null ? "world" : region.toString()) + " -> "
          + h.columns + "x" + h.rows + " " + h.region + " " + Arrays.toString(h.counts));
    }
    reader.close();
  }

  /** Seeded shapes written by the WKT writer, read back by the WKT reader, written again. */
  static void wkt(Random r, PrintStream out) throws Exception {
    SpatialContext ctx = SpatialContext.GEO;
    org.locationtech.spatial4j.io.ShapeWriter writer = ctx.getFormats().getWktWriter();
    org.locationtech.spatial4j.io.ShapeReader reader = ctx.getFormats().getWktReader();
    String[] fixed = {
      "POINT(-0.0009 0.0009)", "ENVELOPE(-10.123456789, 10.987654321, 20.5, -20.5)",
      "BUFFER(POINT(-80 33), 1.0000005)", "LINESTRING(1 2, 3.25 -4.125, -5 6)",
      "MULTIPOINT((10 40), (40 30), (-20 -20))", "ENVELOPE(170.0000001, -170.0000001, 0.0000004, -0.0000004)",
      "POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))", "GEOMETRYCOLLECTION(POINT(4 6),LINESTRING(4 6,7 10))"
    };
    for (String wkt : fixed) {
      roundTripWkt(ctx, writer, reader, wkt, out);
    }
    for (int i = 0; i < 20; i++) {
      StringBuilder desc = new StringBuilder();
      Shape shape = randomShape(r, ctx, desc);
      String wkt = writer.toString(shape);
      out.println("shape " + desc + " -> " + wkt + " | " + shape);
      roundTripWkt(ctx, writer, reader, wkt, out);
    }
  }

  static void roundTripWkt(
      SpatialContext ctx,
      org.locationtech.spatial4j.io.ShapeWriter writer,
      org.locationtech.spatial4j.io.ShapeReader reader,
      String wkt,
      PrintStream out) {
    try {
      Shape s = (Shape) reader.read(wkt);
      out.println("read " + wkt + " -> " + s + " -> " + writer.toString(s));
    } catch (Exception e) {
      out.println("read " + wkt + " -> " + e.getClass().getName() + ": " + escape(e.getMessage()));
    }
  }

  /**
   * {@code LegacyShapeWriter.makeNumberFormat(digits)} with the default (HALF_EVEN), FLOOR and
   * CEILING rounding (as WKTWriter writes ENVELOPE) over edge and seeded values.
   */
  @SuppressWarnings("deprecation") // LegacyShapeWriter.makeNumberFormat, as WKTWriter calls it
  static void decimalFormat(Random r, PrintStream out) {
    List<Double> values = new ArrayList<>(Arrays.asList(
        0.0009, -0.0009, 0.0, -0.0, 1.005, -1.005, 2.5, -2.5, 0.125, -0.125, 123.456789,
        -123.456789, 1e-7, -1e-7, 179.9999999, -179.9999999, 0.995, -0.995, 1e15, -1e15, 0.5, -0.5));
    for (int i = 0; i < 30; i++) {
      double v = (r.nextDouble() * 2 - 1) * Math.pow(10, r.nextInt(8) - 4);
      values.add(v);
    }
    for (int digits : new int[] {0, 1, 2, 6}) {
      NumberFormat def = LegacyShapeWriter.makeNumberFormat(digits);
      NumberFormat floor = LegacyShapeWriter.makeNumberFormat(digits);
      floor.setRoundingMode(RoundingMode.FLOOR);
      NumberFormat ceiling = LegacyShapeWriter.makeNumberFormat(digits);
      ceiling.setRoundingMode(RoundingMode.CEILING);
      for (double v : values) {
        out.println(digits + " " + bits(v) + " " + def.format(v) + " " + floor.format(v) + " "
            + ceiling.format(v));
      }
    }
  }
}
