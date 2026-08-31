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
 * where CASE is FLOAT or BYTE.
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
      System.err.println("Cases: FLOAT, BYTE");
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
