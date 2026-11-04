package org.apache.lucene.tests.util;

import java.io.File;
import java.io.PrintWriter;
import java.util.Random;

/**
 * The oracle of the {@code org.apache.lucene.tests.util} helpers of rmp #342, for {@code
 * crates/rucene-test-framework/tests/test_util_oracle.rs}. Writes into {@code
 * <out>/rucene-test-framework/}:
 *
 * <ul>
 *   <li>{@code testutil_english.tsv}: {@code English.intToEnglish} of every int in [-1000, 1000],
 *       {@code Integer.MIN_VALUE} and {@code Integer.MAX_VALUE}; {@code English.longToEnglish} of
 *       {@code ±10^k} and {@code 10^k - 1} for k in [0, 18], {@code Long.MIN_VALUE}, {@code
 *       Long.MAX_VALUE}, and 500 longs of {@code new Random(0)} (each also divided by {@code
 *       10^(i % 19)}).
 * </ul>
 */
public final class TestUtilOracle {
  private TestUtilOracle() {}

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_english.tsv"), "UTF-8")) {
      w.println("# method\tvalue\tresult");
      for (int i = -1000; i <= 1000; i++) {
        w.println("int\t" + i + "\t" + English.intToEnglish(i));
      }
      for (int i : new int[] {Integer.MIN_VALUE, Integer.MAX_VALUE}) {
        w.println("int\t" + i + "\t" + English.intToEnglish(i));
      }
      long p = 1;
      for (int k = 0; k <= 18; k++, p *= 10) {
        for (long v : new long[] {p, -p, p - 1}) {
          w.println("long\t" + v + "\t" + English.longToEnglish(v));
        }
      }
      for (long v : new long[] {Long.MIN_VALUE, Long.MAX_VALUE}) {
        w.println("long\t" + v + "\t" + English.longToEnglish(v));
      }
      Random r = new Random(0);
      for (int i = 0; i < 500; i++) {
        long v = r.nextLong();
        long d = 1;
        for (int k = 0; k < i % 19; k++) {
          d *= 10;
        }
        w.println("long\t" + v / d + "\t" + English.longToEnglish(v / d));
      }
    }
  }
}
