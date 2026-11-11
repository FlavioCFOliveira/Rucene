// The BaseChunkedDirectoryTestCase suite of BaseSuitesDrawsOracle (rmp #350): the inherited
// cases on the directories TestMultiByteBuffersDirectory binds, recording the draws
// BaseSuitesDrawsOracle writes.
package org.apache.lucene.tests.store;

import java.io.IOException;
import java.nio.file.Path;
import java.util.function.Supplier;
import org.apache.lucene.store.ByteBuffersDataOutput;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.SingleInstanceLockFactory;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.apache.lucene.util.BitUtil;
import org.junit.AfterClass;

public class TestChunkedDirectorySuiteDraws extends BaseChunkedDirectoryTestCase {

  @Override
  protected Directory getDirectory(Path path, int maxChunkSize) throws IOException {
    // round down huge values (above 20) to keep RAM usage low in tests (especially in nightly)
    int bitsPerBlock =
        Math.min(
            20,
            Math.max(
                ByteBuffersDataOutput.LIMIT_MIN_BITS_PER_BLOCK,
                Integer.numberOfTrailingZeros(BitUtil.nextHighestPowerOfTwo(maxChunkSize))));
    Supplier<ByteBuffersDataOutput> outputSupplier =
        () -> {
          return new ByteBuffersDataOutput(
              bitsPerBlock,
              bitsPerBlock,
              ByteBuffersDataOutput.ALLOCATE_BB_ON_HEAP,
              ByteBuffersDataOutput.NO_REUSE);
        };
    return new ByteBuffersDirectory(
        new SingleInstanceLockFactory(),
        outputSupplier,
        ByteBuffersDirectory.OUTPUT_AS_MANY_BUFFERS);
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
