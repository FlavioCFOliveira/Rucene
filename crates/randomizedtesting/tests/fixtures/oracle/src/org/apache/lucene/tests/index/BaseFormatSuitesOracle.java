package org.apache.lucene.tests.index;

import java.io.File;
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import org.junit.runner.Description;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.notification.Failure;
import org.junit.runner.notification.RunListener;

/**
 * The seeded-draw oracle of rmp #351: runs {@link TestPostingsSuiteDraws} (the inherited cases of
 * {@code BaseIndexFileFormatTestCase} and {@code BasePostingsFormatTestCase} on {@code
 * TestUtil.alwaysPostingsFormat(new Lucene104PostingsFormat())}) and {@link
 * TestStoredFieldsSuiteDraws} (those of {@code BaseStoredFieldsFormatTestCase} on {@code
 * TestUtil.getDefaultCodec()}), and the term vectors, norms, compound, live docs and doc values
 * suites ({@link TestTermVectorsSuiteDraws}, {@link TestNormsSuiteDraws}, {@link
 * TestCompoundSuiteDraws}, {@link TestLiveDocsSuiteDraws}, {@link TestDocValuesSuiteDraws}, each on
 * {@code TestUtil.getDefaultCodec()}) with {@code JUnitCore} on fixed master seeds and writes {@code
 * <out>/rucene-test-framework/base_postings_draws.tsv}, {@code base_stored_fields_draws.tsv},
 * {@code base_term_vectors_draws.tsv}, {@code base_norms_draws.tsv}, {@code
 * base_compound_draws.tsv}, {@code base_live_docs_draws.tsv} and {@code base_doc_values_draws.tsv}:
 * per seed,
 * the {@code #counts} row, one row per test with the next {@code random().nextLong()} of its
 * randomness after the test, the class row with the next draw of the class randomness, and one row
 * per failure or assumption failure (class and first line of the message). Rows are sorted within a
 * seed, as the runner shuffles the test order.
 */
public class BaseFormatSuitesOracle {
  static final String[] SEEDS = {"DEADBEEF", "1", "C0FFEE"};
  static final List<String> DRAWS = Collections.synchronizedList(new ArrayList<>());

  static String firstLine(String s) {
    String v = String.valueOf(s);
    int nl = v.indexOf('\n');
    return (nl < 0 ? v : v.substring(0, nl)).replace("\t", "\\t").replace("\r", "");
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    write(
        new File(out, "base_postings_draws.tsv"),
        TestPostingsSuiteDraws.class,
        "TestPostingsSuiteDraws");
    write(
        new File(out, "base_stored_fields_draws.tsv"),
        TestStoredFieldsSuiteDraws.class,
        "TestStoredFieldsSuiteDraws");
    write(
        new File(out, "base_term_vectors_draws.tsv"),
        TestTermVectorsSuiteDraws.class,
        "TestTermVectorsSuiteDraws");
    write(new File(out, "base_norms_draws.tsv"), TestNormsSuiteDraws.class, "TestNormsSuiteDraws");
    write(
        new File(out, "base_compound_draws.tsv"),
        TestCompoundSuiteDraws.class,
        "TestCompoundSuiteDraws");
    write(
        new File(out, "base_live_docs_draws.tsv"),
        TestLiveDocsSuiteDraws.class,
        "TestLiveDocsSuiteDraws");
    write(
        new File(out, "base_doc_values_draws.tsv"),
        TestDocValuesSuiteDraws.class,
        "TestDocValuesSuiteDraws");
  }

  static void write(File file, Class<?> suite, String name) throws Exception {
    try (PrintWriter w = new PrintWriter(file, "UTF-8")) {
      w.println(
          "# seed\tmethod\tkind\tvalue (BaseFormatSuitesOracle, Lucene 10.5.0, JDK 25): "
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
