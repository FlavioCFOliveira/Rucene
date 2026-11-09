// The BaseTestCheckIndex suite of BaseFormatSuitesOracle (rmp #355): the members of TestCheckIndex
// that bind it (its directory and the four wrappers), recording after each test (in tearDown,
// before the rest of it) the next random().nextLong() of the test's randomness, and after the
// class that of the class randomness.
package org.apache.lucene.tests.index;

import java.io.IOException;
import org.apache.lucene.store.Directory;
import org.junit.AfterClass;

public class TestCheckIndexSuiteDraws extends BaseTestCheckIndex {
  private Directory directory;

  @Override
  public void setUp() throws Exception {
    super.setUp();
    directory = newDirectory();
  }

  @Override
  public void tearDown() throws Exception {
    BaseFormatSuitesOracle.DRAWS.add(getTestName() + "\tnext\t" + random().nextLong());
    directory.close();
    super.tearDown();
  }

  public void testDeletedDocs() throws IOException {
    testDeletedDocs(directory);
  }

  public void testChecksumsOnly() throws IOException {
    testChecksumsOnly(directory);
  }

  public void testChecksumsOnlyVerbose() throws IOException {
    testChecksumsOnlyVerbose(directory);
  }

  public void testObtainsLock() throws IOException {
    testObtainsLock(directory);
  }

  @AfterClass
  public static void recordClassDraw() {
    BaseFormatSuitesOracle.DRAWS.add("#class\tnext\t" + random().nextLong());
  }
}
