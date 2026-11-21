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
package org.apache.lucene.util.quantization;

import java.util.Locale;
import org.apache.lucene.index.VectorSimilarityFunction;

/**
 * Prints the exact result of {@link OptimizedScalarQuantizer#scalarQuantize} for
 * a vector and a centroid given on the command line, so a port can be compared
 * against it bit for bit rather than by inspection. It lives in Lucene's own
 * package so it can also reach the package-private helpers.
 *
 * <p>Usage: {@code java ... OsqTraceFixture <bits> <SIMILARITY> <v0,v1,..> <c0,c1,..>},
 * with every float given as its raw bits in hexadecimal.
 */
public final class OsqTraceFixture {

  private OsqTraceFixture() {}

  static float[] parse(String s) {
    String[] parts = s.split(",");
    float[] out = new float[parts.length];
    for (int i = 0; i < parts.length; i++) {
      out[i] = Float.intBitsToFloat((int) Long.parseLong(parts[i], 16));
    }
    return out;
  }

  public static void main(String[] args) {
    byte bits = Byte.parseByte(args[0]);
    VectorSimilarityFunction sim = VectorSimilarityFunction.valueOf(args[1]);
    float[] vector = parse(args[2]);
    float[] centroid = parse(args[3]);
    byte[] dest = new byte[vector.length];
    OptimizedScalarQuantizer q =
        args.length > 5
            ? new OptimizedScalarQuantizer(sim, Float.parseFloat(args[4]), Integer.parseInt(args[5]))
            : new OptimizedScalarQuantizer(sim);
    OptimizedScalarQuantizer.QuantizationResult r = q.scalarQuantize(vector, dest, bits, centroid);
    StringBuilder sb = new StringBuilder("dest=");
    for (int i = 0; i < dest.length; i++) {
      if (i > 0) {
        sb.append(',');
      }
      sb.append(dest[i] & 0xff);
    }
    System.out.println(sb);
    System.out.println(
        String.format(
            Locale.ROOT,
            "lower=%08x upper=%08x corr=%08x sum=%d",
            Float.floatToIntBits(r.lowerInterval()),
            Float.floatToIntBits(r.upperInterval()),
            Float.floatToIntBits(r.additionalCorrection()),
            r.quantizedComponentSum()));
    StringBuilder res = new StringBuilder("residual=");
    for (int i = 0; i < vector.length; i++) {
      if (i > 0) {
        res.append(',');
      }
      res.append(String.format(Locale.ROOT, "%08x", Float.floatToIntBits(vector[i])));
    }
    System.out.println(res);
    System.out.println("read_ok=true");
  }
}
