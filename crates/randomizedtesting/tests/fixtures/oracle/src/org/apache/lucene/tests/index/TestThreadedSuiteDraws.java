// The ThreadedIndexingAndSearchingTestCase suite of BaseFormatSuitesOracle (rmp #355): the members
// of TestNRTThreads, recording after its test (in tearDown, before super.tearDown()) the next
// random().nextLong() of the test's randomness, and after the class that of the class randomness.
package org.apache.lucene.tests.index;

import java.util.Set;
import java.util.concurrent.ExecutorService;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexReader;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.store.Directory;
import org.apache.lucene.tests.store.MockDirectoryWrapper;
import org.apache.lucene.tests.util.LuceneTestCase.SuppressCodecs;
import org.junit.AfterClass;
import org.junit.Before;

@SuppressCodecs({"SimpleText", "Direct"})
public class TestThreadedSuiteDraws extends ThreadedIndexingAndSearchingTestCase {

  private boolean useNonNrtReaders = true;

  @Before
  @Override
  public void setUp() throws Exception {
    super.setUp();
    useNonNrtReaders = random().nextBoolean();
  }

  @Override
  protected void doSearching(ExecutorService es, int maxIterations) throws Exception {

    boolean anyOpenDelFiles = false;

    DirectoryReader r = DirectoryReader.open(writer);
    int iterations = 0;
    while (++iterations < maxIterations && !failed.get()) {
      if (random().nextBoolean()) {
        if (VERBOSE) {
          System.out.println("TEST: now reopen r=" + r);
        }
        final DirectoryReader r2 = DirectoryReader.openIfChanged(r);
        if (r2 != null) {
          r.close();
          r = r2;
        }
      } else {
        if (VERBOSE) {
          System.out.println("TEST: now close reader=" + r);
        }
        r.close();
        writer.commit();
        final Set<String> openDeletedFiles = ((MockDirectoryWrapper) dir).getOpenDeletedFiles();
        if (openDeletedFiles.size() > 0) {
          System.out.println("OBD files: " + openDeletedFiles);
        }
        anyOpenDelFiles |= openDeletedFiles.size() > 0;
        // assertEquals("open but deleted: " + openDeletedFiles, 0, openDeletedFiles.size());
        if (VERBOSE) {
          System.out.println("TEST: now open");
        }
        r = DirectoryReader.open(writer);
      }
      if (VERBOSE) {
        System.out.println("TEST: got new reader=" + r);
      }
      // System.out.println("numDocs=" + r.numDocs() + "
      // openDelFileCount=" + dir.openDeleteFileCount());

      if (r.numDocs() > 0) {
        fixedSearcher = new IndexSearcher(r, es);
        smokeTestSearcher(fixedSearcher);
        runSearchThreads(100);
      }
    }
    r.close();

    // System.out.println("numDocs=" + r.numDocs() + " openDelFileCount=" +
    // dir.openDeleteFileCount());
    final Set<String> openDeletedFiles = ((MockDirectoryWrapper) dir).getOpenDeletedFiles();
    if (openDeletedFiles.size() > 0) {
      System.out.println("OBD files: " + openDeletedFiles);
    }
    anyOpenDelFiles |= openDeletedFiles.size() > 0;

    assertFalse("saw non-zero open-but-deleted count", anyOpenDelFiles);
  }

  @Override
  protected Directory getDirectory(Directory in) {
    assert in instanceof MockDirectoryWrapper;
    if (!useNonNrtReaders) ((MockDirectoryWrapper) in).setAssertNoDeleteOpenFile(true);
    return in;
  }

  @Override
  protected void doAfterWriter(ExecutorService es) throws Exception {
    // Force writer to do reader pooling, always, so that
    // all merged segments, even for merges before
    // doSearching is called, are warmed:
    DirectoryReader.open(writer).close();
  }

  private IndexSearcher fixedSearcher;

  @Override
  protected IndexSearcher getCurrentSearcher() throws Exception {
    return fixedSearcher;
  }

  @Override
  protected void releaseSearcher(IndexSearcher s) throws Exception {
    if (s != fixedSearcher) {
      // Final searcher:
      s.getIndexReader().close();
    }
  }

  @Override
  protected IndexSearcher getFinalSearcher() throws Exception {
    final IndexReader r2;
    if (useNonNrtReaders) {
      if (random().nextBoolean()) {
        r2 = DirectoryReader.open(writer);
      } else {
        writer.commit();
        r2 = DirectoryReader.open(dir);
      }
    } else {
      r2 = DirectoryReader.open(writer);
    }
    return newSearcher(r2);
  }

  public void testNRTThreads() throws Exception {
    runTest("TestNRTThreads");
  }

  @Override
  public void tearDown() throws Exception {
    BaseFormatSuitesOracle.DRAWS.add(getTestName() + "\tnext\t" + random().nextLong());
    super.tearDown();
  }

  @AfterClass
  public static void recordClassDraw() {
    BaseFormatSuitesOracle.DRAWS.add("#class\tnext\t" + random().nextLong());
  }
}
