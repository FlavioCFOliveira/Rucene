package org.apache.lucene.tests.util;

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
 * The oracle of rmp #359 (part 2): runs Lucene 10.5.0's own test classes of {@code
 * lucene/test-framework/src/test} (compiled by {@code regenerate.sh} from {@code $LUCENE_SRC}) with
 * {@code JUnitCore} and records what each test did, for the suites of {@code
 * crates/rucene-test-framework/tests/} that port them. Writes {@code
 * <out>/rucene-test-framework/framework_tests.tsv}: per class, a {@code #counts} row ({@code
 * Result}'s run, failure, ignore and assumption-failure counts) and one row per listener event
 * ({@code S} started, {@code F} failure with the exception's class and the first line of its
 * message, {@code A} assumption failure with the first line of its message, {@code I} ignored),
 * the method {@code null} for a suite-level event; the event rows of a class are sorted, as the
 * runner shuffles the test order. Without the compiled test classes on the class path it writes
 * nothing, and the committed fixture stays.
 */
public class FrameworkTestsOracle {
  static final String[] CLASSES = {
    "org.apache.lucene.tests.util.TestMaxFailuresRule",
    "org.apache.lucene.tests.util.TestExceptionInBeforeClassHooks",
    "org.apache.lucene.tests.util.TestExpectThrows",
    "org.apache.lucene.tests.util.TestFailIfDirectoryNotClosed",
    "org.apache.lucene.tests.util.TestFailIfUnreferencedFiles",
    "org.apache.lucene.tests.util.TestFloatingPointUlpEquality",
    "org.apache.lucene.tests.util.TestGroupFiltering",
    "org.apache.lucene.tests.util.TestJvmInfo",
    "org.apache.lucene.tests.util.TestLineFileDocs",
    "org.apache.lucene.tests.util.TestRamUsageTesterOnWildAnimals",
    "org.apache.lucene.tests.util.TestSeedFromUncaught",
    "org.apache.lucene.tests.util.TestWorstCaseTestBehavior",
    "org.apache.lucene.tests.util.TestRunWithRestrictedPermissions",
    "org.apache.lucene.tests.util.TestTestObjectInputFilterFactory",
    "org.apache.lucene.tests.index.TestAssertingLeafReader",
    "org.apache.lucene.tests.index.TestForceMergePolicy",
    "org.apache.lucene.tests.index.TestMockRandomMergePolicy",
    "org.apache.lucene.tests.search.TestAssertingLeafCollector",
    "org.apache.lucene.tests.search.TestPerThreadPKLookup",
    "org.apache.lucene.tests.search.TestQueryUtils",
    "org.apache.lucene.tests.mockfile.TestDisableFsyncFS",
    "org.apache.lucene.tests.mockfile.TestExtrasFS",
    "org.apache.lucene.tests.mockfile.TestHandleLimitFS",
    "org.apache.lucene.tests.mockfile.TestHandleTrackingFS",
    "org.apache.lucene.tests.mockfile.TestLeakFS",
    "org.apache.lucene.tests.mockfile.TestShuffleFS",
    "org.apache.lucene.tests.mockfile.TestVerboseFS",
    "org.apache.lucene.tests.mockfile.TestVirusCheckingFS",
    "org.apache.lucene.tests.mockfile.TestWindowsFS",
  };

  static String firstLine(String s) {
    String v = String.valueOf(s);
    int nl = v.indexOf('\n');
    return (nl < 0 ? v : v.substring(0, nl)).replace("\t", "\\t").replace("\r", "");
  }

  public static void main(String[] args) throws Exception {
    List<Class<?>> classes = new ArrayList<>();
    for (String name : CLASSES) {
      try {
        classes.add(Class.forName(name));
      } catch (ClassNotFoundException e) {
        return; // no $LUCENE_SRC: keep the committed fixture
      }
    }
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    try (PrintWriter w = new PrintWriter(new File(out, "framework_tests.tsv"), "UTF-8")) {
      w.println("# class\tmethod\tevent\tdetail (FrameworkTestsOracle, Lucene 10.5.0, JDK 25)");
      for (Class<?> c : classes) {
        List<String> events = Collections.synchronizedList(new ArrayList<>());
        JUnitCore core = new JUnitCore();
        core.addListener(
            new RunListener() {
              @Override
              public void testStarted(Description d) {
                events.add(d.getMethodName() + "\tS\t-");
              }

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

              @Override
              public void testIgnored(Description d) {
                events.add(d.getMethodName() + "\tI\t-");
              }
            });
        Result r = core.run(c);
        w.println(
            c.getName()
                + "\t#counts\trun="
                + r.getRunCount()
                + " fail="
                + r.getFailureCount()
                + " ignore="
                + r.getIgnoreCount()
                + " assume="
                + r.getAssumptionFailureCount()
                + "\t-");
        List<String> sorted = new ArrayList<>(events);
        Collections.sort(sorted);
        for (String e : sorted) {
          w.println(c.getName() + "\t" + e);
        }
      }
    }
  }
}
