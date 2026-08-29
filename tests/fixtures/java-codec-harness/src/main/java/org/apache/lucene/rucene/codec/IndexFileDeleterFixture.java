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
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.TreeSet;

import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexCommit;
import org.apache.lucene.index.IndexDeletionPolicy;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.IndexWriterConfig.OpenMode;
import org.apache.lucene.index.KeepOnlyLastCommitDeletionPolicy;
import org.apache.lucene.index.NoDeletionPolicy;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.PersistentSnapshotDeletionPolicy;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;

/**
 * Reference fixture for Rucene's {@code IndexFileDeleter} port.
 *
 * <p>Every shape leaves a real Lucene-written index on disk and prints {@code key=value} lines
 * describing the file-lifecycle decisions Lucene made. The Rust side opens the very same
 * directory with its own {@code IndexFileDeleter} and must reach an identical outcome, which
 * proves both that Rucene reads a Java-written index correctly and that it reproduces Lucene's
 * deletion behaviour on it.
 *
 * <p>Shapes:
 *
 * <ul>
 *   <li>{@code retained} — writes five commits under {@link NoDeletionPolicy} into
 *       {@code <outDir>/index}, reporting the full listing and, for every surviving commit, its
 *       generation and file set. This is the multi-generation starting state the other shapes
 *       reuse.
 *   <li>{@code reopen-keep-only-last} — builds that same five-generation index, copies it to
 *       {@code <outDir>/reopen}, and reopens the copy with {@link
 *       KeepOnlyLastCommitDeletionPolicy}, so that Lucene's {@code IndexFileDeleter.onInit}
 *       collects the superseded commits. Reports the listing before and after.
 *   <li>{@code orphan-cleanup} — builds a single-commit index, drops files that no commit
 *       references into it, reopens it, and reports which survive. Proves which unreferenced
 *       files {@code IndexFileDeleter} removes on init and which it leaves alone.
 *   <li>{@code snapshot-pin} — writes commits under {@link PersistentSnapshotDeletionPolicy},
 *       persisting a snapshot of an early generation, then writes further commits. Reports which
 *       generations and files survive; the pinned generation must be among them.
 * </ul>
 */
public final class IndexFileDeleterFixture {

  private static final int COMMITS = 5;

  private IndexFileDeleterFixture() {}

  public static void main(String[] args) throws IOException {
    if (args.length < 2) {
      throw new IllegalArgumentException("usage: IndexFileDeleterFixture <outDir> <shape>");
    }
    Path outDir = Paths.get(args[0]);
    Files.createDirectories(outDir);
    String shape = args[1];

    System.out.println("version=" + org.apache.lucene.util.Version.LATEST);
    System.out.println("shape=" + shape);

    switch (shape) {
      case "retained" -> retained(outDir);
      case "reopen-keep-only-last" -> reopenKeepOnlyLast(outDir);
      case "orphan-cleanup" -> orphanCleanup(outDir);
      case "snapshot-pin" -> snapshotPin(outDir);
      default -> throw new IllegalArgumentException("unknown shape: " + shape);
    }
  }

  // ---------------------------------------------------------------------------
  // Shapes
  // ---------------------------------------------------------------------------

  /** Five commits, every generation retained. */
  private static void retained(Path outDir) throws IOException {
    Path indexDir = outDir.resolve("index");
    wipe(indexDir);
    try (Directory dir = FSDirectory.open(indexDir)) {
      writeCommits(dir, NoDeletionPolicy.INSTANCE, COMMITS);
      report("listing", dir);
      reportCommits("commit", dir);
    }
  }

  /** The same index, reopened under a policy that keeps only the newest commit. */
  private static void reopenKeepOnlyLast(Path outDir) throws IOException {
    Path indexDir = outDir.resolve("index");
    Path reopenDir = outDir.resolve("reopen");
    wipe(indexDir);
    wipe(reopenDir);

    try (Directory dir = FSDirectory.open(indexDir)) {
      writeCommits(dir, NoDeletionPolicy.INSTANCE, COMMITS);
      report("before", dir);
      reportCommits("before_commit", dir);
    }

    copyDirectory(indexDir, reopenDir);

    // Opening a writer runs IndexFileDeleter's constructor, which calls
    // policy.onInit(commits); KeepOnlyLastCommitDeletionPolicy deletes every
    // commit but the newest, and the deleter then removes their files.
    try (Directory dir = FSDirectory.open(reopenDir)) {
      IndexWriterConfig config = config(new KeepOnlyLastCommitDeletionPolicy());
      config.setOpenMode(OpenMode.APPEND);
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        // No changes: we are exercising the deleter's init path only.
        System.out.println("reopen_num_docs=" + writer.getDocStats().numDocs);
      }
      report("after", dir);
      reportCommits("after_commit", dir);
    }
  }

  /** Unreferenced files dropped into a live index directory. */
  private static void orphanCleanup(Path outDir) throws IOException {
    Path indexDir = outDir.resolve("orphans");
    wipe(indexDir);

    try (Directory dir = FSDirectory.open(indexDir)) {
      writeCommits(dir, new KeepOnlyLastCommitDeletionPolicy(), 1);

      // Three kinds of debris:
      //  - a codec-pattern file no commit references (deleted);
      //  - a segment-info file for a segment that does not exist (deleted);
      //  - a file that is neither (left alone).
      touch(dir, "_9.fdt");
      touch(dir, "_9.si");
      touch(dir, "unrelated.txt");
      report("before", dir);

      IndexWriterConfig config = config(new KeepOnlyLastCommitDeletionPolicy());
      config.setOpenMode(OpenMode.APPEND);
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        System.out.println("reopen_num_docs=" + writer.getDocStats().numDocs);
      }
      report("after", dir);
    }
  }

  /** A persisted snapshot pins its generation against later commits. */
  private static void snapshotPin(Path outDir) throws IOException {
    Path indexDir = outDir.resolve("snapshot");
    Path snapshotDir = outDir.resolve("snapshot-state");
    wipe(indexDir);
    wipe(snapshotDir);

    try (Directory dir = FSDirectory.open(indexDir);
        Directory snapDir = FSDirectory.open(snapshotDir)) {
      PersistentSnapshotDeletionPolicy policy =
          new PersistentSnapshotDeletionPolicy(
              new KeepOnlyLastCommitDeletionPolicy(), snapDir, OpenMode.CREATE);

      IndexWriterConfig config = config(policy);
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        // Two commits, then pin the second.
        addDoc(writer, 0);
        writer.commit();
        addDoc(writer, 1);
        writer.commit();

        IndexCommit pinned = policy.snapshot();
        System.out.println("pinned_gen=" + pinned.getGeneration());
        System.out.println(
            "pinned_files=" + String.join(",", new TreeSet<>(pinned.getFileNames())));

        // Three further commits, which would normally retire the pinned one.
        for (int i = 2; i < COMMITS; i++) {
          addDoc(writer, i);
          writer.commit();
        }
      }
      report("listing", dir);
      reportCommits("commit", dir);
      report("snapshot_state", snapDir);
    }
  }

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  private static IndexWriterConfig config(IndexDeletionPolicy policy) {
    IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
    config.setIndexDeletionPolicy(policy);
    // Merging would rewrite segments and make the file sets non-deterministic;
    // this fixture is about file lifecycle, not merge policy.
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    config.setUseCompoundFile(false);
    config.setCommitOnClose(false);
    return config;
  }

  private static void writeCommits(Directory dir, IndexDeletionPolicy policy, int commits)
      throws IOException {
    IndexWriterConfig config = config(policy);
    config.setOpenMode(OpenMode.CREATE);
    try (IndexWriter writer = new IndexWriter(dir, config)) {
      for (int i = 0; i < commits; i++) {
        addDoc(writer, i);
        writer.commit();
      }
    }
  }

  private static void addDoc(IndexWriter writer, int i) throws IOException {
    Document doc = new Document();
    doc.add(new StringField("id", "doc" + i, Field.Store.YES));
    doc.add(new StringField("body", "term" + i, Field.Store.NO));
    writer.addDocument(doc);
  }

  private static void touch(Directory dir, String name) throws IOException {
    try (IndexOutput out = dir.createOutput(name, IOContext.DEFAULT)) {
      out.writeInt(0);
    }
  }

  /** Prints the sorted directory listing under {@code key}. */
  private static void report(String key, Directory dir) throws IOException {
    Collection<String> files = new TreeSet<>(List.of(dir.listAll()));
    System.out.println(key + "=" + String.join(",", files));
  }

  /** Prints every commit's generation and file set, oldest first. */
  private static void reportCommits(String key, Directory dir) throws IOException {
    List<IndexCommit> commits = DirectoryReader.listCommits(dir);
    List<String> gens = new ArrayList<>();
    for (IndexCommit commit : commits) {
      gens.add(Long.toString(commit.getGeneration()));
      System.out.println(
          key
              + "_files_"
              + commit.getGeneration()
              + "="
              + String.join(",", new TreeSet<>(commit.getFileNames())));
    }
    System.out.println(key + "_gens=" + String.join(",", gens));
  }

  private static void wipe(Path dir) throws IOException {
    if (Files.exists(dir)) {
      try (var stream = Files.walk(dir)) {
        stream
            .sorted((a, b) -> b.getNameCount() - a.getNameCount())
            .forEach(
                p -> {
                  try {
                    Files.deleteIfExists(p);
                  } catch (IOException e) {
                    throw new RuntimeException(e);
                  }
                });
      }
    }
    Files.createDirectories(dir);
  }

  private static void copyDirectory(Path from, Path to) throws IOException {
    Files.createDirectories(to);
    try (var stream = Files.list(from)) {
      for (Path p : stream.toList()) {
        Files.copy(p, to.resolve(p.getFileName()), StandardCopyOption.REPLACE_EXISTING);
      }
    }
  }
}
