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

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.util.HashMap;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.util.ByteBlockPool;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.Counter;

/**
 * Reference tool for the RAM the five doc-values writers charge while documents are buffered.
 * One writer of each kind — numeric, binary (some values longer than a 4 kB {@code PagedBytes}
 * page), sorted, sorted-numeric and sorted-set, sparse and multi-valued — is built exactly as
 * {@code IndexingChain.initializeFieldInfo} builds it: all five charge one {@code Counter}, and the
 * two sorted kinds intern into one {@code ByteBlockPool} over a {@code DirectTrackingAllocator} of
 * that counter, the chain's {@code docValuesBytePool}. The writers are package-private, so they are
 * reached by reflection. After every document the tool prints the counter.
 *
 * <p>{@code tests/doc_values_writer_ram.rs} feeds the same values to Rucene's writers and compares
 * the counter document by document.
 *
 * <p>Command-line usage: {@code java ... DocValuesRamFixture}.
 */
public final class DocValuesRamFixture {

  private DocValuesRamFixture() {}

  /** Number of documents. */
  static final int DOCS = 400;

  private static FieldInfo field(String name, int number, DocValuesType type) {
    return new FieldInfo(
        name, number, false, false, false, IndexOptions.NONE, type, DocValuesSkipIndexType.NONE,
        -1, new HashMap<>(), 0, 0, 0, 0, VectorEncoding.FLOAT32,
        VectorSimilarityFunction.EUCLIDEAN, false, false);
  }

  private static Object writer(String className, Class<?>[] types, Object... args)
      throws Exception {
    Constructor<?> ctor =
        Class.forName("org.apache.lucene.index." + className).getDeclaredConstructor(types);
    ctor.setAccessible(true);
    return ctor.newInstance(args);
  }

  private static void add(Object writer, Class<?> valueType, int docID, Object value)
      throws Exception {
    Method add = writer.getClass().getDeclaredMethod("addValue", int.class, valueType);
    add.setAccessible(true);
    add.invoke(writer, docID, value);
  }

  /** Runs the case. */
  public static void main(String[] args) throws Exception {
    Counter bytesUsed = Counter.newCounter();
    ByteBlockPool docValuesBytePool =
        new ByteBlockPool(new ByteBlockPool.DirectTrackingAllocator(bytesUsed));
    // The batch scratch charges the chain's counter for itself, not for the writers.
    Object scratch =
        writer("SharedIndexingScratch", new Class<?>[] {Counter.class}, Counter.newCounter());
    Class<?> scratchClass = Class.forName("org.apache.lucene.index.SharedIndexingScratch");
    Class<?>[] plain = {FieldInfo.class, Counter.class};
    Class<?>[] pooled = {FieldInfo.class, Counter.class, ByteBlockPool.class, scratchClass};
    Object num =
        writer("NumericDocValuesWriter", plain, field("num", 0, DocValuesType.NUMERIC), bytesUsed);
    Object blob =
        writer("BinaryDocValuesWriter", plain, field("blob", 1, DocValuesType.BINARY), bytesUsed);
    Object kind =
        writer(
            "SortedDocValuesWriter",
            pooled,
            field("kind", 2, DocValuesType.SORTED),
            bytesUsed,
            docValuesBytePool,
            scratch);
    Object multi =
        writer(
            "SortedNumericDocValuesWriter",
            plain,
            field("multi", 3, DocValuesType.SORTED_NUMERIC),
            bytesUsed);
    Object tags =
        writer(
            "SortedSetDocValuesWriter",
            pooled,
            field("tags", 4, DocValuesType.SORTED_SET),
            bytesUsed,
            docValuesBytePool,
            scratch);
    System.out.println("init " + bytesUsed.get());
    for (int i = 0; i < DOCS; i++) {
      if (i % 5 != 3) {
        add(num, long.class, i, (i * 7919L) % 100003L - 50000L);
      }
      if (i % 3 != 1) {
        int length = (i % 17 == 0) ? 5000 + i : (i * 13) % 40;
        byte[] value = new byte[length];
        for (int k = 0; k < length; k++) {
          value[k] = (byte) ((i + k) % 251);
        }
        add(blob, BytesRef.class, i, new BytesRef(value));
      }
      if (i % 4 != 2) {
        add(kind, BytesRef.class, i, new BytesRef("kind" + ((i * 31) % 97)));
      }
      for (int k = 0; k < i % 4; k++) {
        add(multi, long.class, i, ((long) i * 1000003L + k * 17L) % 5000L);
      }
      for (int k = 0; k < (i * 7) % 13; k++) {
        add(tags, BytesRef.class, i, new BytesRef("tag" + ((i + k * 11) % 230)));
      }
      System.out.println("doc " + i + " " + bytesUsed.get());
    }
  }
}
