// Double.toString / Float.toString of JDK 25 (Raffaello Giulietti's shortest
// decimal, JDK 19+) over subnormals, the powers of ten at the formatting
// boundaries (1e-3, 1e7), MIN_NORMAL, MAX_VALUE and 10 000 random bit patterns
// per type for 3 seeds (java.util.Random(seed)). Writes to_string.tsv.
package com.carrotsearch.randomizedtesting;

import java.io.PrintWriter;
import java.nio.file.*;
import java.util.*;

public class ToStringOracle {
  public static void main(String[] args) throws Exception {
    try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Paths.get(args[0]).resolve("to_string.tsv")))) {
      w.println("# double|float\\tbits (hex)\\ttoString; double-random|float-random\\tseed\\tRec digest of 10000 toString");
      List<Long> ds = new ArrayList<>();
      for (long b = 1; b <= 64; b++) ds.add(b);
      for (long b : new long[] {0x000fffffffffffffL, 0x000ffffffffffffeL, 0x0008000000000000L, 0x0000000000000400L}) ds.add(b);
      for (double d : new double[] {0d, -0d, Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, Double.MIN_NORMAL, Double.MAX_VALUE, Double.MIN_VALUE}) ds.add(Double.doubleToRawLongBits(d));
      for (int e = -7; e <= 10; e++) {
        double p = Double.parseDouble("1e" + e);
        for (double d : new double[] {p, Math.nextUp(p), Math.nextDown(p), 2 * p, 9.999999999999999 * p / 10, 5 * p}) ds.add(Double.doubleToRawLongBits(d));
      }
      for (double d : new double[] {0.001, 9.999999e-4, 1e7, 9999999.999999998, 1e23, 2e23, 8.41e21, 5e-324, 1.0E-322, 2.0E-323, 1.7976931348623157E308, 4.35, 0.1, 100.0, 1e-3 - 1e-19}) ds.add(Double.doubleToRawLongBits(d));
      for (long b : ds) {
        double d = Double.longBitsToDouble(b);
        w.println("double\t" + Long.toHexString(b) + "\t" + Double.toString(d));
        w.println("double\t" + Long.toHexString(b ^ Long.MIN_VALUE) + "\t" + Double.toString(-d));
      }
      List<Integer> fs = new ArrayList<>();
      for (int b = 1; b <= 64; b++) fs.add(b);
      for (int b : new int[] {0x007fffff, 0x007ffffe, 0x00400000, 0x00000400}) fs.add(b);
      for (float f : new float[] {0f, -0f, Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY, Float.MIN_NORMAL, Float.MAX_VALUE, Float.MIN_VALUE}) fs.add(Float.floatToRawIntBits(f));
      for (int e = -7; e <= 10; e++) {
        float p = Float.parseFloat("1e" + e);
        for (float f : new float[] {p, Math.nextUp(p), Math.nextDown(p), 2 * p, 5 * p}) fs.add(Float.floatToRawIntBits(f));
      }
      for (float f : new float[] {0.001f, 1e7f, 9999999f, 1.0E-44f, 2.8E-45f, 0.1f, 1.1f, 100f, 3.4028235E38f, 33554450f}) fs.add(Float.floatToRawIntBits(f));
      for (int b : fs) {
        float f = Float.intBitsToFloat(b);
        w.println("float\t" + Integer.toHexString(b) + "\t" + Float.toString(f));
        w.println("float\t" + Integer.toHexString(b ^ Integer.MIN_VALUE) + "\t" + Float.toString(-f));
      }
      for (long seed : new long[] {1L, 2L, 3L}) {
        Random r = new Random(seed);
        OracleSupport.Rec rec = new OracleSupport.Rec();
        for (int i = 0; i < 10000; i++) rec.s(Double.toString(Double.longBitsToDouble(r.nextLong())));
        w.println("double-random\t" + seed + "\t" + rec.line());
        r = new Random(seed);
        rec = new OracleSupport.Rec();
        for (int i = 0; i < 10000; i++) rec.s(Float.toString(Float.intBitsToFloat(r.nextInt())));
        w.println("float-random\t" + seed + "\t" + rec.line());
      }
    }
  }
}
