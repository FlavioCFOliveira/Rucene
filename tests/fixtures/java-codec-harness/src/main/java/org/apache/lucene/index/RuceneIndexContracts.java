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
package org.apache.lucene.index;

import java.util.function.BiConsumer;

import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.Counter;

/**
 * Reaches the package-private {@code Sorter} constructor and the package-private
 * {@code FieldUpdatesBuffer}, whose accessors are measured here rather than
 * derived from the port's own arithmetic.
 */
public final class RuceneIndexContracts {
  private RuceneIndexContracts() {}

  public static void emit(BiConsumer<String, String> out) {
    Sort sort = new Sort(new SortField("price", SortField.Type.LONG));
    Sorter sorter = new Sorter(sort);
    out.accept("Sorter.toString/price-long", sorter.toString());
    out.accept("Sorter.getID/price-long", sorter.getID());

    fieldUpdatesBuffer(out);
  }

  private static DocValuesUpdate.NumericDocValuesUpdate numeric(String value, long v) {
    return new DocValuesUpdate.NumericDocValuesUpdate(
        new Term("id", new BytesRef(value)), "price", v);
  }

  /**
   * {@code getMinNumeric()}, {@code getMaxNumeric()}, {@code hasSingleValue()}
   * and {@code getNumericValue(int)} on a buffer of numeric doc-values updates.
   *
   * <p>Three shapes: every update carrying the same value (which is what keeps
   * Lucene's {@code numericValues} array at length one), updates carrying
   * different values, and a buffer that only ever received no-value updates.
   */
  private static void fieldUpdatesBuffer(BiConsumer<String, String> out) {
    FieldUpdatesBuffer same =
        new FieldUpdatesBuffer(Counter.newCounter(), numeric("a", 7L), 3);
    same.addUpdate(new Term("id", new BytesRef("b")), 7L, 5);
    same.addUpdate(new Term("id", new BytesRef("c")), 7L, 9);
    out.accept("FieldUpdatesBuffer.hasSingleValue/same", String.valueOf(same.hasSingleValue()));
    out.accept("FieldUpdatesBuffer.getMinNumeric/same", String.valueOf(same.getMinNumeric()));
    out.accept("FieldUpdatesBuffer.getMaxNumeric/same", String.valueOf(same.getMaxNumeric()));
    for (int i = 0; i < 3; i++) {
      out.accept(
          "FieldUpdatesBuffer.getNumericValue/same/" + i,
          String.valueOf(same.getNumericValue(i)));
    }

    FieldUpdatesBuffer mixed =
        new FieldUpdatesBuffer(Counter.newCounter(), numeric("a", -4L), 3);
    mixed.addUpdate(new Term("id", new BytesRef("b")), 11L, 5);
    mixed.addUpdate(new Term("id", new BytesRef("c")), 0L, 9);
    out.accept("FieldUpdatesBuffer.hasSingleValue/mixed", String.valueOf(mixed.hasSingleValue()));
    out.accept("FieldUpdatesBuffer.getMinNumeric/mixed", String.valueOf(mixed.getMinNumeric()));
    out.accept("FieldUpdatesBuffer.getMaxNumeric/mixed", String.valueOf(mixed.getMaxNumeric()));
    for (int i = 0; i < 3; i++) {
      out.accept(
          "FieldUpdatesBuffer.getNumericValue/mixed/" + i,
          String.valueOf(mixed.getNumericValue(i)));
    }

    // A reset carries no value at all: `hasValues` gets a bit, `numericValues`
    // stays at its single seeded slot, and both extremes stay at their seeds.
    FieldUpdatesBuffer none =
        new FieldUpdatesBuffer(
            Counter.newCounter(),
            new DocValuesUpdate.NumericDocValuesUpdate(
                new Term("id", new BytesRef("a")), "price", null),
            3);
    none.addNoValue(new Term("id", new BytesRef("b")), 5);
    out.accept("FieldUpdatesBuffer.hasSingleValue/none", String.valueOf(none.hasSingleValue()));
    out.accept("FieldUpdatesBuffer.getMinNumeric/none", String.valueOf(none.getMinNumeric()));
    out.accept("FieldUpdatesBuffer.getMaxNumeric/none", String.valueOf(none.getMaxNumeric()));
    for (int i = 0; i < 2; i++) {
      out.accept(
          "FieldUpdatesBuffer.getNumericValue/none/" + i,
          String.valueOf(none.getNumericValue(i)));
    }
  }
}
