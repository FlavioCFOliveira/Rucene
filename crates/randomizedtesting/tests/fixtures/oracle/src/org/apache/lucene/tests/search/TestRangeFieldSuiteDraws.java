// The BaseRangeFieldQueryTestCase suite of BaseSuitesDrawsOracle (rmp #356): the inherited cases
// on the ranges TestIntRangeFieldQueries binds (its nextRange and IntTestRange), recording the
// draws BaseSuitesDrawsOracle writes.
package org.apache.lucene.tests.search;

import java.util.Arrays;
import org.apache.lucene.document.IntRange;
import org.apache.lucene.search.Query;
import org.apache.lucene.tests.util.BaseSuitesDrawsOracle;
import org.apache.lucene.tests.util.TestUtil;
import org.junit.AfterClass;

public class TestRangeFieldSuiteDraws extends BaseRangeFieldQueryTestCase {
  private static final String FIELD_NAME = "intRangeField";

  private int nextIntInternal() {
    switch (random().nextInt(5)) {
      case 0:
        return Integer.MIN_VALUE;
      case 1:
        return Integer.MAX_VALUE;
      default:
        int bpv = random().nextInt(32);
        switch (bpv) {
          case 32:
            return random().nextInt();
          default:
            int v = TestUtil.nextInt(random(), 0, (1 << bpv) - 1);
            if (bpv > 0) {
              // negative values sometimes
              v -= 1 << (bpv - 1);
            }
            return v;
        }
    }
  }

  @Override
  protected Range nextRange(int dimensions) throws Exception {
    int[] min = new int[dimensions];
    int[] max = new int[dimensions];

    int minV, maxV;
    for (int d = 0; d < dimensions; ++d) {
      minV = nextIntInternal();
      maxV = nextIntInternal();
      min[d] = Math.min(minV, maxV);
      max[d] = Math.max(minV, maxV);
    }
    return new IntTestRange(min, max);
  }

  @Override
  protected org.apache.lucene.document.IntRange newRangeField(Range r) {
    return new IntRange(FIELD_NAME, ((IntTestRange) r).min, ((IntTestRange) r).max);
  }

  @Override
  protected Query newIntersectsQuery(Range r) {
    return IntRange.newIntersectsQuery(FIELD_NAME, ((IntTestRange) r).min, ((IntTestRange) r).max);
  }

  @Override
  protected Query newContainsQuery(Range r) {
    return IntRange.newContainsQuery(FIELD_NAME, ((IntTestRange) r).min, ((IntTestRange) r).max);
  }

  @Override
  protected Query newWithinQuery(Range r) {
    return IntRange.newWithinQuery(FIELD_NAME, ((IntTestRange) r).min, ((IntTestRange) r).max);
  }

  @Override
  protected Query newCrossesQuery(Range r) {
    return IntRange.newCrossesQuery(FIELD_NAME, ((IntTestRange) r).min, ((IntTestRange) r).max);
  }

  /** IntRange test class implementation - use to validate IntRange */
  protected class IntTestRange extends Range {
    int[] min;
    int[] max;

    IntTestRange(int[] min, int[] max) {
      assert min != null && max != null && min.length > 0 && max.length > 0
          : "test box: min/max cannot be null or empty";
      assert min.length == max.length : "test box: min/max length do not agree";
      this.min = min;
      this.max = max;
    }

    @Override
    protected int numDimensions() {
      return min.length;
    }

    @Override
    protected Integer getMin(int dim) {
      return min[dim];
    }

    @Override
    protected void setMin(int dim, Object val) {
      int v = (Integer) val;
      if (min[dim] < v) {
        max[dim] = v;
      } else {
        min[dim] = v;
      }
    }

    @Override
    protected Integer getMax(int dim) {
      return max[dim];
    }

    @Override
    protected void setMax(int dim, Object val) {
      int v = (Integer) val;
      if (max[dim] > v) {
        min[dim] = v;
      } else {
        max[dim] = v;
      }
    }

    @Override
    protected boolean isEqual(Range other) {
      IntTestRange o = (IntTestRange) other;
      return Arrays.equals(min, o.min) && Arrays.equals(max, o.max);
    }

    @Override
    protected boolean isDisjoint(Range o) {
      IntTestRange other = (IntTestRange) o;
      for (int d = 0; d < this.min.length; ++d) {
        if (this.min[d] > other.max[d] || this.max[d] < other.min[d]) {
          // disjoint:
          return true;
        }
      }
      return false;
    }

    @Override
    protected boolean isWithin(Range o) {
      IntTestRange other = (IntTestRange) o;
      for (int d = 0; d < this.min.length; ++d) {
        if ((this.min[d] >= other.min[d] && this.max[d] <= other.max[d]) == false) {
          // not within:
          return false;
        }
      }
      return true;
    }

    @Override
    protected boolean contains(Range o) {
      IntTestRange other = (IntTestRange) o;
      for (int d = 0; d < this.min.length; ++d) {
        if ((this.min[d] <= other.min[d] && this.max[d] >= other.max[d]) == false) {
          // not contains:
          return false;
        }
      }
      return true;
    }

    @Override
    public String toString() {
      StringBuilder b = new StringBuilder();
      b.append("Box(");
      b.append(min[0]);
      b.append(" TO ");
      b.append(max[0]);
      for (int d = 1; d < min.length; ++d) {
        b.append(", ");
        b.append(min[d]);
        b.append(" TO ");
        b.append(max[d]);
      }
      b.append(")");
      return b.toString();
    }
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
