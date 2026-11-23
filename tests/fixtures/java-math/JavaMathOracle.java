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

import java.util.function.DoubleBinaryOperator;
import java.util.function.DoubleUnaryOperator;

/**
 * The Java side of {@code tests/java_math_oracle.rs}: for each {@code java.lang.Math} function
 * the spatial modules call, evaluates {@code Math.f} and {@code StrictMath.f} over the edge cases
 * and {@code n} seeded inputs (SplitMix64, mirrored by the Rust test), and prints per function the
 * count of inputs where they differ, the first differing inputs, and a hash of each per chunk of
 * 10000 results ({@code h = 31 * h + doubleToLongBits(result)}, from 0).
 *
 * <p>Usage: {@code java -XX:+UnlockDiagnosticVMOptions -XX:-UseLibmIntrinsic JavaMathOracle seed n}:
 * the test runs it without HotSpot's LIBM intrinsics, the {@code java.lang.Math} that {@code
 * rucene::util::java_math} reproduces (rmp #628).
 */
public class JavaMathOracle {
  static final int CHUNK = 10000;

  static final double[] EDGES = {
    0.0, -0.0, 1.0, -1.0, 0.5, -0.5, 2.0, -2.0, Double.NaN, Double.POSITIVE_INFINITY,
    Double.NEGATIVE_INFINITY, Double.MIN_VALUE, -Double.MIN_VALUE, Double.MAX_VALUE,
    -Double.MAX_VALUE, Double.MIN_NORMAL, Math.PI, -Math.PI, Math.PI / 2, Math.PI / 4, 1e-300,
    1e300, 0x1p-28, 0x1p-29, 0x1p-57, 0.975, 0.6744, 2.4375, 0.4375, 1.1875, 0x1p31, 0x1p64,
    0x1p66, 3.0, -3.0, 1e22
  };

  static final double[] CENTERS = {0.5, 1.0, Math.PI / 4, Math.PI / 2, Math.PI, 2.4375, 0.4375, 1.1875, 0.975};

  /** SplitMix64. */
  static final class Gen {
    long state;

    Gen(long seed) {
      state = seed;
    }

    long next() {
      long z = (state += 0x9E3779B97F4A7C15L);
      z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
      z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
      return z ^ (z >>> 31);
    }

    int below(int bound) {
      return (int) Long.remainderUnsigned(next(), bound);
    }

    double unit() {
      return (next() >>> 11) * 0x1.0p-53;
    }

    double value() {
      switch (below(6)) {
        case 0:
          return Double.longBitsToDouble(next());
        case 1:
          return unit() * 2 - 1;
        case 2:
          return unit() * 20 - 10;
        case 3:
          return unit() * 2e6 - 1e6;
        case 4:
          {
            long sign = next() & 0x8000000000000000L;
            long exponent = (long) below(1024) << 52;
            long mantissa = next() & 0x000FFFFFFFFFFFFFL;
            return Double.longBitsToDouble(sign | exponent | mantissa);
          }
        default:
          {
            double c = CENTERS[below(CENTERS.length)];
            long j = below(64) - 32;
            double v = Double.longBitsToDouble(Double.doubleToRawLongBits(c) + j);
            return (next() & 1) == 0 ? v : -v;
          }
      }
    }

    double exponent() {
      switch (below(4)) {
        case 0:
          return value();
        case 1:
          return below(41) - 20;
        case 2:
          return below(41) - 20 + 0.5;
        default:
          return unit() * 10 - 5;
      }
    }
  }

  static String name;
  static int differ;
  static StringBuilder firsts;
  static long hStrict, hMath;
  static int count;
  static StringBuilder chunks;

  static void begin(String n) {
    name = n;
    differ = 0;
    firsts = new StringBuilder();
    hStrict = 0;
    hMath = 0;
    count = 0;
    chunks = new StringBuilder();
  }

  static String hex(double d) {
    return Long.toHexString(Double.doubleToRawLongBits(d));
  }

  static void record(double strict, double math, String input) {
    long s = Double.doubleToLongBits(strict);
    long m = Double.doubleToLongBits(math);
    if (s != m) {
      if (differ < 5) {
        firsts.append(' ').append(input).append("->").append(hex(math)).append('/').append(hex(strict));
      }
      differ++;
    }
    hStrict = 31 * hStrict + s;
    hMath = 31 * hMath + m;
    if (++count % CHUNK == 0) flush();
  }

  static void flush() {
    chunks.append("chunk ").append(name).append(' ').append(count).append(' ')
        .append(Long.toHexString(hStrict)).append(' ').append(Long.toHexString(hMath)).append('\n');
    hStrict = 0;
    hMath = 0;
  }

  static void end() {
    if (count % CHUNK != 0) flush();
    System.out.print(chunks);
    System.out.println("fn " + name + " " + count + " " + differ + firsts);
  }

  static void unary(int index, String n, long seed, int total, DoubleUnaryOperator strict, DoubleUnaryOperator math) {
    begin(n);
    for (double x : EDGES) record(strict.applyAsDouble(x), math.applyAsDouble(x), hex(x));
    Gen g = new Gen(seed + index);
    for (int i = 0; i < total; i++) {
      double x = g.value();
      record(strict.applyAsDouble(x), math.applyAsDouble(x), hex(x));
    }
    end();
  }

  static void binary(int index, String n, long seed, int total, boolean exponent, DoubleBinaryOperator strict, DoubleBinaryOperator math) {
    begin(n);
    for (double x : EDGES) {
      for (double y : EDGES) {
        record(strict.applyAsDouble(x, y), math.applyAsDouble(x, y), hex(x) + "," + hex(y));
      }
    }
    Gen g = new Gen(seed + index);
    for (int i = 0; i < total; i++) {
      double x = g.value();
      double y = exponent ? g.exponent() : g.value();
      record(strict.applyAsDouble(x, y), math.applyAsDouble(x, y), hex(x) + "," + hex(y));
    }
    end();
  }

  public static void main(String[] args) {
    long seed = Long.parseLong(args[0]);
    int n = Integer.parseInt(args[1]);
    unary(0, "sin", seed, n, StrictMath::sin, Math::sin);
    unary(1, "cos", seed, n, StrictMath::cos, Math::cos);
    unary(2, "tan", seed, n, StrictMath::tan, Math::tan);
    unary(3, "asin", seed, n, StrictMath::asin, Math::asin);
    unary(4, "acos", seed, n, StrictMath::acos, Math::acos);
    unary(5, "atan", seed, n, StrictMath::atan, Math::atan);
    unary(6, "sqrt", seed, n, StrictMath::sqrt, Math::sqrt);
    binary(7, "atan2", seed, n, false, StrictMath::atan2, Math::atan2);
    binary(8, "pow", seed, n, true, StrictMath::pow, Math::pow);
    binary(9, "IEEEremainder", seed, n, false, StrictMath::IEEEremainder, Math::IEEEremainder);
  }
}
