// The BaseCompoundFormatTestCase suite of BaseFormatSuitesOracle (rmp #352, #353): the
// inherited cases on the codec TestLucene90CompoundFormat binds, recording after each test (in
// tearDown, before super.tearDown()) the next random().nextLong() of the test's
// randomness, and after the class that of the class randomness, so that the Rust
// suite proves it drew the same random choices.
package org.apache.lucene.tests.index;

import org.apache.lucene.codecs.Codec;
import org.apache.lucene.tests.util.TestUtil;
import org.junit.AfterClass;

public class TestCompoundSuiteDraws extends BaseCompoundFormatTestCase {
  private final Codec codec = TestUtil.getDefaultCodec();

  @Override
  protected Codec getCodec() {
    return codec;
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
