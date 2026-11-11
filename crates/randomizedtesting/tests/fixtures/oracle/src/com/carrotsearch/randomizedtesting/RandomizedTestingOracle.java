// Oracle for the Rucene port of randomizedtesting 2.8.1 (crates/randomizedtesting).
//
// Runs java.util.Random (JDK 25), Xoroshiro128PlusRandom, AssertingRandom, the
// seed chain under RandomizedRunner, RandomizedContext resource disposal, the
// generators and the RandomizedTest helpers on fixed inputs, and writes what
// they return. The Rust tests compare against these files.
//
// The digest and encodings are OracleSupport's (see ../../../../README.md);
// regenerate every fixture with ../../../../regenerate.sh.
//
// The class lives in package com.carrotsearch.randomizedtesting so that it can
// call the package-private members the port mirrors (MurmurHash3,
// Randomness.getSeed, RandomizedContext.getRandomnesses).
package com.carrotsearch.randomizedtesting;

import java.io.Closeable;
import java.io.IOException;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.Collections;
import java.util.List;
import java.util.Locale;
import java.util.Random;
import java.util.TimeZone;
import java.util.concurrent.Callable;
import java.util.function.Supplier;

import org.junit.After;
import org.junit.AfterClass;
import org.junit.Before;
import org.junit.BeforeClass;
import org.junit.Test;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.RunWith;
import org.junit.runner.notification.Failure;

import com.carrotsearch.randomizedtesting.annotations.SeedDecorators;
import com.carrotsearch.randomizedtesting.generators.ASCIIGenerator;
import com.carrotsearch.randomizedtesting.generators.BiasedNumbers;
import com.carrotsearch.randomizedtesting.generators.CodepointSetGenerator;
import com.carrotsearch.randomizedtesting.generators.RandomBytes;
import com.carrotsearch.randomizedtesting.generators.RandomNumbers;
import com.carrotsearch.randomizedtesting.generators.RandomPicks;
import com.carrotsearch.randomizedtesting.generators.RandomStrings;
import com.carrotsearch.randomizedtesting.generators.RealisticUnicodeGenerator;
import com.carrotsearch.randomizedtesting.generators.StringGenerator;
import com.carrotsearch.randomizedtesting.generators.UnicodeGenerator;

import static com.carrotsearch.randomizedtesting.OracleSupport.*;

public class RandomizedTestingOracle {
  interface Draw { void draw(Random r, Rec rec); }
  static final class Labeled {
    final String label; final Draw draw;
    Labeled(String label, Draw draw) { this.label = label; this.draw = draw; }
  }

  // Exposes the protected next(int).
  @SuppressWarnings("serial")
  static final class PubRandom extends Random {
    PubRandom(long seed) { super(seed); }
    int pubNext(int bits) { return next(bits); }
  }
  @SuppressWarnings("serial")
  static final class PubXoro extends Xoroshiro128PlusRandom {
    PubXoro(long seed) { super(seed); }
    int pubNext(int bits) { return next(bits); }
  }


  static final int DRAWS = 10_000;

  static List<Labeled> randomMethods() {
    List<Labeled> m = new ArrayList<>();
    for (int bits : new int[] {1, 16, 31, 32}) {
      m.add(new Labeled("next(" + bits + ")", (r, rec) -> {
        int v = r instanceof PubRandom ? ((PubRandom) r).pubNext(bits) : ((PubXoro) r).pubNext(bits);
        rec.i(v);
      }));
    }
    m.add(new Labeled("nextInt()", (r, rec) -> rec.i(r.nextInt())));
    for (int b : new int[] {1, 2, 3, 10, 16, 100, 1 << 30, (1 << 30) + 1, 1000000007, Integer.MAX_VALUE}) {
      m.add(new Labeled("nextInt(" + b + ")", (r, rec) -> rec.i(r.nextInt(b))));
    }
    int[][] intRanges = {
      {0, 1}, {-5, 5}, {10, 26}, {0, 1 << 20}, {-3, -1}, {Integer.MIN_VALUE, Integer.MAX_VALUE},
      {Integer.MIN_VALUE, 0}, {-(1 << 30), 1 << 30}, {Integer.MIN_VALUE, Integer.MIN_VALUE + 1},
      {Integer.MAX_VALUE - 1, Integer.MAX_VALUE}, {-1, Integer.MAX_VALUE}
    };
    for (int[] p : intRanges) {
      m.add(new Labeled("nextInt(" + p[0] + "," + p[1] + ")", (r, rec) -> rec.i(r.nextInt(p[0], p[1]))));
    }
    m.add(new Labeled("nextLong()", (r, rec) -> rec.l(r.nextLong())));
    for (long b : new long[] {1, 2, 3, 1000, 1L << 40, 1L << 62, (1L << 62) + 1, Long.MAX_VALUE}) {
      m.add(new Labeled("nextLong(" + b + ")", (r, rec) -> rec.l(r.nextLong(b))));
    }
    long[][] longRanges = {
      {0, 1}, {-1, 1}, {0, 1L << 33}, {Long.MIN_VALUE, Long.MAX_VALUE}, {Long.MIN_VALUE, 0},
      {-10, 1L << 62}, {Long.MIN_VALUE, Long.MIN_VALUE + 1}, {-(1L << 62), 1L << 62}
    };
    for (long[] p : longRanges) {
      m.add(new Labeled("nextLong(" + p[0] + "," + p[1] + ")", (r, rec) -> rec.l(r.nextLong(p[0], p[1]))));
    }
    m.add(new Labeled("nextBoolean()", (r, rec) -> rec.z(r.nextBoolean())));
    m.add(new Labeled("nextFloat()", (r, rec) -> rec.f(r.nextFloat())));
    for (float b : new float[] {1f, 0.5f, 3f, 1e30f, Float.MIN_VALUE, Float.MAX_VALUE}) {
      m.add(new Labeled("nextFloat(" + Integer.toHexString(Float.floatToIntBits(b)) + ")", (r, rec) -> rec.f(r.nextFloat(b))));
    }
    float[][] floatRanges = {
      {0f, 1f}, {-1f, 1f}, {-Float.MAX_VALUE, Float.MAX_VALUE}, {1e-30f, 2e-30f}, {-5f, -4.5f}, {0f, Float.MIN_VALUE}
    };
    for (float[] p : floatRanges) {
      m.add(new Labeled("nextFloat(" + Integer.toHexString(Float.floatToIntBits(p[0])) + "," + Integer.toHexString(Float.floatToIntBits(p[1])) + ")",
          (r, rec) -> rec.f(r.nextFloat(p[0], p[1]))));
    }
    m.add(new Labeled("nextDouble()", (r, rec) -> rec.d(r.nextDouble())));
    for (double b : new double[] {1d, 0.5d, 3d, 1e300, Double.MIN_VALUE, Double.MAX_VALUE}) {
      m.add(new Labeled("nextDouble(" + Long.toHexString(Double.doubleToLongBits(b)) + ")", (r, rec) -> rec.d(r.nextDouble(b))));
    }
    double[][] doubleRanges = {
      {0d, 1d}, {-1d, 1d}, {-Double.MAX_VALUE, Double.MAX_VALUE}, {1e-300, 2e-300}, {-5d, -4.5d},
      {0d, Double.MIN_VALUE}, {-1e308, 1e308}
    };
    for (double[] p : doubleRanges) {
      m.add(new Labeled("nextDouble(" + Long.toHexString(Double.doubleToLongBits(p[0])) + "," + Long.toHexString(Double.doubleToLongBits(p[1])) + ")",
          (r, rec) -> rec.d(r.nextDouble(p[0], p[1]))));
    }
    m.add(new Labeled("nextGaussian()", (r, rec) -> rec.d(r.nextGaussian())));
    for (int len : new int[] {0, 1, 3, 4, 7, 8, 9, 17}) {
      m.add(new Labeled("nextBytes(" + len + ")", (r, rec) -> { byte[] a = new byte[len]; r.nextBytes(a); rec.bytes(a); }));
    }
    m.add(new Labeled("isDeprecated()", (r, rec) -> rec.z(r.isDeprecated())));
    return m;
  }

  /** Streams: one call consumes the whole stream; recorded element by element. */
  static List<Labeled> streamMethods() {
    List<Labeled> m = new ArrayList<>();
    m.add(new Labeled("ints(10000)", (r, rec) -> r.ints(DRAWS).forEach(rec::i)));
    m.add(new Labeled("ints()", (r, rec) -> r.ints().limit(DRAWS).forEach(rec::i)));
    int[][] intRanges = {{0, 10}, {Integer.MIN_VALUE, Integer.MAX_VALUE}, {-5, 5}, {0, 1 << 20}};
    for (int[] p : intRanges) {
      m.add(new Labeled("ints(10000," + p[0] + "," + p[1] + ")", (r, rec) -> r.ints(DRAWS, p[0], p[1]).forEach(rec::i)));
      m.add(new Labeled("ints(" + p[0] + "," + p[1] + ")", (r, rec) -> r.ints(p[0], p[1]).limit(DRAWS).forEach(rec::i)));
    }
    m.add(new Labeled("longs(10000)", (r, rec) -> r.longs(DRAWS).forEach(rec::l)));
    m.add(new Labeled("longs()", (r, rec) -> r.longs().limit(DRAWS).forEach(rec::l)));
    long[][] longRanges = {{0, 10}, {Long.MIN_VALUE, Long.MAX_VALUE}, {-5, 1L << 40}};
    for (long[] p : longRanges) {
      m.add(new Labeled("longs(10000," + p[0] + "," + p[1] + ")", (r, rec) -> r.longs(DRAWS, p[0], p[1]).forEach(rec::l)));
      m.add(new Labeled("longs(" + p[0] + "," + p[1] + ")", (r, rec) -> r.longs(p[0], p[1]).limit(DRAWS).forEach(rec::l)));
    }
    m.add(new Labeled("doubles(10000)", (r, rec) -> r.doubles(DRAWS).forEach(rec::d)));
    m.add(new Labeled("doubles()", (r, rec) -> r.doubles().limit(DRAWS).forEach(rec::d)));
    double[][] doubleRanges = {{0d, 1d}, {-Double.MAX_VALUE, Double.MAX_VALUE}, {-5d, -4.5d}};
    for (double[] p : doubleRanges) {
      String lbl = Long.toHexString(Double.doubleToLongBits(p[0])) + "," + Long.toHexString(Double.doubleToLongBits(p[1]));
      m.add(new Labeled("doubles(10000," + lbl + ")", (r, rec) -> r.doubles(DRAWS, p[0], p[1]).forEach(rec::d)));
      m.add(new Labeled("doubles(" + lbl + ")", (r, rec) -> r.doubles(p[0], p[1]).limit(DRAWS).forEach(rec::d)));
    }
    for (Object[] p : equiDoublesCases()) {
      double l = (Double) p[0], rt = (Double) p[1];
      boolean li = (Boolean) p[2], ri = (Boolean) p[3];
      String lbl = Long.toHexString(Double.doubleToLongBits(l)) + "," + Long.toHexString(Double.doubleToLongBits(rt)) + "," + li + "," + ri;
      m.add(new Labeled("equiDoubles(" + lbl + ")", (r, rec) -> r.equiDoubles(l, rt, li, ri).limit(DRAWS).forEach(rec::d)));
    }
    return m;
  }

  /** equiDoubles(left, right, isLeftIncluded, isRightIncluded) cases of the draw grid. */
  static Object[][] equiDoublesCases() {
    return new Object[][] {
      {0d, 1d, true, false}, {0d, 1d, false, true}, {-1d, 1d, true, true}, {-1d, 1d, false, false},
      {1e-300, 2e-300, true, true}, {-5d, -4.5d, false, true}, {-Double.MAX_VALUE, Double.MAX_VALUE, true, true},
      {0d, Double.MIN_VALUE, true, true}, {3d, 3d, true, true}, {-1e-310, 1e-310, false, false},
      {-3d, 1e-320, true, false}, {-1e-320, 7d, false, true}, {0d, 0d, true, true}, {-0d, 2d, true, true},
      {1d, Math.nextUp(1d), true, false}, {1e15, 1e15 + 7, true, true}, {-1e18, -1e18 + 4096, true, true},
    };
  }

  static Random gen(String kind, long seed) {
    switch (kind) {
      case "jdk": return new PubRandom(seed);
      case "xoro": return new PubXoro(seed);
      case "asserting": return new AssertingRandom(Thread.currentThread(), new Xoroshiro128PlusRandom(seed));
      default: throw new AssertionError(kind);
    }
  }

  static void randomDraws(Path out) throws Exception {
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("random_draws.tsv")))) {
      w.println("# gen\tseed\tmethod\tfnv64\tfirst values");
      for (String kind : new String[] {"jdk", "xoro", "asserting"}) {
        for (long seed : seeds()) {
          for (Labeled l : randomMethods()) {
            if (kind.equals("asserting") && l.label.startsWith("next(")) continue;
            Random r = gen(kind, seed);
            Rec rec = new Rec();
            for (int k = 0; k < DRAWS; k++) l.draw.draw(r, rec);
            w.println(kind + "\t" + Long.toHexString(seed) + "\t" + l.label + "\t" + rec.line());
          }
          for (Labeled l : streamMethods()) {
            Random r = gen(kind, seed);
            Rec rec = new Rec();
            l.draw.draw(r, rec);
            w.println(kind + "\t" + Long.toHexString(seed) + "\t" + l.label + "\t" + rec.line());
          }
          // Interleaved: one generator, every method in turn, 10 000 rounds.
          Random r = gen(kind, seed);
          Rec rec = new Rec();
          for (int k = 0; k < DRAWS; k++) {
            rec.i(r.nextInt()); rec.i(r.nextInt(7)); rec.l(r.nextLong()); rec.d(r.nextGaussian());
            rec.f(r.nextFloat()); rec.z(r.nextBoolean()); rec.d(r.nextDouble()); rec.i(r.nextInt(-3, 9));
            byte[] a = new byte[5]; r.nextBytes(a); rec.bytes(a); rec.d(r.nextGaussian());
          }
          w.println(kind + "\t" + Long.toHexString(seed) + "\tinterleaved\t" + rec.line());
        }
      }
    }
  }


  static void randomErrors(Path out) throws Exception {
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("random_errors.tsv")))) {
      w.println("# gen\tcall\tclass\tmessage");
      for (String kind : new String[] {"jdk", "xoro", "asserting"}) {
        Random r = gen(kind, 7);
        String k = kind + "\t";
        tryRecord(w, k + "nextInt(0)", () -> r.nextInt(0));
        tryRecord(w, k + "nextInt(-1)", () -> r.nextInt(-1));
        tryRecord(w, k + "nextInt(5,5)", () -> r.nextInt(5, 5));
        tryRecord(w, k + "nextInt(5,4)", () -> r.nextInt(5, 4));
        tryRecord(w, k + "nextLong(0)", () -> r.nextLong(0));
        tryRecord(w, k + "nextLong(-1)", () -> r.nextLong(-1));
        tryRecord(w, k + "nextLong(3,3)", () -> r.nextLong(3, 3));
        tryRecord(w, k + "nextLong(3,2)", () -> r.nextLong(3, 2));
        tryRecord(w, k + "nextFloat(0)", () -> r.nextFloat(0f));
        tryRecord(w, k + "nextFloat(-1)", () -> r.nextFloat(-1f));
        tryRecord(w, k + "nextFloat(NaN)", () -> r.nextFloat(Float.NaN));
        tryRecord(w, k + "nextFloat(inf)", () -> r.nextFloat(Float.POSITIVE_INFINITY));
        tryRecord(w, k + "nextFloat(1,1)", () -> r.nextFloat(1f, 1f));
        tryRecord(w, k + "nextFloat(0,inf)", () -> r.nextFloat(0f, Float.POSITIVE_INFINITY));
        tryRecord(w, k + "nextFloat(-inf,0)", () -> r.nextFloat(Float.NEGATIVE_INFINITY, 0f));
        tryRecord(w, k + "nextFloat(NaN,1)", () -> r.nextFloat(Float.NaN, 1f));
        tryRecord(w, k + "nextDouble(0)", () -> r.nextDouble(0d));
        tryRecord(w, k + "nextDouble(-1)", () -> r.nextDouble(-1d));
        tryRecord(w, k + "nextDouble(NaN)", () -> r.nextDouble(Double.NaN));
        tryRecord(w, k + "nextDouble(inf)", () -> r.nextDouble(Double.POSITIVE_INFINITY));
        tryRecord(w, k + "nextDouble(1,1)", () -> r.nextDouble(1d, 1d));
        tryRecord(w, k + "nextDouble(0,inf)", () -> r.nextDouble(0d, Double.POSITIVE_INFINITY));
        tryRecord(w, k + "nextDouble(-inf,0)", () -> r.nextDouble(Double.NEGATIVE_INFINITY, 0d));
        tryRecord(w, k + "nextDouble(NaN,1)", () -> r.nextDouble(Double.NaN, 1d));
        tryRecord(w, k + "ints(-1)", () -> r.ints(-1));
        tryRecord(w, k + "ints(5,5)", () -> r.ints(5, 5));
        tryRecord(w, k + "ints(10,5,5)", () -> r.ints(10, 5, 5));
        tryRecord(w, k + "ints(-1,0,5)", () -> r.ints(-1, 0, 5));
        tryRecord(w, k + "longs(-1)", () -> r.longs(-1));
        tryRecord(w, k + "longs(5,5)", () -> r.longs(5, 5));
        tryRecord(w, k + "longs(10,5,5)", () -> r.longs(10, 5, 5));
        tryRecord(w, k + "doubles(-1)", () -> r.doubles(-1));
        tryRecord(w, k + "doubles(1,1)", () -> r.doubles(1d, 1d));
        tryRecord(w, k + "doubles(10,0,inf)", () -> r.doubles(10, 0d, Double.POSITIVE_INFINITY));
        tryRecord(w, k + "doubles(10,NaN,1)", () -> r.doubles(10, Double.NaN, 1d));
        tryRecord(w, k + "equiDoubles(NaN,1,T,T)", () -> r.equiDoubles(Double.NaN, 1d, true, true));
        tryRecord(w, k + "equiDoubles(-inf,0,T,T)", () -> r.equiDoubles(Double.NEGATIVE_INFINITY, 0d, true, true));
        tryRecord(w, k + "equiDoubles(0,inf,T,T)", () -> r.equiDoubles(0d, Double.POSITIVE_INFINITY, true, true));
        tryRecord(w, k + "equiDoubles(0,NaN,T,T)", () -> r.equiDoubles(0d, Double.NaN, true, true));
        tryRecord(w, k + "equiDoubles(1,0,T,T)", () -> r.equiDoubles(1d, 0d, true, true));
        tryRecord(w, k + "equiDoubles(1,1,F,T)", () -> r.equiDoubles(1d, 1d, false, true));
        tryRecord(w, k + "equiDoubles(1,1,T,F)", () -> r.equiDoubles(1d, 1d, true, false));
        tryRecord(w, k + "equiDoubles(1,up1,F,F)", () -> r.equiDoubles(1d, Math.nextUp(1d), false, false));
        tryRecord(w, k + "setSeed(1)", () -> r.setSeed(1));
        tryRecord(w, k + "setSeed(0)", () -> r.setSeed(0));
      }
      // AssertingRandom.next(int) is unreachable by design.
      AssertingRandom a = new AssertingRandom(Thread.currentThread(), new Xoroshiro128PlusRandom(1));
      tryRecord(w, "asserting\tnext(32)", () -> a.next(32));
      // Foreign thread.
      final Throwable[] box = new Throwable[1];
      Thread t = new Thread(() -> {
        try { a.nextInt(); } catch (Throwable e) { box[0] = e; }
      }, "foreign-thread");
      t.start(); t.join();
      w.println("asserting\tforeign-thread\t" + err(box[0]));
      w.println("asserting\tforeign-thread-cause\t" + err(box[0].getCause()));
      // Destroyed.
      a.destroy();
      tryRecord(w, "asserting\tdestroyed-nextInt", () -> a.nextInt());
      // Thread formatting.
      w.println("thread\tThread.toString\t" + Thread.currentThread().toString());
      w.println("thread\tThreads.threadName\t" + Threads.threadName(Thread.currentThread()));
      w.println("thread\tisVerifying\t" + AssertingRandom.isVerifying());
      Thread nt = new Thread(() -> {}, "unstarted");
      w.println("thread\tThreads.threadName(new)\t" + Threads.threadName(nt));
      w.println("thread\tThread.toString(new)\t" + nt.toString());
      nt.start(); nt.join();
      w.println("thread\tThreads.threadName(terminated)\t" + Threads.threadName(nt));
      w.println("thread\tThread.toString(terminated)\t" + nt.toString());
    }
  }

  static void strictMathLog(Path out) throws Exception {
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("strict_math_log.tsv")))) {
      w.println("# input bits\toutput bits (edge values one per line; then digests)");
      double[] edges = {0d, -0d, -1d, Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, 1d, 2d, 0.5d,
          Double.MIN_VALUE, Double.MIN_NORMAL, Double.MAX_VALUE, 1d + Math.ulp(1d), 1d - Math.ulp(1d) / 2,
          1d + 1e-7, 1d - 1e-7, 1d + 9e-7, Math.E, 10d, 0.1d, 1e-310, 1.4142135623730951d, 0.7071067811865476d,
          3.0e-5, 123456.789, 1e300};
      for (double x : edges) {
        w.println("edge\t" + Long.toHexString(Double.doubleToRawLongBits(x)) + "\t" + Long.toHexString(Double.doubleToRawLongBits(StrictMath.log(x))));
      }
      for (long seed : seeds()) {
        Random r = new Xoroshiro128PlusRandom(seed);
        Rec rec = new Rec();
        for (int k = 0; k < DRAWS; k++) {
          double s = r.nextDouble();
          rec.d(StrictMath.log(s));
          // Values just around 1 and across binades, to reach every branch.
          rec.d(StrictMath.log(1d + (s - 0.5) * 1e-6));
          rec.d(StrictMath.log(Double.longBitsToDouble(r.nextLong() >>> 1)));
        }
        w.println("digest\t" + Long.toHexString(seed) + "\t" + rec.line());
      }
    }
  }

  // ---------------------------------------------------------------------------
  // java.lang behaviour the port reimplements (parsing system properties,
  // String.hashCode, Character.digit).
  // ---------------------------------------------------------------------------

  static final String[] PROPERTY_VALUES = {
    "", " ", "1", " 42 ", "+7", "-0", "0x10", "1e3", "1.5", "NaN", "-Infinity", "Infinity", "inf", "infinity",
    "1d", "1f", "1.5F", "1.5D", "0x1p3", "0x1.8p1", "0x.8p0", "0X1P-1074", "0x1p-1075", "0x1p-1076", "0x1.8p-1074",
    "\u0663\u0664", "\uFF11\uFF12", "\u0967", "2147483647", "2147483648", "-2147483648", "-2147483649",
    "9223372036854775807", "9223372036854775808", "-9223372036854775808", "1_000", "\t5\n", "\u00a05",
    "\u00075\u001f", "1e400", "-1e400", "4.9e-324", "2.4703282292062328e-324", "2.4703282292062327e-324",
    "1.0000000000000001110223024625156540423631668090820312500001", "0.1", "true", "false", "on", "OFF", "Yes",
    "enabled", "disabled", "TRUE", " no ", "off", "yes", "1.", ".5", ".", "e5", "1e", "1e+", "1e+5", "1E-5",
    "--1", "+-1", "0x", "0x1", "0x1.p1", "0x1P", "1e5d", "Infinityd", "+NaN", "-NaN", "NaNf", "3.4028235e38",
    "3.4028236e38", "3.4028235677973366e38", "1e-46", "1.4e-45", "7.0e-46", "7.006492321624085e-46",
    "0x1.fffffeP127", "0x1.ffffffP127", "0x1.000001p0", "0x1.0000018p0", "0x1.0000010000000001p0",
    "0x1.00000000000008p0", "0x1.00000000000018p0", "0x1.000000000000081p0", "0xFFFFFFFFFFFFFFFFFFFFp0",
    "0x0p0", "-0x0.0p0", "00012", "-00", "+0", "+", "-", "12a", " 1 2 ", "0.0000001e7", "1e-400", "1e2147483648",
    "1e-2147483649", "123456789012345678901234567890", "1.7976931348623157e308", "1.7976931348623158e308",
    "1.797693134862315807e308", "0X1.FFFFFFFFFFFFFP1023", "0x1.fffffffffffff8p1023", "0x1.fffffffffffff7ffp1023",
    "\u0660", "\u06F0\u06F1", "\uD835\uDFCE"
  };

  static void javaLang(Path out) throws Exception {
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("java_lang.tsv")))) {
      w.println("# kind\tinput units\tresult");
      String prop = "rucene.oracle.prop";
      for (String v : PROPERTY_VALUES) {
        System.setProperty(prop, v);
        String u = units(v);
        try { w.println("double\t" + u + "\tOK\t" + Long.toHexString(Double.doubleToRawLongBits(RandomizedTest.systemPropertyAsDouble(prop, -2.5)))); }
        catch (Throwable e) { w.println("double\t" + u + "\t" + err(e)); }
        try { w.println("float\t" + u + "\tOK\t" + Integer.toHexString(Float.floatToRawIntBits(RandomizedTest.systemPropertyAsFloat(prop, -2.5f)))); }
        catch (Throwable e) { w.println("float\t" + u + "\t" + err(e)); }
        try { w.println("int\t" + u + "\tOK\t" + RandomizedTest.systemPropertyAsInt(prop, -3)); }
        catch (Throwable e) { w.println("int\t" + u + "\t" + err(e)); }
        try { w.println("long\t" + u + "\tOK\t" + Integer.toHexString(Float.floatToRawIntBits(RandomizedTest.systemPropertyAsLong(prop, -3)))); }
        catch (Throwable e) { w.println("long\t" + u + "\t" + err(e)); }
        try { w.println("boolean\t" + u + "\tOK\t" + RandomizedTest.systemPropertyAsBoolean(prop, true)); }
        catch (Throwable e) { w.println("boolean\t" + u + "\t" + err(e)); }
      }
      System.clearProperty(prop);
      w.println("double\tunset\tOK\t" + Long.toHexString(Double.doubleToRawLongBits(RandomizedTest.systemPropertyAsDouble(prop, -2.5))));
      w.println("float\tunset\tOK\t" + Integer.toHexString(Float.floatToRawIntBits(RandomizedTest.systemPropertyAsFloat(prop, -2.5f))));
      w.println("int\tunset\tOK\t" + RandomizedTest.systemPropertyAsInt(prop, -3));
      w.println("long\tunset\tOK\t" + Integer.toHexString(Float.floatToRawIntBits(RandomizedTest.systemPropertyAsLong(prop, -3))));
      w.println("boolean\tunset\tOK\t" + RandomizedTest.systemPropertyAsBoolean(prop, true));
      // String.hashCode, MixWithSuiteName and MurmurHash3.
      String[] names = {"", "a", "testAlpha", "test", "com.carrotsearch.randomizedtesting.RandomizedTestingOracle$MixedSuite",
          "\u00e9\u4e2d\uD83D\uDE00", "org.apache.lucene.index.TestIndexWriter"};
      for (String n : names) {
        MixWithSuiteName mix = new MixWithSuiteName();
        w.println("hashCode\t" + units(n) + "\t" + n.hashCode() + "\t" + MurmurHash3.hash((long) n.hashCode()));
      }
      for (long v : new long[] {0, 1, -1, Long.MIN_VALUE, Long.MAX_VALUE, 0xdeadbeefL, 42}) {
        w.println("murmur64\t" + v + "\t" + MurmurHash3.hash(v));
      }
      for (int v : new int[] {0, 1, -1, Integer.MIN_VALUE, Integer.MAX_VALUE, 0xdeadbeef, 42}) {
        w.println("murmur32\t" + v + "\t" + MurmurHash3.hash(v));
      }
    }
    // BMP zero digits of every DECIMAL_DIGIT_NUMBER run, for Character.digit(ch, 10).
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("bmp_decimal_zeros.txt")))) {
      for (int c = 0; c <= 0xFFFF; c++) {
        if (Character.digit((char) c, 10) == 0) w.println(String.format("%04X", c));
      }
      // Sanity: every char with a digit value is zero + value of a listed zero.
      for (int c = 0; c <= 0xFFFF; c++) {
        int d = Character.digit((char) c, 10);
        if (d > 0 && Character.digit((char) (c - d), 10) != 0) throw new AssertionError(Integer.toHexString(c));
      }
    }
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("available_locales.txt")))) {
      Locale[] all = Locale.getAvailableLocales();
      Arrays.sort(all, (a, b) -> a.toString().compareTo(b.toString()));
      for (Locale l : all) w.println(l.toString() + "\t" + l.toLanguageTag());
    }
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("available_time_zones.txt")))) {
      String[] ids = TimeZone.getAvailableIDs();
      Arrays.sort(ids);
      for (String id : ids) w.println(id);
    }
  }

  static void seedUtils(Path out) throws Exception {
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("seed_utils.tsv")))) {
      w.println("# kind\tinput\tresult");
      for (long s : seeds()) w.println("formatSeed\t" + s + "\t" + SeedUtils.formatSeed(s));
      String[] parse = {"0", "F", "f", "DEADBEEF", "deadbeef", "FFFFFFFFFFFFFFFF", "10000000000000000", "8000000000000000",
          "123456789abcdef0", "", "x", "12G", "-1", " 1", "\u0130", "ABCDEFabcdef0123456789"};
      for (String p : parse) {
        try { w.println("parseSeed\t" + units(p) + "\tOK\t" + SeedUtils.parseSeed(p)); }
        catch (Throwable e) { w.println("parseSeed\t" + units(p) + "\t" + err(e)); }
      }
      String[] chains = {"[DEADBEEF]", "[DEADBEEF:CAFEBABE]", "DEADBEEF:CAFEBABE", "[A]:[B]", "[]", "", ":", "A::B", "[A:B:C]",
          "[a-b]", "[[1]]", "1:2:", ":1", "[x]", "A B"};
      for (String c : chains) {
        try { w.println("parseSeedChain\t" + units(c) + "\tOK\t" + Arrays.toString(SeedUtils.parseSeedChain(c))); }
        catch (Throwable e) { w.println("parseSeedChain\t" + units(c) + "\t" + err(e)); }
      }
      long[] sd = seeds();
      for (int i = 0; i + 2 < sd.length; i += 3) {
        Randomness a = new Randomness(sd[i], RandomSupplier.DEFAULT);
        Randomness b = new Randomness(sd[i + 1], RandomSupplier.DEFAULT);
        Randomness c = new Randomness(sd[i + 2], RandomSupplier.DEFAULT);
        w.println("formatSeedChain\t" + sd[i] + "," + sd[i + 1] + "," + sd[i + 2] + "\t" + SeedUtils.formatSeedChain(a, b, c)
            + "\t" + SeedUtils.formatSeedChain(a) + "\t" + SeedUtils.formatSeedChain() + "\t" + a.toString());
      }
      // Randomness with decorators: the random follows the decorated seed, getSeed the plain one.
      for (long s : sd) {
        MixWithSuiteName mix = new MixWithSuiteName();
        mix.initialize(MixedSuite.class);
        Randomness r = new Randomness(s, RandomSupplier.DEFAULT, mix, mix);
        Rec rec = new Rec();
        for (int k = 0; k < 100; k++) rec.l(r.getRandom().nextLong());
        w.println("decorated\t" + s + "\t" + r.getSeed() + "\t" + mix.decorate(s) + "\t" + rec.line());
      }
    }
  }

  // ---------------------------------------------------------------------------
  // Seed chain under RandomizedRunner.
  // ---------------------------------------------------------------------------

  static final List<String> LOG = Collections.synchronizedList(new ArrayList<>());

  static void recordContext(String suite, String where) {
    RandomizedContext ctx = RandomizedContext.current();
    Random r = ctx.getRandom();
    Rec rec = new Rec();
    rec.l(r.nextLong()); rec.i(r.nextInt(100)); rec.d(r.nextDouble()); rec.d(r.nextGaussian());
    for (int k = 0; k < 96; k++) rec.l(r.nextLong());
    LOG.add(System.getProperty("tests.seed") + "\t" + suite + "\t" + where + "\t" + ctx.getRunnerSeedAsString() + "\t"
        + ctx.getRandomness() + "\t" + SeedUtils.formatSeedChain(ctx.getRandomnesses()) + "\t"
        + ctx.getTargetClass().getName() + "\t" + (ctx.getTargetMethod() == null ? "-" : ctx.getTargetMethod().getName()) + "\t"
        + rec.line());
  }

  @RunWith(RandomizedRunner.class)
  public static class PlainSuite {
    @BeforeClass public static void beforeClass() { recordContext("PlainSuite", "beforeClass"); }
    @AfterClass public static void afterClass() { recordContext("PlainSuite", "afterClass"); }
    @Test public void testAlpha() { recordContext("PlainSuite", "testAlpha"); }
    @Test public void testBeta() { recordContext("PlainSuite", "testBeta"); }
    @Test public void test() { recordContext("PlainSuite", "test"); }
  }

  @RunWith(RandomizedRunner.class)
  @SeedDecorators({MixWithSuiteName.class})
  public static class MixedSuite {
    @BeforeClass public static void beforeClass() { recordContext("MixedSuite", "beforeClass"); }
    @Test public void testAlpha() { recordContext("MixedSuite", "testAlpha"); }
    @Test public void testBeta() { recordContext("MixedSuite", "testBeta"); }
  }

  static final String[] MASTER_SEEDS = {"0", "1", "FFFFFFFFFFFFFFFF", "8000000000000000", "7FFFFFFFFFFFFFFF", "DEADBEEF",
      "1234567890ABCDEF", "[CAFEBABE]", "DEADBEEF:CAFEBABE"};

  static void seedChain(Path out) throws Exception {
    for (String seed : MASTER_SEEDS) {
      System.setProperty("tests.seed", seed);
      for (Class<?> c : new Class<?>[] {PlainSuite.class, MixedSuite.class}) {
        Result res = JUnitCore.runClasses(c);
        for (Failure f : res.getFailures()) throw new AssertionError(f.getTrace());
      }
    }
    System.clearProperty("tests.seed");
    List<String> lines = new ArrayList<>(LOG);
    Collections.sort(lines);
    lines.add(0, "# master\tsuite\twhere\trunnerSeed\trandomness\tchain\ttargetClass\ttargetMethod\tfnv64\tfirst");
    Files.write(out.resolve("seed_chain.tsv"), lines, StandardCharsets.UTF_8);
    LOG.clear();
  }

  // ---------------------------------------------------------------------------
  // Resource disposal and context errors under RandomizedRunner.
  // ---------------------------------------------------------------------------

  static final class Named implements Closeable {
    final String name; final boolean fail;
    Named(String name, boolean fail) { this.name = name; this.fail = fail; }
    @Override public void close() throws IOException {
      LOG.add("close:" + name);
      if (fail) throw new IOException("boom " + name);
    }
  }

  static RandomizedContext savedContext;
  static Random savedTestRandom;

  @RunWith(RandomizedRunner.class)
  public static class ResourceSuite {
    @BeforeClass public static void beforeClass() {
      LOG.add("beforeClass");
      RandomizedContext ctx = RandomizedContext.current();
      ctx.closeAtEnd(new Named("suite-1", false), LifecycleScope.SUITE);
      ctx.closeAtEnd(new Named("test-from-beforeClass", false), LifecycleScope.TEST);
      savedContext = ctx;
    }
    @Before public void before() { LOG.add("before"); }
    @Test public void testOne() {
      LOG.add("testOne");
      RandomizedContext ctx = RandomizedContext.current();
      Named a = ctx.closeAtEnd(new Named("test-a", false), LifecycleScope.TEST);
      ctx.closeAtEnd(new Named("suite-b", false), LifecycleScope.SUITE);
      ctx.closeAtEnd(new Named("test-c", false), LifecycleScope.TEST);
      LOG.add("returned-same:" + (a.name.equals("test-a")));
      savedTestRandom = ctx.getRandom();
    }
    @After public void after() { LOG.add("after"); }
    @AfterClass public static void afterClass() {
      LOG.add("afterClass");
      try { savedTestRandom.nextInt(); LOG.add("saved random usable"); }
      catch (Throwable t) { LOG.add("saved random: " + err(t)); }
    }
  }

  @RunWith(RandomizedRunner.class)
  public static class FailingResourceSuite {
    @Test public void testFailing() {
      RandomizedContext ctx = RandomizedContext.current();
      ctx.closeAtEnd(new Named("fails", true), LifecycleScope.TEST);
      ctx.closeAtEnd(new Named("after-fails", false), LifecycleScope.TEST);
    }
  }

  @RunWith(RandomizedRunner.class)
  public static class PrivateRandomnessSuite {
    @Test public void testPrivate() throws Exception {
      RandomizedContext ctx = RandomizedContext.current();
      String before = ctx.getRandomness().toString();
      String inside = ctx.runWithPrivateRandomness(0xCAFEL, () -> {
        Rec rec = new Rec();
        for (int k = 0; k < 100; k++) rec.l(RandomizedContext.current().getRandom().nextLong());
        return RandomizedContext.current().getRandomness() + "\t" + SeedUtils.formatSeedChain(ctx.getRandomnesses()) + "\t" + rec.line();
      });
      LOG.add("private\t" + before + "\t" + inside + "\t" + ctx.getRandomness());
    }
  }

  static void contextAndResources(Path out) throws Exception {
    System.setProperty("tests.seed", "DEADBEEF");
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("context.tsv")))) {
      w.println("# kind\tvalue");
      try { RandomizedContext.current(); w.println("outside\tOK"); }
      catch (Throwable t) { w.println("outside\t" + err(t)); }

      LOG.clear();
      Result res = JUnitCore.runClasses(ResourceSuite.class);
      for (Failure f : res.getFailures()) w.println("resource-failure\t" + err(f.getException()));
      for (String e : LOG) w.println("resource-event\t" + e);
      try { savedContext.getTargetClass(); w.println("disposed\tOK"); }
      catch (Throwable t) { w.println("disposed\t" + err(t)); }
      try { savedContext.getRandomness(); w.println("disposed-getRandomness\tOK"); }
      catch (Throwable t) { w.println("disposed-getRandomness\t" + err(t)); }

      LOG.clear();
      res = JUnitCore.runClasses(FailingResourceSuite.class);
      for (Failure f : res.getFailures()) {
        w.println("failing-failure\t" + err(f.getException()));
        Throwable c = f.getException().getCause();
        w.println("failing-cause\t" + (c == null ? "-" : err(c)));
      }
      for (String e : LOG) w.println("failing-event\t" + e);

      LOG.clear();
      res = JUnitCore.runClasses(PrivateRandomnessSuite.class);
      for (Failure f : res.getFailures()) throw new AssertionError(f.getTrace());
      for (String e : LOG) w.println(e);
    }
    System.clearProperty("tests.seed");
  }

  // ---------------------------------------------------------------------------
  // Generators.
  // ---------------------------------------------------------------------------

  static List<Labeled> generatorMethods() {
    List<Labeled> m = new ArrayList<>();
    int[][] ints = {{0, 0}, {0, 10}, {-10, 10}, {Integer.MIN_VALUE, Integer.MAX_VALUE}, {Integer.MIN_VALUE, 0},
        {0, Integer.MAX_VALUE}, {-1, Integer.MAX_VALUE}, {Integer.MIN_VALUE + 1, Integer.MAX_VALUE}, {5, 5},
        {Integer.MAX_VALUE - 1, Integer.MAX_VALUE}, {-1, Integer.MAX_VALUE - 1}};
    for (int[] p : ints) m.add(new Labeled("randomIntBetween(" + p[0] + "," + p[1] + ")", (r, rec) -> rec.i(RandomNumbers.randomIntBetween(r, p[0], p[1]))));
    long[][] longs = {{0, 0}, {0, 10}, {-10, 10}, {Long.MIN_VALUE, Long.MAX_VALUE}, {Long.MIN_VALUE, 0}, {Long.MIN_VALUE, 1},
        {-1, Long.MAX_VALUE}, {0, Long.MAX_VALUE}, {Long.MIN_VALUE + 1, Long.MAX_VALUE}, {Long.MIN_VALUE, -1}, {-2, Long.MAX_VALUE}};
    for (long[] p : longs) m.add(new Labeled("randomLongBetween(" + p[0] + "," + p[1] + ")", (r, rec) -> rec.l(RandomNumbers.randomLongBetween(r, p[0], p[1]))));
    for (long n : new long[] {1, 2, 3, 1000, 1L << 40, Long.MAX_VALUE}) m.add(new Labeled("nextLong(" + n + ")", (r, rec) -> rec.l(RandomNumbers.nextLong(r, n))));
    double inf = Double.POSITIVE_INFINITY;
    double[][] ds = {{0, 1}, {-1, 1}, {-inf, inf}, {0, inf}, {-inf, 0}, {1, 1}, {-Double.MAX_VALUE, Double.MAX_VALUE},
        {Double.MIN_VALUE, 1}, {-5, -4}, {0, 0}, {1e-310, 1e-300}, {-inf, -inf}, {inf, inf}, {-0.0, 0.0}};
    for (double[] p : ds) {
      String lbl = Long.toHexString(Double.doubleToRawLongBits(p[0])) + "," + Long.toHexString(Double.doubleToRawLongBits(p[1]));
      m.add(new Labeled("randomDoubleBetween(" + lbl + ")", (r, rec) -> rec.d(BiasedNumbers.randomDoubleBetween(r, p[0], p[1]))));
      m.add(new Labeled("randomFloatBetween(" + lbl + ")", (r, rec) -> rec.f(BiasedNumbers.randomFloatBetween(r, (float) p[0], (float) p[1]))));
    }
    double[][] fuzz = {{0, 1}, {1, 1}, {-1, 0}, {0, Double.MIN_VALUE}, {Double.MAX_VALUE, inf}, {-inf, 0}};
    for (double[] p : fuzz) {
      String lbl = Long.toHexString(Double.doubleToRawLongBits(p[0])) + "," + Long.toHexString(Double.doubleToRawLongBits(p[1]));
      m.add(new Labeled("fuzzUp(" + lbl + ")", (r, rec) -> rec.d(BiasedNumbers.fuzzUp(r, p[0], p[1]))));
      m.add(new Labeled("fuzzDown(" + lbl + ")", (r, rec) -> rec.d(BiasedNumbers.fuzzDown(r, p[1], p[0]))));
      m.add(new Labeled("fuzzUpF(" + lbl + ")", (r, rec) -> rec.f(BiasedNumbers.fuzzUp(r, (float) p[0], (float) p[1]))));
      m.add(new Labeled("fuzzDownF(" + lbl + ")", (r, rec) -> rec.f(BiasedNumbers.fuzzDown(r, (float) p[1], (float) p[0]))));
    }
    for (int n : new int[] {0, 1, 5, 100}) m.add(new Labeled("randomBytesOfLength(" + n + ")", (r, rec) -> rec.bytes(RandomBytes.randomBytesOfLength(r, n))));
    int[][] blen = {{0, 10}, {3, 3}, {0, 1000}};
    for (int[] p : blen) m.add(new Labeled("randomBytesOfLengthBetween(" + p[0] + "," + p[1] + ")", (r, rec) -> rec.bytes(RandomBytes.randomBytesOfLengthBetween(r, p[0], p[1]))));
    for (int size : new int[] {1, 7, 100}) {
      Integer[] objs = new Integer[size]; int[] is = new int[size]; long[] ls = new long[size]; byte[] bs = new byte[size];
      short[] ss = new short[size]; char[] cs = new char[size]; float[] fs = new float[size]; double[] dd = new double[size];
      for (int k = 0; k < size; k++) { objs[k] = k * 3; is[k] = k * 5; ls[k] = k * 7L; bs[k] = (byte) k; ss[k] = (short) (k * 11); cs[k] = (char) ('a' + k); fs[k] = k / 2f; dd[k] = k / 4d; }
      List<Integer> list = new ArrayList<>(Arrays.asList(objs));
      Collection<Integer> coll = list;
      m.add(new Labeled("randomFrom(Object[" + size + "])", (r, rec) -> rec.i(RandomPicks.randomFrom(r, objs))));
      m.add(new Labeled("randomFrom(List[" + size + "])", (r, rec) -> rec.i(RandomPicks.randomFrom(r, list))));
      m.add(new Labeled("randomFrom(Collection[" + size + "])", (r, rec) -> rec.i(RandomPicks.randomFrom(r, coll))));
      m.add(new Labeled("randomFrom(int[" + size + "])", (r, rec) -> rec.i(RandomPicks.randomFrom(r, is))));
      m.add(new Labeled("randomFrom(long[" + size + "])", (r, rec) -> rec.l(RandomPicks.randomFrom(r, ls))));
      m.add(new Labeled("randomFrom(byte[" + size + "])", (r, rec) -> rec.i(RandomPicks.randomFrom(r, bs))));
      m.add(new Labeled("randomFrom(short[" + size + "])", (r, rec) -> rec.i(RandomPicks.randomFrom(r, ss))));
      m.add(new Labeled("randomFrom(char[" + size + "])", (r, rec) -> rec.i(RandomPicks.randomFrom(r, cs))));
      m.add(new Labeled("randomFrom(float[" + size + "])", (r, rec) -> rec.f(RandomPicks.randomFrom(r, fs))));
      m.add(new Labeled("randomFrom(double[" + size + "])", (r, rec) -> rec.d(RandomPicks.randomFrom(r, dd))));
    }
    int[][] lens = {{0, 0}, {0, 10}, {5, 5}, {1, 20}, {0, 100}};
    for (int[] p : lens) {
      String a = p[0] + "," + p[1];
      m.add(new Labeled("randomAsciiOfLengthBetween(" + a + ")", (r, rec) -> rec.s(RandomStrings.randomAsciiOfLengthBetween(r, p[0], p[1]))));
      m.add(new Labeled("randomAsciiLettersOfLengthBetween(" + a + ")", (r, rec) -> rec.s(RandomStrings.randomAsciiLettersOfLengthBetween(r, p[0], p[1]))));
      m.add(new Labeled("randomAsciiAlphanumOfLengthBetween(" + a + ")", (r, rec) -> rec.s(RandomStrings.randomAsciiAlphanumOfLengthBetween(r, p[0], p[1]))));
      m.add(new Labeled("randomUnicodeOfLengthBetween(" + a + ")", (r, rec) -> rec.s(RandomStrings.randomUnicodeOfLengthBetween(r, p[0], p[1]))));
      m.add(new Labeled("randomUnicodeOfCodepointLengthBetween(" + a + ")", (r, rec) -> rec.s(RandomStrings.randomUnicodeOfCodepointLengthBetween(r, p[0], p[1]))));
      m.add(new Labeled("randomRealisticUnicodeOfLengthBetween(" + a + ")", (r, rec) -> rec.s(RandomStrings.randomRealisticUnicodeOfLengthBetween(r, p[0], p[1]))));
      m.add(new Labeled("randomRealisticUnicodeOfCodepointLengthBetween(" + a + ")", (r, rec) -> rec.s(RandomStrings.randomRealisticUnicodeOfCodepointLengthBetween(r, p[0], p[1]))));
      m.add(new Labeled("ofUtf8Length(" + a + ")", (r, rec) -> rec.s(RandomStrings.unicodeGenerator.ofUtf8Length(r, p[0], p[1]))));
      m.add(new Labeled("ofStringLength(" + a + ")", (r, rec) -> rec.s(RandomStrings.realisticUnicodeGenerator.ofStringLength(r, p[0], p[1]))));
      m.add(new Labeled("ASCIIGenerator(" + a + ")", (r, rec) -> rec.s(new ASCIIGenerator().ofCodeUnitsLength(r, p[0], p[1]))));
      m.add(new Labeled("asciiGenerator.ofCodePointsLength(" + a + ")", (r, rec) -> rec.s(RandomStrings.asciiGenerator.ofCodePointsLength(r, p[0], p[1]))));
    }
    for (int n : new int[] {0, 1, 7, 30}) {
      m.add(new Labeled("randomAsciiOfLength(" + n + ")", (r, rec) -> rec.s(RandomStrings.randomAsciiOfLength(r, n))));
      m.add(new Labeled("randomAsciiLettersOfLength(" + n + ")", (r, rec) -> rec.s(RandomStrings.randomAsciiLettersOfLength(r, n))));
      m.add(new Labeled("randomAsciiAlphanumOfLength(" + n + ")", (r, rec) -> rec.s(RandomStrings.randomAsciiAlphanumOfLength(r, n))));
      m.add(new Labeled("randomUnicodeOfLength(" + n + ")", (r, rec) -> rec.s(RandomStrings.randomUnicodeOfLength(r, n))));
      m.add(new Labeled("randomUnicodeOfCodepointLength(" + n + ")", (r, rec) -> rec.s(RandomStrings.randomUnicodeOfCodepointLength(r, n))));
      m.add(new Labeled("randomRealisticUnicodeOfLength(" + n + ")", (r, rec) -> rec.s(RandomStrings.randomRealisticUnicodeOfLength(r, n))));
      m.add(new Labeled("randomRealisticUnicodeOfCodepointLength(" + n + ")", (r, rec) -> rec.s(RandomStrings.randomRealisticUnicodeOfCodepointLength(r, n))));
    }
    CodepointSetGenerator fromChars = new CodepointSetGenerator(new char[] {'a', 'b', 'c', '\u00e9', '\uffff'});
    CodepointSetGenerator mixed = new CodepointSetGenerator("ab\uD83D\uDE00\uD834\uDD1Ec\u4e2d");
    CodepointSetGenerator suppOnly = new CodepointSetGenerator("\uD83D\uDE00\uD834\uDD1E");
    int[][] cps = {{0, 0}, {0, 10}, {3, 3}, {1, 20}, {3, 4}, {4, 4}, {3, 9}};
    for (int[] p : cps) {
      String a = p[0] + "," + p[1];
      m.add(new Labeled("fromChars.units(" + a + ")", (r, rec) -> rec.s(fromChars.ofCodeUnitsLength(r, p[0], p[1]))));
      m.add(new Labeled("fromChars.points(" + a + ")", (r, rec) -> rec.s(fromChars.ofCodePointsLength(r, p[0], p[1]))));
      m.add(new Labeled("mixed.units(" + a + ")", (r, rec) -> rec.s(mixed.ofCodeUnitsLength(r, p[0], p[1]))));
      m.add(new Labeled("mixed.points(" + a + ")", (r, rec) -> rec.s(mixed.ofCodePointsLength(r, p[0], p[1]))));
      if (p[0] != p[1] || (p[0] & 1) == 0) {
        m.add(new Labeled("suppOnly.units(" + a + ")", (r, rec) -> rec.s(suppOnly.ofCodeUnitsLength(r, p[0], p[1]))));
      }
      m.add(new Labeled("suppOnly.points(" + a + ")", (r, rec) -> rec.s(suppOnly.ofCodePointsLength(r, p[0], p[1]))));
    }
    return m;
  }

  static final int GEN_DRAWS = 500;

  static void generators(Path out) throws Exception {
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("generators.tsv")))) {
      w.println("# seed\tmethod\tfnv64\tfirst values");
      for (long seed : seeds()) {
        for (Labeled l : generatorMethods()) {
          Random r = new Xoroshiro128PlusRandom(seed);
          Rec rec = new Rec();
          for (int k = 0; k < GEN_DRAWS; k++) l.draw.draw(r, rec);
          w.println(Long.toHexString(seed) + "\t" + l.label + "\t" + rec.line());
        }
      }
    }
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("generator_errors.tsv")))) {
      w.println("# call\tclass\tmessage");
      Random r = new Xoroshiro128PlusRandom(3);
      tryRecord(w, "nextLong(0)", () -> RandomNumbers.nextLong(r, 0));
      tryRecord(w, "nextLong(-5)", () -> RandomNumbers.nextLong(r, -5));
      tryRecord(w, "randomIntBetween(5,4)", () -> RandomNumbers.randomIntBetween(r, 5, 4));
      tryRecord(w, "randomLongBetween(5,4)", () -> RandomNumbers.randomLongBetween(r, 5, 4));
      tryRecord(w, "randomDoubleBetween(1,0.5)", () -> BiasedNumbers.randomDoubleBetween(r, 1d, 0.5d));
      tryRecord(w, "randomDoubleBetween(0,NaN)", () -> BiasedNumbers.randomDoubleBetween(r, 0d, Double.NaN));
      tryRecord(w, "randomDoubleBetween(NaN,NaN)", () -> BiasedNumbers.randomDoubleBetween(r, Double.NaN, Double.NaN));
      tryRecord(w, "randomDoubleBetween(1e-5,1e7)", () -> BiasedNumbers.randomDoubleBetween(r, 1e7, 1e-5));
      tryRecord(w, "randomFloatBetween(1,0.5)", () -> BiasedNumbers.randomFloatBetween(r, 1f, 0.5f));
      tryRecord(w, "randomFloatBetween(3e10,1.25e-4)", () -> BiasedNumbers.randomFloatBetween(r, 3e10f, 1.25e-4f));
      tryRecord(w, "randomFloatBetween(0,NaN)", () -> BiasedNumbers.randomFloatBetween(r, 0f, Float.NaN));
      tryRecord(w, "fuzzUp(2,1)", () -> BiasedNumbers.fuzzUp(r, 2d, 1d));
      tryRecord(w, "fuzzDown(1,2)", () -> BiasedNumbers.fuzzDown(r, 1d, 2d));
      tryRecord(w, "fuzzUpF(2,1)", () -> BiasedNumbers.fuzzUp(r, 2f, 1f));
      tryRecord(w, "fuzzDownF(1,2)", () -> BiasedNumbers.fuzzDown(r, 1f, 2f));
      tryRecord(w, "randomFrom(Object[0])", () -> RandomPicks.randomFrom(r, new Object[0]));
      tryRecord(w, "randomFrom(int[0])", () -> RandomPicks.randomFrom(r, new int[0]));
      tryRecord(w, "randomFrom(List[0])", () -> RandomPicks.randomFrom(r, new ArrayList<Object>()));
      tryRecord(w, "randomFrom(Collection[0])", () -> RandomPicks.randomFrom(r, (Collection<Object>) new ArrayList<Object>()));
      tryRecord(w, "CodepointSetGenerator(char[0])", () -> new CodepointSetGenerator(new char[0]));
      tryRecord(w, "CodepointSetGenerator(\"\")", () -> new CodepointSetGenerator(""));
      tryRecord(w, "CodepointSetGenerator(surrogate)", () -> new CodepointSetGenerator(new char[] {'a', '\ud800'}));
      tryRecord(w, "CodepointSetGenerator(low surrogate)", () -> new CodepointSetGenerator(new char[] {'\udfff'}));
      CodepointSetGenerator suppOnly = new CodepointSetGenerator("\uD83D\uDE00");
      tryRecord(w, "suppOnly.units(3,3)", () -> suppOnly.ofCodeUnitsLength(r, 3, 3));
      tryRecord(w, "randomBytesOfLength(-1)", () -> RandomBytes.randomBytesOfLength(r, -1));
      tryRecord(w, "ofCodeUnitsLength(5,4)", () -> RandomStrings.randomUnicodeOfLengthBetween(r, 5, 4));
    }
  }

  // ---------------------------------------------------------------------------
  // RandomizedTest helpers under RandomizedRunner.
  // ---------------------------------------------------------------------------

  static List<Labeled> helperMethods() {
    List<Labeled> m = new ArrayList<>();
    m.add(new Labeled("randomBoolean", (r, rec) -> rec.z(RandomizedTest.randomBoolean())));
    m.add(new Labeled("randomByte", (r, rec) -> rec.i(RandomizedTest.randomByte())));
    m.add(new Labeled("randomShort", (r, rec) -> rec.i(RandomizedTest.randomShort())));
    m.add(new Labeled("randomInt", (r, rec) -> rec.i(RandomizedTest.randomInt())));
    m.add(new Labeled("randomFloat", (r, rec) -> rec.f(RandomizedTest.randomFloat())));
    m.add(new Labeled("randomDouble", (r, rec) -> rec.d(RandomizedTest.randomDouble())));
    m.add(new Labeled("randomLong", (r, rec) -> rec.l(RandomizedTest.randomLong())));
    m.add(new Labeled("randomGaussian", (r, rec) -> rec.d(RandomizedTest.randomGaussian())));
    m.add(new Labeled("biasedFloatBetween(-1,1)", (r, rec) -> rec.f(RandomizedTest.biasedFloatBetween(-1f, 1f))));
    m.add(new Labeled("biasedDoubleBetween(-inf,inf)", (r, rec) -> rec.d(RandomizedTest.biasedDoubleBetween(Double.NEGATIVE_INFINITY, Double.POSITIVE_INFINITY))));
    m.add(new Labeled("randomBytesOfLength(9)", (r, rec) -> rec.bytes(RandomizedTest.randomBytesOfLength(9))));
    m.add(new Labeled("randomBytesOfLength(0,20)", (r, rec) -> rec.bytes(RandomizedTest.randomBytesOfLength(0, 20))));
    m.add(new Labeled("randomInt(10)", (r, rec) -> rec.i(RandomizedTest.randomInt(10))));
    m.add(new Labeled("randomLong(10)", (r, rec) -> rec.l(RandomizedTest.randomLong(10))));
    m.add(new Labeled("randomIntBetween(-7,7)", (r, rec) -> rec.i(RandomizedTest.randomIntBetween(-7, 7))));
    m.add(new Labeled("between(int)(3,9)", (r, rec) -> rec.i(RandomizedTest.between(3, 9))));
    m.add(new Labeled("randomLongBetween(-7,7)", (r, rec) -> rec.l(RandomizedTest.randomLongBetween(-7, 7))));
    m.add(new Labeled("between(long)(min,max)", (r, rec) -> rec.l(RandomizedTest.between(Long.MIN_VALUE, Long.MAX_VALUE))));
    for (int k : new int[] {0, 1, 10, 1000, Integer.MAX_VALUE}) {
      m.add(new Labeled("atLeast(" + k + ")", (r, rec) -> rec.i(RandomizedTest.atLeast(k))));
      m.add(new Labeled("atMost(" + k + ")", (r, rec) -> rec.i(RandomizedTest.atMost(k))));
    }
    m.add(new Labeled("rarely", (r, rec) -> rec.z(RandomizedTest.rarely())));
    m.add(new Labeled("frequently", (r, rec) -> rec.z(RandomizedTest.frequently())));
    Integer[] objs = {1, 2, 3, 4, 5};
    List<Integer> list = Arrays.asList(objs);
    m.add(new Labeled("randomFrom(Object[])", (r, rec) -> rec.i(RandomizedTest.randomFrom(objs))));
    m.add(new Labeled("randomFrom(List)", (r, rec) -> rec.i(RandomizedTest.randomFrom(list))));
    m.add(new Labeled("randomFrom(byte[])", (r, rec) -> rec.i(RandomizedTest.randomFrom(new byte[] {1, 2, 3}))));
    m.add(new Labeled("randomFrom(short[])", (r, rec) -> rec.i(RandomizedTest.randomFrom(new short[] {1, 2, 3}))));
    m.add(new Labeled("randomFrom(int[])", (r, rec) -> rec.i(RandomizedTest.randomFrom(new int[] {1, 2, 3}))));
    m.add(new Labeled("randomFrom(char[])", (r, rec) -> rec.i(RandomizedTest.randomFrom(new char[] {'x', 'y', 'z'}))));
    m.add(new Labeled("randomFrom(float[])", (r, rec) -> rec.f(RandomizedTest.randomFrom(new float[] {1f, 2f, 3f}))));
    m.add(new Labeled("randomFrom(long[])", (r, rec) -> rec.l(RandomizedTest.randomFrom(new long[] {1, 2, 3}))));
    m.add(new Labeled("randomFrom(double[])", (r, rec) -> rec.d(RandomizedTest.randomFrom(new double[] {1d, 2d, 3d}))));
    m.add(new Labeled("multiplier", (r, rec) -> rec.d(RandomizedTest.multiplier())));
    m.add(new Labeled("isNightly", (r, rec) -> rec.z(RandomizedTest.isNightly())));
    m.add(new Labeled("iterations(1,100)", (r, rec) -> rec.i(RandomizedTest.iterations(1, 100))));
    int[][] scaled = {{0, 0}, {0, 10}, {5, 1000}, {0, Integer.MAX_VALUE}, {Integer.MAX_VALUE, Integer.MAX_VALUE}};
    for (int[] p : scaled) m.add(new Labeled("scaledRandomIntBetween(" + p[0] + "," + p[1] + ")", (r, rec) -> rec.i(RandomizedTest.scaledRandomIntBetween(p[0], p[1]))));
    m.add(new Labeled("randomLocale", (r, rec) -> rec.s(RandomizedTest.randomLocale().toString())));
    m.add(new Labeled("randomTimeZone", (r, rec) -> rec.s(RandomizedTest.randomTimeZone().getID())));
    m.add(new Labeled("randomAsciiOfLengthBetween(0,10)", (r, rec) -> rec.s(RandomizedTest.randomAsciiOfLengthBetween(0, 10))));
    m.add(new Labeled("randomAsciiOfLength(6)", (r, rec) -> rec.s(RandomizedTest.randomAsciiOfLength(6))));
    m.add(new Labeled("randomAsciiLettersOfLengthBetween(0,10)", (r, rec) -> rec.s(RandomizedTest.randomAsciiLettersOfLengthBetween(0, 10))));
    m.add(new Labeled("randomAsciiLettersOfLength(6)", (r, rec) -> rec.s(RandomizedTest.randomAsciiLettersOfLength(6))));
    m.add(new Labeled("randomAsciiAlphanumOfLengthBetween(0,10)", (r, rec) -> rec.s(RandomizedTest.randomAsciiAlphanumOfLengthBetween(0, 10))));
    m.add(new Labeled("randomAsciiAlphanumOfLength(6)", (r, rec) -> rec.s(RandomizedTest.randomAsciiAlphanumOfLength(6))));
    m.add(new Labeled("randomUnicodeOfLengthBetween(0,10)", (r, rec) -> rec.s(RandomizedTest.randomUnicodeOfLengthBetween(0, 10))));
    m.add(new Labeled("randomUnicodeOfLength(6)", (r, rec) -> rec.s(RandomizedTest.randomUnicodeOfLength(6))));
    m.add(new Labeled("randomUnicodeOfCodepointLengthBetween(0,10)", (r, rec) -> rec.s(RandomizedTest.randomUnicodeOfCodepointLengthBetween(0, 10))));
    m.add(new Labeled("randomUnicodeOfCodepointLength(6)", (r, rec) -> rec.s(RandomizedTest.randomUnicodeOfCodepointLength(6))));
    m.add(new Labeled("randomRealisticUnicodeOfLengthBetween(0,10)", (r, rec) -> rec.s(RandomizedTest.randomRealisticUnicodeOfLengthBetween(0, 10))));
    m.add(new Labeled("randomRealisticUnicodeOfLength(6)", (r, rec) -> rec.s(RandomizedTest.randomRealisticUnicodeOfLength(6))));
    m.add(new Labeled("randomRealisticUnicodeOfCodepointLengthBetween(0,10)", (r, rec) -> rec.s(RandomizedTest.randomRealisticUnicodeOfCodepointLengthBetween(0, 10))));
    m.add(new Labeled("randomRealisticUnicodeOfCodepointLength(6)", (r, rec) -> rec.s(RandomizedTest.randomRealisticUnicodeOfCodepointLength(6))));
    return m;
  }

  static final int HELPER_DRAWS = 200;

  @RunWith(RandomizedRunner.class)
  public static class HelpersSuite {
    @Test public void testHelpers() throws Exception {
      String config = System.getProperty("oracle.config");
      for (long seed : seeds()) {
        for (Labeled l : helperMethods()) {
          String line = RandomizedContext.current().runWithPrivateRandomness(seed, () -> {
            Rec rec = new Rec();
            for (int k = 0; k < HELPER_DRAWS; k++) l.draw.draw(null, rec);
            return rec.line();
          });
          LOG.add(config + "\t" + Long.toHexString(seed) + "\t" + l.label + "\t" + line);
        }
      }
      // Errors.
      String[] calls = {"atLeast(-1)", "atMost(-1)", "scaledRandomIntBetween(-1,5)", "scaledRandomIntBetween(5,4)",
          "randomFrom(Object[0])", "randomFrom(List[0])"};
      Thrower[] ts = {() -> RandomizedTest.atLeast(-1), () -> RandomizedTest.atMost(-1),
          () -> RandomizedTest.scaledRandomIntBetween(-1, 5), () -> RandomizedTest.scaledRandomIntBetween(5, 4),
          () -> RandomizedTest.randomFrom(new Object[0]), () -> RandomizedTest.randomFrom(new ArrayList<Object>())};
      for (int k = 0; k < calls.length; k++) {
        try { ts[k].run(); LOG.add(config + "\terror\t" + calls[k] + "\tOK\t-"); }
        catch (Throwable e) { LOG.add(config + "\terror\t" + calls[k] + "\t" + err(e)); }
      }
      if (config.equals("default")) {
        String[] acalls = {"assumeTrue(false)", "assumeFalse(true)", "assumeTrue(msg,false)", "assumeFalse(msg,true)",
            "assumeNotNull(a,null)", "assumeNotNull()", "assumeNotNull(a)", "assumeNoException(msg,ex)", "assumeNoException(ex)",
            "assumeNoException(null)", "assumeTrue(true)", "assumeNotNull(null-array)"};
        Thrower[] as = {() -> RandomizedTest.assumeTrue(false), () -> RandomizedTest.assumeFalse(true),
            () -> RandomizedTest.assumeTrue("the message", false), () -> RandomizedTest.assumeFalse("the message", true),
            () -> RandomizedTest.assumeNotNull("a", null), () -> RandomizedTest.assumeNotNull(), () -> RandomizedTest.assumeNotNull("a"),
            () -> RandomizedTest.assumeNoException("the message", new IOException("io")),
            () -> RandomizedTest.assumeNoException(new IOException("io")), () -> RandomizedTest.assumeNoException(null),
            () -> RandomizedTest.assumeTrue(true), () -> RandomizedTest.assumeNotNull((Object[]) null)};
        for (int k = 0; k < acalls.length; k++) {
          try { as[k].run(); LOG.add(config + "\tassume\t" + acalls[k] + "\tOK\t-"); }
          catch (Throwable e) {
            LOG.add(config + "\tassume\t" + acalls[k] + "\t" + err(e) + "\t" + (e.getCause() == null ? "-" : err(e.getCause())));
          }
        }
        LOG.add(config + "\tprefix\t" + SysGlobals.CURRENT_PREFIX() + "\t" + SysGlobals.prefixProperty("nightly") + "\t"
            + SysGlobals.SYSPROP_RANDOM_SEED() + "\t" + SysGlobals.SYSPROP_TESTFILTER());
      }
    }
  }

  static void helpers(Path out) throws Exception {
    String[][] configs = {
      {"default", null, null},
      {"nightly-x2.5", "true", "2.5"},
      {"x-1.5", "false", "-1.5"},
      {"xNaN", "off", "NaN"},
      {"nightly-malformed", "maybe", "0"},
    };
    LOG.clear();
    System.setProperty("tests.seed", "1234ABCD");
    for (String[] c : configs) {
      System.setProperty("oracle.config", c[0]);
      if (c[1] == null) System.clearProperty("tests.nightly"); else System.setProperty("tests.nightly", c[1]);
      if (c[2] == null) System.clearProperty(RandomizedTest.SYSPROP_MULTIPLIER); else System.setProperty(RandomizedTest.SYSPROP_MULTIPLIER, c[2]);
      Result res = JUnitCore.runClasses(HelpersSuite.class);
      for (Failure f : res.getFailures()) throw new AssertionError(f.getTrace());
    }
    System.clearProperty("tests.seed");
    System.clearProperty("tests.nightly");
    System.clearProperty(RandomizedTest.SYSPROP_MULTIPLIER);
    List<String> lines = new ArrayList<>(LOG);
    lines.add(0, "# config\tseed\thelper\tfnv64\tfirst values");
    Files.write(out.resolve("helpers.tsv"), lines, StandardCharsets.UTF_8);
    LOG.clear();
  }

  public static void main(String[] args) throws Exception {
    Path out = Paths.get(args[0]);
    Files.createDirectories(out);
    randomDraws(out);
    randomErrors(out);
    strictMathLog(out);
    javaLang(out);
    seedUtils(out);
    seedChain(out);
    contextAndResources(out);
    generators(out);
    helpers(out);
  }
}
