// The BaseGeoPointTestCase suite of BaseSuitesDrawsOracle (rmp #358): the inherited cases on the
// overrides TestLatLonPointQueries binds (LatLonPoint and its quantization), recording the draws
// BaseSuitesDrawsOracle writes.
package org.apache.lucene.tests.geo;

import org.apache.lucene.document.Document;
import org.apache.lucene.document.LatLonPoint;
import org.apache.lucene.document.ShapeField;
import org.apache.lucene.geo.GeoEncodingUtils;
import org.apache.lucene.geo.LatLonGeometry;
import org.apache.lucene.geo.Polygon;
import org.apache.lucene.search.Query;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.junit.AfterClass;

public class TestGeoPointSuiteDraws extends BaseGeoPointTestCase {

  @Override
  protected void addPointToDoc(String field, Document doc, double lat, double lon) {
    doc.add(new LatLonPoint(field, lat, lon));
  }

  @Override
  protected Query newRectQuery(
      String field, double minLat, double maxLat, double minLon, double maxLon) {
    return LatLonPoint.newBoxQuery(field, minLat, maxLat, minLon, maxLon);
  }

  @Override
  protected Query newDistanceQuery(
      String field, double centerLat, double centerLon, double radiusMeters) {
    return LatLonPoint.newDistanceQuery(field, centerLat, centerLon, radiusMeters);
  }

  @Override
  protected Query newPolygonQuery(String field, Polygon... polygons) {
    return LatLonPoint.newPolygonQuery(field, polygons);
  }

  @Override
  protected Query newGeometryQuery(String field, LatLonGeometry... geometry) {
    return LatLonPoint.newGeometryQuery(field, ShapeField.QueryRelation.INTERSECTS, geometry);
  }

  @Override
  protected double quantizeLat(double latRaw) {
    return GeoEncodingUtils.decodeLatitude(GeoEncodingUtils.encodeLatitude(latRaw));
  }

  @Override
  protected double quantizeLon(double lonRaw) {
    return GeoEncodingUtils.decodeLongitude(GeoEncodingUtils.encodeLongitude(lonRaw));
  }

  @Override
  public void tearDown() throws Exception {
    BaseSuitesDrawsOracle.recordTest(getTestName(), random().nextLong());
    super.tearDown();
  }

  @AfterClass
  public static void recordClassDraw() {
    BaseSuitesDrawsOracle.recordClass(random().nextLong());
  }
}
