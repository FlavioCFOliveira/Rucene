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

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.HexFormat;
import java.util.List;
import java.util.TreeSet;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.store.FSDirectory;

/**
 * Reference tool for the merge of segments that carry an index sort: three
 * segments sorted by the numeric doc-values field {@code rank} are written by a
 * real Apache Lucene Core 10.5.0 {@link IndexWriter} with the default codec and
 * force-merged into one. Their {@code rank} values interleave, so {@code
 * MergeState.buildDocMaps} merge-sorts them through {@code MultiSorter} instead
 * of concatenating them. The index is left in the output directory; the tool
 * prints the merged segment's name, id and document count, and the name and
 * length of every file it is made of.
 *
 * <p>This is the output Rucene's sorted merge is judged against ({@code
 * tests/index_writer_sorted_merge.rs}), which builds the same documents with
 * {@link #doc(int)}'s Rust twin and compares every merged file byte for byte.
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... SortedMergeFixture &lt;output-dir&gt;
 * </pre>
 */
public final class SortedMergeFixture {

  private SortedMergeFixture() {}

  /** Documents per segment. */
  static final int[] SIZES = {300, 300, 300};

  /** Document {@code i}. Integer arithmetic only, so a port reproduces every value. */
  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", String.format("id%05d", i), Field.Store.NO));
    d.add(new NumericDocValuesField("rank", (i * 7919L) % 1000L));
    d.add(new StoredField("sv", "v" + i));
    d.add(new TextField("body", "all w" + (i % 31) + " x" + i, Field.Store.NO));
    return d;
  }

  /** Runs the case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 1) {
      throw new IllegalArgumentException("usage: SortedMergeFixture <output-dir>");
    }
    Path out = Paths.get(args[0]);
    Files.createDirectories(out);
    try (var listing = Files.list(out)) {
      for (Path file : listing.toList()) {
        Files.delete(file);
      }
    }
    System.out.println("version=" + org.apache.lucene.util.Version.LATEST);

    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig config = new IndexWriterConfig();
      config.setIndexSort(new Sort(new SortField("rank", SortField.Type.LONG)));
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        int i = 0;
        for (int size : SIZES) {
          for (int k = 0; k < size; k++, i++) {
            writer.addDocument(doc(i));
          }
          writer.commit();
        }
        System.out.println("segmentsBeforeMerge=" + SegmentInfos.readLatestCommit(dir).size());
        writer.forceMerge(1);
        writer.commit();
      }
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      SegmentCommitInfo merged = infos.info(0);
      System.out.println("segments=" + infos.size());
      System.out.println("name=" + merged.info.name);
      System.out.println("id=" + HexFormat.of().formatHex(merged.info.getId()));
      System.out.println("compound=" + merged.info.getUseCompoundFile());
      System.out.println("maxDoc=" + merged.info.maxDoc() + " delCount=" + merged.getDelCount());
      List<String> files = List.copyOf(new TreeSet<>(merged.files()));
      for (String file : files) {
        System.out.println("file " + file + " " + Files.size(out.resolve(file)));
      }
    }
  }
}
