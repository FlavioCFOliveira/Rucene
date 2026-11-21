// The BaseDocIdSetTestCase suite of BaseSuitesDrawsOracle (rmp #350): the inherited cases on the
// sets TestRoaringDocIdSet binds (its copyOf and its assertEquals override), recording the draws
// BaseSuitesDrawsOracle writes.
package org.apache.lucene.util;

import java.io.IOException;
import java.util.BitSet;
import org.apache.lucene.tests.util.BaseDocIdSetTestCase;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.junit.AfterClass;

public class TestRoaringDocIdSetDraws extends BaseDocIdSetTestCase<RoaringDocIdSet> {

  @Override
  public RoaringDocIdSet copyOf(BitSet bs, int length) throws IOException {
    final RoaringDocIdSet.Builder builder = new RoaringDocIdSet.Builder(length);
    for (int i = bs.nextSetBit(0); i != -1; i = bs.nextSetBit(i + 1)) {
      builder.add(i);
    }
    return builder.build();
  }

  @Override
  public void assertEquals(int numBits, BitSet ds1, RoaringDocIdSet ds2) throws IOException {
    super.assertEquals(numBits, ds1, ds2);
    assertEquals(ds1.cardinality(), ds2.cardinality());
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
