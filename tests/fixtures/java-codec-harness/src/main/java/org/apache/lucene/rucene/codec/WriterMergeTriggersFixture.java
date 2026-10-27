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
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.LogDocMergePolicy;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.SoftDeletesRetentionMergePolicy;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;

/**
 * Reference tool for what triggers, keeps and orders merges in a real Apache
 * Lucene Core 10.5.0 {@link IndexWriter}. Each case prints what {@code
 * tests/index_writer_merge_triggers.rs} compares Rucene against:
 *
 * <ul>
 *   <li>{@code retain}: a {@link SoftDeletesRetentionMergePolicy} whose
 *       retention query matches every document keeps a flushed segment whose
 *       two documents soft-deleted themselves.
 *   <li>{@code commit-merge}: with a {@link LogDocMergePolicy} of merge factor
 *       2 and a {@link SerialMergeScheduler}, eight single-document commits;
 *       the segments of every commit point. The default {@code
 *       maxFullFlushMergeWaitMillis} also merges during each commit.
 *   <li>{@code commit-merge-nowait}: the same with {@code
 *       maxFullFlushMergeWaitMillis} 0, so only the merges a commit asks for
 *       once it is done ({@code maybeMerge}) shape the commit points.
 *   <li>{@code commit-pit}: the default configuration but a {@link
 *       SerialMergeScheduler}, twelve single-document commits; the segments of
 *       every commit point, which the merges prepared during each commit
 *       ({@code maxFullFlushMergeWaitMillis}, 500 ms by default) shape.
 *   <li>{@code blocks &lt;dir&gt;}: an index sorted by {@code rank} with
 *       document blocks (parent field {@code _parent}) is written as six
 *       segments and left in {@code dir}; a copy is force-merged into one
 *       segment and the {@code blk} value of every merged document is printed
 *       in document order.
 *   <li>{@code force-merge}: four single-document segments, written without
 *       merging, are force-merged into one by a {@link LogDocMergePolicy} of
 *       merge factor 2, which takes a
 *       cascade of merges over the segments {@code segmentsToMerge} tracks.
 *   <li>{@code log-packing}: four segments of 400 documents, written without
 *       merging, then {@code maybeMerge()} with a {@link LogDocMergePolicy} of
 *       merge factor 2: a merge below the minimum merge size keeps packing
 *       segments only up to that size.
 *   <li>{@code log-levels}: segments of 24, 3, 3 and 3 documents, then {@code
 *       maybeMerge()} with merge factor 4: the 3-document segments sit exactly
 *       on the level boundary, which Lucene computes in {@code float}.
 *   <li>{@code counts}: the package-private {@code getFlushCount()} and {@code
 *       getFlushDeletesCount()}, read by reflection, after flushes with and
 *       without changes and after a delete.
 *   <li>{@code deletes-applied}: a 0.01 MB RAM buffer and no merging; one
 *       committed document, then deletes of distinct missing terms until
 *       {@code getFlushCount()} moves from its value after the commit,
 *       printing both counts before, and whether that happened and both
 *       counts after. How many deletes it takes depends on the RAM the
 *       buffered deletes account for, which this case does not compare. The delete RAM crossing the buffer makes {@code
 *       DocumentsWriter.applyAllDeletes} freeze the global deletes into a
 *       ticket and call {@code onDeletesApplied}, whose event publishes the
 *       ticket and increments {@code flushCount}
 *       ({@code IndexWriter.java:450-460}, {@code DocumentsWriter.java:168-185}).
 *   <li>{@code flush-publish}: ten documents with {@code maxBufferedDocs=2} and
 *       a {@link SerialMergeScheduler}, first under {@link NoMergePolicy} and
 *       then under a {@link LogDocMergePolicy} of merge factor 2 and minimum
 *       merge size 1; after each {@code addDocument}, the package-private
 *       {@code getSegmentCount()}, read by reflection: the segments a flush
 *       triggered by {@code addDocument} produces are published at once
 *       ({@code afterSegmentsFlushed}) and may trigger a merge.
 * </ul>
 */
public final class WriterMergeTriggersFixture {

  private WriterMergeTriggersFixture() {}

  static String segments(Directory dir) throws IOException {
    SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
    List<String> parts = new ArrayList<>();
    for (SegmentCommitInfo info : infos) {
      parts.add(info.info.name + ":" + info.info.maxDoc());
    }
    return String.join(",", parts);
  }

  static Document idDoc(int i) {
    Document d = new Document();
    d.add(new StringField("id", "id" + i, Field.Store.NO));
    return d;
  }

  static void retain() throws IOException {
    try (FSDirectory dir = FSDirectory.open(Files.createTempDirectory("retain"))) {
      IndexWriterConfig config = new IndexWriterConfig();
      config.setSoftDeletesField("soft");
      config.setMergePolicy(
          new SoftDeletesRetentionMergePolicy(
              "soft", MatchAllDocsQuery::new, NoMergePolicy.INSTANCE));
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        Document live = new Document();
        live.add(new StringField("id", "live", Field.Store.NO));
        writer.addDocument(live);
        writer.commit();
        for (String id : new String[] {"t1", "t2"}) {
          Document tombstone = new Document();
          tombstone.add(new StringField("id", id, Field.Store.NO));
          tombstone.add(new NumericDocValuesField("soft", 1));
          writer.addDocument(tombstone);
        }
        writer.commit();
      }
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        System.out.println(
            "retain segments="
                + segments(dir)
                + " maxDoc="
                + reader.maxDoc()
                + " numDocs="
                + reader.numDocs());
      }
    }
  }

  static void commits(String kase, IndexWriterConfig config, int count) throws IOException {
    try (FSDirectory dir = FSDirectory.open(Files.createTempDirectory(kase))) {
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        for (int i = 0; i < count; i++) {
          writer.addDocument(idDoc(i));
          long seqNo = writer.commit();
          System.out.println(kase + " commit " + i + " seqNo=" + seqNo + " segments=" + segments(dir));
        }
      }
    }
  }

  static IndexWriterConfig blocksConfig() {
    IndexWriterConfig config = new IndexWriterConfig();
    config.setParentField("_parent");
    config.setIndexSort(new Sort(new SortField("rank", SortField.Type.LONG)));
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    return config;
  }

  static void blocks(Path out) throws IOException {
    Files.createDirectories(out);
    try (var listing = Files.list(out)) {
      for (Path file : listing.toList()) {
        Files.delete(file);
      }
    }
    try (FSDirectory dir = FSDirectory.open(out)) {
      try (IndexWriter writer = new IndexWriter(dir, blocksConfig())) {
        for (int b = 0; b < 30; b++) {
          List<Document> block = new ArrayList<>();
          for (int c = 0; c < 2; c++) {
            Document child = new Document();
            child.add(new NumericDocValuesField("blk", b));
            child.add(new StoredField("role", "child"));
            block.add(child);
          }
          Document parent = new Document();
          parent.add(new NumericDocValuesField("blk", b));
          parent.add(new NumericDocValuesField("rank", (b * 7L) % 30));
          parent.add(new StoredField("role", "parent"));
          block.add(parent);
          writer.addDocuments(block);
          if (b % 5 == 4) {
            writer.flush();
          }
        }
        writer.commit();
      }
      System.out.println("blocks before segments=" + segments(dir));
    }
    Path merged = Files.createTempDirectory("blocks-merged");
    try (var listing = Files.list(out)) {
      for (Path file : listing.toList()) {
        Files.copy(file, merged.resolve(file.getFileName()));
      }
    }
    try (FSDirectory dir = FSDirectory.open(merged)) {
      IndexWriterConfig config = blocksConfig();
      config.setOpenMode(IndexWriterConfig.OpenMode.APPEND);
      config.setMergePolicy(new LogDocMergePolicy());
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        writer.forceMerge(1);
        writer.commit();
      }
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        StringBuilder order = new StringBuilder();
        for (LeafReaderContext leaf : reader.leaves()) {
          NumericDocValues blk = leaf.reader().getNumericDocValues("blk");
          int doc;
          while ((doc = blk.nextDoc()) != DocIdSetIterator.NO_MORE_DOCS) {
            if (order.length() > 0) {
              order.append(',');
            }
            order.append(blk.longValue());
          }
        }
        System.out.println("blocks after segments=" + segments(dir));
        System.out.println("blocks order=" + order);
      }
    }
  }

  static void forceMerge() throws IOException, ReflectiveOperationException {
    try (FSDirectory dir = FSDirectory.open(Files.createTempDirectory("force-merge"))) {
      IndexWriterConfig initial = new IndexWriterConfig();
      initial.setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(dir, initial)) {
        for (int i = 0; i < 4; i++) {
          writer.addDocument(idDoc(i));
          writer.flush();
        }
        writer.commit();
      }
      IndexWriterConfig config = new IndexWriterConfig();
      LogDocMergePolicy policy = new LogDocMergePolicy();
      policy.setMergeFactor(2);
      config.setMergePolicy(policy);
      config.setMergeScheduler(new SerialMergeScheduler());
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        writer.forceMerge(1);
        // Before the commit, whose own merges would hide a missing cascade.
        Method segmentCount = IndexWriter.class.getDeclaredMethod("getSegmentCount");
        segmentCount.setAccessible(true);
        System.out.println("force-merge segmentCount=" + segmentCount.invoke(writer));
        writer.commit();
      }
      System.out.println("force-merge segments=" + segments(dir));
    }
  }

  static void logMerge(String kase, int mergeFactor, int[] sizes)
      throws IOException {
    try (FSDirectory dir = FSDirectory.open(Files.createTempDirectory(kase))) {
      IndexWriterConfig initial = new IndexWriterConfig();
      initial.setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(dir, initial)) {
        int id = 0;
        for (int size : sizes) {
          for (int i = 0; i < size; i++) {
            writer.addDocument(idDoc(id++));
          }
          writer.flush();
        }
        writer.commit();
      }
      IndexWriterConfig config = new IndexWriterConfig();
      LogDocMergePolicy policy = new LogDocMergePolicy();
      policy.setMergeFactor(mergeFactor);
      config.setMergePolicy(policy);
      config.setMergeScheduler(new SerialMergeScheduler());
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        writer.maybeMerge();
        writer.commit();
      }
      System.out.println(kase + " segments=" + segments(dir));
    }
  }

  static String counts(IndexWriter writer) throws ReflectiveOperationException {
    Method flushCount = IndexWriter.class.getDeclaredMethod("getFlushCount");
    Method flushDeletesCount = IndexWriter.class.getDeclaredMethod("getFlushDeletesCount");
    flushCount.setAccessible(true);
    flushDeletesCount.setAccessible(true);
    return "flushCount=" + flushCount.invoke(writer) + " flushDeletesCount=" + flushDeletesCount.invoke(writer);
  }

  static void counts() throws IOException, ReflectiveOperationException {
    try (FSDirectory dir = FSDirectory.open(Files.createTempDirectory("counts"))) {
      IndexWriterConfig config = new IndexWriterConfig();
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        writer.addDocument(idDoc(0));
        writer.addDocument(idDoc(1));
        writer.flush();
        System.out.println("counts after flush " + counts(writer));
        writer.flush();
        System.out.println("counts after empty flush " + counts(writer));
        writer.deleteDocuments(new Term("id", "id0"));
        writer.flush();
        System.out.println("counts after delete flush " + counts(writer));
        writer.addDocument(idDoc(2));
        writer.commit();
        System.out.println("counts after commit " + counts(writer));
      }
    }
  }

  static void deletesApplied() throws IOException, ReflectiveOperationException {
    Method flushCount = IndexWriter.class.getDeclaredMethod("getFlushCount");
    flushCount.setAccessible(true);
    try (Directory dir = new ByteBuffersDirectory()) {
      IndexWriterConfig config = new IndexWriterConfig();
      config.setRAMBufferSizeMB(0.01);
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      config.setMergeScheduler(new SerialMergeScheduler());
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        writer.addDocument(idDoc(0));
        writer.commit();
        int base = (int) flushCount.invoke(writer);
        System.out.println("before " + counts(writer));
        int deletes = 0;
        while ((int) flushCount.invoke(writer) == base && deletes < 100_000) {
          writer.deleteDocuments(new Term("id", "missing" + deletes));
          deletes++;
        }
        System.out.println("applied=" + (deletes < 100_000) + " " + counts(writer));
      }
    }
  }

  static void flushPublish() throws IOException, ReflectiveOperationException {
    Method segmentCount = IndexWriter.class.getDeclaredMethod("getSegmentCount");
    segmentCount.setAccessible(true);
    for (String kase : new String[] {"none", "logdoc2"}) {
      try (Directory dir = new ByteBuffersDirectory()) {
        IndexWriterConfig config = new IndexWriterConfig();
        config.setMaxBufferedDocs(2);
        config.setMergeScheduler(new SerialMergeScheduler());
        if (kase.equals("none")) {
          config.setMergePolicy(NoMergePolicy.INSTANCE);
        } else {
          LogDocMergePolicy policy = new LogDocMergePolicy();
          policy.setMergeFactor(2);
          policy.setMinMergeDocs(1);
          config.setMergePolicy(policy);
        }
        try (IndexWriter writer = new IndexWriter(dir, config)) {
          StringBuilder counts = new StringBuilder();
          for (int i = 0; i < 10; i++) {
            writer.addDocument(idDoc(i));
            counts.append(i == 0 ? "" : " ").append(segmentCount.invoke(writer));
          }
          writer.commit();
          System.out.println(
              "flush-publish " + kase + " counts=" + counts + " segments=" + segments(dir));
        }
      }
    }
  }

  /** Runs one case. */
  public static void main(String[] args) throws IOException, ReflectiveOperationException {
    switch (args[0]) {
      case "retain" -> retain();
      case "commit-merge", "commit-merge-nowait" -> {
        IndexWriterConfig config = new IndexWriterConfig();
        LogDocMergePolicy policy = new LogDocMergePolicy();
        policy.setMergeFactor(2);
        config.setMergePolicy(policy);
        config.setMergeScheduler(new SerialMergeScheduler());
        if (args[0].equals("commit-merge-nowait")) {
          config.setMaxFullFlushMergeWaitMillis(0);
        }
        commits(args[0], config, 8);
      }
      case "commit-pit" -> {
        IndexWriterConfig config = new IndexWriterConfig();
        config.setMergeScheduler(new SerialMergeScheduler());
        commits("commit-pit", config, 12);
      }
      case "blocks" -> blocks(Paths.get(args[1]));
      case "force-merge" -> forceMerge();
      case "counts" -> counts();
      case "flush-publish" -> flushPublish();
      case "deletes-applied" -> deletesApplied();
      case "log-packing" -> logMerge("log-packing", 2, new int[] {400, 400, 400, 400});
      case "log-levels" -> logMerge("log-levels", 4, new int[] {24, 3, 3, 3});
      default -> throw new IllegalArgumentException("unknown case: " + args[0]);
    }
  }
}
