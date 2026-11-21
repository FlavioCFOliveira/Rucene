// The suites of TempFilesOracle: LuceneTestCase's temporary-files rule
// (TestRuleTemporaryFilesCleanup) under fixed seeds and annotations.
package org.apache.lucene.tests.util;

import java.lang.reflect.Field;
import java.nio.file.*;
import java.nio.file.spi.FileSystemProvider;
import org.apache.lucene.tests.mockfile.FilterFileSystemProvider;
import org.apache.lucene.tests.mockfile.HandleLimitFS;

public class TestTempFilesSuite extends LuceneTestCase {
  static String describe(FileSystemProvider p) throws Exception {
    StringBuilder sb = new StringBuilder();
    while (p instanceof FilterFileSystemProvider) {
      String name = p.getClass().getSimpleName();
      sb.append(name);
      switch (name) {
        case "ShuffleFS": sb.append("(seed=").append(field(p, "seed")).append(')'); break;
        case "HandleLimitFS": sb.append("(limit=").append(field(p, "limit")).append(')'); break;
        case "ExtrasFS": sb.append("(active=").append(field(p, "active")).append(",createDirectory=").append(field(p, "createDirectory")).append(')'); break;
        default: break;
      }
      sb.append(" > ");
      p = (FileSystemProvider) field(p, "delegate");
    }
    return sb.append("default").toString();
  }

  static Object field(Object o, String name) throws Exception {
    Class<?> c = o.getClass();
    while (c != null) {
      try {
        Field f = c.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(o);
      } catch (NoSuchFieldException e) {
        c = c.getSuperclass();
      }
    }
    throw new NoSuchFieldException(name);
  }

  static String record() throws Exception {
    Path tmp = Paths.get(System.getProperty("java.io.tmpdir")).toRealPath();
    Path a = createTempDir();
    Path b = createTempDir("x");
    Path c = createTempDir();
    Path d = createTempFile();
    Path e = createTempFile("p", ".s");
    StringBuilder sb = new StringBuilder();
    sb.append(describe(a.getFileSystem().provider()));
    for (Path p : new Path[] {a, b, c, d, e}) {
      sb.append('\t').append(Paths.get(p.toString()).toString().replace(tmp.toString(), "$T"));
    }
    sb.append('\t').append(Files.isDirectory(a)).append(Files.isRegularFile(d));
    return sb.toString();
  }

  public void testRecord() throws Exception {
    TempFilesOracle.ROWS.add(record());
  }

  @SuppressFileSystems({"ShuffleFS", "LeakFS", "ExtrasFS"})
  public static class TestSuppressed extends LuceneTestCase {
    public void testRecord() throws Exception {
      TempFilesOracle.ROWS.add(record());
    }
  }

  @SuppressFsync
  @HandleLimitFS.MaxOpenHandles(limit = 17)
  public static class TestFsyncLimit extends LuceneTestCase {
    public void testRecord() throws Exception {
      TempFilesOracle.ROWS.add(record());
    }
  }

  @SuppressFileSystems("*")
  public static class TestAll extends LuceneTestCase {
    public void testRecord() throws Exception {
      TempFilesOracle.ROWS.add(record());
    }
  }

  public static class TestFailing extends LuceneTestCase {
    public void testRecord() throws Exception {
      TempFilesOracle.ROWS.add(record());
      fail("boom");
    }
  }
}
