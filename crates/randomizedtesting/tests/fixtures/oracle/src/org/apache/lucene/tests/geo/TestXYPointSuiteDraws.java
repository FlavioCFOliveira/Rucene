// The BaseXYPointTestCase suite of BaseSuitesDrawsOracle (rmp #358): the inherited cases on the
// overrides TestXYPointQueries binds (XYPointField), recording the draws BaseSuitesDrawsOracle
// writes.
package org.apache.lucene.tests.geo;

import org.apache.lucene.document.Document;
import org.apache.lucene.document.XYPointField;
import org.apache.lucene.geo.XYGeometry;
import org.apache.lucene.geo.XYPolygon;
import org.apache.lucene.search.Query;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.junit.AfterClass;

public class TestXYPointSuiteDraws extends BaseXYPointTestCase {

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
  public static void recordClassDraw() {
    BaseSuitesDrawsOracle.recordClass(random().nextLong());
  }
}
