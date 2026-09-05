/*
 * Reference values for the Rucene portability tests of rmp task #167.
 *
 * Prints, one `key<TAB>value` line per case, the exact strings Apache Lucene
 * Core 10.5.0 produces for the `Object` contract methods this port has to
 * reproduce: `toString` on the geometry, document-field and range types, on
 * `FieldType`, `Document`, `Field`, `DocValuesLongHashSet`, `FilterCollector`
 * and `FilterLeafCollector`, plus the `equals` verdicts that decide identity in
 * the query cache.
 *
 * It lives in `org.apache.lucene.document` on purpose: `DocValuesLongHashSet`
 * and `FeatureDoubleValuesSource` are package-private, and `exec:java` loads
 * this class and the jar through one `URLClassLoader`, so the split package
 * resolves.
 */
package org.apache.lucene.document;

import java.net.InetAddress;
import java.util.List;
import org.apache.lucene.geo.Circle;
import org.apache.lucene.geo.Line;
import org.apache.lucene.geo.Point;
import org.apache.lucene.geo.Polygon;
import org.apache.lucene.geo.Rectangle;
import org.apache.lucene.geo.XYCircle;
import org.apache.lucene.geo.XYLine;
import org.apache.lucene.geo.XYPoint;
import org.apache.lucene.geo.XYPolygon;
import org.apache.lucene.geo.XYRectangle;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.search.Collector;
import org.apache.lucene.search.FilterCollector;
import org.apache.lucene.search.FilterLeafCollector;
import org.apache.lucene.search.LeafCollector;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.Scorable;
import org.apache.lucene.search.ScoreMode;
import org.apache.lucene.search.SimpleCollector;
import org.apache.lucene.search.TotalHitCountCollector;

/** Prints the reference strings and equality verdicts, one per line. */
public final class RuceneObjectContractFixture {

  private RuceneObjectContractFixture() {}

  private static void emit(String key, Object value) {
    System.out.println(key + "\t" + value);
  }

  public static void main(String[] args) throws Exception {
    geometry();
    pointFields();
    numericFields();
    ranges();
    fieldTypeAndDocument();
    hashSetAndFeature();
    collectors();
  }

  private static void geometry() {
    emit("geo.Point", new Point(35.0, -70.5));
    emit("geo.Point.exp", new Point(1.0E-4, -1.0E-5));
    emit("geo.Rectangle", new Rectangle(-10.0, 10.0, -20.0, 20.0));
    emit("geo.Rectangle.dateline", new Rectangle(-10.0, 10.0, 20.0, -20.0));
    emit("geo.Circle", new Circle(35.0, -70.5, 1000.0));
    emit("geo.Line", new Line(new double[] {0.0, 1.5}, new double[] {2.5, 3.0}));
    emit(
        "geo.Polygon",
        new Polygon(new double[] {0, 1, 1, 0, 0}, new double[] {0, 0, 1, 1, 0}));
    emit(
        "geo.Polygon.hole",
        new Polygon(
            new double[] {0, 10, 10, 0, 0},
            new double[] {0, 0, 10, 10, 0},
            new Polygon(new double[] {2, 3, 3, 2, 2}, new double[] {2, 2, 3, 3, 2})));
    emit("geo.XYPoint", new XYPoint(1.5f, -2.25f));
    emit("geo.XYRectangle", new XYRectangle(-1.5f, 2.5f, -3.5f, 4.5f));
    emit("geo.XYCircle", new XYCircle(1.5f, -2.25f, 3.0f));
    emit("geo.XYLine", new XYLine(new float[] {0f, 1.5f}, new float[] {2.5f, 3f}));
    emit(
        "geo.XYPolygon",
        new XYPolygon(new float[] {0, 1, 1, 0, 0}, new float[] {0, 0, 1, 1, 0}));
    emit(
        "geo.XYPolygon.hole",
        new XYPolygon(
            new float[] {0, 10, 10, 0, 0},
            new float[] {0, 0, 10, 10, 0},
            new XYPolygon(new float[] {2, 3, 3, 2, 2}, new float[] {2, 2, 3, 3, 2})));
  }

  private static void pointFields() throws Exception {
    emit("IntPoint.1d", new IntPoint("count", 42));
    emit("IntPoint.2d", new IntPoint("count", -1, 7));
    emit("LongPoint.1d", new LongPoint("ts", 123456789L));
    emit("LongPoint.2d", new LongPoint("ts", -5L, 9L));
    emit("FloatPoint.1d", new FloatPoint("temp", 2.5f));
    emit("FloatPoint.2d", new FloatPoint("temp", -0.5f, 1.0E8f));
    emit("DoublePoint.1d", new DoublePoint("d", 2.5));
    emit("DoublePoint.2d", new DoublePoint("d", -0.5, 1.0E-4));
    emit("LatLonPoint", new LatLonPoint("loc", 35.0, -70.5));
    emit("LatLonDocValuesField", new LatLonDocValuesField("loc", 35.0, -70.5));
    emit("XYPointField", new XYPointField("xy", 1.5f, -2.25f));
    emit("XYDocValuesField", new XYDocValuesField("xy", 1.5f, -2.25f));
    emit("InetAddressPoint.v4", new InetAddressPoint("ip", InetAddress.getByName("1.2.3.4")));
    emit("InetAddressPoint.v6", new InetAddressPoint("ip", InetAddress.getByName("::1")));
    emit(
        "InetAddressPoint.v6.full",
        new InetAddressPoint("ip", InetAddress.getByName("fe80::1234:5678")));
  }

  private static void numericFields() {
    emit("IntField", new IntField("n", 42, Field.Store.NO));
    emit("LongField", new LongField("n", -7L, Field.Store.NO));
    emit("FloatField", new FloatField("n", 2.5f, Field.Store.NO));
    emit("FloatField.exp", new FloatField("n", 1.0E-8f, Field.Store.NO));
    emit("DoubleField", new DoubleField("n", 2.5, Field.Store.NO));
    emit("DoubleField.exp", new DoubleField("n", 1.0E-8, Field.Store.NO));
  }

  private static void ranges() {
    emit("IntRange.1d", new IntRange("r", new int[] {10}, new int[] {20}));
    emit("IntRange.2d", new IntRange("r", new int[] {10, -5}, new int[] {20, 5}));
    emit("LongRange.1d", new LongRange("r", new long[] {10}, new long[] {20}));
    emit("LongRange.2d", new LongRange("r", new long[] {10, -5}, new long[] {20, 5}));
    emit("FloatRange.1d", new FloatRange("r", new float[] {1.5f}, new float[] {2.5f}));
    emit(
        "FloatRange.2d",
        new FloatRange("r", new float[] {1.5f, -1e-8f}, new float[] {2.5f, 1e8f}));
    emit("DoubleRange.1d", new DoubleRange("r", new double[] {1.5}, new double[] {2.5}));
    emit(
        "DoubleRange.2d",
        new DoubleRange("r", new double[] {1.5, -1e-8}, new double[] {2.5, 1e8}));

    // RangeFieldQuery.toString(String): the abstract per-dimension rendering is
    // supplied by the anonymous subclass each newXxxQuery builds.
    Query q1 = IntRange.newIntersectsQuery("r", new int[] {10}, new int[] {20});
    emit("IntRangeQuery.sameField", q1.toString("r"));
    emit("IntRangeQuery.otherField", q1.toString("other"));
    Query q2 = IntRange.newWithinQuery("r", new int[] {10, -5}, new int[] {20, 5});
    emit("IntRangeQuery.2d", q2.toString("r"));
    emit(
        "LongRangeQuery",
        LongRange.newContainsQuery("r", new long[] {10}, new long[] {20}).toString("r"));
    emit(
        "FloatRangeQuery",
        FloatRange.newCrossesQuery("r", new float[] {1.5f}, new float[] {2.5f}).toString("r"));
    emit(
        "DoubleRangeQuery",
        DoubleRange.newIntersectsQuery("r", new double[] {1.5}, new double[] {2.5}).toString("r"));

    // `RangeFieldQuery.equals` is `sameClassAs(o) && equalsTo(...)`, and each
    // `XxxRange.newRelationQuery` builds one anonymous subclass shared by its
    // four relations. So relations of the same range type share a class, and
    // different range types never do.
    Query intIntersects = IntRange.newIntersectsQuery("r", new int[] {10}, new int[] {20});
    Query intWithin = IntRange.newWithinQuery("r", new int[] {10}, new int[] {20});
    Query floatIntersects =
        FloatRange.newIntersectsQuery("r", new float[] {1.5f}, new float[] {2.5f});
    emit("RangeQuery.class.sameType", intIntersects.getClass() == intWithin.getClass());
    emit("RangeQuery.class.crossType", intIntersects.getClass() == floatIntersects.getClass());
    emit("RangeQuery.eq.sameRelation", intIntersects.equals(intWithin));

    try {
      java.net.InetAddress lo = java.net.InetAddress.getByName("1.2.3.4");
      java.net.InetAddress hi = java.net.InetAddress.getByName("1.2.3.10");
      emit("InetAddressRange", new InetAddressRange("r", lo, hi));
      emit("InetAddressRangeQuery", InetAddressRange.newIntersectsQuery("r", lo, hi).toString("r"));
      java.net.InetAddress lo6 = java.net.InetAddress.getByName("::1");
      java.net.InetAddress hi6 = java.net.InetAddress.getByName("::9");
      emit("InetAddressRange.v6", new InetAddressRange("r", lo6, hi6));
      emit(
          "InetAddressRangeQuery.v6",
          InetAddressRange.newIntersectsQuery("r", lo6, hi6).toString("r"));
    } catch (java.net.UnknownHostException e) {
      throw new AssertionError(e);
    }
  }

  private static void fieldTypeAndDocument() {
    emit("FieldType.default", new FieldType());
    emit("FieldType.stored", StoredField.TYPE);
    emit("FieldType.text", TextField.TYPE_NOT_STORED);
    emit("FieldType.textStored", TextField.TYPE_STORED);
    emit("FieldType.string", StringField.TYPE_NOT_STORED);
    FieldType vectors = new FieldType(TextField.TYPE_STORED);
    vectors.setStoreTermVectors(true);
    vectors.setStoreTermVectorOffsets(true);
    vectors.setStoreTermVectorPositions(true);
    vectors.setStoreTermVectorPayloads(true);
    vectors.setOmitNorms(true);
    vectors.freeze();
    emit("FieldType.vectors", vectors);
    FieldType docs = new FieldType();
    docs.setIndexOptions(IndexOptions.DOCS);
    docs.setTokenized(false);
    docs.setDocValuesType(DocValuesType.SORTED_SET);
    docs.setDimensions(2, 4);
    emit("FieldType.points", docs);

    // The same configurations, built with explicit setters, so a port that has
    // no access to the package-private TYPE constants can still be compared.
    FieldType builtStored = new FieldType();
    builtStored.setStored(true);
    emit("FieldType.built.stored", builtStored);
    FieldType builtText = new FieldType();
    builtText.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    builtText.setTokenized(true);
    emit("FieldType.built.text", builtText);
    FieldType builtTextStored = new FieldType();
    builtTextStored.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    builtTextStored.setTokenized(true);
    builtTextStored.setStored(true);
    emit("FieldType.built.textStored", builtTextStored);
    FieldType builtString = new FieldType();
    builtString.setIndexOptions(IndexOptions.DOCS);
    builtString.setTokenized(false);
    builtString.setOmitNorms(true);
    emit("FieldType.built.string", builtString);
    emit("FieldType.built.storedEqualsConstant", builtStored.equals(StoredField.TYPE));
    emit("FieldType.built.textEqualsConstant", builtText.equals(TextField.TYPE_NOT_STORED));
    emit(
        "FieldType.built.textStoredEqualsConstant",
        builtTextStored.equals(TextField.TYPE_STORED));
    emit(
        "FieldType.built.stringEqualsConstant",
        builtString.equals(StringField.TYPE_NOT_STORED));
    emit("Field.builtString", new Field("f", "hello", builtTextStored));
    emit("Field.builtNumber", new Field("f", new org.apache.lucene.util.BytesRef(
        new byte[] {0x0a, (byte) 0xff, 0x00}), builtStored));

    emit("Field.string", new Field("f", "hello", TextField.TYPE_STORED));
    emit("Field.int", new StoredField("f", 42));
    emit("Field.bytes", new StoredField("f", new byte[] {0x0a, (byte) 0xff, 0x00}));

    Document doc = new Document();
    doc.add(new StringField("id", "1", Field.Store.YES));
    doc.add(new IntPoint("count", 42));
    emit("Document.two", doc);
    emit("Document.empty", new Document());
    emit("Document.one", singleFieldDocument());

    // equals verdicts that decide identity in the query cache.
    FieldType a = new FieldType(TextField.TYPE_STORED);
    FieldType b = new FieldType(TextField.TYPE_STORED);
    emit("FieldType.eq.same", a.equals(b));
    FieldType frozenA = new FieldType(TextField.TYPE_STORED);
    frozenA.freeze();
    emit("FieldType.eq.frozenIgnored", frozenA.equals(b));
    FieldType vectorDim = new FieldType(TextField.TYPE_STORED);
    vectorDim.setVectorAttributes(
        4,
        org.apache.lucene.index.VectorEncoding.FLOAT32,
        org.apache.lucene.index.VectorSimilarityFunction.COSINE);
    emit("FieldType.eq.vectorIgnored", vectorDim.equals(b));
    FieldType attr = new FieldType(TextField.TYPE_STORED);
    attr.putAttribute("k", "v");
    emit("FieldType.eq.attributesIgnored", attr.equals(b));
    FieldType tokenized = new FieldType(TextField.TYPE_STORED);
    tokenized.setTokenized(false);
    emit("FieldType.eq.tokenized", tokenized.equals(b));
    emit("FieldType.hash.same", a.hashCode() == b.hashCode());
    emit("FieldType.hash.vectorIgnored", vectorDim.hashCode() == b.hashCode());
  }

  private static Document singleFieldDocument() {
    Document doc = new Document();
    doc.add(new StringField("id", "1", Field.Store.YES));
    return doc;
  }

  private static void hashSetAndFeature() {
    emit("DocValuesLongHashSet.small", new DocValuesLongHashSet(new long[] {1L, 2L, 3L}));
    emit("DocValuesLongHashSet.empty", new DocValuesLongHashSet(new long[] {}));
    emit(
        "DocValuesLongHashSet.missing",
        new DocValuesLongHashSet(new long[] {Long.MIN_VALUE, 5L}));
    emit(
        "DocValuesLongHashSet.spread",
        new DocValuesLongHashSet(new long[] {-100L, 0L, 7L, 42L, 1000L}));
    emit("FeatureDoubleValuesSource", new FeatureDoubleValuesSource("field", "feature"));
    emit(
        "FeatureDoubleValuesSource.eq",
        new FeatureDoubleValuesSource("f", "n").equals(new FeatureDoubleValuesSource("f", "n")));
  }

  private static void collectors() {
    // A collector with a stable `toString`, so the wrapper's is decidable: the
    // identity hash in `Object.toString` is not reproducible.
    Collector noop = new NoopCollector();
    emit("FilterCollector.noop", new NamedFilterCollector(noop).toString());
    emit("FilterCollector.anonNoop", new FilterCollector(noop) {}.toString());
    Collector inner = new TotalHitCountCollector();
    emit("FilterCollector", new FilterCollector(inner) {}.toString());
    emit("FilterCollector.named", new NamedFilterCollector(inner).toString());
    LeafCollector leaf = new NoopLeafCollector();
    emit("FilterLeafCollector.anon", new FilterLeafCollector(leaf) {}.toString());
    emit("FilterLeafCollector.named", new NamedFilterLeafCollector(leaf).toString());
    emit("Collector.inner", inner.toString().startsWith("org.apache.lucene.search"));
  }

  /** A named subclass, so `getClass().getSimpleName()` is not empty. */
  private static final class NamedFilterCollector extends FilterCollector {
    NamedFilterCollector(Collector in) {
      super(in);
    }
  }

  /** A named subclass, so `getClass().getSimpleName()` is not empty. */
  private static final class NamedFilterLeafCollector extends FilterLeafCollector {
    NamedFilterLeafCollector(LeafCollector in) {
      super(in);
    }
  }

  /** A collector with a stable `toString`, so the wrapper's is decidable. */
  private static final class NoopCollector extends SimpleCollector {
    @Override
    public void collect(int doc) {}

    @Override
    public ScoreMode scoreMode() {
      return ScoreMode.COMPLETE_NO_SCORES;
    }

    @Override
    public void setScorer(Scorable scorer) {}

    @Override
    public String toString() {
      return "NOOP";
    }
  }

  /** A leaf collector with a stable `toString`, so the wrapper's is decidable. */
  private static final class NoopLeafCollector extends SimpleCollector {
    @Override
    public void collect(int doc) {}

    @Override
    public ScoreMode scoreMode() {
      return ScoreMode.COMPLETE_NO_SCORES;
    }

    @Override
    public void setScorer(Scorable scorer) {}

    @Override
    public String toString() {
      return "NOOP";
    }
  }

  @SuppressWarnings("unused")
  private static List<String> unused() {
    return List.of();
  }
}
