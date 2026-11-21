// The BaseBitSetTestCase suite of BaseSuitesDrawsOracle (rmp #350): the inherited cases on the
// sets TestFixedBitSet binds (its copyOf and its testPrevSetBit override with the helpers it
// calls), recording the draws BaseSuitesDrawsOracle writes.
package org.apache.lucene.util;

import java.io.IOException;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.tests.util.BaseBitSetTestCase;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.junit.AfterClass;

public class TestFixedBitSetDraws extends BaseBitSetTestCase<FixedBitSet> {

  @Override
  public FixedBitSet copyOf(BitSet bs, int length) throws IOException {
    final FixedBitSet set = new FixedBitSet(length);
    for (int doc = bs.nextSetBit(0);
        doc != DocIdSetIterator.NO_MORE_DOCS;
        doc = doc + 1 >= length ? DocIdSetIterator.NO_MORE_DOCS : bs.nextSetBit(doc + 1)) {
      set.set(doc);
    }
    return set;
  }

  void doPrevSetBit(java.util.BitSet a, FixedBitSet b) {
    assertEquals(a.cardinality(), b.cardinality());
    int aa = a.size() + random().nextInt(100);
    int bb = aa;
    do {
      // aa = a.prevSetBit(aa-1);
      aa--;
      while ((aa >= 0) && (!a.get(aa))) {
        aa--;
      }
      if (b.length() == 0) {
        bb = -1;
      } else if (bb > b.length() - 1) {
        bb = b.prevSetBit(b.length() - 1);
      } else if (bb < 1) {
        bb = -1;
      } else {
        bb = bb >= 1 ? b.prevSetBit(bb - 1) : -1;
      }
      assertEquals(aa, bb);
    } while (aa >= 0);
  }

  private FixedBitSet makeFixedBitSet(int[] a, int numBits) {
    FixedBitSet bs;
    if (random().nextBoolean()) {
      int bits2words = FixedBitSet.bits2words(numBits);
      long[] words = new long[bits2words + random().nextInt(100)];
      bs = new FixedBitSet(words, numBits);
    } else {
      bs = new FixedBitSet(numBits);
    }
    for (int e : a) {
      bs.set(e);
    }
    return bs;
  }

  private java.util.BitSet makeBitSet(int[] a) {
    java.util.BitSet bs = new java.util.BitSet();
    for (int e : a) {
      bs.set(e);
    }
    return bs;
  }

  private void checkPrevSetBitArray(int[] a, int numBits) {
    FixedBitSet obs = makeFixedBitSet(a, numBits);
    java.util.BitSet bs = makeBitSet(a);
    doPrevSetBit(bs, obs);
  }

  @Override
  public void testPrevSetBit() {
    checkPrevSetBitArray(new int[] {}, 0);
    checkPrevSetBitArray(new int[] {0}, 1);
    checkPrevSetBitArray(new int[] {0, 2}, 3);
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
