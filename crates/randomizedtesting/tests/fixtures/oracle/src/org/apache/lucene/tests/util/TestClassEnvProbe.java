// The suites ClassEnvOracle runs (a Lucene suite must be named Test*): each
// records the class environment TestRuleSetupAndRestoreClassEnv set up, the
// order of its own rules and hooks around LuceneTestCase's, and what the
// LuceneTestCase hooks did (query cache, core count, executor).
package org.apache.lucene.tests.util;

import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.TimeZone;
import java.util.concurrent.ThreadPoolExecutor;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.util.InfoStream;
import org.junit.After;
import org.junit.AfterClass;
import org.junit.Before;
import org.junit.BeforeClass;
import org.junit.ClassRule;
import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TestRule;
import org.junit.runners.model.Statement;

public class TestClassEnvProbe extends LuceneTestCase {
  public static final List<String> EVENTS = new ArrayList<>();

  static String env() throws Exception {
    Field f = LuceneTestCase.class.getDeclaredField("executor");
    f.setAccessible(true);
    ThreadPoolExecutor ex = (ThreadPoolExecutor) f.get(null);
    return "codec=" + Codec.getDefault()
        + "\tsim=" + classEnvRule.similarity
        + "\tlocale=" + Locale.getDefault().toLanguageTag()
        + "\ttz=" + TimeZone.getDefault().getID()
        + "\tinfo=" + InfoStream.getDefault().getClass().getName()
        + "\tflush=" + liveIWCFlushMode
        + "\tcache=" + (IndexSearcher.getDefaultQueryCache() == null ? "null" : IndexSearcher.getDefaultQueryCache().getClass().getSimpleName())
        + "\tpolicy=" + (IndexSearcher.getDefaultQueryCachingPolicy() == LuceneTestCase.MAYBE_CACHE_POLICY)
        + "\tcores=" + System.getProperty("lucene.cms.override_core_count")
        + "\texecutor=" + (ex == null ? "null" : Integer.toString(ex.getCorePoolSize()));
  }

  @ClassRule
  public static final TestRule classProbe =
      (base, d) ->
          new Statement() {
            @Override
            public void evaluate() throws Throwable {
              EVENTS.add("classRule.before\tcodec=" + Codec.getDefault() + "\tclass=" + LuceneTestCase.getTestClass().getSimpleName());
              try {
                base.evaluate();
              } finally {
                EVENTS.add("classRule.after\tcodec=" + Codec.getDefault());
              }
            }
          };

  @Rule
  public final TestRule testProbe =
      (base, d) ->
          new Statement() {
            @Override
            public void evaluate() throws Throwable {
              EVENTS.add("rule.before\t" + d.getMethodName());
              try {
                base.evaluate();
              } finally {
                EVENTS.add("rule.after\t" + d.getMethodName());
              }
            }
          };

  @BeforeClass
  public static void probeBeforeClass() throws Exception {
    EVENTS.add("beforeClass\t" + env() + "\tnext=" + random().nextLong());
  }

  @Before
  public void probeBefore() {
    EVENTS.add("before\t" + random().nextLong());
  }

  @Test
  public void testProbe() throws Exception {
    Field f = LuceneTestCase.class.getDeclaredField("threadAndTestNameRule");
    f.setAccessible(true);
    TestRuleThreadAndTestName r = (TestRuleThreadAndTestName) f.get(this);
    EVENTS.add("test\tname=" + r.testMethodName + "\t" + env() + "\tnext=" + random().nextLong());
  }

  @After
  public void probeAfter() {
    EVENTS.add("after\t" + random().nextLong());
  }

  @AfterClass
  public static void probeAfterClass() {
    EVENTS.add("afterClass\t" + random().nextLong());
  }

  /** A suite that fails, for the reproduce line and the test params. */
  public static class TestClassEnvFailing extends LuceneTestCase {
    @Test
    public void testFails() {
      fail("boom");
    }
  }

  /** A suite that suppresses codecs. */
  @SuppressCodecs({"Lucene104", "MockRandom", "Direct", "FST50", "SimpleText"})
  public static class TestClassEnvSuppressed extends LuceneTestCase {
    @Test
    public void testCodec() {
      EVENTS.add("suppressed\tcodec=" + Codec.getDefault());
    }
  }
}
