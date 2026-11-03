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
import java.util.List;
import java.util.TreeSet;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.store.FSDirectory;

/**
 * Reference tool for {@code IndexWriter} opened with {@code OpenMode.CREATE} over an existing
 * index. Two commits of one document each leave the index at generation 2 with two segments; a
 * second writer then opens it with {@code OpenMode.CREATE}, adds one document and commits. Lucene
 * continues the previous commit's generation, version and segment counter
 * ({@code sis.updateGenerationVersionAndCounter(previous)}, {@code IndexWriter.java:1013-1016}),
 * so the new segment is not named {@code _0} again and {@code segments_N} is not overwritten.
 *
 * <p>The tool prints the generation, version and counter of the commit before and after, the
 * names of the committed segments, and every file in the directory. {@code
 * tests/index_writer_create_over_existing.rs} replays the same operations in Rucene.
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... CreateOverFixture &lt;output-dir&gt;
 * </pre>
 */
public final class CreateOverFixture {

  private CreateOverFixture() {}

  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", "doc" + i, Field.Store.YES));
    return d;
  }

  private static void report(String prefix, FSDirectory dir) throws IOException {
    SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
    System.out.println(prefix + "_gen=" + infos.getGeneration());
    System.out.println(prefix + "_version=" + infos.getVersion());
    System.out.println(prefix + "_counter=" + infos.counter);
    List<String> names = new ArrayList<>();
    for (SegmentCommitInfo info : infos) {
      names.add(info.info.name);
    }
    System.out.println(prefix + "_segments=" + String.join(",", names));
  }

  /** Runs the case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 1) {
      throw new IllegalArgumentException("usage: CreateOverFixture <output-dir>");
    }
    Path out = Paths.get(args[0]);
    Files.createDirectories(out);
    try (var listing = Files.list(out)) {
      for (Path file : listing.toList()) {
        Files.delete(file);
      }
    }
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig first = new IndexWriterConfig();
      first.setUseCompoundFile(false);
      try (IndexWriter writer = new IndexWriter(dir, first)) {
        writer.addDocument(doc(0));
        writer.commit();
        writer.addDocument(doc(1));
        writer.commit();
      }
      report("before", dir);

      IndexWriterConfig create = new IndexWriterConfig();
      create.setUseCompoundFile(false);
      create.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      try (IndexWriter writer = new IndexWriter(dir, create)) {
        writer.addDocument(doc(2));
        writer.commit();
      }
      report("after", dir);
      System.out.println("files=" + String.join(",", new TreeSet<>(List.of(dir.listAll()))));
    }
  }
}
