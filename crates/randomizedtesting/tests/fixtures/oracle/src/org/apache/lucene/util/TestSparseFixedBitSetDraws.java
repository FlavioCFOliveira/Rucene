// The BaseBitSetTestCase suite of BaseSuitesDrawsOracle (rmp #350): the inherited cases on the
// sets TestSparseFixedBitSet binds (its copyOf and its assertEquals override), recording the
// draws BaseSuitesDrawsOracle writes.
package org.apache.lucene.util;

import java.io.IOException;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.tests.util.BaseBitSetTestCase;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.junit.AfterClass;

public class TestSparseFixedBitSetDraws extends BaseBitSetTestCase<SparseFixedBitSet> {

  @Override
  public SparseFixedBitSet copyOf(BitSet bs, int length) throws IOException {
    final SparseFixedBitSet set = new SparseFixedBitSet(length);
    for (int doc = bs.nextSetBit(0);
        doc != DocIdSetIterator.NO_MORE_DOCS;
        doc = doc + 1 >= length ? DocIdSetIterator.NO_MORE_DOCS : bs.nextSetBit(doc + 1)) {
      set.set(doc);
    }
    return set;
  }

  @Override
  protected void assertEquals(BitSet set1, SparseFixedBitSet set2, int maxDoc) {
    super.assertEquals(set1, set2, maxDoc);
    // check invariants of the sparse set
    int nonZeroLongCount = 0;
    for (int i = 0; i < set2.indices.length; ++i) {
      final int n = Long.bitCount(set2.indices[i]);
      if (n != 0) {
        nonZeroLongCount += n;
        for (int j = n; j < set2.bits[i].length; ++j) {
          assertEquals(0, set2.bits[i][j]);
        }
      }
    }
    assertEquals(nonZeroLongCount, set2.nonZeroLongCount);
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
