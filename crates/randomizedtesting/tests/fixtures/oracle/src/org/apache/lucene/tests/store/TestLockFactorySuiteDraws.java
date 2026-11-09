// The BaseLockFactoryTestCase suite of BaseSuitesDrawsOracle (rmp #350): the inherited cases on
// the directories TestSingleInstanceLockFactory binds, recording the draws BaseSuitesDrawsOracle
// writes.
package org.apache.lucene.tests.store;

import java.io.IOException;
import java.nio.file.Path;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.SingleInstanceLockFactory;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.junit.AfterClass;

public class TestLockFactorySuiteDraws extends BaseLockFactoryTestCase {

  @Override
  protected Directory getDirectory(Path path) throws IOException {
    return newDirectory(random(), new SingleInstanceLockFactory());
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
