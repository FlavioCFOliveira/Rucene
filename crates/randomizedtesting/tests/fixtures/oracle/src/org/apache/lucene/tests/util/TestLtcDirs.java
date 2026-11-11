// The suite of LuceneTestCaseOracle's task #349 "dirs" rows (ltc_members.tsv):
// the lock-factory directory overloads, newMockFSDirectory, randomVectorFormat
// and getDataPath/getDataInputStream under RandomizedRunner.
package org.apache.lucene.tests.util;

import java.nio.file.Path;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.Lock;
import org.apache.lucene.store.LockFactory;
import org.apache.lucene.store.NativeFSLockFactory;
import org.apache.lucene.store.NoLockFactory;
import org.apache.lucene.store.SimpleFSLockFactory;
import org.apache.lucene.store.SingleInstanceLockFactory;

public class TestLtcDirs extends LuceneTestCase {
  public void testDirs() throws Exception {
    StringBuilder sb = new StringBuilder();
    Path base = createTempDir("base").getParent();
    String[] masks = {base.toRealPath().toString(), base.toString()};
    for (int i = 0; i < 3; i++) {
      for (LockFactory lf : lockFactories()) {
        sb.append(" nd=").append(describe(newDirectory(random(), lf), masks));
        sb.append(" nmd=").append(describe(newMockDirectory(random(), lf), masks));
      }
    }
    for (LockFactory lf : fsLockFactories()) {
      sb.append(" nfs=").append(describe(newFSDirectory(createTempDir("fs"), lf), masks));
      sb.append(" nmfs=").append(describe(newMockFSDirectory(createTempDir("mfs")), masks));
      sb.append(" nmfsl=").append(describe(newMockFSDirectory(createTempDir("mfsl"), lf), masks));
    }
    for (VectorEncoding e : VectorEncoding.values()) {
      sb.append(" rvf").append(e).append('=');
      for (int i = 0; i < 12; i++) sb.append(i == 0 ? "" : ",").append(randomVectorFormat(e).getName());
    }
    for (String name : new String[] {"nope.txt", "/nope.txt"}) {
      try {
        getDataPath(name);
        sb.append(" dp=found");
      } catch (Exception ex) {
        sb.append(" dp=").append(ex.getClass().getName()).append(':').append(ex.getMessage());
      }
      try {
        getDataInputStream(name).close();
        sb.append(" dis=found");
      } catch (Exception ex) {
        sb.append(" dis=").append(ex.getClass().getName()).append(':').append(ex.getMessage());
      }
    }
    sb.append(" last=").append(random().nextLong());
    LuceneTestCaseOracle.DRAWS.add(sb.toString());
  }

  static LockFactory[] lockFactories() {
    return new LockFactory[] {
      NoLockFactory.INSTANCE, new SingleInstanceLockFactory(), NativeFSLockFactory.INSTANCE, SimpleFSLockFactory.INSTANCE
    };
  }

  static LockFactory[] fsLockFactories() {
    return new LockFactory[] {NativeFSLockFactory.INSTANCE, SimpleFSLockFactory.INSTANCE, NoLockFactory.INSTANCE};
  }

  /** toString (temp root as $D, identity hashes as @HASH), then a second obtainLock of a held lock. */
  static String describe(Directory d, String[] masks) throws Exception {
    String s = d.toString();
    for (String m : masks) s = s.replace(m, "$D");
    s = s.replaceAll("@[0-9a-f]+", "@HASH");
    String again;
    try (Lock l = d.obtainLock("t.lock")) {
      try (Lock l2 = d.obtainLock("t.lock")) {
        again = "again";
      } catch (Exception e) {
        again = e.getClass().getSimpleName();
      }
    }
    d.close();
    return s + "|" + again;
  }
}
