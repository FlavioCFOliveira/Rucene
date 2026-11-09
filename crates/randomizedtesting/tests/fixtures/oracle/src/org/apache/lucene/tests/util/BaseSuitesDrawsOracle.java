package org.apache.lucene.tests.util;

import java.io.File;
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.notification.Failure;
import org.junit.runner.notification.RunListener;

/**
 * The seeded-draw oracle of rmp #350 and #356: runs, with {@code JUnitCore} on fixed master seeds,
 * one draws class per base suite (a subclass with the overrides of the Lucene class the Rust suite
 * binds, whose {@code tearDown()} records the next {@code random().nextLong()} of each test's
 * randomness before {@code super.tearDown()}, and whose {@code @AfterClass} records that of the class
 * randomness), and writes {@code <out>/rucene-test-framework/base_*_draws.tsv}: per seed, the
 * {@code #counts} row, one row per test, the class row and one row per failure or assumption
 * failure (class and first line of the message). Rows are sorted within a seed, as the runner
 * shuffles the test order. The format is {@code BaseFormatSuitesOracle}'s.
 */
public class BaseSuitesDrawsOracle {
  static final String[] SEEDS = {"DEADBEEF", "1", "C0FFEE"};

  /** The rows the draws classes record. */
  public static final List<String> DRAWS = Collections.synchronizedList(new ArrayList<>());

  /** Records the next draw of the test's randomness, as each draws class's tearDown() does. */
  public static void recordTest(String testName, long next) {
    DRAWS.add(testName + "\tnext\t" + next);
  }

  /** Records the next draw of the class randomness, as each draws class's @AfterClass does. */
  public static void recordClass(long next) {
    DRAWS.add("#class\tnext\t" + next);
  }

  static String firstLine(String s) {
    String v = String.valueOf(s);
    int nl = v.indexOf('\n');
    return (nl < 0 ? v : v.substring(0, nl)).replace("\t", "\\t").replace("\r", "");
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    write(out, "base_sparse_fixed_bit_set_draws.tsv", "org.apache.lucene.util.TestSparseFixedBitSetDraws");
    write(out, "base_fixed_bit_set_draws.tsv", "org.apache.lucene.util.TestFixedBitSetDraws");
    write(out, "base_roaring_doc_id_set_draws.tsv", "org.apache.lucene.util.TestRoaringDocIdSetDraws");
    write(
        out,
        "base_sparse_fixed_bit_doc_id_set_draws.tsv",
        "org.apache.lucene.util.TestSparseFixedBitDocIdSetDraws");
    write(out, "base_directory_draws.tsv", "org.apache.lucene.tests.store.TestDirectorySuiteDraws");
    write(
        out,
        "base_chunked_directory_draws.tsv",
        "org.apache.lucene.tests.store.TestChunkedDirectorySuiteDraws");
    write(
        out, "base_lock_factory_draws.tsv", "org.apache.lucene.tests.store.TestLockFactorySuiteDraws");
    write(
        out,
        "base_similarity_draws.tsv",
        "org.apache.lucene.tests.search.similarities.TestSimilaritySuiteDraws");
    write(out, "base_range_field_draws.tsv", "org.apache.lucene.tests.search.TestRangeFieldSuiteDraws");
  }

  static void write(File dir, String fileName, String className) throws Exception {
    Class<?> suite = Class.forName(className);
    String name = suite.getSimpleName();
    try (PrintWriter w = new PrintWriter(new File(dir, fileName), "UTF-8")) {
      w.println(
          "# seed\tmethod\tkind\tvalue (BaseSuitesDrawsOracle, Lucene 10.5.0, JDK 25): "
              + name
              + ", next random().nextLong() after each test and the class");
      for (String seed : SEEDS) {
        System.setProperty("tests.seed", seed);
        DRAWS.clear();
        List<String> events = Collections.synchronizedList(new ArrayList<>());
        JUnitCore core = new JUnitCore();
        core.addListener(
            new RunListener() {
              @Override
              public void testFailure(Failure f) {
                Throwable t = f.getException();
                events.add(
                    f.getDescription().getMethodName()
                        + "\tF\t"
                        + t.getClass().getName()
                        + ": "
                        + firstLine(t.getMessage()));
              }

              @Override
              public void testAssumptionFailure(Failure f) {
                events.add(
                    f.getDescription().getMethodName() + "\tA\t" + firstLine(f.getMessage()));
              }
            });
        Result r = core.run(suite);
        List<String> rows = new ArrayList<>(DRAWS);
        rows.addAll(events);
        rows.add(
            "#counts\trun="
                + r.getRunCount()
                + " fail="
                + r.getFailureCount()
                + " assume="
                + r.getAssumptionFailureCount()
                + "\t-");
        Collections.sort(rows);
        for (String row : rows) {
          w.println(seed + "\t" + row);
        }
      }
      System.clearProperty("tests.seed");
    }
  }
}
