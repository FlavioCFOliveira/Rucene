package org.apache.lucene.rucene.codec;

import java.lang.foreign.MemorySegment;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.internal.vectorization.VectorUtilSupport;
import org.apache.lucene.util.Constants;
import org.apache.lucene.util.FixedBitSet;
import org.apache.lucene.util.LongValues;
import org.apache.lucene.util.VectorUtil;

/**
 * Oracle for rmp task #319: runs a corpus of vector-utility operations through the {@code
 * VectorUtilSupport} Lucene selected ({@code VectorUtil.IMPL}: {@code PanamaVectorUtilSupport}
 * with {@code --add-modules jdk.incubator.vector}, {@code DefaultVectorUtilSupport} without),
 * and, with the vector module, through {@code MemorySegmentBulkVectorOps}, {@code
 * PanamaDocValuesRangeSupport} and {@code FixedBitSet.orMask}.
 *
 * <p>Usage: {@code PanamaVectorizationFixture <corpus> <passes>}. The corpus is the text file
 * {@code tests/portability/panama_vectorization.rs} writes, one operation per line. The whole
 * corpus runs {@code passes} times so that C2 compiles the kernels; the last two passes must agree
 * (exit status 3 otherwise), and the last one is printed, one line per operation. Floats are
 * printed as {@code Float.floatToIntBits} hex (NaN canonical), doubles likewise.
 */
public class PanamaVectorizationFixture {

  static VectorUtilSupport impl;
  static boolean vector;
  static Object dot, cos, sqr, range;
  static Method dotBulkQ, dotBulkSeg, dotOne, cosBulkQ, cosBulkSeg, cosOne;
  static Method sqrBulkQ, sqrBulkSeg, sqrOne, rangeInto;

  public static void main(String[] args) throws Exception {
    List<String> lines = Files.readAllLines(Path.of(args[0]));
    int passes = Integer.parseInt(args[1]);
    Field f = VectorUtil.class.getDeclaredField("IMPL");
    f.setAccessible(true);
    impl = (VectorUtilSupport) f.get(null);
    vector = impl.getClass().getSimpleName().startsWith("Panama");

    StringBuilder header = new StringBuilder();
    header.append("impl=").append(impl.getClass().getSimpleName()).append('\n');
    if (vector) {
      Class<?> pvc =
          Class.forName("org.apache.lucene.internal.vectorization.PanamaVectorConstants");
      Field bits = pvc.getDeclaredField("PREFERRED_VECTOR_BITSIZE");
      bits.setAccessible(true);
      Field fast = pvc.getDeclaredField("HAS_FAST_INTEGER_VECTORS");
      fast.setAccessible(true);
      header.append("preferredBitSize=").append(bits.getInt(null)).append('\n');
      header.append("hasFastIntegerVectors=").append(fast.getBoolean(null)).append('\n');
      setUpPanama();
    }
    header.append("HAS_FAST_VECTOR_FMA=").append(Constants.HAS_FAST_VECTOR_FMA).append('\n');
    header.append("HAS_FAST_SCALAR_FMA=").append(Constants.HAS_FAST_SCALAR_FMA).append('\n');
    header
        .append("HAS_FAST_COMPRESS_MASK_CAST=")
        .append(Constants.HAS_FAST_COMPRESS_MASK_CAST)
        .append('\n');

    List<String[]> ops = new ArrayList<>();
    for (String line : lines) {
      if (!line.isEmpty()) {
        ops.add(line.split(" "));
      }
    }
    String first = null;
    String previous = null;
    String current = null;
    for (int p = 0; p < passes; p++) {
      previous = current;
      current = runAll(ops);
      if (p == 0) {
        first = current;
      }
    }
    if (previous != null && !previous.equals(current)) {
      System.err.println("results still change between the last two passes");
      System.exit(3);
    }
    String[] cold = first.split("\n");
    String[] warm = current.split("\n");
    int differing = 0;
    for (int i = 0; i < warm.length; i++) {
      if (!cold[i].equals(warm[i])) {
        differing++;
      }
    }
    System.err.println("cold_vs_warm_differing_lines=" + differing + " of " + warm.length);
    System.out.print(header);
    System.out.print(current);
  }

  static void setUpPanama() throws Exception {
    Class<?> ops = Class.forName("org.apache.lucene.internal.vectorization.MemorySegmentBulkVectorOps");
    dot = ops.getField("DOT_INSTANCE").get(null);
    cos = ops.getField("COS_INSTANCE").get(null);
    sqr = ops.getField("SQR_INSTANCE").get(null);
    Class<?> ms = MemorySegment.class;
    Class<?> L = long.class;
    Class<?> I = int.class;
    dotBulkQ =
        dot.getClass()
            .getMethod("dotProductBulk", ms, float[].class, float[].class, L, L, L, L, I);
    dotBulkSeg = dot.getClass().getMethod("dotProductBulk", ms, float[].class, L, L, L, L, L, I);
    dotOne = dot.getClass().getMethod("dotProduct", ms, L, L, I);
    cosBulkQ =
        cos.getClass().getMethod("cosineBulk", ms, float[].class, float[].class, L, L, L, L, I);
    cosBulkSeg = cos.getClass().getMethod("cosineBulk", ms, float[].class, L, L, L, L, L, I);
    cosOne = cos.getClass().getMethod("cosine", ms, L, L, I);
    sqrBulkQ =
        sqr.getClass()
            .getMethod("sqrDistanceBulk", ms, float[].class, float[].class, L, L, L, L, I);
    sqrBulkSeg = sqr.getClass().getMethod("sqrDistanceBulk", ms, float[].class, L, L, L, L, L, I);
    sqrOne = sqr.getClass().getMethod("sqrDistance", ms, L, L, I);

    Class<?> rs = Class.forName("org.apache.lucene.internal.vectorization.PanamaDocValuesRangeSupport");
    Field inst = rs.getDeclaredField("INSTANCE");
    inst.setAccessible(true);
    range = inst.get(null);
    rangeInto =
        rs.getDeclaredMethod(
            "rangeIntoBitSet", LongValues.class, I, I, L, L, FixedBitSet.class, I);
    rangeInto.setAccessible(true);
  }

  static String runAll(List<String[]> ops) throws Exception {
    StringBuilder out = new StringBuilder();
    for (int i = 0; i < ops.size(); i++) {
      String[] t = ops.get(i);
      out.append(i).append(' ').append(t[0]);
      run(t, out);
      out.append('\n');
    }
    return out.toString();
  }

  static void run(String[] t, StringBuilder out) throws Exception {
    switch (t[0]) {
      case "F" -> {
        float[] a = floats(t[1]);
        float[] b = floats(t[2]);
        out.append(' ').append(fx(impl.dotProduct(a, b)));
        out.append(' ').append(fx(impl.cosine(a, b)));
        out.append(' ').append(fx(impl.squareDistance(a, b)));
      }
      case "B" -> {
        byte[] a = bytes(t[1]);
        byte[] b = bytes(t[2]);
        out.append(' ').append(impl.dotProduct(a, b));
        out.append(' ').append(impl.uint8DotProduct(a, b));
        out.append(' ').append(fx(impl.cosine(a, b)));
        out.append(' ').append(impl.squareDistance(a, b));
        out.append(' ').append(impl.uint8SquareDistance(a, b));
        out.append(' ').append(impl.int4DotProduct(a, b));
        out.append(' ').append(impl.int4SquareDistance(a, b));
        out.append(' ').append(impl.int4DotProductBothPacked(a, b));
        out.append(' ').append(impl.int4SquareDistanceBothPacked(a, b));
      }
      case "P" -> {
        byte[] unpacked = bytes(t[1]);
        byte[] packed = bytes(t[2]);
        out.append(' ').append(impl.int4DotProductSinglePacked(unpacked, packed));
        out.append(' ').append(impl.int4SquareDistanceSinglePacked(unpacked, packed));
      }
      case "Q" -> out.append(' ').append(impl.int4BitDotProduct(bytes(t[1]), bytes(t[2])));
      case "D" -> out.append(' ').append(impl.int4DibitDotProduct(bytes(t[1]), bytes(t[2])));
      case "G" -> {
        int target = Integer.parseInt(t[1]);
        int from = Integer.parseInt(t[2]);
        int to = Integer.parseInt(t[3]);
        out.append(' ').append(impl.findNextGEQ(ints(t[4]), target, from, to));
      }
      case "M" -> {
        float scale = f(t[1]);
        float alpha = f(t[2]);
        float minQ = f(t[3]);
        float maxQ = f(t[4]);
        float[] v = floats(t[5]);
        byte[] dest = new byte[v.length];
        float c = impl.minMaxScalarQuantize(v, dest, scale, alpha, minQ, maxQ);
        out.append(' ').append(fx(c)).append(' ').append(hex(dest));
      }
      case "R" -> {
        float c =
            impl.recalculateScalarQuantizationOffset(
                bytes(t[7]), f(t[1]), f(t[2]), f(t[3]), f(t[4]), f(t[5]), f(t[6]));
        out.append(' ').append(fx(c));
      }
      case "S" -> {
        double min = Double.longBitsToDouble(Long.parseUnsignedLong(t[1], 16));
        int upTo = Integer.parseInt(t[2]);
        int[] docs = ints(t[3]);
        double[] scores = doubles(t[4]);
        int n = impl.filterByScore(docs, scores, min, upTo);
        out.append(' ').append(n);
        for (int d : docs) {
          out.append(' ').append(d);
        }
        for (double s : scores) {
          out.append(' ').append(Long.toHexString(Double.doubleToLongBits(s)));
        }
      }
      case "K" -> {
        int count = Integer.parseInt(t[1]);
        int dims = Integer.parseInt(t[2]);
        long stride = 4L * dims;
        float[] data = floats(t[8]);
        ByteBuffer buf = ByteBuffer.allocate(data.length * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (float x : data) {
          buf.putFloat(x);
        }
        MemorySegment seg = MemorySegment.ofArray(buf.array());
        int qi = Integer.parseInt(t[3]);
        long q = qi * stride;
        long d1 = Integer.parseInt(t[4]) * stride;
        long d2 = Integer.parseInt(t[5]) * stride;
        long d3 = Integer.parseInt(t[6]) * stride;
        long d4 = Integer.parseInt(t[7]) * stride;
        float[] qArray = new float[dims];
        System.arraycopy(data, qi * dims, qArray, 0, dims);
        float[] scores = new float[4];
        Object[][] calls = {
          {dot, dotBulkQ, dotBulkSeg, dotOne}, {cos, cosBulkQ, cosBulkSeg, cosOne},
          {sqr, sqrBulkQ, sqrBulkSeg, sqrOne}
        };
        for (Object[] c : calls) {
          ((Method) c[1]).invoke(c[0], seg, scores, qArray, d1, d2, d3, d4, count);
          for (float s : scores) {
            out.append(' ').append(fx(s));
          }
          ((Method) c[2]).invoke(c[0], seg, scores, q, d1, d2, d3, d4, count);
          for (float s : scores) {
            out.append(' ').append(fx(s));
          }
          out.append(' ').append(fx((float) ((Method) c[3]).invoke(c[0], seg, q, d1, count)));
        }
      }
      case "V" -> {
        int numBits = Integer.parseInt(t[1]);
        int from = Integer.parseInt(t[2]);
        int to = Integer.parseInt(t[3]);
        long min = Long.parseLong(t[4]);
        long max = Long.parseLong(t[5]);
        int offset = Integer.parseInt(t[6]);
        long[] values = longs(t[7]);
        LongValues lv =
            new LongValues() {
              @Override
              public long get(long index) {
                return values[(int) index];
              }
            };
        FixedBitSet bits = new FixedBitSet(numBits);
        rangeInto.invoke(range, lv, from, to, min, max, bits, offset);
        words(bits, out);
      }
      case "O" -> {
        FixedBitSet bits = new FixedBitSet(Integer.parseInt(t[1]));
        for (String op : t[2].split(";")) {
          String[] p = op.split(":");
          bits.orMask(
              Integer.parseInt(p[0]), Long.parseUnsignedLong(p[1], 16), Integer.parseInt(p[2]));
        }
        words(bits, out);
      }
      default -> throw new IllegalArgumentException(t[0]);
    }
  }

  static void words(FixedBitSet bits, StringBuilder out) {
    for (long w : bits.getBits()) {
      out.append(' ').append(Long.toHexString(w));
    }
  }

  static String fx(float x) {
    return Integer.toHexString(Float.floatToIntBits(x));
  }

  static float f(String s) {
    return Float.intBitsToFloat(Integer.parseUnsignedInt(s, 16));
  }

  static float[] floats(String s) {
    if (s.equals("-")) {
      return new float[0];
    }
    String[] p = s.split(",");
    float[] r = new float[p.length];
    for (int i = 0; i < p.length; i++) {
      r[i] = f(p[i]);
    }
    return r;
  }

  static double[] doubles(String s) {
    if (s.equals("-")) {
      return new double[0];
    }
    String[] p = s.split(",");
    double[] r = new double[p.length];
    for (int i = 0; i < p.length; i++) {
      r[i] = Double.longBitsToDouble(Long.parseUnsignedLong(p[i], 16));
    }
    return r;
  }

  static int[] ints(String s) {
    if (s.equals("-")) {
      return new int[0];
    }
    String[] p = s.split(",");
    int[] r = new int[p.length];
    for (int i = 0; i < p.length; i++) {
      r[i] = Integer.parseInt(p[i]);
    }
    return r;
  }

  static long[] longs(String s) {
    if (s.equals("-")) {
      return new long[0];
    }
    String[] p = s.split(",");
    long[] r = new long[p.length];
    for (int i = 0; i < p.length; i++) {
      r[i] = Long.parseLong(p[i]);
    }
    return r;
  }

  static byte[] bytes(String s) {
    if (s.equals("-")) {
      return new byte[0];
    }
    byte[] r = new byte[s.length() / 2];
    for (int i = 0; i < r.length; i++) {
      r[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
    }
    return r;
  }

  static String hex(byte[] b) {
    if (b.length == 0) {
      return "-";
    }
    StringBuilder sb = new StringBuilder();
    for (byte x : b) {
      sb.append(String.format("%02x", x & 0xFF));
    }
    return sb.toString();
  }
}
