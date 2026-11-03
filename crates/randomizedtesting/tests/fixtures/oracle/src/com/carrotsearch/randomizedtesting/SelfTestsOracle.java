// Oracle for the port of randomizedtesting 2.8.1's own test suite
// (randomized-runner/src/test, task #330; Rust: tests/self_tests/).
//
// Runs every test class of that suite under JUnit 4.13.1, as Maven's
// surefire would run it, and writes self_tests.tsv: per class the run,
// failure, ignored and assumption-failure counts, and per failure its test,
// exception class and message. The Rust target runs the ported classes and
// compares.
//
// The test classes must be on the class path; regenerate.sh compiles them
// from $RANDOMIZEDTESTING_SRC (a checkout of tag release/2.8.1) and passes
// that directory as the second argument. A test class is a concrete
// top-level class with a @Test method in its hierarchy, or one annotated
// @RunWith (what JUnit can run); support classes (Utils, WithNestedTestClass,
// PrintEventListener, the snippet) and the abstract base are not.
package com.carrotsearch.randomizedtesting;

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Collectors;
import java.util.stream.Stream;

import org.junit.Test;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.RunWith;
import org.junit.runner.notification.Failure;
import org.junit.runner.notification.RunListener;

public class SelfTestsOracle {
  static boolean isTestClass(Class<?> c) {
    if (Modifier.isAbstract(c.getModifiers())) return false;
    if (c.isAnnotationPresent(RunWith.class)) return true;
    for (Class<?> k = c; k != null; k = k.getSuperclass()) {
      for (Method m : k.getDeclaredMethods()) {
        if (m.isAnnotationPresent(Test.class)) return true;
      }
    }
    return false;
  }

  static String esc(String s) {
    return s == null ? "<null>" : s.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t");
  }

  public static void main(String[] args) throws Exception {
    Path out = Paths.get(args[0]);
    if (args.length < 2) {
      System.err.println("SelfTestsOracle: no RANDOMIZEDTESTING_SRC; self_tests.tsv kept as is");
      return;
    }
    Path src = Paths.get(args[1], "randomized-runner", "src", "test", "java");
    List<String> names;
    try (Stream<Path> s = Files.walk(src)) {
      names = s.filter(p -> p.toString().endsWith(".java"))
          .map(p -> src.relativize(p).toString().replace('/', '.').replaceAll("\\.java$", ""))
          .sorted()
          .collect(Collectors.toList());
    }
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("self_tests.tsv")))) {
      w.println("# C<TAB>class<TAB>run<TAB>fail<TAB>ignored<TAB>assumption failures");
      w.println("# F<TAB>class<TAB>test<TAB>exception class<TAB>message (\\\\, \\n, \\t escaped)");
      for (String name : names) {
        Class<?> c = Class.forName(name);
        if (!isTestClass(c)) continue;
        final List<String> failures = new ArrayList<>();
        final int[] assumptions = {0};
        JUnitCore core = new JUnitCore();
        core.addListener(new RunListener() {
          @Override
          public void testFailure(Failure f) {
            Throwable t = f.getException();
            failures.add("F\t" + name + "\t" + esc(f.getDescription().getDisplayName()) + "\t"
                + t.getClass().getName() + "\t" + esc(t.getMessage()));
          }

          @Override
          public void testAssumptionFailure(Failure f) {
            assumptions[0]++;
          }
        });
        Result r = core.run(c);
        w.println("C\t" + name + "\t" + r.getRunCount() + "\t" + r.getFailureCount() + "\t"
            + r.getIgnoreCount() + "\t" + assumptions[0]);
        for (String f : failures) w.println(f);
      }
    }
    System.exit(0);
  }
}
