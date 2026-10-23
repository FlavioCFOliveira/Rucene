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
import java.util.Arrays;
import java.util.List;

import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.StringHelper;

/**
 * Reference tool for the flush path: it runs a scripted indexing sequence with a
 * real Apache Lucene Core 10.5.0 {@link IndexWriter} and reports exactly which
 * files the directory holds after the commit.
 *
 * <p>This is the measurement the Rust side is judged against. The Rucene test
 * replays the same sequence and asserts the two directories hold the same set of
 * file extensions, so that a segment Rucene writes is a segment Lucene would
 * have written.
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... FlushFilesFixture &lt;output-dir&gt; &lt;case&gt;
 * </pre>
 *
 * <p>Supported cases:
 *
 * <ul>
 *   <li>{@code cfs} — five documents, one commit, compound file on (the Lucene
 *       default);
 *   <li>{@code nocfs} — the same, with {@code setUseCompoundFile(false)};
 *   <li>{@code deletes} — eight documents with two deleted before the flush, so
 *       the segment is sealed with a {@code .liv} file;
 *   <li>{@code twosegments} — two commits with {@link NoMergePolicy}, so the
 *       directory holds two independently sealed segments;
 *   <li>{@code read} — reads back an index written elsewhere (by Rucene) and
 *       prints its documents and fields.
 * </ul>
 *
 * <p>Except for {@code read}, the tool prints {@code files=} (the sorted
 * directory listing), one {@code segment=} line per segment naming its files,
 * and the documents it indexed, so the Rust side can compare both the file
 * layout and the content.
 */
public final class FlushFilesFixture {

  private FlushFilesFixture() {}

  /** Runs one case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 2) {
      throw new IllegalArgumentException("usage: FlushFilesFixture <output-dir> <case>");
    }
    Path out = Paths.get(args[0]);
    String shape = args[1];
    System.out.println("version=" + org.apache.lucene.util.Version.LATEST);

    if (shape.equals("read")) {
      read(out);
      return;
    }

    Files.createDirectories(out);
    for (String name : listing(out)) {
      Files.deleteIfExists(out.resolve(name));
    }

    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setCodec(new Lucene104Codec());
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    if (shape.equals("nocfs")) {
      config.setUseCompoundFile(false);
    }

    try (FSDirectory dir = FSDirectory.open(out);
        IndexWriter writer = new IndexWriter(dir, config)) {
      switch (shape) {
        case "cfs":
        case "nocfs":
          for (int d = 0; d < 5; d++) {
            writer.addDocument(doc(d));
          }
          writer.commit();
          break;
        case "deletes":
          for (int d = 0; d < 8; d++) {
            writer.addDocument(doc(d));
          }
          writer.deleteDocuments(new Term("id", "doc-2"));
          writer.deleteDocuments(new Term("id", "doc-5"));
          writer.commit();
          break;
        case "twosegments":
          for (int d = 0; d < 3; d++) {
            writer.addDocument(doc(d));
          }
          writer.commit();
          for (int d = 3; d < 6; d++) {
            writer.addDocument(doc(d));
          }
          writer.commit();
          break;
        default:
          throw new IllegalArgumentException("unknown case: " + shape);
      }
    }

    System.out.println("files=" + String.join(",", listing(out)));
    try (FSDirectory dir = FSDirectory.open(out)) {
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      System.out.println("generation=" + infos.getGeneration());
      for (SegmentCommitInfo sci : infos) {
        List<String> files = new ArrayList<>(sci.files());
        files.sort(null);
        System.out.println(
            "segment="
                + sci.info.name
                + " maxDoc="
                + sci.info.maxDoc()
                + " delCount="
                + sci.getDelCount()
                + " cfs="
                + sci.info.getUseCompoundFile()
                + " id="
                + StringHelper.idToString(sci.info.getId())
                + " files="
                + String.join(",", files));
      }
    }
    read(out);
  }

  /** Prints every live document and every field name of the index at {@code out}. */
  private static void read(Path out) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out);
        DirectoryReader reader = DirectoryReader.open(dir)) {
      System.out.println("numDocs=" + reader.numDocs() + " maxDoc=" + reader.maxDoc());
      List<String> fields = new ArrayList<>();
      for (LeafReaderContext ctx : reader.leaves()) {
        FieldInfos infos = ctx.reader().getFieldInfos();
        for (FieldInfo fi : infos) {
          if (!fields.contains(fi.name)) {
            fields.add(fi.name);
          }
        }
      }
      fields.sort(null);
      System.out.println("fields=" + String.join(",", fields));
      StoredFields stored = reader.storedFields();
      List<String> docs = new ArrayList<>();
      for (int i = 0; i < reader.maxDoc(); i++) {
        Document d = stored.document(i);
        docs.add(d.get("id") + "|" + d.get("body"));
      }
      System.out.println("docs=" + String.join(";", docs));
    }
  }

  private static Document doc(int d) {
    Document doc = new Document();
    doc.add(new StringField("id", "doc-" + d, Field.Store.YES));
    doc.add(new TextField("body", "hello world number " + d, Field.Store.YES));
    return doc;
  }

  private static List<String> listing(Path out) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out)) {
      String[] all = dir.listAll();
      List<String> files = new ArrayList<>(Arrays.asList(all));
      files.sort(null);
      return files;
    }
  }
}
