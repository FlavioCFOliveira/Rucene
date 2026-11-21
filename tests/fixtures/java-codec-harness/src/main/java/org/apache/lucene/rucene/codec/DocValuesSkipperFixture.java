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

import java.nio.file.Path;
import java.nio.file.Paths;
import org.apache.lucene.codecs.DocValuesFormat;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.lucene90.Lucene90DocValuesFormat;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.DocValuesSkipper;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.SortedNumericDocValues;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.FixedBitSet;

/**
 * Writes one segment whose doc-values fields carry a skip index, then walks
 * every skipper and evaluates {@code rangeIntoBitSet} with Lucene's own reader,
 * printing what it sees so the Rust portability test can compare.
 *
 * <p>The values follow fixed formulas the Rust test mirrors:
 *
 * <ul>
 *   <li>{@code num}: dense numeric, GCD 1 and minimum 0, one block;
 *   <li>{@code gcd}: dense numeric with a GCD of 6 and a minimum of 1000;
 *   <li>{@code snfix}: dense sorted numeric, exactly two values per document;
 *   <li>{@code snvar}: sparse sorted numeric, one to three values per document;
 *   <li>{@code snconst}: dense sorted numeric, two identical constant values;
 *   <li>{@code seq}: dense numeric, increasing with noise, written with a skip-index interval of
 *       {@value #SEQ_INTERVAL} documents so that all four skip levels exist.
 * </ul>
 *
 * <p>Besides the interval-by-interval walk, each field is walked with targets several intervals
 * and level groups ahead ({@code jump=} lines), which takes the per-level jump of {@code
 * Lucene90DocValuesProducer.getSkipper} ({@code Lucene90DocValuesProducer.java:2408-2411}), and
 * with {@code DocValuesSkipper.advance(long, long)} over value ranges ({@code vrange=} lines).
 *
 * <p>Usage: {@code DocValuesSkipperFixture <dir>}.
 */
public final class DocValuesSkipperFixture {
  static final int MAX_DOC = 40000;
  static final String[] SKIPPER_FIELDS = {"num", "gcd", "snfix", "snvar", "snconst"};
  static final String[] JUMP_FIELDS = {"num", "gcd", "snfix", "snvar", "snconst", "seq"};
  static final int SEQ_INTERVAL = 16;
  static final int[] JUMP_STRIDES = {700, 1500, 5000, 9000, 13000, 33000};
  static final long[][] VALUE_RANGES = {
    {43, 50}, {2000, 2100}, {5000, 5000}, {9000, 9500}, {13000, 13010}
  };

  private DocValuesSkipperFixture() {}

  static long numValue(int doc) {
    return (doc * 7919L) % 100003L;
  }

  public static void main(String[] args) throws Exception {
    Path dir = Paths.get(args[0]);
    try (FSDirectory directory = FSDirectory.open(dir)) {
      IndexWriterConfig config = new IndexWriterConfig();
      DocValuesFormat seqFormat = new Lucene90DocValuesFormat(SEQ_INTERVAL);
      config.setCodec(
          new Lucene104Codec() {
            @Override
            public DocValuesFormat getDocValuesFormatForField(String field) {
              return "seq".equals(field) ? seqFormat : super.getDocValuesFormatForField(field);
            }
          });
      try (IndexWriter writer = new IndexWriter(directory, config)) {
        for (int doc = 0; doc < MAX_DOC; doc++) {
          Document d = new Document();
          d.add(NumericDocValuesField.indexedField("num", numValue(doc)));
          d.add(NumericDocValuesField.indexedField("gcd", numValue(doc) * 6 + 1000));
          long a = (doc * 31L) % 5000L;
          d.add(SortedNumericDocValuesField.indexedField("snfix", a));
          d.add(SortedNumericDocValuesField.indexedField("snfix", a + (doc % 11) * 1000L));
          if (doc % 3 != 0) {
            for (int i = 0; i < 1 + doc % 3; i++) {
              d.add(
                  SortedNumericDocValuesField.indexedField("snvar", (doc * 13L + i * 977L) % 20000L));
            }
          }
          d.add(SortedNumericDocValuesField.indexedField("snconst", 42));
          d.add(SortedNumericDocValuesField.indexedField("snconst", 42));
          d.add(NumericDocValuesField.indexedField("seq", doc / 3 + doc % 5));
          writer.addDocument(d);
        }
        writer.forceMerge(1);
      }

      try (DirectoryReader reader = DirectoryReader.open(directory)) {
        LeafReader leaf = reader.leaves().get(0).reader();
        System.out.println("max_doc=" + leaf.maxDoc());
        for (String field : SKIPPER_FIELDS) {
          DocValuesSkipper skipper = leaf.getDocValuesSkipper(field);
          System.out.println(
              "skipper="
                  + field
                  + " min="
                  + skipper.minValue()
                  + " max="
                  + skipper.maxValue()
                  + " docCount="
                  + skipper.docCount()
                  + " maxValueCount="
                  + skipper.maxValueCount());
          int target = 0;
          while (true) {
            skipper.advance(target);
            if (skipper.minDocID(0) == DocIdSetIterator.NO_MORE_DOCS) {
              System.out.println("interval=" + field + " exhausted");
              break;
            }
            StringBuilder sb = new StringBuilder();
            sb.append("interval=").append(field).append(" levels=").append(skipper.numLevels());
            for (int level = 0; level < skipper.numLevels(); level++) {
              sb.append(" [")
                  .append(skipper.minDocID(level))
                  .append(',')
                  .append(skipper.maxDocID(level))
                  .append(',')
                  .append(skipper.minValue(level))
                  .append(',')
                  .append(skipper.maxValue(level))
                  .append(',')
                  .append(skipper.docCount(level))
                  .append(']');
            }
            System.out.println(sb);
            target = skipper.maxDocID(0) + 1;
          }
        }

        for (String field : JUMP_FIELDS) {
          for (int stride : JUMP_STRIDES) {
            DocValuesSkipper skipper = leaf.getDocValuesSkipper(field);
            int target = stride;
            while (true) {
              skipper.advance(target);
              if (skipper.minDocID(0) == DocIdSetIterator.NO_MORE_DOCS) {
                System.out.println("jump=" + field + " stride=" + stride + " exhausted");
                break;
              }
              System.out.println(
                  "jump=" + field + " stride=" + stride + " target=" + target + levels(skipper));
              target = Math.max(skipper.maxDocID(0) + 1, target + stride);
            }
          }
          for (long[] range : VALUE_RANGES) {
            DocValuesSkipper skipper = leaf.getDocValuesSkipper(field);
            while (true) {
              skipper.advance(range[0], range[1]);
              if (skipper.minDocID(0) == DocIdSetIterator.NO_MORE_DOCS) {
                System.out.println(
                    "vrange=" + field + " " + range[0] + ".." + range[1] + " exhausted");
                break;
              }
              System.out.println(
                  "vrange=" + field + " " + range[0] + ".." + range[1] + levels(skipper));
              skipper.advance(skipper.maxDocID(0) + 1);
            }
          }
        }

        printNumericRange(leaf, "num", 100, 30000, 20000, 60000);
        printNumericRange(leaf, "gcd", 100, 30000, 100000, 300000);
        printSortedNumericRange(leaf, "snfix", 100, 30000, 2000, 2500);
        printSortedNumericRange(leaf, "snvar", 100, 30000, 5000, 9000);
        printSortedNumericRange(leaf, "snconst", 100, 30000, 40, 50);
        printSortedNumericRange(leaf, "snconst", 100, 30000, 43, 50);
      }
    }
    System.out.println("done=true");
  }

  static String levels(DocValuesSkipper skipper) {
    StringBuilder sb = new StringBuilder();
    sb.append(" levels=").append(skipper.numLevels());
    for (int level = 0; level < skipper.numLevels(); level++) {
      sb.append(" [")
          .append(skipper.minDocID(level))
          .append(',')
          .append(skipper.maxDocID(level))
          .append(',')
          .append(skipper.minValue(level))
          .append(',')
          .append(skipper.maxValue(level))
          .append(',')
          .append(skipper.docCount(level))
          .append(']');
    }
    return sb.toString();
  }

  static void printNumericRange(
      LeafReader leaf, String field, int from, int to, long min, long max) throws Exception {
    NumericDocValues values = leaf.getNumericDocValues(field);
    FixedBitSet bits = new FixedBitSet(to - from);
    values.rangeIntoBitSet(from, to, min, max, bits, from);
    System.out.println(describe(field, bits, values.docID()));
  }

  static void printSortedNumericRange(
      LeafReader leaf, String field, int from, int to, long min, long max) throws Exception {
    SortedNumericDocValues values = leaf.getSortedNumericDocValues(field);
    FixedBitSet bits = new FixedBitSet(to - from);
    values.rangeIntoBitSet(from, to, min, max, bits, from);
    System.out.println(describe(field, bits, values.docID()));
  }

  static String describe(String field, FixedBitSet bits, int docId) {
    long hash = 0;
    for (int i = bits.nextSetBit(0);
        i != DocIdSetIterator.NO_MORE_DOCS;
        i = i + 1 < bits.length() ? bits.nextSetBit(i + 1) : DocIdSetIterator.NO_MORE_DOCS) {
      hash = hash * 31 + i;
    }
    return "range="
        + field
        + " cardinality="
        + bits.cardinality()
        + " hash="
        + hash
        + " docID="
        + docId;
  }
}
