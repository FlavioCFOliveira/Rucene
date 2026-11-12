/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
package org.apache.lucene.rucene.codec;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;

import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.util.Constants;
import org.apache.lucene.util.VectorUtil;
import org.apache.lucene.util.Version;

/**
 * Prints reference values of {@link VectorUtil} and
 * {@link VectorSimilarityFunction} for a deterministic corpus of vectors.
 *
 * <p>Every float result is emitted as the hexadecimal form of
 * {@code Float.floatToRawIntBits}, so the Rust side can compare bit for bit
 * rather than through a tolerance. The input vectors themselves are emitted in
 * Base64 (big-endian IEEE-754 for floats, raw bytes for byte vectors) so the
 * Rust side reproduces exactly the same inputs.
 *
 * <p><strong>The JVM must be started with {@code -Dlucene.useScalarFMA=false}
 * and without {@code --add-modules jdk.incubator.vector}.</strong> Lucene
 * otherwise selects a Panama or fused-multiply-add implementation whose
 * low-order bits depend on the CPU, and the captured values would not be a
 * reproducible reference. The header records the selected path so the consumer
 * can refuse a run made under the wrong flags. Those two booleans fully
 * determine the implementation Lucene picks: without the incubator module the
 * provider is necessarily the scalar one, and {@code HAS_FAST_SCALAR_FMA}
 * selects between {@code Math.fma} and {@code a * b + c} inside it.
 * ({@code VectorizationProvider.getInstance()} itself refuses to be called from
 * outside Lucene, so it cannot be reported directly.)
 *
 * <p>Usage:
 * <pre>
 *   mvn -q -f tests/fixtures/java-codec-harness/pom.xml \
 *       -Dlucene.useScalarFMA=false \
 *       exec:java -Dexec.mainClass=org.apache.lucene.rucene.codec.VectorSimilarityFixture \
 *       -Dexec.args="/tmp/ignored FLOAT"
 * </pre>
 * where CASE is FLOAT, BYTE or KERNELS.
 *
 * <p>{@code KERNELS} prints the remaining public {@link VectorUtil} methods over fixed inputs:
 * the int4 dot products and square distances (unpacked, single-packed and both-packed), {@code
 * int4BitDotProduct}, {@code int4DibitDotProduct}, {@code uint8DotProduct}, {@code
 * uint8SquareDistance}, {@code xorBitCount}, {@code minMaxScalarQuantize}, {@code
 * recalculateOffset}, {@code findNextGEQ}, {@code filterByScore}, {@code l2normalize}, {@code
 * isUnitVector}, both {@code isZeroVector}, {@code add}, {@code checkFinite}, {@code
 * scaleMaxInnerProductScore}, {@code normalizeToUnitInterval} and {@code
 * normalizeDistanceToUnitInterval}.
 */
public final class VectorSimilarityFixture {

  private VectorSimilarityFixture() {}

  /**
   * Dimensions covering the scalar tail, the four-accumulator unrolling
   * boundary at 32, the usual embedding sizes, and 2048 — the first power of
   * two where a byte dot product can exceed 2^24 with an odd increment, which
   * is where an {@code f32} accumulator stops being exact.
   */
  private static final int[] DIMENSIONS = {1, 2, 3, 32, 33, 64, 128, 384, 768, 1024, 2048};

  public static void main(String[] args) {
    if (args.length != 2) {
      System.err.println("Usage: VectorSimilarityFixture <output-dir> <case>");
      System.err.println("Cases: FLOAT, BYTE, KERNELS");
      System.exit(1);
    }
    String testCase = args[1];

    System.out.println("fixture=VectorSimilarityFixture");
    System.out.println("version=" + Version.LATEST);
    System.out.println("case=" + testCase);
    System.out.println("jvm_flag_use_scalar_fma=" + System.getProperty("lucene.useScalarFMA", "auto"));
    System.out.println("has_fast_scalar_fma=" + Constants.HAS_FAST_SCALAR_FMA);
    System.out.println(
        "incubator_vector_module="
            + ModuleLayer.boot().findModule("jdk.incubator.vector").isPresent());

    switch (testCase) {
      case "FLOAT" -> emitFloatCases();
      case "BYTE" -> emitByteCases();
      case "KERNELS" -> emitKernelCases();
      default -> {
        System.err.println("unknown case: " + testCase);
        System.exit(1);
      }
    }
  }

  // ---------------------------------------------------------------------------
  // Float vectors
  // ---------------------------------------------------------------------------

  private static void emitFloatCases() {
    for (int dim : DIMENSIONS) {
      List<float[][]> pairs = floatPairs(dim);
      for (int id = 0; id < pairs.size(); id++) {
        float[] a = pairs.get(id)[0];
        float[] b = pairs.get(id)[1];
        System.out.println(
            "vec dim=" + dim + " id=" + id + " a=" + encodeFloats(a) + " b=" + encodeFloats(b));
        StringBuilder line = new StringBuilder();
        line.append("f32 dim=").append(dim).append(" id=").append(id);
        line.append(" dotProduct=").append(bits(VectorUtil.dotProduct(a, b)));
        line.append(" cosine=").append(bits(VectorUtil.cosine(a, b)));
        line.append(" squareDistance=").append(bits(VectorUtil.squareDistance(a, b)));
        for (VectorSimilarityFunction fn : VectorSimilarityFunction.values()) {
          line.append(' ').append(fn.name()).append('=').append(bits(fn.compare(a, b)));
        }
        System.out.println(line);
      }
    }
  }

  /** Six deterministic float vector pairs for the given dimension. */
  private static List<float[][]> floatPairs(int dim) {
    List<float[][]> pairs = new ArrayList<>();

    float[] randomA = randomFloats(dim, 0x51ED0001L);
    float[] randomB = randomFloats(dim, 0x51ED0002L);
    // 0: two unrelated vectors.
    pairs.add(new float[][] {randomA, randomB});
    // 1: identical vectors (cosine 1, distance 0).
    pairs.add(new float[][] {randomA, randomA.clone()});
    // 2: anti-parallel vectors (cosine -1, exercises the clamp).
    pairs.add(new float[][] {randomA, negate(randomA)});
    // 3: a zero vector, whose cosine is NaN in Java.
    pairs.add(new float[][] {new float[dim], randomB});
    // 4: alternating +/-1 against all ones; every product is exactly +/-1, so
    //    the accumulation order alone decides the low bits.
    float[] alternating = new float[dim];
    float[] ones = new float[dim];
    for (int i = 0; i < dim; i++) {
      alternating[i] = (i % 2 == 0) ? 1.0f : -1.0f;
      ones[i] = 1.0f;
    }
    pairs.add(new float[][] {alternating, ones});
    // 5: unit-normalized vectors, the shape DOT_PRODUCT is meant for.
    pairs.add(
        new float[][] {
          VectorUtil.l2normalize(randomFloats(dim, 0x51ED0003L), false),
          VectorUtil.l2normalize(randomFloats(dim, 0x51ED0004L), false)
        });
    return pairs;
  }

  // ---------------------------------------------------------------------------
  // Byte vectors
  // ---------------------------------------------------------------------------

  private static void emitByteCases() {
    for (int dim : DIMENSIONS) {
      List<byte[][]> pairs = bytePairs(dim);
      for (int id = 0; id < pairs.size(); id++) {
        byte[] a = pairs.get(id)[0];
        byte[] b = pairs.get(id)[1];
        System.out.println(
            "vec dim=" + dim + " id=" + id + " a=" + encodeBytes(a) + " b=" + encodeBytes(b));
        StringBuilder line = new StringBuilder();
        line.append("i8 dim=").append(dim).append(" id=").append(id);
        // Integer results are exact, so decimal is lossless here.
        line.append(" dotProduct=").append(VectorUtil.dotProduct(a, b));
        line.append(" squareDistance=").append(VectorUtil.squareDistance(a, b));
        line.append(" cosine=").append(bits(VectorUtil.cosine(a, b)));
        line.append(" dotProductScore=").append(bits(VectorUtil.dotProductScore(a, b)));
        for (VectorSimilarityFunction fn : VectorSimilarityFunction.values()) {
          line.append(' ').append(fn.name()).append('=').append(bits(fn.compare(a, b)));
        }
        System.out.println(line);
      }
    }
  }

  /** Six deterministic byte vector pairs for the given dimension. */
  private static List<byte[][]> bytePairs(int dim) {
    List<byte[][]> pairs = new ArrayList<>();

    byte[] randomA = randomBytes(dim, 0xB17E0001L);
    byte[] randomB = randomBytes(dim, 0xB17E0002L);
    // 0: two unrelated vectors.
    pairs.add(new byte[][] {randomA, randomB});
    // 1: identical vectors.
    pairs.add(new byte[][] {randomA, randomA.clone()});
    // 2: the extremes. At dimension 1024 the exact dot product is -16_646_144
    //    and the exact square distance 66_585_600, both beyond the 2^24 limit
    //    of exact float integers: an f32 accumulator would answer differently.
    byte[] maxima = new byte[dim];
    byte[] minima = new byte[dim];
    java.util.Arrays.fill(maxima, (byte) 127);
    java.util.Arrays.fill(minima, (byte) -128);
    pairs.add(new byte[][] {maxima, minima});
    // 3: two zero vectors, whose cosine is NaN in Java.
    pairs.add(new byte[][] {new byte[dim], new byte[dim]});
    // 4: alternating extremes against their mirror.
    byte[] alternating = new byte[dim];
    byte[] mirrored = new byte[dim];
    for (int i = 0; i < dim; i++) {
      alternating[i] = (byte) (i % 2 == 0 ? 127 : -128);
      mirrored[i] = (byte) (i % 2 == 0 ? -128 : 127);
    }
    pairs.add(new byte[][] {alternating, mirrored});
    // 5: one vector against itself negated, so the dot product is negative and
    //    MAXIMUM_INNER_PRODUCT takes its reciprocal branch.
    byte[] source = randomBytes(dim, 0xB17E0003L);
    byte[] negated = new byte[dim];
    for (int i = 0; i < dim; i++) {
      negated[i] = (byte) (-source[i]);
    }
    pairs.add(new byte[][] {source, negated});
    // 6: every component at the positive extreme. The per-element product is
    //    16_129, an odd number, so from dimension 2048 the running total passes
    //    2^24 and an f32 accumulator would start rounding it. The i32
    //    accumulation Lucene uses stays exact.
    byte[] positives = new byte[dim];
    java.util.Arrays.fill(positives, (byte) 127);
    pairs.add(new byte[][] {positives, positives.clone()});
    return pairs;
  }

  // ---------------------------------------------------------------------------
  // Remaining VectorUtil kernels
  // ---------------------------------------------------------------------------

  /**
   * Lengths around every SIMD width (4, 8, 16, 32 and 64 lanes) and their non-multiples, up to a
   * length whose tails are all non-empty.
   */
  private static final int[] KERNEL_LENGTHS = {
    1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129, 255, 256, 257, 1031
  };

  private static void emitKernelCases() {
    for (int dim : KERNEL_LENGTHS) {
      emitInt4Cases(dim);
      emitBitCases(dim);
      emitUint8Cases(dim);
      emitQuantizeCases(dim);
      emitFloatHelperCases(dim);
    }
    emitFindNextGeqCases();
    emitFilterByScoreCases();
    emitScalarCases();
  }

  /** Four uint4 vector pairs: unrelated, both at 15, both at 0, and alternating 0/15 mirrored. */
  private static List<byte[][]> int4Pairs(int dim) {
    List<byte[][]> pairs = new ArrayList<>();
    pairs.add(new byte[][] {nibbles(dim, 0x14F40001L), nibbles(dim, 0x14F40002L)});
    byte[] fifteen = new byte[dim];
    java.util.Arrays.fill(fifteen, (byte) 15);
    pairs.add(new byte[][] {fifteen, fifteen.clone()});
    pairs.add(new byte[][] {new byte[dim], nibbles(dim, 0x14F40003L)});
    byte[] alternating = new byte[dim];
    byte[] mirrored = new byte[dim];
    for (int i = 0; i < dim; i++) {
      alternating[i] = (byte) (i % 2 == 0 ? 15 : 0);
      mirrored[i] = (byte) (i % 2 == 0 ? 0 : 15);
    }
    pairs.add(new byte[][] {alternating, mirrored});
    return pairs;
  }

  /** {@code packed[i] = (raw[i] << 4) | raw[packed.length + i]}, the packing VectorUtil assumes. */
  private static byte[] pack(byte[] raw) {
    byte[] packed = new byte[(raw.length + 1) >> 1];
    for (int i = 0; i < packed.length; i++) {
      packed[i] = (byte) ((raw[i] << 4) | raw[packed.length + i]);
    }
    return packed;
  }

  private static void emitInt4Cases(int dim) {
    List<byte[][]> pairs = int4Pairs(dim);
    for (int id = 0; id < pairs.size(); id++) {
      byte[] a = pairs.get(id)[0];
      byte[] b = pairs.get(id)[1];
      StringBuilder line = new StringBuilder();
      line.append("int4 dim=").append(dim).append(" id=").append(id);
      line.append(" a=").append(encodeBytes(a)).append(" b=").append(encodeBytes(b));
      line.append(" dotProduct=").append(VectorUtil.int4DotProduct(a, b));
      line.append(" squareDistance=").append(VectorUtil.int4SquareDistance(a, b));
      // The single-packed forms read unpacked[i + packed.length], which needs an even length.
      if (dim % 2 == 0) {
        byte[] pa = pack(a);
        byte[] pb = pack(b);
        line.append(" pa=").append(encodeBytes(pa)).append(" pb=").append(encodeBytes(pb));
        line.append(" singlePackedDot=").append(VectorUtil.int4DotProductSinglePacked(a, pb));
        line.append(" bothPackedDot=").append(VectorUtil.int4DotProductBothPacked(pa, pb));
        line.append(" singlePackedSquare=")
            .append(VectorUtil.int4SquareDistanceSinglePacked(a, pb));
        line.append(" bothPackedSquare=").append(VectorUtil.int4SquareDistanceBothPacked(pa, pb));
      }
      System.out.println(line);
    }
  }

  private static void emitBitCases(int dim) {
    // int4BitDotProduct: a 4-stripe query against a binary document of dim bytes.
    byte[][] bitDocs = {randomBytes(dim, 0xB1700001L), filled(dim, (byte) -1), new byte[dim]};
    for (int id = 0; id < bitDocs.length; id++) {
      byte[] d = bitDocs[id];
      byte[] q = id == 1 ? filled(4 * dim, (byte) -1) : randomBytes(4 * dim, 0xB1700002L + id);
      System.out.println(
          "bit dim="
              + dim
              + " id="
              + id
              + " q="
              + encodeBytes(q)
              + " d="
              + encodeBytes(d)
              + " int4BitDotProduct="
              + VectorUtil.int4BitDotProduct(q, d));
    }
    // int4DibitDotProduct: a 4-stripe query against a 2-stripe document of 2 * dim bytes.
    byte[][] dibitDocs = {
      randomBytes(2 * dim, 0xD1B10001L), filled(2 * dim, (byte) -1), new byte[2 * dim]
    };
    for (int id = 0; id < dibitDocs.length; id++) {
      byte[] d = dibitDocs[id];
      byte[] q = id == 1 ? filled(4 * dim, (byte) -1) : randomBytes(4 * dim, 0xD1B10002L + id);
      System.out.println(
          "dibit dim="
              + dim
              + " id="
              + id
              + " q="
              + encodeBytes(q)
              + " d="
              + encodeBytes(d)
              + " int4DibitDotProduct="
              + VectorUtil.int4DibitDotProduct(q, d));
    }
  }

  private static void emitUint8Cases(int dim) {
    List<byte[][]> pairs = new ArrayList<>();
    pairs.add(new byte[][] {randomBytes(dim, 0x0D180001L), randomBytes(dim, 0x0D180002L)});
    pairs.add(new byte[][] {filled(dim, (byte) 255), filled(dim, (byte) 255)});
    pairs.add(new byte[][] {filled(dim, (byte) 255), new byte[dim]});
    byte[] alternating = new byte[dim];
    byte[] mirrored = new byte[dim];
    for (int i = 0; i < dim; i++) {
      alternating[i] = (byte) (i % 2 == 0 ? 255 : 0);
      mirrored[i] = (byte) (i % 2 == 0 ? 0 : 255);
    }
    pairs.add(new byte[][] {alternating, mirrored});
    pairs.add(new byte[][] {filled(dim, (byte) 127), filled(dim, (byte) -128)});
    for (int id = 0; id < pairs.size(); id++) {
      byte[] a = pairs.get(id)[0];
      byte[] b = pairs.get(id)[1];
      System.out.println(
          "u8 dim="
              + dim
              + " id="
              + id
              + " a="
              + encodeBytes(a)
              + " b="
              + encodeBytes(b)
              + " uint8DotProduct="
              + VectorUtil.uint8DotProduct(a, b)
              + " uint8SquareDistance="
              + VectorUtil.uint8SquareDistance(a, b)
              + " xorBitCount="
              + VectorUtil.xorBitCount(a, b));
    }
  }

  private static void emitQuantizeCases(int dim) {
    // {minQuantile, maxQuantile, levels} of the quantization, then of the recalculation.
    float[][] settings = {
      {-0.8f, 0.9f, 127f, -0.6f, 0.95f, 127f}, {-1f, 1f, 15f, -0.5f, 0.5f, 15f}
    };
    float[] vector = randomFloats(dim, 0x5CA10001L);
    for (int id = 0; id < settings.length; id++) {
      float[] s = settings[id];
      float minQ = s[0];
      float maxQ = s[1];
      float scale = s[2] / (maxQ - minQ);
      float alpha = (maxQ - minQ) / s[2];
      byte[] dest = new byte[dim];
      float offset = VectorUtil.minMaxScalarQuantize(vector, dest, scale, alpha, minQ, maxQ);
      float rMinQ = s[3];
      float rMaxQ = s[4];
      float rScale = s[5] / (rMaxQ - rMinQ);
      float rAlpha = (rMaxQ - rMinQ) / s[5];
      float recalculated =
          VectorUtil.recalculateOffset(dest, alpha, minQ, rScale, rAlpha, rMinQ, rMaxQ);
      System.out.println(
          "quant dim="
              + dim
              + " id="
              + id
              + " v="
              + encodeFloats(vector)
              + " scale="
              + bits(scale)
              + " alpha="
              + bits(alpha)
              + " min="
              + bits(minQ)
              + " max="
              + bits(maxQ)
              + " dest="
              + encodeBytes(dest)
              + " offset="
              + bits(offset)
              + " rscale="
              + bits(rScale)
              + " ralpha="
              + bits(rAlpha)
              + " rmin="
              + bits(rMinQ)
              + " rmax="
              + bits(rMaxQ)
              + " roffset="
              + bits(recalculated));
    }
  }

  private static void emitFloatHelperCases(int dim) {
    float[][] vectors = {
      randomFloats(dim, 0xF1E10001L),
      new float[dim],
      VectorUtil.l2normalize(randomFloats(dim, 0xF1E10002L), false),
      filled(dim, -0.0f)
    };
    for (int id = 0; id < vectors.length; id++) {
      float[] v = vectors[id];
      float[] normalized = VectorUtil.l2normalize(v.clone(), false);
      float[] u = randomFloats(dim, 0xF1E10003L + id);
      float[] sum = u.clone();
      VectorUtil.add(sum, v);
      byte[] asBytes = new byte[dim];
      for (int i = 0; i < dim; i++) {
        asBytes[i] = (byte) (id == 1 ? 0 : (i * 31 + id) % 3);
      }
      System.out.println(
          "floats dim="
              + dim
              + " id="
              + id
              + " v="
              + encodeFloats(v)
              + " l2="
              + encodeFloats(normalized)
              + " unit="
              + VectorUtil.isUnitVector(v)
              + " zero="
              + VectorUtil.isZeroVector(v)
              + " u="
              + encodeFloats(u)
              + " sum="
              + encodeFloats(sum)
              + " bytes="
              + encodeBytes(asBytes)
              + " zeroBytes="
              + VectorUtil.isZeroVector(asBytes));
    }
    float[][] checked = {
      randomFloats(dim, 0xF1E10010L),
      withValue(randomFloats(dim, 0xF1E10011L), dim / 2, Float.NaN),
      withValue(randomFloats(dim, 0xF1E10012L), dim - 1, Float.POSITIVE_INFINITY),
      withValue(randomFloats(dim, 0xF1E10013L), 0, Float.NEGATIVE_INFINITY)
    };
    for (int id = 0; id < checked.length; id++) {
      String result;
      try {
        VectorUtil.checkFinite(checked[id]);
        result = "ok";
      } catch (IllegalArgumentException e) {
        result = "error message=" + e.getMessage();
      }
      System.out.println(
          "finite dim=" + dim + " id=" + id + " v=" + encodeFloats(checked[id]) + " " + result);
    }
  }

  private static void emitFindNextGeqCases() {
    int[] buffer = new int[128];
    for (int i = 0; i < buffer.length; i++) {
      // Sorted, with every value twice.
      buffer[i] = (i / 2) * 3;
    }
    StringBuilder values = new StringBuilder();
    for (int i = 0; i < buffer.length; i++) {
      values.append(i == 0 ? "" : ",").append(buffer[i]);
    }
    System.out.println("geqbuf values=" + values);
    int[] targets = {-5, 0, 1, 2, 3, 50, 51, 52, 100, 188, 189, 190, 1000};
    int[][] ranges = {{0, 128}, {0, 1}, {5, 128}, {17, 64}, {64, 65}, {100, 128}, {128, 128}};
    for (int[] range : ranges) {
      for (int target : targets) {
        System.out.println(
            "geq target="
                + target
                + " from="
                + range[0]
                + " to="
                + range[1]
                + " result="
                + VectorUtil.findNextGEQ(buffer, target, range[0], range[1]));
      }
    }
  }

  private static void emitFilterByScoreCases() {
    int[] lengths = {0, 1, 7, 8, 9, 64, 129};
    double[] thresholds = {Double.NEGATIVE_INFINITY, -0.5, 0.0, 0.25, 0.5, 0.999, 2.0};
    for (int length : lengths) {
      for (int t = 0; t < thresholds.length; t++) {
        int[] docs = new int[length];
        double[] scores = new double[length];
        long[] state = {0xF117E000L + length};
        for (int i = 0; i < length; i++) {
          docs[i] = i * 7 + 3;
          scores[i] = (next(state) >>> 11) * 0x1.0p-53 * 2.0 - 0.5;
        }
        if (length > 2) {
          scores[1] = thresholds[t];
        }
        int upTo = length - (length > 8 ? 3 : 0);
        String in = joinDocs(docs) + " scores=" + joinScores(scores);
        int count = VectorUtil.filterByScore(docs, scores, thresholds[t], upTo);
        System.out.println(
            "filter length="
                + length
                + " t="
                + t
                + " docs="
                + in
                + " min="
                + String.format("%016x", Double.doubleToRawLongBits(thresholds[t]))
                + " upTo="
                + upTo
                + " count="
                + count
                + " outDocs="
                + joinDocs(docs)
                + " outScores="
                + joinScores(scores));
      }
    }
  }

  private static void emitScalarCases() {
    float[] values = {
      -1e30f, -100f, -3f, -1f, -0.5f, -0.0f, 0f, 1e-30f, 0.25f, 0.5f, 1f, 7.5f, 1e30f,
      Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY
    };
    for (float x : values) {
      System.out.println(
          "scalar x="
              + bits(x)
              + " scaleMaxInnerProductScore="
              + bits(VectorUtil.scaleMaxInnerProductScore(x))
              + " normalizeToUnitInterval="
              + bits(VectorUtil.normalizeToUnitInterval(x))
              + " normalizeDistanceToUnitInterval="
              + bits(VectorUtil.normalizeDistanceToUnitInterval(x)));
    }
  }

  private static byte[] nibbles(int dim, long seed) {
    long[] state = {seed};
    byte[] values = new byte[dim];
    for (int i = 0; i < dim; i++) {
      values[i] = (byte) ((next(state) >>> 40) & 0x0F);
    }
    return values;
  }

  private static byte[] filled(int dim, byte value) {
    byte[] values = new byte[dim];
    java.util.Arrays.fill(values, value);
    return values;
  }

  private static float[] filled(int dim, float value) {
    float[] values = new float[dim];
    java.util.Arrays.fill(values, value);
    return values;
  }

  private static float[] withValue(float[] v, int index, float value) {
    v[index] = value;
    return v;
  }

  private static String joinDocs(int[] docs) {
    StringBuilder sb = new StringBuilder("[");
    for (int i = 0; i < docs.length; i++) {
      sb.append(i == 0 ? "" : ",").append(docs[i]);
    }
    return sb.append(']').toString();
  }

  private static String joinScores(double[] scores) {
    StringBuilder sb = new StringBuilder("[");
    for (int i = 0; i < scores.length; i++) {
      sb.append(i == 0 ? "" : ",")
          .append(String.format("%016x", Double.doubleToRawLongBits(scores[i])));
    }
    return sb.append(']').toString();
  }

  // ---------------------------------------------------------------------------
  // Deterministic generation and encoding
  // ---------------------------------------------------------------------------

  /**
   * A 64-bit linear congruential generator, so the corpus does not depend on
   * {@code java.util.Random}'s implementation details.
   */
  private static long next(long[] state) {
    state[0] = state[0] * 6364136223846793005L + 1442695040888963407L;
    return state[0];
  }

  private static float[] randomFloats(int dim, long seed) {
    long[] state = {seed};
    float[] values = new float[dim];
    for (int i = 0; i < dim; i++) {
      int bits = (int) (next(state) >>> 33);
      // Map into [-1, 1) with an exactly representable scale.
      values[i] = (bits / (float) (1 << 31)) - 1.0f;
    }
    return values;
  }

  private static byte[] randomBytes(int dim, long seed) {
    long[] state = {seed};
    byte[] values = new byte[dim];
    for (int i = 0; i < dim; i++) {
      values[i] = (byte) (next(state) >>> 40);
    }
    return values;
  }

  private static float[] negate(float[] v) {
    float[] out = new float[v.length];
    for (int i = 0; i < v.length; i++) {
      out[i] = -v[i];
    }
    return out;
  }

  private static String bits(float value) {
    return String.format("%08x", Float.floatToRawIntBits(value));
  }

  private static String encodeFloats(float[] v) {
    ByteBuffer buffer = ByteBuffer.allocate(v.length * Float.BYTES).order(ByteOrder.BIG_ENDIAN);
    for (float value : v) {
      buffer.putFloat(value);
    }
    return Base64.getEncoder().encodeToString(buffer.array());
  }

  private static String encodeBytes(byte[] v) {
    return Base64.getEncoder().encodeToString(v);
  }
}
