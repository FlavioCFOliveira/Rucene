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

import java.util.SplittableRandom;

/**
 * Prints the exact {@code java.util.SplittableRandom} draw sequence Lucene's
 * {@code HnswGraphBuilder} consumes, so that the Rust port can be measured
 * against it instead of being written from a guess at the algorithm.
 *
 * <p>{@code HnswGraphBuilder} seeds a {@code SplittableRandom} with
 * {@code HnswGraphBuilder.randSeed} (42 by default) and draws
 * {@code nextDouble()} once per node — retrying on an exact zero — to pick the
 * node's graph level as {@code (int)(-log(randDouble) * ml)}, where
 * {@code ml = M == 1 ? 1 : 1 / log(M)}.
 *
 * <p>The raw bits of each draw are printed, not a decimal rendering, so the
 * comparison cannot be blurred by formatting.
 *
 * <p>Command-line usage: {@code java ... HnswRandomFixture <seed> <count> <M>}.
 */
public final class HnswRandomFixture {

  private HnswRandomFixture() {}

  public static void main(String[] args) {
    long seed = args.length > 0 ? Long.parseLong(args[0]) : 42L;
    int count = args.length > 1 ? Integer.parseInt(args[1]) : 32;
    int m = args.length > 2 ? Integer.parseInt(args[2]) : 16;
    double ml = m == 1 ? 1 : 1 / Math.log(1.0 * m);

    SplittableRandom random = new SplittableRandom(seed);
    System.out.println("ml=" + Long.toHexString(Double.doubleToRawLongBits(ml)));
    for (int i = 0; i < count; i++) {
      double randDouble;
      do {
        randDouble = random.nextDouble();
      } while (randDouble == 0.0);
      int level = (int) (-Math.log(randDouble) * ml);
      System.out.println(
          "draw="
              + i
              + ",bits="
              + Long.toHexString(Double.doubleToRawLongBits(randDouble))
              + ",level="
              + level);
    }
    System.out.println("read_ok=true");
  }
}
