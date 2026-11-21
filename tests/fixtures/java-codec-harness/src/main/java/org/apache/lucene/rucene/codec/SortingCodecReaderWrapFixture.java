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

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.index.CodecReader;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FilterCodecReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.SortingCodecReader;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.search.SortedNumericSelector;
import org.apache.lucene.search.SortedNumericSortField;
import org.apache.lucene.search.SortedSetSelector;
import org.apache.lucene.search.SortedSetSortField;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.util.BytesRef;

/**
 * Reference tool for {@code SortingCodecReader.wrap(CodecReader, Sort)}: one segment of five
 * documents is wrapped with every sort below, and the tool prints the stored ids in the wrapped
 * reader's document order, the sort its metadata reports and its cache helpers — or the exception
 * the wrap raises. A line wraps an already sorted view again. The last two wrap segments of
 * document blocks, with and without a parent field ({@code Sorter.java:190-207}): with one, a
 * block sorts by the value of its parent (last) document; without one, an index created by
 * Lucene 10 is refused as corrupt.
 *
 * <p>{@code tests/sorting_codec_reader_wrap.rs} builds the same segment and sorts with Rucene.
 *
 * <p>Command-line usage: {@code java -cp lucene-core-10.5.0.jar SortingCodecReaderWrapFixture.java}.
 */
public final class SortingCodecReaderWrapFixture {

  private SortingCodecReaderWrapFixture() {}

  /** The value of {@code l} per document. */
  static final long[] LS = {5, 3, 9, 3, 1};

  static List<Sort> sorts() {
    SortField sparseMissing = new SortField("sparse", SortField.Type.LONG);
    sparseMissing.setMissingValue(100L);
    return new ArrayList<>(
        Arrays.asList(
            new Sort(new SortField("l", SortField.Type.LONG)),
            new Sort(new SortField("l", SortField.Type.LONG, true)),
            new Sort(new SortField("s", SortField.Type.STRING)),
            new Sort(new SortField("sparse", SortField.Type.LONG)),
            new Sort(sparseMissing),
            new Sort(new SortedNumericSortField("sn", SortField.Type.LONG)),
            new Sort(
                new SortedNumericSortField(
                    "sn", SortField.Type.LONG, false, SortedNumericSelector.Type.MAX)),
            new Sort(new SortedSetSortField("ss", false)),
            new Sort(new SortedSetSortField("ss", true, SortedSetSelector.Type.MAX)),
            new Sort(new SortedNumericSortField("l", SortField.Type.LONG)),
            new Sort(new SortedSetSortField("s", false)),
            new Sort(new SortField("id", SortField.Type.LONG)),
            new Sort(new SortField("absent", SortField.Type.LONG)),
            new Sort(new SortField("id", SortField.Type.STRING)),
            new Sort(new SortedNumericSortField("id", SortField.Type.LONG)),
            new Sort(new SortedSetSortField("id", false)),
            new Sort(SortField.FIELD_DOC),
            new Sort(SortField.FIELD_SCORE),
            new Sort(
                new SortField("l", SortField.Type.LONG), new SortField("s", SortField.Type.STRING))));
  }

  /** Runs the cases. */
  public static void main(String[] args) throws Exception {
    Directory d = new ByteBuffersDirectory();
    try (IndexWriter w = new IndexWriter(d, new IndexWriterConfig())) {
      for (int i = 0; i < LS.length; i++) {
        Document doc = new Document();
        doc.add(new StoredField("id", "d" + i));
        doc.add(new NumericDocValuesField("l", LS[i]));
        doc.add(new SortedDocValuesField("s", new BytesRef("s" + (9 - LS[i]))));
        if (i != 2) {
          doc.add(new NumericDocValuesField("sparse", 10 - i));
        }
        doc.add(new SortedNumericDocValuesField("sn", LS[i]));
        doc.add(new SortedNumericDocValuesField("sn", -LS[i]));
        doc.add(new SortedSetDocValuesField("ss", new BytesRef("a" + LS[i])));
        doc.add(new SortedSetDocValuesField("ss", new BytesRef("z" + i)));
        w.addDocument(doc);
      }
    }
    try (DirectoryReader r = DirectoryReader.open(d)) {
      CodecReader leaf = (CodecReader) r.leaves().get(0).reader();
      for (Sort sort : sorts()) {
        try {
          CodecReader sorted = SortingCodecReader.wrap(leaf, sort);
          StoredFields stored = sorted.storedFields();
          StringBuilder ids = new StringBuilder();
          for (int i = 0; i < sorted.maxDoc(); i++) {
            ids.append(i == 0 ? "" : " ").append(stored.document(i).get("id"));
          }
          System.out.println(
              "W "
                  + sort
                  + " -> "
                  + ids
                  + " | metaSort="
                  + sorted.getMetaData().sort()
                  + " core="
                  + sorted.getCoreCacheHelper()
                  + " readerCache="
                  + sorted.getReaderCacheHelper());
        } catch (Exception e) {
          System.out.println(
              "W " + sort + " -> " + e.getClass().getSimpleName() + ": " + e.getMessage());
        }
      }
      Sort byL = new Sort(new SortField("l", SortField.Type.LONG));
      CodecReader once = SortingCodecReader.wrap(leaf, byL);
      CodecReader twice = SortingCodecReader.wrap(once, byL);
      // Document blocks: three blocks of a child and its parent, the parents' values out of order.
      for (boolean withParent : new boolean[] {true, false}) {
        Directory bd = new ByteBuffersDirectory();
        IndexWriterConfig config = new IndexWriterConfig();
        if (withParent) {
          config.setParentField("parent");
        }
        try (IndexWriter w = new IndexWriter(bd, config)) {
          long[] parents = {7, 2, 5};
          for (int b = 0; b < parents.length; b++) {
            Document child = new Document();
            child.add(new StoredField("id", "b" + b + "c"));
            child.add(new NumericDocValuesField("l", 100 - b));
            Document parent = new Document();
            parent.add(new StoredField("id", "b" + b + "p"));
            parent.add(new NumericDocValuesField("l", parents[b]));
            w.addDocuments(List.of(child, parent));
          }
        }
        try (DirectoryReader br = DirectoryReader.open(bd)) {
          LeafReader blocks = br.leaves().get(0).reader();
          String label = withParent ? "B parent" : "B no-parent";
          try {
            CodecReader sorted =
                SortingCodecReader.wrap(
                    (CodecReader) blocks, new Sort(new SortField("l", SortField.Type.LONG)));
            StoredFields stored = sorted.storedFields();
            StringBuilder ids = new StringBuilder();
            for (int i = 0; i < sorted.maxDoc(); i++) {
              ids.append(i == 0 ? "" : " ").append(stored.document(i).get("id"));
            }
            System.out.println(label + " -> " + ids);
          } catch (Exception e) {
            System.out.println(
                label + " -> " + e.getClass().getSimpleName() + ": " + e.getMessage());
          }
        }
      }
      System.out.println(
          "A delegate="
              + (((FilterCodecReader) twice).getDelegate() == once)
              + " unwrap="
              + (FilterCodecReader.unwrap(twice) == leaf)
              + " toString="
              + twice.toString().equals("SortingCodecReader(" + once + ")")
              + " metaSort="
              + twice.getMetaData().sort()
              + " core="
              + twice.getCoreCacheHelper()
              + " readerCache="
              + twice.getReaderCacheHelper());
    }
  }
}
