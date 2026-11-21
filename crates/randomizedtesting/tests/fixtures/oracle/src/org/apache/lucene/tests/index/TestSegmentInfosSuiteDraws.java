// The BaseSegmentInfoFormatTestCase suite of BaseFormatSuitesOracle (rmp #355): the inherited
// cases on the codec and versions TestLucene99SegmentInfoFormat binds, recording after each test
// (in tearDown, before super.tearDown()) the next random().nextLong() of the test's randomness, and
// after the class that of the class randomness.
package org.apache.lucene.tests.index;

import org.apache.lucene.codecs.Codec;
import org.apache.lucene.tests.util.TestUtil;
import org.apache.lucene.util.Version;
import org.junit.AfterClass;

public class TestSegmentInfosSuiteDraws extends BaseSegmentInfoFormatTestCase {
  @Override
  protected Version[] getVersions() {
    return new Version[] {Version.LATEST};
  }

  @Override
  protected Codec getCodec() {
    return TestUtil.getDefaultCodec();
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
