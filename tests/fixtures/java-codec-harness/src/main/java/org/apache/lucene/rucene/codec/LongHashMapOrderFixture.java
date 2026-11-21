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

import java.util.HashMap;
import java.util.Map;
import java.util.StringJoiner;

/**
 * Reference tool for the iteration order of a {@code java.util.HashMap<Long, Integer>}: prints the
 * entry order after every operation of a deterministic script of puts, removes and {@code putAll}
 * copies over colliding keys, so that bins treeify, split and untreeify.
 *
 * <p>{@code tests/java_long_hash_map_order.rs} replays the script on {@code JavaLongHashMap}.
 *
 * <p>Command-line usage: {@code java LongHashMapOrderFixture.java}.
 */
public final class LongHashMapOrderFixture {

  private LongHashMapOrderFixture() {}

  static String order(Map<Long, Integer> map) {
    StringJoiner joiner = new StringJoiner(",");
    for (Map.Entry<Long, Integer> e : map.entrySet()) {
      joiner.add(e.getKey() + "=" + e.getValue());
    }
    return joiner.toString();
  }

  /** Runs the script. */
  public static void main(String[] args) {
    long[] universe = new long[3 * 40 + 8];
    int u = 0;
    for (long b : new long[] {0, 1, 5}) {
      for (long j = 0; j < 40; j++) {
        universe[u++] = b + 64 * j;
      }
    }
    for (long j = 1; j <= 8; j++) {
      universe[u++] = (j << 32) | 3;
    }

    HashMap<Long, Integer> map = new HashMap<>();
    // Phase 1: fill the table to 64 slots, then put 12 keys into bucket 0.
    for (long k = 1; k < 48; k++) {
      map.put(k + 1000, (int) k);
      System.out.println("put " + order(map));
    }
    for (long j = 1; j <= 12; j++) {
      map.put(64 * j, (int) j);
      System.out.println("put " + order(map));
    }
    // Phase 2: a random script over colliding keys.
    long x = 42;
    for (int i = 0; i < 3000; i++) {
      x = x * 6364136223846793005L + 1442695040888963407L;
      int op = (int) Long.remainderUnsigned(x >>> 33, 20);
      long key = universe[(int) Long.remainderUnsigned(x >>> 40, universe.length)];
      String tag;
      if (op < 13) {
        map.put(key, i);
        tag = "put";
      } else if (op < 19) {
        map.remove(key);
        tag = "remove";
      } else {
        HashMap<Long, Integer> copy = new HashMap<>();
        copy.putAll(map);
        map = copy;
        tag = "putAll";
      }
      System.out.println(tag + " " + order(map));
    }
  }
}
