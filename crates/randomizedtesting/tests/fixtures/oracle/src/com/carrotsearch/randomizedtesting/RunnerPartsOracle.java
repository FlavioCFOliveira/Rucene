// Oracle of the runner's helpers: GlobFilter (regex text of a glob),
// FilterExpressionParser.toExpression, TraceFormatting, the argument
// template formatting, RandomizedRunner.methodName, Description's
// class/method split, MultipleFailureException messages, WriterOutputStream,
// TeeOutputStream errors and RamUsageEstimator. Writes runner_parts.tsv:
// kind<TAB>input...<TAB>result, strings as OracleSupport.units.
package com.carrotsearch.randomizedtesting;

import static com.carrotsearch.randomizedtesting.OracleSupport.*;

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

import org.junit.runner.Description;
import org.junit.runners.model.MultipleFailureException;

import com.carrotsearch.randomizedtesting.rules.RamOracle;

public class RunnerPartsOracle {
  static String res(Thrower t, java.util.function.Supplier<String> ok) {
    try { t.run(); return ok.get(); } catch (Throwable e) { return "ERR " + e.getClass().getName() + ": " + units(String.valueOf(e.getMessage())); }
  }

  public static void main(String[] args) throws Exception {
    Path dir = Paths.get(args[0]);
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(dir.resolve("runner_parts.tsv")))) {
      w.println("# kind\tinputs...\tresult");
      String[] globs = {"*", "test*", "test?", "*a*b*", "te.st", "a$b", "a-b", "{x}", "[y]", "(z)", "a\\b", "a|b", "^a", "a+", "*+", "+a", "a++b", "é*", "😀?", "", "**", "?*?"};
      String[] inputs = {"", "test", "testA", "tests", "te.st", "teXst", "a$b", "a-b", "{x}", "[y]", "(z)", "a\\b", "a", "b", "aaa", "aab", "aaab", "x\ny", "été", "😀x", "ab", "^a", "a+", "abba", "test\n"};
      for (String g : globs) {
        String compiled = res(() -> new MethodGlobFilter(g), () -> "OK");
        w.println("globCompile\t" + units(g) + "\t" + compiled);
        if (!compiled.equals("OK")) continue;
        GlobFilter f = new MethodGlobFilter(g);
        for (String in : inputs) w.println("glob\t" + units(g) + "\t" + units(in) + "\t" + f.globMatches(in));
      }
      String[] exprs = {"", "default", "@a", "@a and @b", "@a or @b and @c", "not @a or @b", "not (@a or @b)", "(@a)", "((@a and (default)))",
          "@a and", "@a or", "and", "(", ")", "(@a", "@a)", "@a @b", "not", "foo", "@a AND NOT @b OR DEFAULT", "  @a\tand\n@b  ", "@a(@b)"};
      for (String e : exprs) {
        w.println("filter\t" + units(e) + "\t" + res(() -> new FilterExpressionParser().parse(e), () -> units(new FilterExpressionParser().parse(e).toExpression())));
      }
      // TraceFormatting.
      Exception cause = new IllegalStateException("cause");
      cause.setStackTrace(new StackTraceElement[] {
          new StackTraceElement("foo.Cause", "m", "Cause.java", 3),
          new StackTraceElement("org.junit.Runner", "run", null, -1)});
      RuntimeException t = new RuntimeException("top", cause);
      t.setStackTrace(new StackTraceElement[] {
          new StackTraceElement("__randomizedtesting.SeedInfo", "seed", "[DEADBEEF:1]", 0),
          new StackTraceElement("foo.Bar", "test", "Bar.java", 10),
          new StackTraceElement("sun.reflect.X", "invoke", null, -2),
          new StackTraceElement("org.junit.Y", "run", "Y.java", 5),
          new StackTraceElement("com.carrotsearch.randomizedtesting.Z", "go", "Z.java", 7),
          new StackTraceElement("java.lang.reflect.Method", "invoke", "Method.java", 1),
          new StackTraceElement("foo.Baz", "after", "Baz.java", -1),
          new StackTraceElement("junit.framework.A", "a", "A.java", 1)});
      w.println("trace\tfiltered\t" + units(new TraceFormatting(Arrays.asList("org.junit.", "junit.framework.", "sun.", "java.lang.reflect.", "com.carrotsearch.randomizedtesting.")).formatThrowable(t)));
      w.println("trace\tplain\t" + units(new TraceFormatting().formatThrowable(t)));
      // Argument templates.
      Object[][] fmts = {{"p0=%s p1=%s seed=%3$s", "a", 1, "[S]"}, {"%2$s:%1$s", "x", 10, "[S]"}, {" seed=%1$s", "[S]"}, {"%s-%s-%s", "a", "b", "c"}, {"%-5s|%5s|%%|%n|%d", "ab", "cd", 7}, {"%b %b %B", true, null, "x"}};
      for (Object[] f : fmts) {
        Object[] a = Arrays.copyOfRange(f, 1, f.length);
        w.println("format\t" + units((String) f[0]) + "\t" + units(String.format(Locale.ROOT, (String) f[0], a)));
      }
      w.println("format-missing\t" + res(() -> String.format(Locale.ROOT, "%s %s", "a"), () -> "OK"));
      // methodName and Description parsing.
      String[] names = {"testA(Cls)", "testA {seed=[A:B]}(Cls)", "testA {p0=a p1=b seed=[A:B]}(Cls)", "testA {a} {b}(Cls)", "testA{x}(Cls)", "test {}(Cls)", "x (y)(Cls)", "noParens", "a(b\nc)", "a\n(b)", "m(c)d", "m()", "{a}{b}(C)", "t {a\nb}(C)"};
      for (String n : names) {
        Description d = Description.createSuiteDescription(n);
        w.println("desc\t" + units(n) + "\t" + units(String.valueOf(d.getMethodName())) + "\t" + units(String.valueOf(d.getClassName()))
            + "\t" + (d.getMethodName() == null ? "-" : units(RandomizedRunner.methodName(d))));
      }
      // MultipleFailureException.
      List<Throwable> errs = Arrays.asList(new RuntimeException("one"), new IllegalStateException((String) null), new Error("three\nlines"));
      w.println("mfe\tmessage\t" + units(new MultipleFailureException(errs).getMessage()));
      try { MultipleFailureException.assertEmpty(errs); } catch (Throwable e) { w.println("mfe\tassertEmpty\t" + e.getClass().getName()); }
      try { MultipleFailureException.assertEmpty(Arrays.asList(new RuntimeException("solo"))); } catch (Throwable e) { w.println("mfe\tassertEmpty1\t" + e.getClass().getName() + ": " + e.getMessage()); }
      // WriterOutputStream.
      byte[][] inputsB = {
          "hello".getBytes(StandardCharsets.UTF_8),
          "héllo 😀 €".getBytes(StandardCharsets.UTF_8),
          {(byte) 0x61, (byte) 0x80, (byte) 0x62, (byte) 0xe2, (byte) 0x82, (byte) 0x63, (byte) 0xf0, (byte) 0x9f, (byte) 0x98},
          {(byte) 0xc0, (byte) 0xaf, (byte) 0xed, (byte) 0xa0, (byte) 0x80, (byte) 0xf4, (byte) 0x90, (byte) 0x80, (byte) 0x80, (byte) 0xff},
          new byte[300]};
      for (int k = 0; k < 300; k++) inputsB[4][k] = (byte) "xé😀".getBytes(StandardCharsets.UTF_8)[k % 7];
      String[] charsets = {"UTF-8", "ISO-8859-1", "US-ASCII"};
      int[][] splits = {{1000}, {1}, {3, 5}};
      for (String cs : charsets) for (byte[] in : inputsB) for (int bufSize : new int[] {1024, 2, 3}) for (boolean imm : new boolean[] {false, true}) for (int[] split : splits) {
        StringWriter sw = new StringWriter();
        List<Integer> writes = new ArrayList<>();
        Writer rec = new Writer() {
          public void write(char[] c, int o, int l) { writes.add(l); sw.write(c, o, l); }
          public void flush() {}
          public void close() {}
        };
        WriterOutputStream os = new WriterOutputStream(rec, cs, bufSize, imm);
        int off = 0, si = 0;
        List<Integer> afterWrite = new ArrayList<>();
        while (off < in.length) {
          int n = Math.min(split[si++ % split.length], in.length - off);
          os.write(in, off, n); off += n; afterWrite.add(sw.getBuffer().length());
        }
        os.close();
        StringBuilder hex = new StringBuilder();
        for (byte b : in) hex.append(String.format("%02x", b & 0xff));
        w.println("wos\t" + cs + "\t" + bufSize + "\t" + imm + "\t" + Arrays.toString(split) + "\t" + hex + "\t" + units(sw.toString()) + "\t" + writes.size() + "\t" + afterWrite.hashCode());
      }
      // TeeOutputStream errors.
      OutputStream bad1 = new OutputStream() { public void write(int b) throws IOException { throw new IOException("first"); } public void write(byte[] b, int o, int l) throws IOException { throw new IOException("first"); } };
      OutputStream bad2 = new OutputStream() { public void write(int b) throws IOException { throw new IOException("second"); } public void write(byte[] b, int o, int l) throws IOException { throw new IOException("second"); } };
      ByteArrayOutputStream good = new ByteArrayOutputStream();
      try { new TeeOutputStream(bad1, bad2).write(new byte[] {1}, 0, 1); } catch (IOException e) { w.println("tee\tboth\t" + e.getMessage() + "\t" + e.getSuppressed().length + "\t" + e.getSuppressed()[0].getMessage()); }
      try { new TeeOutputStream(good, bad2).write(new byte[] {1, 2}, 0, 2); } catch (IOException e) { w.println("tee\tsecond\t" + e.getMessage() + "\t" + e.getSuppressed().length + "\t" + good.size()); }
      try { new TeeOutputStream(bad1, good).write(new byte[] {3}, 0, 1); } catch (IOException e) { w.println("tee\tfirst\t" + e.getMessage() + "\t" + e.getSuppressed().length + "\t" + good.size()); }
      // RamUsageEstimator.
      w.println("ram\tlayout\t" + RamOracle.layout());
      long[] hs = {0, 1, 1023, 1024, 1025, 1536, 1587, 1588, 1638, 1639, 10 * 1024 * 1024 - 1, 1024L * 1024 * 1024, 1024L * 1024 * 1024 * 3 / 2, Long.MAX_VALUE, 5000, 2560, 2662, 2663, 1075, 1076, 1126};
      for (long h : hs) w.println("ram\thuman\t" + h + "\t" + RamOracle.human(h));
      for (long a : new long[] {0, 1, 7, 8, 9, 15, 16, 17}) w.println("ram\talign\t" + a + "\t" + RamOracle.align(a));
      w.println("ram\tsizeOf\tbyte[0]\t" + RamOracle.sizeOf(new byte[0]));
      w.println("ram\tsizeOf\tbyte[1]\t" + RamOracle.sizeOf(new byte[1]));
      w.println("ram\tsizeOf\tchar[3]\t" + RamOracle.sizeOf(new char[3]));
      w.println("ram\tsizeOf\tlong[3]\t" + RamOracle.sizeOf(new long[3]));
      w.println("ram\tsizeOf\tboolean[9]\t" + RamOracle.sizeOf(new boolean[9]));
      w.println("ram\tsizeOf\tObject[2]\t" + RamOracle.sizeOf(new Object[] {new int[5], Integer.valueOf(1000)}));
      w.println("ram\tsizeOf\tInteger\t" + RamOracle.sizeOf(Integer.valueOf(1000)));
      w.println("ram\tsizeOf\tLong\t" + RamOracle.sizeOf(Long.valueOf(1000)));
      w.println("ram\tsizeOf\tDouble\t" + RamOracle.sizeOf(Double.valueOf(1)));
      w.println("ram\tsizeOf\tBoolean\t" + RamOracle.sizeOf(Boolean.TRUE));
      w.println("ram\tsizeOf\tString\t" + res(() -> RamOracle.sizeOf("x"), () -> "OK"));
    }
  }
}
