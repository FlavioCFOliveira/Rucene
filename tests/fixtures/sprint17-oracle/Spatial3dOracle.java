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
import java.util.List;
import java.util.Random;
import java.util.TreeSet;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.spatial3d.Geo3DPoint;
import org.apache.lucene.spatial3d.geom.DistanceStyle;
import org.apache.lucene.spatial3d.geom.GeoArea;
import org.apache.lucene.spatial3d.geom.GeoAreaFactory;
import org.apache.lucene.spatial3d.geom.GeoAreaShape;
import org.apache.lucene.spatial3d.geom.GeoBBoxFactory;
import org.apache.lucene.spatial3d.geom.GeoCircle;
import org.apache.lucene.spatial3d.geom.GeoCircleFactory;
import org.apache.lucene.spatial3d.geom.GeoDistanceShape;
import org.apache.lucene.spatial3d.geom.GeoMembershipShape;
import org.apache.lucene.spatial3d.geom.GeoPathFactory;
import org.apache.lucene.spatial3d.geom.GeoPoint;
import org.apache.lucene.spatial3d.geom.GeoPolygonFactory;
import org.apache.lucene.spatial3d.geom.GeoShape;
import org.apache.lucene.spatial3d.geom.LatLonBounds;
import org.apache.lucene.spatial3d.geom.PlanetModel;
import org.apache.lucene.spatial3d.geom.XYZBounds;
import org.apache.lucene.store.ByteBuffersDirectory;

/**
 * The Java side of the sprint-17 oracle harness for {@code lucene/spatial3d} (rmp #628,
 * {@code tests/sprint17_oracle/mod.rs}): {@code main(seed, scenario, input, args...)} prints the
 * dump of the scenario, which Rucene's test computes from the same seed.
 */
public class Spatial3dOracle {
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
      case "geom" -> geom(r, out);
      case "queries" -> queries(r, out);
      default -> throw unknown(scenario);
    }
  }

  /**
   * Twenty seeded circles on {@code PlanetModel.SPHERE} ({@code GeoCircleFactory.makeGeoCircle}),
   * each probed with five seeded points: the point's coordinates, {@code isWithin} and the
   * outside arc distance, all as bits.
   */
  static void smoke(Random r, PrintStream out) throws Exception {
    for (int i = 0; i < 20; i++) {
      double lat = (r.nextDouble() * 2 - 1) * Math.PI / 2;
      double lon = (r.nextDouble() * 2 - 1) * Math.PI;
      double radius = r.nextDouble() * 0.5;
      GeoCircle circle = GeoCircleFactory.makeGeoCircle(PlanetModel.SPHERE, lat, lon, radius);
      out.println("circle " + bits(lat) + " " + bits(lon) + " " + bits(radius) + " " + bits(circle.getRadius()));
      for (int j = 0; j < 5; j++) {
        double plat = (r.nextDouble() * 2 - 1) * Math.PI / 2;
        double plon = (r.nextDouble() * 2 - 1) * Math.PI;
        GeoPoint p = new GeoPoint(PlanetModel.SPHERE, plat, plon);
        out.println(
            "point " + bits(p.x) + " " + bits(p.y) + " " + bits(p.z) + " " + circle.isWithin(p)
                + " " + bits(circle.computeOutsideDistance(DistanceStyle.ARC, p)));
      }
    }
  }

  /** A seeded planet model: SPHERE, WGS84 or CLARKE_1866. */
  static PlanetModel model(Random r) {
    switch (r.nextInt(3)) {
      case 0:
        return PlanetModel.SPHERE;
      case 1:
        return PlanetModel.WGS84;
      default:
        return PlanetModel.CLARKE_1866;
    }
  }

  static double lat(Random r) {
    return (r.nextDouble() * 2 - 1) * Math.PI / 2;
  }

  static double lon(Random r) {
    return (r.nextDouble() * 2 - 1) * Math.PI;
  }

  /** {@code count} points around {@code center} at seeded bearings (increasing) and distances. */
  static List<GeoPoint> ring(Random r, PlanetModel pm, GeoPoint center, int count, double maxDist) {
    List<GeoPoint> points = new ArrayList<>();
    for (int i = 0; i < count; i++) {
      double bearing = (i + r.nextDouble() * 0.8) * 2 * Math.PI / count;
      double dist = (0.2 + 0.8 * r.nextDouble()) * maxDist;
      points.add(pm.surfacePointOnBearing(center, dist, bearing));
    }
    return points;
  }

  /**
   * A seeded shape (circle, exact circle, bbox, polygon or path) through the public factories;
   * {@code null} for a degenerate polygon. Throws what the factory throws.
   */
  static GeoShape shape(Random r, PlanetModel pm) {
    int type = r.nextInt(5);
    double lat = lat(r);
    double lon = lon(r);
    switch (type) {
      case 0:
        return GeoCircleFactory.makeGeoCircle(pm, lat, lon, r.nextDouble() * 0.5);
      case 1:
        return GeoCircleFactory.makeExactGeoCircle(pm, lat, lon, r.nextDouble() * 0.5, 1e-6);
      case 2:
        {
          double lat2 = lat(r);
          double lon2 = lon(r);
          return GeoBBoxFactory.makeGeoBBox(
              pm, Math.max(lat, lat2), Math.min(lat, lat2), Math.min(lon, lon2), Math.max(lon, lon2));
        }
      case 3:
        {
          GeoPoint center = new GeoPoint(pm, lat, lon);
          List<GeoPoint> points = ring(r, pm, center, 3 + r.nextInt(5), r.nextDouble() * 0.6);
          return GeoPolygonFactory.makeGeoPolygon(pm, points);
        }
      default:
        {
          GeoPoint start = new GeoPoint(pm, lat, lon);
          int n = 1 + r.nextInt(3);
          GeoPoint[] points = new GeoPoint[n];
          points[0] = start;
          for (int i = 1; i < n; i++) {
            points[i] = pm.surfacePointOnBearing(points[i - 1], r.nextDouble() * 0.3, r.nextDouble() * 2 * Math.PI);
          }
          return GeoPathFactory.makeGeoPath(pm, r.nextDouble() * 0.2, points);
        }
    }
  }

  static String bits(Double d) {
    return d == null ? "null" : bits(d.doubleValue());
  }

  /**
   * Forty seeded shapes on seeded planet models: each shape's {@code toString}, its {@code
   * XYZBounds} and {@code LatLonBounds}, ten seeded points ({@code isWithin}, outside and inside
   * arc distances), five seeded XYZ solids ({@code GeoArea.getRelationship}) and its relationship
   * to the previous area shape. An exception prints its class.
   */
  static void geom(Random r, PrintStream out) {
    GeoAreaShape previous = null;
    for (int i = 0; i < 40; i++) {
      PlanetModel pm = model(r);
      out.println("model " + pm);
      GeoShape shape;
      try {
        shape = shape(r, pm);
      } catch (RuntimeException e) {
        out.println("shape " + e.getClass().getSimpleName());
        continue;
      }
      if (shape == null) {
        out.println("shape null");
        continue;
      }
      out.println("shape " + escape(shape.toString()));
      XYZBounds xyz = new XYZBounds();
      shape.getBounds(xyz);
      out.println(
          "xyz " + bits(xyz.getMinimumX()) + " " + bits(xyz.getMaximumX()) + " " + bits(xyz.getMinimumY())
              + " " + bits(xyz.getMaximumY()) + " " + bits(xyz.getMinimumZ()) + " " + bits(xyz.getMaximumZ()));
      LatLonBounds ll = new LatLonBounds();
      shape.getBounds(ll);
      out.println(
          "latlon " + ll.checkNoLongitudeBound() + " " + ll.checkNoTopLatitudeBound() + " "
              + ll.checkNoBottomLatitudeBound() + " " + bits(ll.getMinLatitude()) + " "
              + bits(ll.getMaxLatitude()) + " " + bits(ll.getLeftLongitude()) + " "
              + bits(ll.getRightLongitude()));
      for (int j = 0; j < 10; j++) {
        GeoPoint p = new GeoPoint(pm, lat(r), lon(r));
        StringBuilder line = new StringBuilder("point " + shape.isWithin(p));
        if (shape instanceof GeoMembershipShape) {
          line.append(" ").append(bits(((GeoMembershipShape) shape).computeOutsideDistance(DistanceStyle.ARC, p)));
        }
        if (shape instanceof GeoDistanceShape) {
          line.append(" ").append(bits(((GeoDistanceShape) shape).computeDistance(DistanceStyle.ARC, p)));
        }
        out.println(line);
      }
      for (int j = 0; j < 5; j++) {
        double[] v = new double[6];
        for (int k = 0; k < 6; k++) {
          v[k] = r.nextDouble() * 2.2 - 1.1;
        }
        GeoArea solid =
            GeoAreaFactory.makeGeoArea(
                pm, Math.min(v[0], v[1]), Math.max(v[0], v[1]), Math.min(v[2], v[3]),
                Math.max(v[2], v[3]), Math.min(v[4], v[5]), Math.max(v[4], v[5]));
        out.println("solid " + solid.getRelationship(shape));
      }
      if (shape instanceof GeoAreaShape) {
        if (previous != null && previous.getPlanetModel().equals(pm)) {
          out.println("previous " + ((GeoAreaShape) shape).getRelationship(previous));
        }
        previous = (GeoAreaShape) shape;
      }
    }
  }

  /**
   * Three hundred seeded points of one seeded planet model indexed as {@code Geo3DPoint} (with an
   * {@code id} doc value), then twenty seeded shapes ({@code Geo3DPoint.newShapeQuery}): each
   * query's {@code toString} and the sorted ids it matches.
   */
  static void queries(Random r, PrintStream out) throws Exception {
    PlanetModel pm = model(r);
    out.println("model " + pm);
    try (ByteBuffersDirectory dir = new ByteBuffersDirectory()) {
      try (IndexWriter w = new IndexWriter(dir, new IndexWriterConfig())) {
        for (int id = 0; id < 300; id++) {
          Document doc = new Document();
          doc.add(new NumericDocValuesField("id", id));
          doc.add(new Geo3DPoint("point", pm, Math.toDegrees(lat(r)), Math.toDegrees(lon(r))));
          w.addDocument(doc);
        }
      }
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        IndexSearcher searcher = new IndexSearcher(reader);
        for (int i = 0; i < 20; i++) {
          GeoShape shape;
          try {
            shape = shape(r, pm);
          } catch (RuntimeException e) {
            out.println("shape " + e.getClass().getSimpleName());
            continue;
          }
          if (shape == null) {
            out.println("shape null");
            continue;
          }
          Query q = Geo3DPoint.newShapeQuery("point", shape);
          out.println("query " + escape(q.toString()));
          TopDocs top = searcher.search(q, 300);
          TreeSet<Long> ids = new TreeSet<>();
          for (ScoreDoc sd : top.scoreDocs) {
            LeafReaderContext leaf = reader.leaves().get(org.apache.lucene.index.ReaderUtil.subIndex(sd.doc, reader.leaves()));
            NumericDocValues dv = leaf.reader().getNumericDocValues("id");
            dv.advanceExact(sd.doc - leaf.docBase);
            ids.add(dv.longValue());
          }
          out.println("hits " + top.totalHits.value() + " " + ids);
        }
      }
    }
  }
}
