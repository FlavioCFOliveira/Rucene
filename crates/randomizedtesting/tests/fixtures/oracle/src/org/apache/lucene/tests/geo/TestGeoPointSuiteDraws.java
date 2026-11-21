// The BaseGeoPointTestCase suite of BaseSuitesDrawsOracle (rmp #358): the inherited cases on the
// overrides TestLatLonPointQueries binds (LatLonPoint and its quantization), recording the draws
// BaseSuitesDrawsOracle writes.
// The class runs with LuceneTestCase's shared executor set to null (as TestRangeFieldSuiteDraws
// does): with it, IndexSearcher's TaskExecutor runs each slice on the test thread or on a pool
// thread as the race falls, and MAYBE_CACHE_POLICY.shouldCache draws random() on the thread that
// runs the slice, so the test randomness's next draw varied run to run.
package org.apache.lucene.tests.geo;

import java.lang.reflect.Field;
import java.util.concurrent.ExecutorService;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.LatLonPoint;
import org.apache.lucene.document.ShapeField;
import org.apache.lucene.geo.GeoEncodingUtils;
import org.apache.lucene.geo.LatLonGeometry;
import org.apache.lucene.geo.Polygon;
import org.apache.lucene.search.Query;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.apache.lucene.tests.util.LuceneTestCase;
import org.junit.AfterClass;
import org.junit.BeforeClass;

public class TestGeoPointSuiteDraws extends BaseGeoPointTestCase {

  /** The shared executor {@link #searchOnTestThread} sets aside; restored by the class draw. */
  private static ExecutorService sharedExecutor;

  /** Sets LuceneTestCase's shared executor to null, so every search draw is on the test thread. */
  @BeforeClass
  public static void searchOnTestThread() throws Exception {
    Field f = LuceneTestCase.class.getDeclaredField("executor");
    f.setAccessible(true);
    sharedExecutor = (ExecutorService) f.get(null);
    f.set(null, null);
  }

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
  public static void recordClassDraw() throws Exception {
    BaseSuitesDrawsOracle.recordClass(random().nextLong());
    Field f = LuceneTestCase.class.getDeclaredField("executor");
    f.setAccessible(true);
    f.set(null, sharedExecutor);
    sharedExecutor = null;
  }
}
