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
import java.util.concurrent.Executor;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.CodecReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.MergePolicy;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.OneMergeWrappingMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.Sorter;
import org.apache.lucene.index.TieredMergePolicy;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Reference tool for a merge whose {@code OneMerge.reorder} renumbers the documents: two
 * segments carrying postings, norms, stored fields, every kind of doc values, points and float
 * vectors are written by a real Apache Lucene Core 10.5.0 {@link IndexWriter} with the default
 * codec, then force-merged by a {@link TieredMergePolicy} whose merges reverse the document
 * order. {@code IndexWriter.mergeMiddle} reads the segments through {@code
 * SlowCompositeCodecReaderWrapper}, hands that view to {@code reorder}, and merges a {@code
 * SortingCodecReader} over it (IndexWriter.java:5340-5362).
 *
 * <p>The index is left in the output directory; the tool prints the merged segment's name, id
 * and document count, and the name and length of every file it is made of. This is the output
 * Rucene's reordering merge is judged against ({@code tests/index_writer_reorder_merge.rs}).
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... ReorderMergeFixture &lt;output-dir&gt;
 * </pre>
 */
public final class ReorderMergeFixture {

  private ReorderMergeFixture() {}

  /** Documents per segment. */
  static final int[] SIZES = {40, 60};

  /** Document {@code i}. Integer arithmetic only, so a port reproduces every value. */
  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", String.format("id%04d", i), Field.Store.NO));
    d.add(new TextField("body", "w" + (i % 7) + " w" + (i % 13) + " x" + i, Field.Store.NO));
    d.add(new StoredField("sv", "v" + i));
    d.add(new NumericDocValuesField("ndv", (i * 31L) % 100L));
    d.add(new BinaryDocValuesField("bdv", new BytesRef("b" + (i % 17))));
    d.add(new SortedDocValuesField("sdv", new BytesRef("s" + (i % 11))));
    d.add(new SortedNumericDocValuesField("sndv", i % 5));
    d.add(new SortedNumericDocValuesField("sndv", i % 3));
    d.add(new SortedSetDocValuesField("ssdv", new BytesRef("a" + (i % 4))));
    d.add(new SortedSetDocValuesField("ssdv", new BytesRef("b" + (i % 6))));
    d.add(new IntPoint("pt", i, (i * 7) % 50));
    d.add(new KnnFloatVectorField("vec", new float[] {i % 9, (i * 3) % 7, 1, i % 4}));
    return d;
  }

  /** The reversing doc map {@code reorder} answers with. */
  static Sorter.DocMap reverse(int maxDoc) {
    return new Sorter.DocMap() {
      @Override
      public int oldToNew(int docID) {
        return maxDoc - 1 - docID;
      }

      @Override
      public int newToOld(int docID) {
        return maxDoc - 1 - docID;
      }

      @Override
      public int size() {
        return maxDoc;
      }
    };
  }

  /** Runs the case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 1) {
      throw new IllegalArgumentException("usage: ReorderMergeFixture <output-dir>");
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
      IndexWriterConfig initial = new IndexWriterConfig();
      initial.setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(dir, initial)) {
        int i = 0;
        for (int size : SIZES) {
          for (int k = 0; k < size; k++, i++) {
            writer.addDocument(doc(i));
          }
          writer.commit();
        }
      }
      System.out.println("segmentsBeforeMerge=" + SegmentInfos.readLatestCommit(dir).size());

      IndexWriterConfig config = new IndexWriterConfig();
      config.setMergePolicy(
          new OneMergeWrappingMergePolicy(
              new TieredMergePolicy(),
              merge ->
                  new MergePolicy.OneMerge(merge) {
                    @Override
                    public Sorter.DocMap reorder(
                        CodecReader reader, Directory directory, Executor executor) {
                      return reverse(reader.maxDoc());
                    }
                  }));
      try (IndexWriter writer = new IndexWriter(dir, config)) {
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
