// The BaseXYPointTestCase suite of BaseSuitesDrawsOracle (rmp #358): the inherited cases on the
// overrides TestXYPointQueries binds (XYPointField), recording the draws BaseSuitesDrawsOracle
// writes.
// The class runs with LuceneTestCase's shared executor set to null (as TestRangeFieldSuiteDraws
// does): with it, IndexSearcher's TaskExecutor runs each slice on the test thread or on a pool
// thread as the race falls, and MAYBE_CACHE_POLICY.shouldCache draws random() on the thread that
// runs the slice, so the test randomness's next draw varied run to run.
package org.apache.lucene.tests.geo;

import java.lang.reflect.Field;
import java.util.concurrent.ExecutorService;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.XYPointField;
import org.apache.lucene.geo.XYGeometry;
import org.apache.lucene.geo.XYPolygon;
import org.apache.lucene.search.Query;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.apache.lucene.tests.util.LuceneTestCase;
import org.junit.AfterClass;
import org.junit.BeforeClass;

public class TestXYPointSuiteDraws extends BaseXYPointTestCase {

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
  protected void addPointToDoc(String field, Document doc, float x, float y) {
    doc.add(new XYPointField(field, x, y));
  }

  @Override
  protected Query newRectQuery(String field, float minX, float maxX, float minY, float maxY) {
    return XYPointField.newBoxQuery(field, minX, maxX, minY, maxY);
  }

  @Override
  protected Query newDistanceQuery(String field, float centerX, float centerY, float radius) {
    return XYPointField.newDistanceQuery(field, centerX, centerY, radius);
  }

  @Override
  protected Query newPolygonQuery(String field, XYPolygon... polygons) {
    return XYPointField.newPolygonQuery(field, polygons);
  }

  @Override
  protected Query newGeometryQuery(String field, XYGeometry... geometries) {
    return XYPointField.newGeometryQuery(field, geometries);
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
