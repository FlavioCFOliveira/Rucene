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
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.TreeSet;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Reference tool for the flush of a segment that holds document blocks and carries an index sort.
 * Blocks of one to four documents are added with {@code addDocuments}; only the last (parent)
 * document of a block carries the sort field {@code rank}, and the index declares the parent field
 * {@code _parent}. {@code IndexingChain.maybeSortSegment} compares the parent of each block, so the
 * flushed segment holds the blocks in {@code rank} order, each block contiguous and in its original
 * order. Every document also carries one value of each other doc-values kind, so the five
 * doc-values writers are flushed through the sort map.
 *
 * <p>The tool prints the segment's name, id and document count, and the name and length of every
 * file of the one flushed segment. {@code tests/index_writer_sorted_blocks_flush.rs} builds the same
 * blocks with {@link #block(int)}'s Rust twin and compares every file byte for byte.
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... SortedBlocksFlushFixture &lt;output-dir&gt;
 * </pre>
 */
public final class SortedBlocksFlushFixture {

  private SortedBlocksFlushFixture() {}

  /** Number of blocks. */
  static final int BLOCKS = 40;

  /** Block {@code b}: {@code (b * 3) % 4 + 1} documents, the last one the parent. */
  static List<Document> block(int b) {
    int size = (b * 3) % 4 + 1;
    List<Document> docs = new ArrayList<>();
    for (int k = 0; k < size; k++) {
      int i = b * 10 + k;
      Document d = new Document();
      d.add(new StringField("id", "id" + i, Field.Store.YES));
      d.add(new TextField("body", "w" + (i % 7) + " x" + (b % 5), Field.Store.NO));
      d.add(new SortedDocValuesField("kind", new BytesRef("k" + (i % 6))));
      d.add(new BinaryDocValuesField("blob", new BytesRef("b" + (i * 31 % 97))));
      d.add(new SortedNumericDocValuesField("multi", (i * 13) % 50));
      d.add(new SortedNumericDocValuesField("multi", (i * 7) % 50));
      d.add(new SortedSetDocValuesField("tags", new BytesRef("t" + (i % 4))));
      d.add(new SortedSetDocValuesField("tags", new BytesRef("t" + (i % 3))));
      if (k == size - 1) {
        d.add(new NumericDocValuesField("rank", (b * 7919L) % 101L));
      }
      docs.add(d);
    }
    return docs;
  }

  /** Runs the case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 1) {
      throw new IllegalArgumentException("usage: SortedBlocksFlushFixture <output-dir>");
    }
    Path out = Paths.get(args[0]);
    Files.createDirectories(out);
    try (var listing = Files.list(out)) {
      for (Path file : listing.toList()) {
        Files.delete(file);
      }
    }
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig config = new IndexWriterConfig();
      config.setIndexSort(new Sort(new SortField("rank", SortField.Type.LONG)));
      config.setParentField("_parent");
      config.setUseCompoundFile(false);
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        for (int b = 0; b < BLOCKS; b++) {
          writer.addDocuments(block(b));
        }
        writer.commit();
      }
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      SegmentCommitInfo segment = infos.info(0);
      System.out.println("segments=" + infos.size());
      System.out.println("name=" + segment.info.name);
      System.out.println("id=" + HexFormat.of().formatHex(segment.info.getId()));
      System.out.println("maxDoc=" + segment.info.maxDoc());
      for (String file : new TreeSet<>(segment.files())) {
        System.out.println("file " + file + " " + Files.size(out.resolve(file)));
      }
    }
  }
}
