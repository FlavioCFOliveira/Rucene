// The BaseDocIdSetTestCase suite of BaseSuitesDrawsOracle (rmp #350): the inherited cases on the
// sets TestSparseFixedBitDocIdSet binds (its copyOf, whose shuffles draw randomness, and its
// assertEquals override), recording the draws BaseSuitesDrawsOracle writes.
package org.apache.lucene.util;

import java.io.IOException;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.Collections;
import java.util.List;
import org.apache.lucene.tests.util.BaseDocIdSetTestCase;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.junit.AfterClass;

public class TestSparseFixedBitDocIdSetDraws extends BaseDocIdSetTestCase<BitDocIdSet> {

  @Override
  public BitDocIdSet copyOf(BitSet bs, int length) throws IOException {
    final SparseFixedBitSet set = new SparseFixedBitSet(length);
    // SparseFixedBitSet can be sensitive to the order of insertion so
    // randomize insertion a bit
    List<Integer> buffer = new ArrayList<>();
    for (int doc = bs.nextSetBit(0); doc != -1; doc = bs.nextSetBit(doc + 1)) {
      buffer.add(doc);
      if (buffer.size() >= 100000) {
        Collections.shuffle(buffer, random());
        for (int i : buffer) {
          set.set(i);
        }
        buffer.clear();
      }
    }
    Collections.shuffle(buffer, random());
    for (int i : buffer) {
      set.set(i);
    }
    return new BitDocIdSet(set, set.approximateCardinality());
  }

  @Override
  public void assertEquals(int numBits, BitSet ds1, BitDocIdSet ds2) throws IOException {
    for (int i = 0; i < numBits; ++i) {
      assertEquals(ds1.get(i), ds2.bits().get(i));
    }
    assertEquals(ds1.cardinality(), ds2.bits().cardinality());
    super.assertEquals(numBits, ds1, ds2);
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
