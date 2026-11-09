// The BaseSimilarityTestCase suite of BaseSuitesDrawsOracle (rmp #356): the inherited cases on
// the similarities TestBM25Similarity binds, recording the draws BaseSuitesDrawsOracle writes.
package org.apache.lucene.tests.search.similarities;

import java.util.Random;
import org.apache.lucene.search.similarities.BM25Similarity;
import org.apache.lucene.search.similarities.Similarity;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.junit.AfterClass;

public class TestSimilaritySuiteDraws extends BaseSimilarityTestCase {

  @Override
  protected Similarity getSimilarity(Random random) {
    // term frequency normalization parameter k1
    final float k1;
    switch (random.nextInt(4)) {
      case 0:
        // minimum value
        k1 = 0;
        break;
      case 1:
        // tiny value
        k1 = Float.MIN_VALUE;
        break;
      case 2:
        // maximum value
        // upper bounds on individual term's score is 43.262806 * (k1 + 1) * boost
        // we just limit the test to "reasonable" k1 values but don't enforce this anywhere.
        k1 = Integer.MAX_VALUE;
        break;
      default:
        // random value
        k1 = Integer.MAX_VALUE * random.nextFloat();
        break;
    }

    // length normalization parameter b [0 .. 1]
    final float b;
    switch (random.nextInt(4)) {
      case 0:
        // minimum value
        b = 0;
        break;
      case 1:
        // tiny value
        b = Float.MIN_VALUE;
        break;
      case 2:
        // maximum value
        b = 1;
        break;
      default:
        // random value
        b = random.nextFloat();
        break;
    }
    return new BM25Similarity(k1, b);
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
