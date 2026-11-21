// The suite AnalysisMocksOracle runs its scenarios in, so that MockTokenizer (which draws from
// RandomizedContext.current()) and BaseTokenStreamTestCase have a test context (a Lucene suite
// must be named Test*).
package org.apache.lucene.tests.analysis;

public class TestAnalysisMocksScenarios extends BaseTokenStreamTestCase {
  public void testScenarios() throws Exception {
    AnalysisMocksOracle.scenarios();
  }
}
