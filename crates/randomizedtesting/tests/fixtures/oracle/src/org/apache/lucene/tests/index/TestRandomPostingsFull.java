// The suite of RandomIndexOracle's RandomPostingsTester.testFull runs (a
// Lucene suite must be named Test*).
package org.apache.lucene.tests.index;

import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.tests.util.LuceneTestCase;

public class TestRandomPostingsFull extends LuceneTestCase {
  public void testFull() throws Exception {
    for (String codec : new String[] {"default", "FST50", "BlockTreeOrds"}) {
      String outcome;
      try {
        new RandomPostingsTester(random())
            .testFull(
                RandomIndexOracle.codecOf(codec),
                createTempDir(),
                IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
                true);
        outcome = "ok";
      } catch (Throwable t) {
        outcome = "error " + t;
      }
      RandomIndexOracle.FULL.add(codec + "=" + outcome);
    }
    RandomIndexOracle.FULL.add("next=" + random().nextLong());
  }
}
