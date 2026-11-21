// The BaseDirectoryTestCase suite of BaseSuitesDrawsOracle (rmp #350): the inherited cases on the
// directory TestTrackingDirectoryWrapper binds, recording the draws BaseSuitesDrawsOracle writes.
package org.apache.lucene.tests.store;

import java.io.IOException;
import java.nio.file.Path;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.TrackingDirectoryWrapper;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.junit.AfterClass;

public class TestDirectorySuiteDraws extends BaseDirectoryTestCase {

  @Override
  protected Directory getDirectory(Path path) throws IOException {
    return new TrackingDirectoryWrapper(new ByteBuffersDirectory());
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
