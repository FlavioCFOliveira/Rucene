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
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.Bits;

/**
 * Reference tool for delete-by-query: it runs a scripted sequence of additions
 * and {@link IndexWriter#deleteDocuments(org.apache.lucene.search.Query...)}
 * calls with a real Apache Lucene Core 10.5.0 {@link IndexWriter} and reports
 * which documents survived, which files the directory holds, and what each
 * segment records about its deletions.
 *
 * <p>This is the measurement the Rust side is judged against for
 * {@code FrozenBufferedUpdates.applyQueryDeletes}. The Rucene test replays the
 * same sequence and asserts the two runs agree on the live documents, on the
 * per-segment {@code delCount} and {@code delGen}, and on the {@code .liv}
 * files produced.
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... DeleteByQueryFixture &lt;output-dir&gt; &lt;case&gt;
 * </pre>
 *
 * <p>Supported cases:
 *
 * <ul>
 *   <li>{@code byquery} — six documents committed, then one delete-by-query
 *       matching the three {@code color:red} documents, then a second commit,
 *       so the deletes resolve against an already-flushed segment;
 *   <li>{@code multi} — the same corpus with two queries passed in one
 *       {@code deleteDocuments} call, proving both are applied;
 *   <li>{@code mixed} — a delete-by-query and a delete-by-term issued against
 *       the same segment, proving the two paths compose;
 *   <li>{@code nomatch} — a query that matches nothing, so no document is
 *       deleted and no {@code .liv} is written;
 *   <li>{@code pending} — the delete-by-query is issued while documents are
 *       still buffered in RAM and more documents are added afterwards, so the
 *       packet becomes segment-private and its {@code docIDUpto} limit must
 *       spare the documents that arrived after the delete;
 *   <li>{@code repeat} — the same query is deleted twice and then a document it
 *       already removed is deleted again by term, proving that a document that
 *       is already deleted is not counted twice;
 *   <li>{@code read} — reads back an index written elsewhere (by Rucene) and
 *       prints what it sees.
 * </ul>
 */
public final class DeleteByQueryFixture {

  private DeleteByQueryFixture() {}

  /** Runs one case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 2) {
      throw new IllegalArgumentException("usage: DeleteByQueryFixture <output-dir> <case>");
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

    try (FSDirectory dir = FSDirectory.open(out);
        IndexWriter writer = new IndexWriter(dir, config)) {
      script(writer, shape);
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
                + " delGen="
                + sci.getDelGen()
                + " files="
                + String.join(",", files));
      }
    }
    read(out);
  }

  /** Replays one scripted sequence of additions and deletions. */
  static void script(IndexWriter writer, String shape) throws IOException {
    switch (shape) {
      case "byquery":
        for (int d = 0; d < 6; d++) {
          writer.addDocument(doc(d));
        }
        writer.commit();
        writer.deleteDocuments(IntPoint.newRangeQuery("rank", 0, 2));
        writer.commit();
        break;
      case "multi":
        for (int d = 0; d < 6; d++) {
          writer.addDocument(doc(d));
        }
        writer.commit();
        writer.deleteDocuments(
            IntPoint.newRangeQuery("rank", 1, 1), IntPoint.newRangeQuery("rank", 3, 3));
        writer.commit();
        break;
      case "nomatch":
        for (int d = 0; d < 5; d++) {
          writer.addDocument(doc(d));
        }
        writer.commit();
        writer.deleteDocuments(IntPoint.newRangeQuery("rank", 99, 99));
        writer.commit();
        break;
      case "pending":
        for (int d = 0; d < 4; d++) {
          writer.addDocument(doc(d));
        }
        writer.deleteDocuments(IntPoint.newRangeQuery("rank", 0, 9));
        for (int d = 4; d < 6; d++) {
          writer.addDocument(doc(d));
        }
        writer.commit();
        break;
      case "repeat":
        for (int d = 0; d < 6; d++) {
          writer.addDocument(doc(d));
        }
        writer.commit();
        writer.deleteDocuments(IntPoint.newRangeQuery("rank", 0, 2));
        writer.commit();
        writer.deleteDocuments(IntPoint.newRangeQuery("rank", 0, 2));
        writer.commit();
        break;
      case "all":
        for (int d = 0; d < 6; d++) {
          writer.addDocument(doc(d));
        }
        writer.commit();
        writer.deleteDocuments(new MatchAllDocsQuery());
        writer.commit();
        break;
      default:
        throw new IllegalArgumentException("unknown case: " + shape);
    }
  }

  /** Prints what a reader sees of the index at {@code out}. */
  private static void read(Path out) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out);
        DirectoryReader reader = DirectoryReader.open(dir)) {
      System.out.println("numDocs=" + reader.numDocs() + " maxDoc=" + reader.maxDoc());
      StoredFields stored = reader.storedFields();
      List<String> live = new ArrayList<>();
      List<String> deleted = new ArrayList<>();
      for (LeafReaderContext ctx : reader.leaves()) {
        Bits liveDocs = ctx.reader().getLiveDocs();
        for (int i = 0; i < ctx.reader().maxDoc(); i++) {
          String id = stored.document(ctx.docBase + i).get("id");
          if (liveDocs == null || liveDocs.get(i)) {
            live.add(id);
          } else {
            deleted.add(id);
          }
        }
      }
      System.out.println("live=" + String.join(",", live));
      System.out.println("deleted=" + String.join(",", deleted));
    }
  }

  /**
   * One document of the scripted corpus: a stored {@code id}, a stored
   * {@code body} already lower case and space separated, and an
   * {@link IntPoint} {@code rank} holding the document's ordinal, so a point
   * range query selects any contiguous run of the corpus.
   */
  static Document doc(int d) {
    Document doc = new Document();
    doc.add(new StringField("id", "doc-" + d, Field.Store.YES));
    doc.add(new TextField("body", "hello world number " + d, Field.Store.YES));
    doc.add(new IntPoint("rank", d));
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
