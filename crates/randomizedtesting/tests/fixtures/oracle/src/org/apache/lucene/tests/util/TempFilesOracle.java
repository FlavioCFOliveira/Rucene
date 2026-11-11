// Oracle of LuceneTestCase.createTempDir / createTempFile and the
// filesystem chain TestRuleTemporaryFilesCleanup.before() installs, for
// fixed seeds (rucene-test-framework/tests/temp_files_oracle.rs). Writes
// <out>/rucene-test-framework/temp_files.tsv; java.io.tmpdir is masked as $T.
package org.apache.lucene.tests.util;

import java.io.*;
import java.nio.file.*;
import java.util.*;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.notification.Failure;

public class TempFilesOracle {
  static final List<String> ROWS = new ArrayList<>();

  /** The child JVM of the tests.leavetmpdir=true run. */
  public static class LeaveChild {
    public static void main(String[] args) throws Exception {
      System.setProperty("tests.seed", "DEADBEEF");
      ByteArrayOutputStream err = new ByteArrayOutputStream();
      PrintStream saved = System.err;
      System.setErr(new PrintStream(err, true, "UTF-8"));
      Result r = new JUnitCore().run(TestTempFilesSuite.TestAll.class);
      System.setErr(saved);
      Path tmp = Paths.get(System.getProperty("java.io.tmpdir")).toRealPath();
      List<String> info = new ArrayList<>();
      for (String line : err.toString("UTF-8").split("\n")) {
        if (line.startsWith("INFO: Will leave temporary file: ")) info.add(line.replace(tmp.toString(), "$T"));
      }
      System.out.println("leave\t" + r.getFailureCount() + "\t" + left("lucene.tests.util.TestTempFilesSuite$TestAll_DEADBEEF") + "\t" + String.join(" | ", info));
      for (String row : ROWS) System.out.println("leave-row\t" + row);
    }
  }

  static String left(String prefix) throws IOException {
    Path tmp = Paths.get(System.getProperty("java.io.tmpdir")).toRealPath();
    List<String> l = new ArrayList<>();
    try (DirectoryStream<Path> s = Files.newDirectoryStream(tmp)) {
      for (Path p : s) if (p.getFileName().toString().startsWith(prefix)) l.add(p.getFileName().toString());
    }
    Collections.sort(l);
    return l.toString();
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    // Once, before LuceneTestCase's sysout rule captures the streams.
    System.setErr(new PrintStream(new ByteArrayOutputStream(), true, "UTF-8"));
    try (PrintWriter w = new PrintWriter(new File(out, "temp_files.tsv"), "UTF-8")) {
      w.println("# suite\tseed\tchain\tcreateTempDir()\tcreateTempDir(x)\tcreateTempDir()\tcreateTempFile()\tcreateTempFile(p,.s)\tdir,file\tfailures\tleft after the suite");
      Class<?>[] suites = {TestTempFilesSuite.class, TestTempFilesSuite.TestSuppressed.class, TestTempFilesSuite.TestFsyncLimit.class, TestTempFilesSuite.TestAll.class, TestTempFilesSuite.TestFailing.class};
      String[] seeds = {"0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D", "E", "F", "10", "11", "12", "13", "DEADBEEF", "CAFEBABE", "1BADC0DE", "7FFFFFFFFFFFFFFF"};
      for (Class<?> suite : suites) {
        for (String seed : seeds) {
          if (suite != TestTempFilesSuite.class && seed.length() < 3 && !seed.equals("0") && !seed.equals("1")) continue;
          ROWS.clear();
          System.setProperty("tests.seed", seed);
          Result r = new JUnitCore().run(suite);
          String name = suite.getName();
          String leftNow = left(name.replaceFirst("^org.apache.lucene.", "lucene.") + "_" + seed + "-");
          for (Failure f : r.getFailures()) if (ROWS.isEmpty()) w.println("# " + suite.getSimpleName() + " " + seed + " " + f.toString().replace("\n", " ").substring(0, Math.min(300, f.toString().length())));
          for (String row : ROWS) w.println(suite.getSimpleName() + "\t" + seed + "\t" + row + "\t" + r.getFailureCount() + "\t" + leftNow);
          // A failed suite leaves its files: remove them.
          if (r.getFailureCount() > 0) {
            Path tmp = Paths.get(System.getProperty("java.io.tmpdir")).toRealPath();
            try (DirectoryStream<Path> s = Files.newDirectoryStream(tmp)) {
              for (Path p : s) if (p.getFileName().toString().startsWith(name.replaceFirst("^org.apache.lucene.", "lucene.") + "_" + seed + "-")) org.apache.lucene.util.IOUtils.rm(p);
            }
          }
        }
      }
      System.clearProperty("tests.seed");
      String java = ProcessHandle.current().info().command().orElse("java");
      Process p = new ProcessBuilder(java, "-ea", "-Dtests.leavetmpdir=true", "-cp", System.getProperty("java.class.path"), LeaveChild.class.getName()).start();
      try (BufferedReader r = new BufferedReader(new InputStreamReader(p.getInputStream(), "UTF-8"))) {
        String line;
        while ((line = r.readLine()) != null) w.println(line);
      }
      p.waitFor();
      Path tmp = Paths.get(System.getProperty("java.io.tmpdir")).toRealPath();
      try (DirectoryStream<Path> s = Files.newDirectoryStream(tmp)) {
        for (Path q : s) if (q.getFileName().toString().startsWith("lucene.tests.util.TestTempFilesSuite$TestAll_DEADBEEF")) org.apache.lucene.util.IOUtils.rm(q);
      }
    }
  }
}
