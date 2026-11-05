// Oracle of task #348 and #226 (rucene-test-framework/tests/class_env_oracle.rs):
// the class environment TestRuleSetupAndRestoreClassEnv sets up, the order of
// a suite's rules and hooks around LuceneTestCase's chains, the reproduce line
// and test params RunListenerPrintReproduceInfo prints, the effect of
// @SuppressCodecs, for the OracleSupport.seeds() grid and under tests.*
// configurations (one child JVM per configuration, as the constants are
// computed once per JVM); and StringHelper.randomId() for fixed tests.seed
// values (one child JVM each). Writes <out>/rucene-test-framework/class_env.tsv
// and string_helper_ids.tsv.
package org.apache.lucene.tests.util;

import com.carrotsearch.randomizedtesting.OracleSupport;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.util.InfoStream;
import org.apache.lucene.util.StringHelper;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;

public class ClassEnvOracle {
  /** The tests.* configurations, each run in its own JVM. */
  static final String[][] CONFIGS = {
    {},
    {"tests.codec=Asserting"},
    {"tests.codec=SimpleText"},
    {"tests.codec=CheapBastard"},
    {"tests.codec=Compressing"},
    {"tests.codec=Lucene104"},
    {"tests.codec=default"},
    {"tests.postingsformat=Direct"},
    {"tests.postingsformat=MockRandom"},
    {"tests.docvaluesformat=Asserting"},
    {"tests.locale=fr-CA", "tests.timezone=America/Toronto"},
    {"tests.timezone=Bogus/Zone"},
    {"tests.failfast=true"},
    {"tests.maxfailures=2", "tests.failfast=true"},
    {"tests.multiplier=3", "tests.nightly=true"},
  };

  static final String[] ID_SEEDS = {"DEADBEEF", "0123456789ABCDEF", "1", "-5", "FFFFFFFF", "zz", "7FFFFFFFFFFFFFFF", ""};

  static ByteArrayOutputStream err;

  /** The child JVM of one configuration: prints the rows. */
  public static class Child {
    public static void main(String[] args) throws Exception {
      // Captured before LuceneTestCase initializes, so that TestRuleLimitSysouts wraps it.
      PrintStream realOut = System.out;
      err = new ByteArrayOutputStream();
      System.setErr(new PrintStream(err, true, StandardCharsets.UTF_8));
      List<String> rows = new ArrayList<>();
      long[] seeds = OracleSupport.seeds();
      for (int i = 0; i < seeds.length; i++) {
        String seed = Long.toHexString(seeds[i]).toUpperCase(Locale.ROOT);
        System.setProperty("tests.seed", seed);
        run(rows, seed, "probe", TestClassEnvProbe.class);
        if (i < 4) {
          run(rows, seed, "failing", TestClassEnvProbe.TestClassEnvFailing.class);
        }
        run(rows, seed, "suppressed", TestClassEnvProbe.TestClassEnvSuppressed.class);
      }
      // A wider grid of the probe alone, so that every codec branch is drawn.
      SplittableRandom more = new SplittableRandom(348L);
      for (int i = 0; i < 60; i++) {
        String seed = Long.toHexString(more.nextLong()).toUpperCase(Locale.ROOT);
        System.setProperty("tests.seed", seed);
        run(rows, seed, "probe", TestClassEnvProbe.class);
      }
      System.clearProperty("tests.seed");
      for (String r : rows) realOut.println(r);
      realOut.flush();
    }

    static void run(List<String> rows, String seed, String kind, Class<?> suite) {
      TestClassEnvProbe.EVENTS.clear();
      err.reset();
      Result r = new JUnitCore().run(suite);
      String p = seed + "\t" + kind + "\t";
      rows.add(p + "result\trun=" + r.getRunCount() + "\tfailures=" + r.getFailureCount() + "\tignored=" + r.getIgnoreCount() + "\tassumptions=" + r.getAssumptionFailureCount());
      for (org.junit.runner.notification.Failure f : r.getFailures()) {
        rows.add(p + "failure\t" + f.getException().getClass().getName() + "\t" + String.valueOf(f.getMessage()).replace("\n", "\\n"));
      }
      for (String e : TestClassEnvProbe.EVENTS) rows.add(p + "event\t" + e);
      for (String line : err.toString(StandardCharsets.UTF_8).split("\n")) {
        if (line.startsWith("NOTE: reproduce with:") || line.startsWith("NOTE: test params are:")
            || line.startsWith("NOTE: Class not allowed") || line.startsWith("Property '")) {
          rows.add(p + "stderr\t" + line);
        }
      }
      rows.add(p + "restored\tcodec=" + Codec.getDefault().getName() + "\tinfo=" + InfoStream.getDefault().getClass().getName());
    }
  }

  /** The child JVM of one tests.seed value: three random ids. */
  public static class IdChild {
    public static void main(String[] args) {
      try {
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < 3; i++) {
          if (i > 0) b.append('\t');
          b.append(StringHelper.idToString(StringHelper.randomId()));
        }
        System.out.println(b);
      } catch (ExceptionInInitializerError t) {
        System.out.println("error\t" + OracleSupport.err(t.getCause()));
      }
    }
  }

  static List<String> child(String main, String[] props) throws Exception {
    String java = ProcessHandle.current().info().command().orElse("java");
    List<String> cmd = new ArrayList<>();
    cmd.add(java);
    cmd.add("-ea");
    for (String p : props) cmd.add("-D" + p);
    cmd.add("-cp");
    cmd.add(System.getProperty("java.class.path"));
    cmd.add(main);
    Process p = new ProcessBuilder(cmd).redirectError(ProcessBuilder.Redirect.DISCARD).start();
    List<String> lines = new ArrayList<>();
    try (BufferedReader r = new BufferedReader(new InputStreamReader(p.getInputStream(), StandardCharsets.UTF_8))) {
      String l;
      while ((l = r.readLine()) != null) lines.add(l);
    }
    p.waitFor();
    return lines;
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    try (PrintWriter w = new PrintWriter(new File(out, "class_env.tsv"), "UTF-8")) {
      w.println("# config\tseed\tsuite\tkind\tvalues... (ClassEnvOracle.Child per configuration)");
      for (String[] config : CONFIGS) {
        String name = config.length == 0 ? "-" : String.join(" ", config);
        for (String line : child(Child.class.getName(), config)) w.println(name + "\t" + line);
      }
    }
    try (PrintWriter w = new PrintWriter(new File(out, "string_helper_ids.tsv"), "UTF-8")) {
      w.println("# tests.seed\tStringHelper.randomId() x3, or error class<TAB>message");
      for (String seed : ID_SEEDS) {
        for (String line : child(IdChild.class.getName(), new String[] {"tests.seed=" + seed})) {
          w.println((seed.isEmpty() ? "(empty)" : seed) + "\t" + line);
        }
      }
    }
  }
}
