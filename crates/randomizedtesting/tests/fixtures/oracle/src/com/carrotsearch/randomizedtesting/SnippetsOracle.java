// Oracle for the port of randomizedtesting 2.8.1's one snippet,
// snippets/TheGoodBadAndUglySnippet.java (task #515; Rust:
// tests/self_tests/snippets/).
//
// The snippet is no test class (SelfTestsOracle skips it): its nested suite
// TheGoodBadAndUgly runs only nested, its @BeforeClass assuming
// WithNestedTestClass's running-nested flag. This oracle records both ways
// of running that suite under JUnit 4.13.1 and writes
// self_tests_snippets.tsv:
//
// * `direct`: JUnitCore runs the suite as is (the assumption fails at class
//   level);
// * `nested <seed>`: with tests.seed=<seed>, between
//   WithNestedTestClass.setupNested() and clearNested() (what its
//   @BeforeClass and @AfterClass do), WithNestedTestClass.runTests(suite).
//
// Rows: `C<TAB>scenario<TAB>run<TAB>fail<TAB>ignored<TAB>assumption failures`,
// then per failure `F<TAB>scenario<TAB>test<TAB>exception class<TAB>first
// line of the message` (a thread dump follows the first line of a leak).
// The classes come from the class path by name (regenerate.sh compiles them
// from $RANDOMIZEDTESTING_SRC), so this file compiles without them.
package com.carrotsearch.randomizedtesting;

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.notification.Failure;
import org.junit.runner.notification.RunListener;

public class SnippetsOracle {
  static final String SUITE =
      "com.carrotsearch.randomizedtesting.snippets.TheGoodBadAndUglySnippet$TheGoodBadAndUgly";
  static final String SEED = "DEADBEEF";

  static String firstLine(String s) {
    if (s == null) return "<null>";
    int nl = s.indexOf('\n');
    return (nl < 0 ? s : s.substring(0, nl)).replace("\\", "\\\\").replace("\t", "\\t");
  }

  static void rows(List<String> out, String scenario, int run, int fail, int ignored,
      int assumptions, List<Failure> failures) {
    out.add("C\t" + scenario + "\t" + run + "\t" + fail + "\t" + ignored + "\t" + assumptions);
    for (Failure f : failures) {
      Throwable t = f.getException();
      out.add("F\t" + scenario + "\t" + firstLine(f.getDescription().getDisplayName()) + "\t"
          + t.getClass().getName() + "\t" + firstLine(t.getMessage()));
    }
  }

  public static void main(String[] args) throws Exception {
    Path out = Paths.get(args[0]);
    if (args.length < 2) {
      System.err.println("SnippetsOracle: no RANDOMIZEDTESTING_SRC; self_tests_snippets.tsv kept as is");
      return;
    }
    Class<?> suite = Class.forName(SUITE);
    Class<?> nested = Class.forName("com.carrotsearch.randomizedtesting.WithNestedTestClass");
    List<String> lines = new ArrayList<>();

    final List<Failure> failures = new ArrayList<>();
    final int[] assumptions = {0};
    JUnitCore core = new JUnitCore();
    core.addListener(new RunListener() {
      @Override
      public void testFailure(Failure f) {
        failures.add(f);
      }

      @Override
      public void testAssumptionFailure(Failure f) {
        assumptions[0]++;
      }
    });
    Result r = core.run(suite);
    rows(lines, "direct", r.getRunCount(), r.getFailureCount(), r.getIgnoreCount(), assumptions[0],
        failures);

    System.setProperty("tests.seed", SEED);
    nested.getMethod("setupNested").invoke(null);
    Object full;
    try {
      Method runTests = nested.getMethod("runTests", Class[].class);
      full = runTests.invoke(null, (Object) new Class<?>[] {suite});
    } finally {
      nested.getMethod("clearNested").invoke(null);
      System.clearProperty("tests.seed");
    }
    Class<?> fr = full.getClass();
    @SuppressWarnings("unchecked")
    List<Failure> nestedFailures = (List<Failure>) fr.getMethod("getFailures").invoke(full);
    rows(lines, "nested " + SEED,
        (Integer) fr.getMethod("getRunCount").invoke(full),
        (Integer) fr.getMethod("getFailureCount").invoke(full),
        (Integer) fr.getMethod("getIgnoreCount").invoke(full),
        (Integer) fr.getMethod("getAssumptionIgnored").invoke(full),
        nestedFailures);

    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("self_tests_snippets.tsv")))) {
      w.println("# C<TAB>scenario<TAB>run<TAB>fail<TAB>ignored<TAB>assumption failures");
      w.println("# F<TAB>scenario<TAB>test<TAB>exception class<TAB>first line of the message");
      for (String l : lines) w.println(l);
    }
    System.exit(0);
  }
}
