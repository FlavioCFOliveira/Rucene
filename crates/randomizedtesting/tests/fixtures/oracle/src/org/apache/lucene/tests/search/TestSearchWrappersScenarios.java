// The suite SearchWrappersOracle runs its scenarios in, so that the test-framework code that
// draws from the test context (AssertingLeafCollector's rarely()) has one (a Lucene suite must be
// named Test*).
package org.apache.lucene.tests.search;

import org.apache.lucene.tests.util.LuceneTestCase;

public class TestSearchWrappersScenarios extends LuceneTestCase {
  public void testScenarios() throws Exception {
    SearchWrappersOracle.scenarios();
    SearchWrappersOracle.rec("ctx", "context", "next", () -> random().nextLong());
  }
}
