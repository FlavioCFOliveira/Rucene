// Oracle of the runner (RandomizedRunner, ThreadLeakControl, rules): suites
// run under the real RandomizedRunner 2.8.1 by JUnitCore; every listener
// event, hook and rule call, seed, draw, failure (class, message, seed
// chain), reproduce line and runner log record is printed to
// runner_traces.tsv, one "=== scenario" block per run. The Rust test target
// tests/runner_oracle.rs declares the same suites and must print the same
// trace (after masking thread ids, identity hashes, thread states and stack
// frames).
package com.carrotsearch.randomizedtesting;

import java.io.PrintWriter;
import java.lang.annotation.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.logging.*;

import org.junit.*;
import org.junit.rules.RuleChain;
import org.junit.rules.TestRule;
import org.junit.runner.*;
import org.junit.runner.notification.*;
import org.junit.runners.model.Statement;

import com.carrotsearch.randomizedtesting.annotations.*;
import com.carrotsearch.randomizedtesting.annotations.ThreadLeakAction.Action;
import com.carrotsearch.randomizedtesting.annotations.ThreadLeakScope.Scope;
import com.carrotsearch.randomizedtesting.listeners.ReproduceInfoPrinter;
import com.carrotsearch.randomizedtesting.rules.*;

public class RunnerOracle {
  static final List<String> out = Collections.synchronizedList(new ArrayList<>());

  public static void log(String s) { out.add(s); }

  static String draw() {
    Random r = RandomizedContext.current().getRandom();
    return "runner=" + RandomizedContext.current().getRunnerSeedAsString() + " int=" + r.nextInt() + " long=" + r.nextLong();
  }

  static String thr(Throwable t) {
    String seed = RandomizedRunner.seedFromThrowable(t);
    StringBuilder b = new StringBuilder(t.toString().replace("\n", "\\n"));
    if (seed != null) b.append(" [seed ").append(seed).append("]");
    Throwable c = t.getCause();
    while (c != null) { b.append(" <- ").append(c.toString().replace("\n", "\\n")); c = c.getCause(); }
    return b.toString();
  }

  static void tree(Description d, String indent) {
    log("desc " + indent + d.getDisplayName());
    for (Description c : d.getChildren()) tree(c, indent + "  ");
  }

  public static class Trace extends RunListener {
    @Override public void testRunStarted(Description d) { log("runStarted tests=" + d.testCount()); tree(d, ""); }
    @Override public void testRunFinished(Result r) {
      log("runFinished run=" + r.getRunCount() + " fail=" + r.getFailureCount() + " ignore=" + r.getIgnoreCount()
          + " assumption=" + r.getAssumptionFailureCount());
    }
    @Override public void testStarted(Description d) { log("started " + d.getDisplayName()); }
    @Override public void testFinished(Description d) { log("finished " + d.getDisplayName()); }
    @Override public void testIgnored(Description d) { log("ignored " + d.getDisplayName()); }
    @Override public void testAssumptionFailure(Failure f) { log("assumption " + f.getDescription().getDisplayName() + " | " + thr(f.getException())); }
    @Override public void testFailure(Failure f) {
      log("failure " + f.getDescription().getDisplayName() + " | " + thr(f.getException()));
      StringBuilder b = new StringBuilder();
      new ReproduceErrorMessageBuilder(b).appendAllOpts(f.getDescription());
      log("reproduce " + b);
    }
  }

  static void scenario(Path dir, PrintWriter w, String name, Class<?> c, String... props) throws Exception {
    out.clear();
    Map<String, String> saved = new LinkedHashMap<>();
    for (int i = 0; i < props.length; i += 2) {
      saved.put(props[i], System.getProperty(props[i]));
      System.setProperty(props[i], props[i + 1]);
    }
    try {
      JUnitCore core = new JUnitCore();
      core.addListener(new Trace());
      core.run(Request.aClass(c));
    } finally {
      for (Map.Entry<String, String> e : saved.entrySet()) {
        if (e.getValue() == null) System.clearProperty(e.getKey()); else System.setProperty(e.getKey(), e.getValue());
      }
    }
    w.println("=== " + name);
    synchronized (out) { for (String s : out) w.println(s); }
  }

  // ---------------------------------------------------------------------------
  // Hook, rule and seed order.
  // ---------------------------------------------------------------------------

  public static class LoggingRule implements TestRule {
    final String name;
    public LoggingRule(String name) { this.name = name; }
    public Statement apply(final Statement base, final Description d) {
      return new Statement() {
        public void evaluate() throws Throwable {
          log("rule " + name + " before " + d.getDisplayName());
          try { base.evaluate(); } finally { log("rule " + name + " after"); }
        }
      };
    }
  }

  @RunWith(RandomizedRunner.class)
  public static class HookSuper {
    @ClassRule public static TestRule superClassRule = new LoggingRule("superClassRule");
    @Rule public TestRule superRule = new LoggingRule("superRule");
    @BeforeClass public static void beforeClassSuper() { log("beforeClassSuper"); }
    @AfterClass public static void afterClassSuper() { log("afterClassSuper"); }
    @Before public void beforeSuper() { log("beforeSuper"); }
    @After public void afterSuper() { log("afterSuper"); }
  }

  public static class HookSub extends HookSuper {
    @ClassRule public static TestRule classRuleA = new LoggingRule("classRuleA");
    @ClassRule public static TestRule classRuleB = new LoggingRule("classRuleB");
    @Rule public TestRule ruleA = new LoggingRule("ruleA");
    @Rule public TestRule ruleB = new LoggingRule("ruleB");
    @Rule public TestRule chain = RuleChain.outerRule(new LoggingRule("outer")).around(new LoggingRule("inner"));
    public HookSub() { log("constructor"); }
    @BeforeClass public static void beforeClass1() { log("beforeClass1 " + draw()); }
    @BeforeClass public static void beforeClass2() { log("beforeClass2"); }
    @BeforeClass private static void beforeClass3() { log("beforeClass3"); }
    @AfterClass public static void afterClass1() { log("afterClass1"); }
    @AfterClass public static void afterClass2() { log("afterClass2"); }
    @Before public void before1() { log("before1"); }
    @Before public void before2() { log("before2"); }
    @After public void after1() { log("after1"); }
    @After public void after2() { log("after2"); }
    @Test public void testA() { log("testA " + draw() + " thread=" + Thread.currentThread().getName()); }
    @Test public void testB() { log("testB " + draw()); }
    @Test public void testC() { log("testC " + draw() + " method=" + RandomizedContext.current().getTargetMethod().getName()); }
    @Test public void testD() { log("testD " + draw()); }
  }

  @RunWith(RandomizedRunner.class)
  @Seed("DEADBEEF")
  public static class Seeds1 {
    @Test public void testPlain() { log("testPlain " + draw()); }
    @Test @Seed("1234") public void testFixed() { log("testFixed " + draw()); }
    @Test @com.carrotsearch.randomizedtesting.annotations.Seeds({@Seed("1"), @Seed("2"), @Seed("random"), @Seed("FFFFFFFFFFFFFFFF")})
    public void testSeeds() { log("testSeeds " + draw()); }
  }

  @RunWith(RandomizedRunner.class)
  @Seed("CAFEBABE:12345678")
  public static class SeedChainOnClass {
    @Test public void testOne() { log("testOne " + draw()); }
    @Test public void testTwo() { log("testTwo " + draw()); }
  }

  @RunWith(RandomizedRunner.class)
  public static class Repeats {
    @Test @Repeat(iterations = 3) public void testRepeat() { log("testRepeat " + draw()); }
    @Test @Repeat(iterations = 2, useConstantSeed = true) public void testConstant() { log("testConstant " + draw()); }
    @Test public void testOnce() { log("testOnce " + draw()); }
  }

  @RunWith(RandomizedRunner.class)
  @Repeat(iterations = 2)
  public static class RepeatsOnClass {
    @Test public void testX() { log("testX " + draw()); }
    @Test @Repeat(iterations = 1) public void testY() { log("testY " + draw()); }
  }

  // ---------------------------------------------------------------------------
  // Failures, assumptions, ignores, expectations.
  // ---------------------------------------------------------------------------

  @RunWith(RandomizedRunner.class)
  public static class Failures {
    @Test public void testAssertion() { log("testAssertion"); Assert.fail("expected failure"); }
    @Test public void testRuntime() { throw new RuntimeException("runtime boom"); }
    @Test public void testAssume() { RandomizedTest.assumeTrue("not this time", false); }
    @Test @Ignore("ignored for now") public void testIgnored() { log("never"); }
    @Test(expected = IllegalArgumentException.class) public void testExpected() { throw new IllegalArgumentException("ok"); }
    @Test(expected = IllegalArgumentException.class) public void testExpectedMissing() { log("testExpectedMissing"); }
    @Test(expected = IllegalArgumentException.class) public void testExpectedWrong() { throw new IllegalStateException("wrong"); }
    @Test public void testPass() { log("testPass"); }
  }

  @RunWith(RandomizedRunner.class)
  public static class HookFailures {
    @Before public void beforeA() { log("beforeA"); throw new RuntimeException("beforeA failed"); }
    @Before public void beforeB() { log("beforeB"); throw new RuntimeException("beforeB failed"); }
    @After public void afterA() { log("afterA"); throw new RuntimeException("afterA failed"); }
    @After public void afterB() { log("afterB"); throw new RuntimeException("afterB failed"); }
    @AfterClass public static void afterClass() { log("afterClass"); throw new RuntimeException("afterClass failed"); }
    @Test public void testOne() { log("testOne"); }
  }

  @RunWith(RandomizedRunner.class)
  public static class ClassAssumption {
    @BeforeClass public static void beforeClass() { log("beforeClass"); RandomizedTest.assumeTrue("class-level", false); }
    @Test public void testOne() { log("never"); }
    @Test public void testTwo() { log("never"); }
  }

  @RunWith(RandomizedRunner.class)
  public static class BeforeClassFailure {
    @BeforeClass public static void beforeClass() { throw new IllegalStateException("beforeClass failed"); }
    @AfterClass public static void afterClass() { log("afterClass"); }
    @Test public void testOne() { log("never"); }
  }

  // ---------------------------------------------------------------------------
  // Groups and filters.
  // ---------------------------------------------------------------------------

  @Documented @Inherited @Retention(RetentionPolicy.RUNTIME) @Target({ElementType.METHOD, ElementType.TYPE})
  @TestGroup(enabled = false)
  public static @interface Slow {}

  @Documented @Inherited @Retention(RetentionPolicy.RUNTIME) @Target({ElementType.METHOD, ElementType.TYPE})
  @TestGroup(name = "fast", sysProperty = "oracle.fast", enabled = true)
  public static @interface Fast {}

  @RunWith(RandomizedRunner.class)
  public static class Groups {
    @Test public void testPlain() { log("testPlain"); }
    @Test @Nightly public void testNightly() { log("testNightly nightly=" + RandomizedTest.isNightly()); }
    @Test @Slow public void testSlow() { log("testSlow"); }
    @Test @Fast public void testFast() { log("testFast"); }
    @Test @Slow @Fast public void testSlowFast() { log("testSlowFast"); }
    @Test public void testOther() { log("testOther"); }
  }

  @RunWith(RandomizedRunner.class)
  @Nightly
  public static class NightlySuite {
    @Test public void testOne() { log("testOne"); }
  }

  // ---------------------------------------------------------------------------
  // Parameterized suites, providers, ordering, listeners.
  // ---------------------------------------------------------------------------

  @RunWith(RandomizedRunner.class)
  public static class Params {
    static int instances;
    final String value; final int number;
    public Params(@Name("value") String value, int number) { this.value = value; this.number = number; instances++; log("constructor " + value + " " + number); }
    @ParametersFactory public static Iterable<Object[]> parameters() {
      return Arrays.asList(new Object[] {"a", 1}, new Object[] {"b", 2}, new Object[] {"c", 3}, new Object[] {"a", 1});
    }
    @Test public void testOne() { log("testOne " + value + " " + number + " " + draw()); }
    @Test @Repeat(iterations = 2) public void testTwo() { log("testTwo " + value + " " + number); }
  }

  @RunWith(RandomizedRunner.class)
  @TestCaseInstanceProvider(TestCaseInstanceProvider.Type.INSTANCE_PER_CONSTRUCTOR_ARGS)
  @TestCaseOrdering(TestCaseOrdering.AlphabeticOrder.class)
  public static class ParamsFormatted {
    final String value; final int number;
    public ParamsFormatted(String value, int number) { this.value = value; this.number = number; log("constructor " + value); }
    @ParametersFactory(argumentFormatting = "%2$s:%1$s", shuffle = false) public static Iterable<Object[]> parameters() {
      return Arrays.asList(new Object[] {"x", 10}, new Object[] {"y", 20});
    }
    @Test public void testZ() { log("testZ " + value); }
    @Test public void testA() { log("testA " + value); }
    @Test public void testM() { log("testM " + value); }
  }

  public static class AutoListener extends RunListener {
    @Override public void testRunStarted(Description d) { log("auto runStarted " + d.getDisplayName()); }
    @Override public void testRunFinished(Result r) { log("auto runFinished run=" + r.getRunCount()); }
    @Override public void testStarted(Description d) { log("auto started " + d.getDisplayName()); }
  }

  @RunWith(RandomizedRunner.class)
  @Listeners({AutoListener.class})
  @TestMethodProviders({JUnit3MethodProvider.class})
  public static class JUnit3Style {
    public void test1() { log("test1"); }
    public void test2() { log("test2"); }
    public void helper() { log("never"); }
  }

  public static class JdkSupplier implements RandomSupplier {
    public Random get(long seed) { return new Random(seed); }
  }

  @RunWith(RandomizedRunner.class)
  @SeedDecorators({MixWithSuiteName.class})
  @TestContextRandomSupplier(JdkSupplier.class)
  public static class Decorated {
    @Test public void testOne() { log("testOne " + draw()); }
    @Test public void testTwo() { log("testTwo " + draw()); }
  }

  public static class ReproducePrinter {
    @RunWith(RandomizedRunner.class)
    @Listeners({ReproduceInfoPrinter.class})
    public static class Suite {
      @Test public void testFails() { throw new RuntimeException("reproduce me"); }
    }
  }

  // ---------------------------------------------------------------------------
  // Validation errors.
  // ---------------------------------------------------------------------------

  @RunWith(RandomizedRunner.class)
  public static class NonPublicTest {
    @Test void testPackage() {}
  }

  @RunWith(RandomizedRunner.class)
  public static class StaticTest {
    @Test public static void testStatic() {}
  }

  @RunWith(RandomizedRunner.class)
  public static class ArgsTest {
    @Test public void testArgs(String s, int i) {}
  }

  @RunWith(RandomizedRunner.class)
  public static class InstanceBeforeClass {
    @BeforeClass public void beforeClass() {}
    @Test public void testOne() {}
  }

  @RunWith(RandomizedRunner.class)
  public static class NoFactory {
    public NoFactory(String s) {}
    @Test public void testOne() {}
  }

  @RunWith(RandomizedRunner.class)
  public static class BadMethodSeed {
    @Test @Seed("1:2") public void testOne() {}
  }

  @RunWith(RandomizedRunner.class)
  public static class Simple {
    @Test public void testOne() { log("testOne " + draw()); }
    @Test public void testTwo() { log("testTwo " + draw()); }
  }

  // ---------------------------------------------------------------------------
  // Thread leaks, timeouts, uncaught exceptions (ThreadLeakControl).
  // ---------------------------------------------------------------------------

  static Thread sleeper(String name, long millis) {
    Thread t = new Thread(() -> {
      try { Thread.sleep(millis); } catch (InterruptedException e) { log("thread " + name + " interrupted"); }
    }, name);
    return t;
  }

  @RunWith(RandomizedRunner.class)
  public static class LeakTest {
    @Test public void testLeak() { sleeper("leaked-test", 10000).start(); }
    @Test public void testAfterLeak() { log("testAfterLeak"); }
  }

  // A thread leaked while it sleeps with RandomizedTest.sleep, which
  // rethrows the InterruptedException of the leak control's interrupt.
  @RunWith(RandomizedRunner.class)
  public static class LeakRandomizedSleep {
    @Test public void testLeak() {
      new Thread(() -> {
        try {
          while (true) RandomizedTest.sleep(1000);
        } catch (Throwable e) {
          log("thread leaked-randomized-sleep ended by " + e);
        }
      }, "leaked-randomized-sleep").start();
    }
  }

  // A class annotated @Ignore: JUnit's IgnoredClassRunner, not RandomizedRunner.
  @RunWith(RandomizedRunner.class)
  @Ignore("whole class")
  public static class IgnoredClass {
    @BeforeClass public static void beforeClass() { log("beforeClass"); }
    @Test public void testOne() { log("testOne"); }
  }

  @RunWith(RandomizedRunner.class)
  @ThreadLeakScope(Scope.SUITE)
  public static class LeakSuite {
    @BeforeClass public static void beforeClass() { sleeper("leaked-suite", 10000).start(); }
    @Test public void testOne() { log("testOne"); }
  }

  @RunWith(RandomizedRunner.class)
  @ThreadLeakScope(Scope.NONE)
  public static class LeakNone {
    static Thread t;
    @Test public void testOne() { t = sleeper("leaked-none", 300); t.start(); log("testOne"); }
    @AfterClass public static void afterClass() throws Exception { t.join(); }
  }

  @RunWith(RandomizedRunner.class)
  @ThreadLeakLingering(linger = 3000)
  public static class LeakLinger {
    @Test public void testOne() { sleeper("lingering", 300).start(); log("testOne"); }
  }

  @RunWith(RandomizedRunner.class)
  @ThreadLeakAction({Action.WARN})
  public static class LeakWarnOnly {
    static Thread t;
    @Test public void testOne() { t = sleeper("warned", 500); t.start(); }
    @AfterClass public static void afterClass() throws Exception { t.join(); }
  }

  @RunWith(RandomizedRunner.class)
  public static class TestTimeout {
    @Test @Timeout(millis = 300) public void testSlow() {
      try { Thread.sleep(10000); } catch (InterruptedException e) { log("testSlow interrupted"); }
    }
    @Test(timeout = 200) public void testJUnitTimeout() {
      try { Thread.sleep(10000); } catch (InterruptedException e) { log("testJUnitTimeout interrupted"); }
    }
    @Test @Timeout(millis = 5000) public void testFast() { log("testFast"); }
  }

  @RunWith(RandomizedRunner.class)
  @TimeoutSuite(millis = 1000)
  public static class SuiteTimeout {
    @Test public void testA() { log("testA"); }
    @Test public void testB() { log("testB"); }
    @Test public void testHangs() {
      try { Thread.sleep(20000); } catch (InterruptedException e) { log("testHangs interrupted"); }
    }
  }

  @RunWith(RandomizedRunner.class)
  public static class Uncaught {
    @Test public void testUncaught() throws Exception {
      Thread t = new Thread(() -> { throw new RuntimeException("boom in thread"); }, "uncaught-thread");
      t.start(); t.join();
      log("testUncaught joined");
    }
    @Test public void testClean() { log("testClean"); }
  }

  @RunWith(RandomizedRunner.class)
  public static class UncaughtSuite {
    @BeforeClass public static void beforeClass() throws Exception {
      Thread t = new Thread(() -> { throw new IllegalStateException("boom in suite thread"); }, "uncaught-suite");
      t.start(); t.join();
    }
    @Test public void testOne() { log("testOne"); }
  }

  public static class RejectAll implements ThreadFilter {
    public boolean reject(Thread t) { return t.getName().startsWith("filtered-"); }
  }

  @RunWith(RandomizedRunner.class)
  @ThreadLeakFilters(filters = {RejectAll.class})
  public static class LeakFiltered {
    static Thread t;
    @Test public void testOne() { t = sleeper("filtered-1", 300); t.start(); log("testOne"); }
    @AfterClass public static void afterClass() throws Exception { t.join(); }
  }

  static volatile boolean releaseZombie;

  @RunWith(RandomizedRunner.class)
  public static class Zombies {
    @Test public void testA() {
      Thread t = new Thread(() -> {
        while (!releaseZombie) {
          try { Thread.sleep(50); } catch (InterruptedException e) { /* ignore */ }
        }
      }, "zombie");
      t.setDaemon(true);
      t.start();
      log("testA");
    }
    @Test public void testB() { log("testB"); }
    @Test public void testC() { log("testC"); }
  }

  @RunWith(RandomizedRunner.class)
  public static class AfterZombies {
    @Test public void testOne() { log("never"); }
  }

  // ---------------------------------------------------------------------------
  // Rules.
  // ---------------------------------------------------------------------------

  @RunWith(RandomizedRunner.class)
  public static class RestoreRule {
    @Rule public TestRule restore = new SystemPropertiesRestoreRule();
    @Rule public TestRule restoreSet = new SystemPropertiesRestoreRule("oracle.set", "during");
    @Test public void testSets() {
      log("testSets oracle.set=" + System.getProperty("oracle.set"));
      System.setProperty("oracle.prop", "changed");
    }
    @Test public void testSees() { log("testSees oracle.prop=" + System.getProperty("oracle.prop") + " oracle.set=" + System.getProperty("oracle.set")); }
  }

  @RunWith(RandomizedRunner.class)
  public static class InvariantRule {
    @Rule public TestRule invariant = new SystemPropertiesInvariantRule("oracle.ignored");
    @Test public void testAdds() { System.setProperty("oracle.added", "x"); }
    @Test public void testChanges() { System.setProperty("oracle.existing", "changed"); }
    @Test public void testRemoves() { System.clearProperty("oracle.existing"); }
    @Test public void testIgnored() { System.setProperty("oracle.ignored", "y"); }
    @Test public void testClean() { log("testClean"); }
  }

  @RunWith(RandomizedRunner.class)
  public static class ShadowSuper {
    @ClassRule public static TestRule noShadow = new NoClassHooksShadowingRule();
    @Rule public TestRule noOverride = new NoInstanceHooksOverridesRule();
    @BeforeClass public static void setup() { log("super setup"); }
    @Before public void init() { log("super init"); }
    @Test public void testOne() { log("testOne"); }
  }

  public static class ShadowSub extends ShadowSuper {
    @BeforeClass public static void setup() { log("sub setup"); }
  }

  public static class OverrideSub extends ShadowSuper {
    @Before @Override public void init() { log("sub init"); }
  }

  // Method.invoke dispatches virtually: the runner invokes the topmost
  // annotated hook, which runs the receiver's most-derived override (an
  // override without the annotation included); a private method is not
  // overridden.
  @RunWith(RandomizedRunner.class)
  public static class VirtualSuper {
    @Before public void setUp() { log("super setUp"); }
    @Before void packageHook() { log("super packageHook"); }
    @Before private void privateHook() { log("super privateHook"); }
    @After public void tearDown() { log("super tearDown"); }
    @Test public void testOne() { log("testOne"); }
  }

  public static class VirtualSub extends VirtualSuper {
    @Override public void setUp() { log("sub setUp"); super.setUp(); }
    @Override void packageHook() { log("sub packageHook"); }
    private void privateHook() { log("sub privateHook"); }
    @Override public void tearDown() { super.tearDown(); log("sub tearDown"); }
  }

  public static class VirtualSubSub extends VirtualSub {
    @Override public void setUp() { log("subsub setUp"); }
  }

  public static class StaticFields {
    @RunWith(RandomizedRunner.class)
    public static class Base {
      @ClassRule public static final TestRule rule = new StaticFieldsInvariantRule(1024 * 1024, true);
      @Test public void testOne() { log("testOne"); }
    }
    public static class Exceeding extends Base {
      static byte[] field0;
      static long[] field1;
      static int[] field2;
      static Object field3;
      static Long field4;
      final static long[] field5 = new long[1024 * 1024];
      @BeforeClass public static void setup() {
        field0 = new byte[600 * 1024];
        field1 = new long[64 * 1024];
        field2 = new int[100];
        field3 = new Object[] {new byte[16], null, 7};
        field4 = 42L;
      }
    }
    public static class Under extends Base {
      static byte[] small;
      @BeforeClass public static void setup() { small = new byte[1000]; }
    }
    public static class Unmeasurable extends Base {
      static String text;
      static int[] ok;
      @BeforeClass public static void setup() { text = "hello"; ok = new int[3]; }
    }
  }

  public static class AdapterRule extends TestRuleAdapter {
    @Override protected void before() { log("adapter before"); }
    @Override protected void afterIfSuccessful() { log("adapter afterIfSuccessful"); }
    @Override protected void afterAlways(List<Throwable> errors) { log("adapter afterAlways errors=" + errors.size()); throw new RuntimeException("afterAlways adds"); }
  }

  @RunWith(RandomizedRunner.class)
  public static class Adapters {
    @Rule public TestRule adapter = new AdapterRule();
    @Test public void testPass() { log("testPass"); }
    @Test public void testFail() { throw new RuntimeException("test fails"); }
  }

  @RunWith(RandomizedRunner.class)
  public static class Assertions {
    @ClassRule public static final TestRule rule = new RequireAssertionsRule(Assertions.class);
    @Test public void testOne() { log("testOne"); }
  }

  public static void main(String[] args) throws Exception {
    Path dir = Paths.get(args[0]);
    // Runner log records, one line each (the first line of the message).
    Logger root = Logger.getLogger("");
    for (Handler h : root.getHandlers()) root.removeHandler(h);
    root.addHandler(new Handler() {
      public void publish(LogRecord r) {
        String m = r.getMessage();
        int nl = m.indexOf('\n');
        log("log " + r.getLevel() + " " + (nl < 0 ? m : m.substring(0, nl)));
      }
      public void flush() {}
      public void close() {}
    });
    boolean assertsOnly = args.length > 1 && args[1].equals("asserts-false");
    String file = assertsOnly ? "runner_traces_asserts.tsv" : "runner_traces.tsv";
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(dir.resolve(file)))) {
      if (assertsOnly) {
        System.setProperty("tests.asserts", "false");
        scenario(dir, w, "require-assertions-mismatch", Assertions.class, "tests.seed", "DEADBEEF");
        return;
      }
      String S = "tests.seed";
      scenario(dir, w, "hooks-deadbeef", HookSub.class, S, "DEADBEEF");
      scenario(dir, w, "hooks-cafebabe", HookSub.class, S, "CAFEBABE");
      scenario(dir, w, "hooks-seed-chain", HookSub.class, S, "DEADBEEF:1234");
      scenario(dir, w, "seeds-annotations", Seeds1.class);
      scenario(dir, w, "seeds-annotations-overridden", Seeds1.class, S, "1");
      scenario(dir, w, "seed-chain-on-class", SeedChainOnClass.class);
      scenario(dir, w, "repeat", Repeats.class, S, "DEADBEEF");
      scenario(dir, w, "repeat-iters-property", Repeats.class, S, "DEADBEEF", "tests.iters", "2");
      scenario(dir, w, "repeat-on-class", RepeatsOnClass.class, S, "DEADBEEF");
      scenario(dir, w, "repeat-append-seed", Simple.class, S, "DEADBEEF", "tests.appendseed", "true");
      scenario(dir, w, "failures", Failures.class, S, "DEADBEEF");
      scenario(dir, w, "hook-failures", HookFailures.class, S, "DEADBEEF");
      scenario(dir, w, "class-assumption", ClassAssumption.class, S, "DEADBEEF");
      scenario(dir, w, "before-class-failure", BeforeClassFailure.class, S, "DEADBEEF");
      scenario(dir, w, "groups-default", Groups.class, S, "DEADBEEF");
      scenario(dir, w, "groups-nightly", Groups.class, S, "DEADBEEF", "tests.nightly", "true");
      scenario(dir, w, "groups-slow-disabled-fast", Groups.class, S, "DEADBEEF", "tests.slow", "true", "oracle.fast", "false");
      scenario(dir, w, "groups-malformed", Groups.class, S, "DEADBEEF", "tests.slow", "maybe");
      scenario(dir, w, "filter-slow-and-not-nightly", Groups.class, S, "DEADBEEF", "tests.filter", "@slow and not @nightly");
      scenario(dir, w, "filter-default-or-nightly", Groups.class, S, "DEADBEEF", "tests.filter", "default or @NIGHTLY");
      scenario(dir, w, "filter-not-default", Groups.class, S, "DEADBEEF", "tests.filter", "not (default)");
      scenario(dir, w, "filter-syntax-error", Groups.class, S, "DEADBEEF", "tests.filter", "@slow and");
      scenario(dir, w, "filter-bad-token", Groups.class, S, "DEADBEEF", "tests.filter", "slow");
      scenario(dir, w, "nightly-suite", NightlySuite.class, S, "DEADBEEF");
      scenario(dir, w, "method-filter", Groups.class, S, "DEADBEEF", "tests.method", "test*l*");
      scenario(dir, w, "method-filter-none", Groups.class, S, "DEADBEEF", "tests.method", "nothing");
      scenario(dir, w, "method-filter-repeat", Repeats.class, S, "DEADBEEF", "tests.method", "testRepeat");
      scenario(dir, w, "class-filter-none", Simple.class, S, "DEADBEEF", "tests.class", "*Other*");
      scenario(dir, w, "class-filter-match", Simple.class, S, "DEADBEEF", "tests.class", "*RunnerOracle$Sim?le");
      scenario(dir, w, "params", Params.class, S, "DEADBEEF");
      scenario(dir, w, "params-formatted", ParamsFormatted.class, S, "DEADBEEF");
      scenario(dir, w, "junit3-listeners", JUnit3Style.class, S, "DEADBEEF");
      scenario(dir, w, "decorated", Decorated.class, S, "DEADBEEF");
      scenario(dir, w, "invalid-non-public", NonPublicTest.class, S, "DEADBEEF");
      scenario(dir, w, "invalid-static", StaticTest.class, S, "DEADBEEF");
      scenario(dir, w, "invalid-args", ArgsTest.class, S, "DEADBEEF");
      scenario(dir, w, "invalid-instance-before-class", InstanceBeforeClass.class, S, "DEADBEEF");
      scenario(dir, w, "invalid-no-factory", NoFactory.class, S, "DEADBEEF");
      scenario(dir, w, "invalid-method-seed", BadMethodSeed.class, S, "DEADBEEF");
      scenario(dir, w, "invalid-tests-seed", Simple.class, S, "XYZ");
      scenario(dir, w, "invalid-tests-seed-chain", Simple.class, S, "1:2:3");
      scenario(dir, w, "invalid-tests-iters", Simple.class, S, "DEADBEEF", "tests.iters", "0");
      // ReproduceInfoPrinter writes to System.err.
      {
        java.io.PrintStream saved = System.err;
        java.io.ByteArrayOutputStream buf = new java.io.ByteArrayOutputStream();
        System.setErr(new java.io.PrintStream(buf, true, "UTF-8"));
        try {
          scenario(dir, w, "reproduce-info-printer", ReproducePrinter.Suite.class, S, "DEADBEEF", "tests.iters", "1");
        } finally {
          System.setErr(saved);
        }
        for (String line : buf.toString("UTF-8").split("\n")) {
          if (line.startsWith("FAILURE") || line.startsWith("Message") || line.startsWith("Reproduce") || line.startsWith("Throwable") || line.startsWith("java.")) {
            w.println("stderr " + line);
          }
        }
      }
      scenario(dir, w, "leak-test", LeakTest.class, S, "DEADBEEF", "tests.killwait", "200");
      scenario(dir, w, "leak-suite", LeakSuite.class, S, "DEADBEEF", "tests.killwait", "200");
      scenario(dir, w, "leak-none", LeakNone.class, S, "DEADBEEF");
      scenario(dir, w, "leak-linger", LeakLinger.class, S, "DEADBEEF");
      scenario(dir, w, "leak-warn-only", LeakWarnOnly.class, S, "DEADBEEF");
      scenario(dir, w, "leak-filtered", LeakFiltered.class, S, "DEADBEEF");
      scenario(dir, w, "leak-randomized-sleep", LeakRandomizedSleep.class, S, "DEADBEEF", "tests.killwait", "200");
      scenario(dir, w, "class-ignore", IgnoredClass.class, S, "DEADBEEF");
      scenario(dir, w, "timeout-test", TestTimeout.class, S, "DEADBEEF", "tests.killwait", "200");
      scenario(dir, w, "timeout-test-global-first", TestTimeout.class, S, "DEADBEEF", "tests.timeout", "100!", "tests.killwait", "200");
      scenario(dir, w, "timeout-suite", SuiteTimeout.class, S, "DEADBEEF", "tests.killwait", "200");
      scenario(dir, w, "uncaught", Uncaught.class, S, "DEADBEEF");
      scenario(dir, w, "uncaught-suite", UncaughtSuite.class, S, "DEADBEEF");
      System.setProperty("oracle.existing", "before");
      scenario(dir, w, "rule-restore", RestoreRule.class, S, "DEADBEEF");
      w.println("after oracle.prop=" + System.getProperty("oracle.prop") + " oracle.set=" + System.getProperty("oracle.set"));
      scenario(dir, w, "rule-invariant", InvariantRule.class, S, "DEADBEEF");
      w.println("after oracle.added=" + System.getProperty("oracle.added") + " oracle.existing=" + System.getProperty("oracle.existing") + " oracle.ignored=" + System.getProperty("oracle.ignored"));
      System.clearProperty("oracle.existing");
      System.clearProperty("oracle.ignored");
      scenario(dir, w, "rule-no-shadow-pass", ShadowSuper.class, S, "DEADBEEF");
      scenario(dir, w, "rule-no-class-hooks-shadowing", ShadowSub.class, S, "DEADBEEF");
      scenario(dir, w, "rule-no-instance-hooks-overrides", OverrideSub.class, S, "DEADBEEF");
      scenario(dir, w, "virtual-dispatch-super", VirtualSuper.class, S, "DEADBEEF");
      scenario(dir, w, "virtual-dispatch-sub", VirtualSub.class, S, "DEADBEEF");
      scenario(dir, w, "virtual-dispatch-subsub", VirtualSubSub.class, S, "DEADBEEF");
      scenario(dir, w, "rule-static-fields-exceeding", StaticFields.Exceeding.class, S, "DEADBEEF");
      scenario(dir, w, "rule-static-fields-under", StaticFields.Under.class, S, "DEADBEEF");
      scenario(dir, w, "rule-static-fields-unmeasurable", StaticFields.Unmeasurable.class, S, "DEADBEEF");
      scenario(dir, w, "rule-adapter", Adapters.class, S, "DEADBEEF");
      scenario(dir, w, "require-assertions-match", Assertions.class, S, "DEADBEEF");
      // Zombies last: they disable every later suite of this JVM.
      scenario(dir, w, "zombies", Zombies.class, S, "DEADBEEF", "tests.killattempts", "2", "tests.killwait", "100");
      scenario(dir, w, "after-zombies", AfterZombies.class, S, "DEADBEEF");
      releaseZombie = true;
    }
  }
}
