// Oracle of task #338 (rucene-test-framework/tests/lucene_test_case_oracle.rs):
// LuceneTestCase's static environment under tests.* property sets (one child
// JVM per set, as the constants are computed once per JVM), its random
// helpers under RandomizedRunner for fixed seeds, and its deterministic
// statics. Writes <out>/rucene-test-framework/ltc_env.tsv, ltc_draws.tsv,
// ltc_statics.tsv and ltc_locales.tsv, and the JDK data list
// <out>/rucene-test-framework/available_time_zone_ids.txt.
package org.apache.lucene.tests.util;

import com.carrotsearch.randomizedtesting.OracleSupport;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import org.apache.lucene.util.BytesRef;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.notification.Failure;
import static org.hamcrest.CoreMatchers.equalTo;
import static org.hamcrest.CoreMatchers.is;

public class LuceneTestCaseOracle {
  static final String[][] CONFIGS = {
    {},
    {"tests.verbose=true"},
    {"tests.verbose=true", "tests.infostream=false"},
    {"tests.infostream=true"},
    {"tests.nightly=true"},
    {"tests.nightly=true", "tests.multiplier=5"},
    {"tests.multiplier=3"},
    {"tests.codec=SimpleText", "tests.postingsformat=Direct", "tests.docvaluesformat=Asserting", "tests.directory=NIOFSDirectory", "tests.linedocsfile=x.txt"},
    {"tests.weekly=true", "tests.monster=yes", "tests.awaitsfix=on"},
    {"tests.leavetmpdir=true"},
    {"tests.leaveTemporary=true"},
    {"tests.leavetemporary=true"},
    {"tests.asserts=false"},
    {"tests.verbose=bogus"},
    {"tests.multiplier=abc"},
    {"tests.nightly= TRUE "},
  };

  /** The child JVM: prints the constants of one configuration. */
  public static class EnvChild {
    public static void main(String[] args) {
      try {
        System.out.println("VERBOSE\t" + LuceneTestCase.VERBOSE);
        System.out.println("INFOSTREAM\t" + LuceneTestCase.INFOSTREAM);
        System.out.println("TEST_ASSERTS_ENABLED\t" + LuceneTestCase.TEST_ASSERTS_ENABLED);
        System.out.println("TEST_CODEC\t" + LuceneTestCase.TEST_CODEC);
        System.out.println("TEST_POSTINGSFORMAT\t" + LuceneTestCase.TEST_POSTINGSFORMAT);
        System.out.println("TEST_DOCVALUESFORMAT\t" + LuceneTestCase.TEST_DOCVALUESFORMAT);
        System.out.println("TEST_DIRECTORY\t" + LuceneTestCase.TEST_DIRECTORY);
        System.out.println("TEST_LINE_DOCS_FILE\t" + LuceneTestCase.TEST_LINE_DOCS_FILE);
        System.out.println("TEST_NIGHTLY\t" + LuceneTestCase.TEST_NIGHTLY);
        System.out.println("TEST_WEEKLY\t" + LuceneTestCase.TEST_WEEKLY);
        System.out.println("TEST_MONSTER\t" + LuceneTestCase.TEST_MONSTER);
        System.out.println("TEST_AWAITSFIX\t" + LuceneTestCase.TEST_AWAITSFIX);
        System.out.println("TEST_THROTTLING\t" + LuceneTestCase.TEST_THROTTLING);
        System.out.println("RANDOM_MULTIPLIER\t" + LuceneTestCase.RANDOM_MULTIPLIER);
        System.out.println("LEAVE_TEMPORARY\t" + LuceneTestCase.LEAVE_TEMPORARY);
        System.out.println("defaultRandomMultiplier\t" + LuceneTestCase.defaultRandomMultiplier());
      } catch (ExceptionInInitializerError t) {
        // The exception the static initializer threw.
        System.out.println("error\t" + OracleSupport.err(t.getCause()));
      }
    }
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    try (PrintWriter w = new PrintWriter(new File(out, "available_time_zone_ids.txt"), "UTF-8")) {
      for (String id : TimeZone.getAvailableIDs()) w.print(id + "\n");
    }
    env(out);
    statics(out);
    locales(out);
    draws(out);
  }

  static void env(File out) throws Exception {
    String java = ProcessHandle.current().info().command().orElse("java");
    String cp = System.getProperty("java.class.path");
    try (PrintWriter w = new PrintWriter(new File(out, "ltc_env.tsv"), "UTF-8")) {
      w.println("# config\tconstant\tvalue (a child JVM per tests.* property set)");
      w.println("-\tSYSPROP_NIGHTLY\t" + LuceneTestCase.SYSPROP_NIGHTLY);
      w.println("-\tSYSPROP_WEEKLY\t" + LuceneTestCase.SYSPROP_WEEKLY);
      w.println("-\tSYSPROP_MONSTER\t" + LuceneTestCase.SYSPROP_MONSTER);
      w.println("-\tSYSPROP_AWAITSFIX\t" + LuceneTestCase.SYSPROP_AWAITSFIX);
      w.println("-\tSYSPROP_MAXFAILURES\t" + LuceneTestCase.SYSPROP_MAXFAILURES);
      w.println("-\tSYSPROP_FAILFAST\t" + LuceneTestCase.SYSPROP_FAILFAST);
      w.println("-\tDEFAULT_LINE_DOCS_FILE\t" + LuceneTestCase.DEFAULT_LINE_DOCS_FILE);
      w.println("-\tJENKINS_LARGE_LINE_DOCS_FILE\t" + LuceneTestCase.JENKINS_LARGE_LINE_DOCS_FILE);
      for (String[] config : CONFIGS) {
        List<String> cmd = new ArrayList<>(List.of(java, "-ea"));
        for (String kv : config) cmd.add("-D" + kv);
        cmd.addAll(List.of("-cp", cp, EnvChild.class.getName()));
        Process p = new ProcessBuilder(cmd).redirectErrorStream(false).start();
        String label = config.length == 0 ? "{}" : String.join(";", config);
        try (BufferedReader r = new BufferedReader(new InputStreamReader(p.getInputStream(), "UTF-8"))) {
          String line;
          while ((line = r.readLine()) != null) w.println(label + "\t" + line);
        }
        p.waitFor();
      }
    }
  }

  interface Op { Object run() throws Throwable; }

  static void row(PrintWriter w, String label, Op op) {
    try {
      Object r = op.run();
      w.println(label + "\tOK\t" + String.valueOf(r).replace("\n", "\\n"));
    } catch (Throwable t) {
      String c = t.getCause() == null ? "" : " <- " + t.getCause();
      w.println(label + "\t" + OracleSupport.err(t) + c.replace("\n", "\\n"));
    }
  }

  static void statics(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "ltc_statics.tsv"), "UTF-8")) {
      w.println("# label\tOK\tresult | label\tclass\tmessage[ <- cause]");
      float[] fs = {0f, -0f, 1f, Float.MIN_VALUE, -Float.MIN_VALUE, Float.NaN, Float.POSITIVE_INFINITY, Float.MAX_VALUE, 1.0000001f, 0.1f};
      short[] fu = {0, 1, 2, 100};
      for (float x : fs) for (float y : fs) for (short u : fu) {
        row(w, "assertFloatUlpEquals " + Integer.toHexString(Float.floatToRawIntBits(x)) + " " + Integer.toHexString(Float.floatToRawIntBits(y)) + " " + u, () -> { LuceneTestCase.assertFloatUlpEquals(x, y, u); return null; });
      }
      double[] ds = {0d, -0d, 1d, Double.MIN_VALUE, -Double.MIN_VALUE, Double.NaN, Double.NEGATIVE_INFINITY, 1.0000000000000002, 0.1, 1e300};
      int[] du = {0, 1, 2, Integer.MAX_VALUE};
      for (double x : ds) for (double y : ds) for (int u : du) {
        row(w, "assertDoubleUlpEquals " + Long.toHexString(Double.doubleToRawLongBits(x)) + " " + Long.toHexString(Double.doubleToRawLongBits(y)) + " " + u, () -> { LuceneTestCase.assertDoubleUlpEquals(x, y, u); return null; });
      }
      row(w, "expectThrows match", () -> LuceneTestCase.expectThrows(IllegalStateException.class, () -> { throw new IllegalStateException("boom"); }));
      row(w, "expectThrows subclass", () -> LuceneTestCase.expectThrows(RuntimeException.class, () -> { throw new IllegalArgumentException("sub"); }));
      row(w, "expectThrows none", () -> LuceneTestCase.expectThrows(IllegalStateException.class, () -> {}));
      row(w, "expectThrows wrong", () -> LuceneTestCase.expectThrows(IllegalStateException.class, () -> { throw new IOException("io"); }));
      row(w, "expectThrows assertion", () -> LuceneTestCase.expectThrows(IllegalStateException.class, () -> { throw new AssertionError("ae"); }));
      row(w, "expectThrows assertion expected", () -> LuceneTestCase.expectThrows(AssertionError.class, () -> { throw new AssertionError("ae"); }));
      row(w, "expectThrows msg none", () -> LuceneTestCase.expectThrows(IOException.class, "custom", () -> {}));
      row(w, "expectThrowsAnyOf match", () -> LuceneTestCase.expectThrowsAnyOf(List.of(IOException.class, IllegalStateException.class), () -> { throw new IllegalStateException("x"); }));
      row(w, "expectThrowsAnyOf none", () -> LuceneTestCase.expectThrowsAnyOf(List.of(IOException.class, IllegalStateException.class), () -> {}));
      row(w, "expectThrowsAnyOf wrong", () -> LuceneTestCase.expectThrowsAnyOf(List.of(IOException.class, IllegalStateException.class), () -> { throw new IllegalArgumentException("y"); }));
      row(w, "expectThrowsAnyOf empty", () -> LuceneTestCase.expectThrowsAnyOf(List.<Class<? extends Throwable>>of(), () -> {}));
      row(w, "expectThrows wrapped match", () -> LuceneTestCase.expectThrows(RuntimeException.class, IOException.class, () -> { throw new RuntimeException("outer", new IOException("inner")); }));
      row(w, "expectThrows wrapped wrong cause", () -> LuceneTestCase.expectThrows(RuntimeException.class, IOException.class, () -> { throw new RuntimeException("outer", new IllegalStateException("inner")); }));
      row(w, "expectThrows wrapped no cause", () -> LuceneTestCase.expectThrows(RuntimeException.class, IOException.class, () -> { throw new RuntimeException("outer"); }));
      row(w, "expectThrows wrapped wrong outer", () -> LuceneTestCase.expectThrows(IllegalStateException.class, IOException.class, () -> { throw new RuntimeException("outer"); }));
      row(w, "expectThrows wrapped none", () -> LuceneTestCase.expectThrows(RuntimeException.class, IOException.class, () -> {}));
      LinkedHashMap<Class<? extends Throwable>, List<Class<? extends Throwable>>> map = new LinkedHashMap<>();
      map.put(IllegalStateException.class, List.of());
      map.put(RuntimeException.class, List.of(IOException.class, IllegalArgumentException.class));
      row(w, "expectThrowsAnyOf wrapped empty list", () -> LuceneTestCase.expectThrowsAnyOf(map, () -> { throw new IllegalStateException("s"); }));
      row(w, "expectThrowsAnyOf wrapped match", () -> LuceneTestCase.expectThrowsAnyOf(map, () -> { throw new RuntimeException("r", new IOException("i")); }));
      row(w, "expectThrowsAnyOf wrapped wrong cause", () -> LuceneTestCase.expectThrowsAnyOf(map, () -> { throw new RuntimeException("r", new IllegalStateException("i")); }));
      row(w, "expectThrowsAnyOf wrapped wrong outer", () -> LuceneTestCase.expectThrowsAnyOf(map, () -> { throw new Exception("e"); }));
      row(w, "expectThrowsAnyOf wrapped none", () -> LuceneTestCase.expectThrowsAnyOf(map, () -> {}));
      row(w, "assertThat ok", () -> { LuceneTestCase.assertThat(5, equalTo(5)); return null; });
      row(w, "assertThat int", () -> { LuceneTestCase.assertThat(5, equalTo(6)); return null; });
      row(w, "assertThat is string", () -> { LuceneTestCase.assertThat("a\"b", is(equalTo("c\td"))); return null; });
      row(w, "assertThat reason long", () -> { LuceneTestCase.assertThat("why", 5L, is(equalTo(6L))); return null; });
      row(w, "assertThat float", () -> { LuceneTestCase.assertThat(1.5f, equalTo(2f)); return null; });
      row(w, "assertThat double", () -> { LuceneTestCase.assertThat(1.5d, equalTo(1e-10)); return null; });
      row(w, "assertThat bool", () -> { LuceneTestCase.assertThat(true, is(equalTo(false))); return null; });
      row(w, "dumpArray", () -> { ByteArrayOutputStream b = new ByteArrayOutputStream(); LuceneTestCase.dumpArray("lbl", new Object[] {1, "two", 3.0}, new PrintStream(b, true, "UTF-8")); return b.toString("UTF-8"); });
      row(w, "dumpArray null", () -> { ByteArrayOutputStream b = new ByteArrayOutputStream(); LuceneTestCase.dumpArray("lbl", null, new PrintStream(b, true, "UTF-8")); return b.toString("UTF-8"); });
      row(w, "dumpIterator empty", () -> { ByteArrayOutputStream b = new ByteArrayOutputStream(); LuceneTestCase.dumpIterator("e", Collections.emptyIterator(), new PrintStream(b, true, "UTF-8")); return b.toString("UTF-8"); });
      row(w, "asSet", () -> new TreeSet<>(LuceneTestCase.asSet(3, 1, 3, 2)));
      row(w, "assumeTrue", () -> { LuceneTestCase.assumeTrue("msg", false); return null; });
      row(w, "assumeFalse", () -> { LuceneTestCase.assumeFalse("msg", true); return null; });
      row(w, "assumeNoException", () -> { LuceneTestCase.assumeNoException("msg", new IOException("x")); return null; });
      row(w, "TestUtil.nextInt range", () -> { Random r = new Random(1); int[] v = new int[8]; for (int i = 0; i < 8; i++) v[i] = TestUtil.nextInt(r, -3, 9); return Arrays.toString(v); });
      row(w, "TestUtil.nextInt bad", () -> TestUtil.nextInt(new Random(1), 5, 3));
      Path tmp = Files.createTempFile("fork", ".args");
      Files.write(tmp, "-Da=1\n-Xmx1g\r\n-ea".getBytes("UTF-8"));
      row(w, "getJvmForkArguments unset", () -> LuceneTestCase.getJvmForkArguments());
      System.setProperty("tests.jvmForkArgsFile", tmp.toString());
      row(w, "getJvmForkArguments file", () -> LuceneTestCase.getJvmForkArguments());
      System.setProperty("tests.jvmForkArgsFile", tmp.toString() + ".missing");
      row(w, "getJvmForkArguments missing", () -> LuceneTestCase.getJvmForkArguments());
      System.clearProperty("tests.jvmForkArgsFile");
      Files.delete(tmp);
      row(w, "getTestClass outside", () -> LuceneTestCase.getTestClass());
    }
  }

  static void locales(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "ltc_locales.tsv"), "UTF-8")) {
      w.println("# tag (UTF-16 units)\tOK\ttoString|toLanguageTag | tag\tclass\tmessage");
      List<String> tags = new ArrayList<>();
      for (Locale l : Locale.getAvailableLocales()) tags.add(l.toLanguageTag());
      Collections.sort(tags);
      tags.addAll(List.of("", "e", "en-", "en--US", "12", "en-US-a", "en-x", "toolonglang", "en-U$", "und", "i-ami", "no-nyn", "iw", "ji", "in", "x-foo", "en-a-bb-x-lvariant-cc", "zh-cmn-Hans", "art-lojban", "cel-gaulish", "en-GB-oed", "i-default", "i-enochian", "zh-min", "zh-min-nan", "sgn-BE-FR", "EN-us", "en-latn-us-a-xx-b-yy", "de-u-co-phonebk-ka-shifted", "de-u-attr-co-phonebk", "en-x-lvariant-A-b", "no-NO-x-lvariant-NY", "en-US-1994-x-lvariant-abc", "und-x-i-enochian", "en-u-ca", "en-a-b-c", "ja-JP-x-lvariant-JP", "th-TH-x-lvariant-TH", "en-a-xx-a-yy", "en-1234", "en-12345678", "zh-yue-HK", "sr-Latn-RS-u-nu-latn"));
      for (String t : tags) {
        row(w, OracleSupport.units(t), () -> { Locale l = LuceneTestCase.localeForLanguageTag(t); return l + "|" + l.toLanguageTag(); });
      }
    }
  }

  // ---------------------------------------------------------------------------
  // Random helpers under RandomizedRunner.
  // ---------------------------------------------------------------------------

  static final List<String> DRAWS = new ArrayList<>();

  static void draws(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "ltc_draws.tsv"), "UTF-8")) {
      w.println("# seed\tdraws of TestLtcDraws.testDraws under RandomizedRunner (tests.seed=seed)");
      for (String seed : new String[] {"DEADBEEF", "0", "1", "CAFEBABE", "123456789ABCDEF", "7FFFFFFFFFFFFFFF", "1BADC0DE", "42"}) {
        DRAWS.clear();
        System.setProperty("tests.seed", seed);
        Result r = new JUnitCore().run(TestLtcDraws.class);
        for (Failure f : r.getFailures()) w.println("# failure " + f.toString().replace("\n", "\\n"));
        for (String d : DRAWS) w.println(seed + "\t" + d);
      }
      System.clearProperty("tests.seed");
    }
  }
}
