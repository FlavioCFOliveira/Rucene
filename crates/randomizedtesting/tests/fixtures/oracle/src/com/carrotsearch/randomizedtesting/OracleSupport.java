// Shared support of the randomizedtesting oracle programs (see README.md in
// this directory): the digest every fixture uses, string and error encodings,
// the fixed seed grid, and error capture.
package com.carrotsearch.randomizedtesting;

import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.List;
import java.util.SplittableRandom;

public final class OracleSupport {
  private OracleSupport() {}

  // ---------------------------------------------------------------------------
  // Digest of a sequence of values.
  // ---------------------------------------------------------------------------

  public static final class Rec {
    long h = 0xcbf29ce484222325L;
    final List<String> first = new ArrayList<>();

    void b(int v) { h ^= (v & 0xff); h *= 0x100000001b3L; }
    void first(String s) { if (first.isEmpty()) first.add(s); }

    void i(int v) { for (int k = 0; k < 4; k++) b(v >>> (8 * k)); first(Integer.toString(v)); }
    void l(long v) { for (int k = 0; k < 8; k++) b((int) (v >>> (8 * k))); first(Long.toString(v)); }
    void f(float v) {
      int bits = Float.floatToRawIntBits(v);
      for (int k = 0; k < 4; k++) b(bits >>> (8 * k));
      first(String.format("%08x", bits));
    }
    void d(double v) {
      long bits = Double.doubleToRawLongBits(v);
      for (int k = 0; k < 8; k++) b((int) (bits >>> (8 * k)));
      first(String.format("%016x", bits));
    }
    void z(boolean v) { b(v ? 1 : 0); first(Boolean.toString(v)); }
    void bytes(byte[] a) {
      i0(a.length);
      StringBuilder sb = new StringBuilder();
      for (byte x : a) { b(x); sb.append(String.format("%02x", x & 0xff)); }
      first(sb.toString());
    }
    void s(String v) {
      i0(v.length());
      for (int k = 0; k < v.length(); k++) { char c = v.charAt(k); b(c); b(c >>> 8); }
      first(units(v));
    }
    // Length prefix, not recorded among the first values.
    void i0(int v) { for (int k = 0; k < 4; k++) b(v >>> (8 * k)); }

    String line() { return String.format("%016x", h) + "\t" + String.join(",", first); }
  }

  /** A string as hex UTF-16 code units joined by '.', so that TSV needs no escaping. */
  public static String units(String v) {
    StringBuilder sb = new StringBuilder();
    for (int k = 0; k < v.length(); k++) {
      if (k > 0) sb.append('.');
      sb.append(String.format("%04x", (int) v.charAt(k)));
    }
    return sb.length() == 0 ? "-" : sb.toString();
  }

  public static String err(Throwable t) {
    String m = t.getMessage();
    return t.getClass().getName() + "\t" + (m == null ? "<null>" : m.replace("\n", "\\n").replace("\t", "\\t"));
  }

  /** The fixed seed grid: edge values, then ten draws of SplittableRandom(20261103). */
  public static long[] seeds() {
    long[] s = new long[20];
    s[0] = 0L; s[1] = 1L; s[2] = -1L; s[3] = Long.MIN_VALUE; s[4] = Long.MAX_VALUE;
    s[5] = 42L; s[6] = 0x5DEECE66DL; s[7] = 0xDEADBEEFL; s[8] = 0x123456789ABCDEFL; s[9] = -0x123456789ABCDEFL;
    SplittableRandom sr = new SplittableRandom(20261103L);
    for (int i = 10; i < 20; i++) s[i] = sr.nextLong();
    return s;
  }

  public interface Thrower { void run() throws Throwable; }
  public static void tryRecord(PrintWriter w, String label, Thrower t) {
    try {
      t.run();
      w.println(label + "\tOK\t-");
    } catch (Throwable e) {
      w.println(label + "\t" + err(e));
    }
  }
}
