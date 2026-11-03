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

import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.index.CodecReader;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.MergeState;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.store.ByteBuffersDirectory;

/**
 * Reference tool for the merge order {@code MultiSorter.sort} gives documents whose sort keys tie
 * across readers: three single-segment readers sorted by a long field, {@code [1, 2]}, {@code [1,
 * 2]} and {@code [2]}. The package-private method is reached by reflection, and the tool prints
 * the merged document number of every document, reader by reader.
 *
 * <p>{@code tests/multi_sorter_tie_break.rs} sorts the same readers with Rucene's {@code
 * MultiSorter}.
 *
 * <p>Command-line usage: {@code java -cp lucene-core-10.5.0.jar MultiSorterTieFixture.java}.
 */
public final class MultiSorterTieFixture {

  private MultiSorterTieFixture() {}

  /** The sort keys of each reader. */
  static final long[][] READERS = {{1, 2}, {1, 2}, {2}};

  private static CodecReader reader(long[] values) throws Exception {
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    try (IndexWriter w = new IndexWriter(dir, new IndexWriterConfig())) {
      for (long v : values) {
        Document d = new Document();
        d.add(new NumericDocValuesField("n", v));
        w.addDocument(d);
      }
    }
    return (CodecReader) DirectoryReader.open(dir).leaves().get(0).reader();
  }

  /** Runs the case. */
  public static void main(String[] args) throws Exception {
    List<CodecReader> readers = new ArrayList<>();
    for (long[] values : READERS) {
      readers.add(reader(values));
    }
    Method sort =
        Class.forName("org.apache.lucene.index.MultiSorter")
            .getDeclaredMethod("sort", Sort.class, List.class);
    sort.setAccessible(true);
    Object[] maps =
        (Object[]) sort.invoke(null, new Sort(new SortField("n", SortField.Type.LONG)), readers);
    for (int i = 0; i < readers.size(); i++) {
      StringBuilder line = new StringBuilder("reader ").append(i).append(':');
      for (int doc = 0; doc < readers.get(i).maxDoc(); doc++) {
        line.append(' ').append(((MergeState.DocMap) maps[i]).get(doc));
      }
      System.out.println(line);
    }
  }
}
