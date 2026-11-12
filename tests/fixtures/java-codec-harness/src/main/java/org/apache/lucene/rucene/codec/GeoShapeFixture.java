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
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.LatLonDocValuesField;
import org.apache.lucene.document.LatLonPoint;
import org.apache.lucene.document.LatLonShape;
import org.apache.lucene.document.ShapeField.QueryRelation;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.XYPointField;
import org.apache.lucene.document.XYShape;
import org.apache.lucene.geo.Circle;
import org.apache.lucene.geo.GeoEncodingUtils;
import org.apache.lucene.geo.LatLonGeometry;
import org.apache.lucene.geo.Line;
import org.apache.lucene.geo.Point;
import org.apache.lucene.geo.Polygon;
import org.apache.lucene.geo.Rectangle;
import org.apache.lucene.geo.XYGeometry;
import org.apache.lucene.geo.XYPoint;
import org.apache.lucene.geo.XYPolygon;
import org.apache.lucene.geo.XYRectangle;
import org.apache.lucene.geo.XYLine;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.search.FieldDoc;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.TopFieldDocs;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Oracle of {@code tests/portability/geo_shapes.rs} (rmp task #408).
 *
 * <p>The geo corpus is defined here and in the Rust test by the same formulas. Modes:
 *
 * <ul>
 *   <li>{@code write <dir>}: writes the corpus as one segment (one commit, no compound file), so
 *       that its points files ({@code .kdd}, {@code .kdi}, {@code .kdm}) can be compared byte for
 *       byte with the Rust ones.
 *   <li>{@code query <dir>}: runs the point, doc-values and shape query set and the distance sort
 *       and prints the hits with their score bits and sort-value bits.
 *   <li>{@code triangles}: prints the packed bytes of every triangle {@code LatLonShape} and {@code
 *       XYShape} {@code createIndexableFields} produce for the corpus shapes.
 * </ul>
 */
public final class GeoShapeFixture {

  /** Documents of the corpus. */
  static final int DOCS = 200;

  /** Hits printed per query. */
  static final int TOP_N = 20;

  private GeoShapeFixture() {}

  // ---------------------------------------------------------------------------------------------
  // Corpus
  // ---------------------------------------------------------------------------------------------

  static String id(int i) {
    return String.format("id-%04d", i);
  }

  static double lat(int i) {
    return -80 + ((i * 37) % 1600) / 10.0;
  }

  static double lon(int i) {
    return -170 + ((i * 53) % 3400) / 10.0;
  }

  static float x(int i) {
    return ((i * 29) % 1000) / 10.0f - 50.0f;
  }

  static float y(int i) {
    return ((i * 43) % 1000) / 10.0f - 50.0f;
  }

  /** Documents without a point location, so that the distance sort sees missing values. */
  static boolean hasLocation(int i) {
    return i % 17 != 5;
  }

  /** Half-size of the shape of document {@code i}. */
  static double size(int i) {
    return 0.5 + (i % 5) * 0.25;
  }

  static float xySize(int i) {
    return 0.5f + (i % 5) * 0.25f;
  }

  /** {@code i % 3}: 0 a polygon (with a hole when {@code i % 9 == 0}), 1 a line, 2 a point. */
  static Field[] latLonShape(int i) {
    double a = lat(i);
    double o = lon(i);
    double d = size(i);
    return switch (i % 3) {
      case 0 -> {
        Polygon[] holes = new Polygon[0];
        if (i % 9 == 0) {
          double h = d / 4;
          holes =
              new Polygon[] {
                new Polygon(
                    new double[] {a - h, a + h, a + h, a - h, a - h},
                    new double[] {o - h, o - h, o + h, o + h, o - h})
              };
        }
        yield LatLonShape.createIndexableFields(
            "shape",
            new Polygon(
                new double[] {a - d, a - d, a + d, a + d, a - d},
                new double[] {o - d, o + d, o + d, o - d, o - d},
                holes));
      }
      case 1 ->
          LatLonShape.createIndexableFields(
              "shape", new Line(new double[] {a, a + d, a + 2 * d}, new double[] {o, o + d, o - d}));
      default -> LatLonShape.createIndexableFields("shape", a, o);
    };
  }

  static Field[] xyShape(int i) {
    float a = x(i);
    float o = y(i);
    float d = xySize(i);
    return switch (i % 3) {
      case 0 -> {
        XYPolygon[] holes = new XYPolygon[0];
        if (i % 9 == 0) {
          float h = d / 4;
          holes =
              new XYPolygon[] {
                new XYPolygon(
                    new float[] {a - h, a + h, a + h, a - h, a - h},
                    new float[] {o - h, o - h, o + h, o + h, o - h})
              };
        }
        yield XYShape.createIndexableFields(
            "xyshape",
            new XYPolygon(
                new float[] {a - d, a - d, a + d, a + d, a - d},
                new float[] {o - d, o + d, o + d, o - d, o - d},
                holes));
      }
      case 1 ->
          XYShape.createIndexableFields(
              "xyshape", new XYLine(new float[] {a, a + d, a + 2 * d}, new float[] {o, o + d, o - d}));
      default -> XYShape.createIndexableFields("xyshape", a, o);
    };
  }

  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", id(i), Field.Store.YES));
    if (hasLocation(i)) {
      d.add(new LatLonPoint("loc", lat(i), lon(i)));
      d.add(new LatLonDocValuesField("loc", lat(i), lon(i)));
      d.add(new XYPointField("xy", x(i), y(i)));
    }
    for (Field f : latLonShape(i)) {
      d.add(f);
    }
    for (Field f : xyShape(i)) {
      d.add(f);
    }
    return d;
  }

  /** One commit, no compound file, flushing only on commit: a single deterministic segment. */
  static void write(Directory dir) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    config.setUseCompoundFile(false);
    config.setRAMBufferSizeMB(256);
    try (IndexWriter w = new IndexWriter(dir, config)) {
      for (int i = 0; i < DOCS; i++) {
        w.addDocument(doc(i));
      }
      w.commit();
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Query set
  // ---------------------------------------------------------------------------------------------

  static String bits(float f) {
    return String.format("%08x", Float.floatToIntBits(f));
  }

  static String bits(double d) {
    return String.format("%016x", Double.doubleToLongBits(d));
  }

  /** The query polygon: a box with a box hole. */
  static Polygon queryPolygon() {
    return new Polygon(
        new double[] {-50, -50, 50, 50, -50},
        new double[] {-100, 100, 100, -100, -100},
        new Polygon(new double[] {-10, 10, 10, -10, -10}, new double[] {-20, -20, 20, 20, -20}));
  }

  static XYPolygon queryXYPolygon() {
    return new XYPolygon(
        new float[] {-40, 40, 40, -40, -40},
        new float[] {-40, -40, 40, 40, -40},
        new XYPolygon(new float[] {-5, -5, 5, 5, -5}, new float[] {-5, 5, 5, -5, -5}));
  }

  static final QueryRelation[] RELATIONS = {
    QueryRelation.INTERSECTS, QueryRelation.WITHIN, QueryRelation.DISJOINT, QueryRelation.CONTAINS
  };

  /** The queries, by name, in the order they are printed. */
  static List<Object[]> queries() {
    List<Object[]> q = new ArrayList<>();
    q.add(
        new Object[] {"latlon distance (-5,-60) 2500km", LatLonPoint.newDistanceQuery("loc", -5, -60, 2_500_000)});
    q.add(
        new Object[] {
          "latlon distance (-40,-100) 3500km", LatLonPoint.newDistanceQuery("loc", -40, -100, 3_500_000)
        });
    q.add(new Object[] {"latlon box [-30,30]x[-60,60]", LatLonPoint.newBoxQuery("loc", -30, 30, -60, 60)});
    q.add(
        new Object[] {"latlon box dateline [-60,60]x[150,-150]", LatLonPoint.newBoxQuery("loc", -60, 60, 150, -150)});
    q.add(new Object[] {"latlon polygon with hole", LatLonPoint.newPolygonQuery("loc", queryPolygon())});
    q.add(
        new Object[] {
          "latlon geometry INTERSECTS circle+box",
          LatLonPoint.newGeometryQuery(
              "loc",
              QueryRelation.INTERSECTS,
              new Circle(-5, -60, 2_500_000),
              new Rectangle(-30, 30, -60, 60))
        });
    q.add(
        new Object[] {
          "latlon geometry WITHIN polygon",
          LatLonPoint.newGeometryQuery("loc", QueryRelation.WITHIN, queryPolygon())
        });
    q.add(
        new Object[] {
          "latlon geometry DISJOINT box",
          LatLonPoint.newGeometryQuery("loc", QueryRelation.DISJOINT, new Rectangle(-30, 30, -60, 60))
        });
    q.add(
        new Object[] {
          "latlon geometry CONTAINS point of doc 3",
          // A point field contains a query point only when the decoded indexed point equals it
          // exactly (Point2D.withinPoint): the query point is document 3's quantized location.
          LatLonPoint.newGeometryQuery(
              "loc",
              QueryRelation.CONTAINS,
              new Point(
                  GeoEncodingUtils.decodeLatitude(GeoEncodingUtils.encodeLatitude(lat(3))),
                  GeoEncodingUtils.decodeLongitude(GeoEncodingUtils.encodeLongitude(lon(3)))))
        });
    q.add(
        new Object[] {
          "latlon dv slowDistance (-5,-60) 2500km",
          LatLonDocValuesField.newSlowDistanceQuery("loc", -5, -60, 2_500_000)
        });
    q.add(
        new Object[] {
          "latlon dv slowBox [-30,30]x[-60,60]", LatLonDocValuesField.newSlowBoxQuery("loc", -30, 30, -60, 60)
        });
    q.add(
        new Object[] {
          "latlon dv slowBox dateline [-60,60]x[150,-150]",
          LatLonDocValuesField.newSlowBoxQuery("loc", -60, 60, 150, -150)
        });
    q.add(new Object[] {"xy box [-20,20]x[-20,20]", XYPointField.newBoxQuery("xy", -20, 20, -20, 20)});
    q.add(new Object[] {"xy distance (0,0) 25", XYPointField.newDistanceQuery("xy", 0, 0, 25)});
    q.add(new Object[] {"xy polygon with hole", XYPointField.newPolygonQuery("xy", queryXYPolygon())});
    double cLat = lat(3);
    double cLon = lon(3);
    float cX = x(3);
    float cY = y(3);
    for (QueryRelation r : RELATIONS) {
      boolean contains = r == QueryRelation.CONTAINS;
      q.add(
          new Object[] {
            "latlonshape box " + r,
            contains
                ? LatLonShape.newBoxQuery("shape", r, cLat - 0.01, cLat + 0.01, cLon - 0.01, cLon + 0.01)
                : LatLonShape.newBoxQuery("shape", r, -30, 30, -60, 60)
          });
      q.add(
          new Object[] {
            "latlonshape geometry " + r,
            contains
                ? LatLonShape.newGeometryQuery("shape", r, new Point(cLat, cLon))
                : LatLonShape.newGeometryQuery("shape", r, queryPolygon())
          });
      q.add(
          new Object[] {
            "xyshape box " + r,
            contains
                ? XYShape.newBoxQuery("xyshape", r, cX - 0.01f, cX + 0.01f, cY - 0.01f, cY + 0.01f)
                : XYShape.newBoxQuery("xyshape", r, -20, 20, -20, 20)
          });
      q.add(
          new Object[] {
            "xyshape geometry " + r,
            contains
                ? XYShape.newGeometryQuery("xyshape", r, new XYPoint(cX, cY))
                : XYShape.newGeometryQuery("xyshape", r, queryXYPolygon())
          });
    }
    q.add(
        new Object[] {
          "latlonshape geometry INTERSECTS circle+line",
          LatLonShape.newGeometryQuery(
              "shape",
              QueryRelation.INTERSECTS,
              new LatLonGeometry[] {
                new Circle(-5, -60, 2_500_000), new Line(new double[] {-20, 0, 20}, new double[] {100, 120, 100})
              })
        });
    q.add(
        new Object[] {
          "xyshape geometry INTERSECTS box+line",
          XYShape.newGeometryQuery(
              "xyshape",
              QueryRelation.INTERSECTS,
              new XYGeometry[] {
                new XYRectangle(-10, 10, 20, 30),
                new XYLine(new float[] {-45, 0, 45}, new float[] {-45, 10, -45})
              })
        });
    return q;
  }

  static void printHits(StoredFields stored, String header, TopDocs top) throws IOException {
    System.out.println(header + " total=" + top.totalHits.value() + " " + top.totalHits.relation());
    for (ScoreDoc hit : top.scoreDocs) {
      StringBuilder line = new StringBuilder();
      line.append("  hit doc=").append(hit.doc).append(" score=").append(bits(hit.score));
      if (hit instanceof FieldDoc fd) {
        Object v = fd.fields[0];
        line.append(" sort=")
            .append(v instanceof Double dv ? "Double:" + bits(dv) : String.valueOf(v));
      }
      line.append(" id=").append(stored.document(hit.doc).get("id"));
      System.out.println(line);
    }
  }

  static void query(Directory dir) throws IOException {
    try (DirectoryReader reader = DirectoryReader.open(dir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      StoredFields stored = searcher.storedFields();
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
      for (Object[] q : queries()) {
        printHits(stored, "query " + q[0], searcher.search((Query) q[1], TOP_N));
      }
      Sort sort = new Sort(LatLonDocValuesField.newDistanceSort("loc", -5, -60));
      TopFieldDocs sorted = searcher.search(MatchAllDocsQuery.INSTANCE, DOCS, sort);
      printHits(stored, "sort latlon distance from (-5,-60)", sorted);
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Triangle encodings
  // ---------------------------------------------------------------------------------------------

  static String hex(BytesRef b) {
    StringBuilder s = new StringBuilder();
    for (int k = 0; k < b.length; k++) {
      s.append(String.format("%02x", b.bytes[b.offset + k] & 0xff));
    }
    return s.toString();
  }

  static void triangles() {
    for (int i = 0; i < DOCS; i++) {
      Field[] shape = latLonShape(i);
      System.out.println("latlonshape doc=" + i + " triangles=" + shape.length);
      for (Field f : shape) {
        System.out.println("  " + hex(f.binaryValue()));
      }
      Field[] xy = xyShape(i);
      System.out.println("xyshape doc=" + i + " triangles=" + xy.length);
      for (Field f : xy) {
        System.out.println("  " + hex(f.binaryValue()));
      }
    }
  }

  public static void main(String[] args) throws Exception {
    if (args[0].equals("triangles")) {
      triangles();
      return;
    }
    try (Directory dir = FSDirectory.open(Paths.get(args[1]))) {
      switch (args[0]) {
        case "write" -> write(dir);
        case "query" -> query(dir);
        default -> throw new IllegalArgumentException(args[0]);
      }
    }
  }
}
