// The BaseKnnVectorsFormatTestCase suite of BaseFormatSuitesOracle (rmp #354): the inherited cases
// on the codec TestLucene99HnswVectorsFormat binds (TestUtil.getDefaultCodec(), no float-vector
// fallback), recording after each test (in tearDown, before super.tearDown()) the next
// random().nextLong() of the test's randomness, and after the class that of the class randomness.
package org.apache.lucene.tests.index;

import org.apache.lucene.codecs.Codec;
import org.apache.lucene.tests.util.TestUtil;
import org.junit.AfterClass;

public class TestKnnVectorsSuiteDraws extends BaseKnnVectorsFormatTestCase {
  @Override
  protected Codec getCodec() {
    return TestUtil.getDefaultCodec();
  }

  @Override
  protected boolean supportsFloatVectorFallback() {
    return false;
  }

  @Override
  public void tearDown() throws Exception {
    BaseFormatSuitesOracle.DRAWS.add(getTestName() + "\tnext\t" + random().nextLong());
    super.tearDown();
  }

  @AfterClass
  public static void recordClassDraw() {
    BaseFormatSuitesOracle.DRAWS.add("#class\tnext\t" + random().nextLong());
  }
}
