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
package org.apache.lucene.util.automaton;

import java.util.function.BiConsumer;

/** Reaches the package-private {@code IntSet} hierarchy. */
public final class RuceneAutomatonContracts {
  private RuceneAutomatonContracts() {}

  public static void emit(BiConsumer<String, String> out) {
    // FrozenIntSet(int[] values, long hashCode, int state)
    FrozenIntSet empty = new FrozenIntSet(new int[0], 0L, 0);
    out.accept("IntSet.hashCode/long=0", Integer.toString(empty.hashCode()));

    FrozenIntSet one = new FrozenIntSet(new int[] {1, 2, 3}, 0x0123456789ABCDEFL, 7);
    out.accept("IntSet.hashCode/long=0x0123456789ABCDEF", Integer.toString(one.hashCode()));

    FrozenIntSet neg = new FrozenIntSet(new int[] {5}, -1L, 1);
    out.accept("IntSet.hashCode/long=-1", Integer.toString(neg.hashCode()));
  }
}
