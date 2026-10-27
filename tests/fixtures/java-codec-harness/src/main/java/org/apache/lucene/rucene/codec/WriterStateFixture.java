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

import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.Term;
import org.apache.lucene.store.ByteBuffersDirectory;
import java.util.Map;

/**
 * Reference tool for the state queries of an Apache Lucene Core 10.5.0 {@link IndexWriter}. Prints
 * what {@code tests/index_writer_state.rs} compares Rucene against:
 *
 * <ul>
 *   <li>{@code create=}: {@code hasUncommittedChanges()} of a writer opened with {@code
 *       OpenMode.CREATE} over an existing index — true, because opening it called {@code changed()}
 *       ({@code IndexWriter.java:1022-1024}); the method is {@code changeCount !=
 *       lastCommitChangeCount || hasChangesInRam()} ({@code IndexWriter.java:4207-4214}).
 *   <li>{@code append}: {@code hasUncommittedChanges()} ({@code h}) and {@code hasDeletions()}
 *       ({@code d}) of a writer opened over a two-document index: before any change, after a
 *       buffered delete, after an NRT reader that does not apply deletes (the frozen packet waits in
 *       {@code BufferedUpdatesStream}), after flushing, after committing, and after a flushed delete
 *       that matches nothing; {@code hasDeletions()} reads the buffered packets, the documents
 *       writer, the reader pool and the segments ({@code IndexWriter.java:1457-1469}).
 *   <li>{@code pooled=}: {@code hasDeletions()} of a pooling writer once an NRT reader applied a
 *       delete: the deletion is pending in the reader pool only, not yet in any segment.
 *   <li>{@code closed=}: what {@code hasDeletions()} throws once the writer is closed ({@code
 *       ensureOpen()}, {@code IndexWriter.java:1458}).
 *   <li>{@code reopen=}: {@code hasUncommittedChanges()} of a writer opened after another one rolled
 *       back flushed segments.
 *   <li>{@code live}: {@code hasUncommittedChanges()} ({@code huc}) after {@code
 *       setLiveCommitData(data)} alone on a just-committed writer, then the user data ({@code data})
 *       and the version delta ({@code dv}) of the commit that follows; and the same for {@code
 *       setLiveCommitData(data, false)} ({@code IndexWriter.java:4138-4157}): both overloads
 *       advance {@code changeCount}, only the first calls {@code segmentInfos.changed()}.
 * </ul>
 */
public final class WriterStateFixture {

  private WriterStateFixture() {}

  static Document doc(int id) {
    Document d = new Document();
    d.add(new StringField("id", Integer.toString(id), Field.Store.YES));
    return d;
  }

  static IndexWriterConfig config() {
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergeScheduler(new SerialMergeScheduler());
    return config;
  }

  public static void main(String[] args) throws Exception {
    {
      ByteBuffersDirectory dir = new ByteBuffersDirectory();
      try (IndexWriter w = new IndexWriter(dir, config())) {
        w.addDocument(doc(0));
        w.commit();
      }
      IndexWriterConfig create = config();
      create.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      IndexWriter w = new IndexWriter(dir, create);
      System.out.println("create=" + w.hasUncommittedChanges());
      w.rollback();
    }
    {
      ByteBuffersDirectory dir = new ByteBuffersDirectory();
      try (IndexWriter w = new IndexWriter(dir, config())) {
        w.addDocument(doc(0));
        w.addDocument(doc(1));
        w.commit();
      }
      IndexWriter w = new IndexWriter(dir, config());
      StringBuilder line = new StringBuilder("append");
      line.append(" h0=").append(w.hasUncommittedChanges()).append(" d0=").append(w.hasDeletions());
      w.deleteDocuments(new Term("id", "0"));
      line.append(" h1=").append(w.hasUncommittedChanges()).append(" d1=").append(w.hasDeletions());
      DirectoryReader.open(w, false, false).close();
      line.append(" h2=").append(w.hasUncommittedChanges()).append(" d2=").append(w.hasDeletions());
      w.flush();
      line.append(" h3=").append(w.hasUncommittedChanges()).append(" d3=").append(w.hasDeletions());
      w.commit();
      line.append(" h4=").append(w.hasUncommittedChanges()).append(" d4=").append(w.hasDeletions());
      w.deleteDocuments(new Term("id", "nomatch"));
      w.flush();
      line.append(" h5=").append(w.hasUncommittedChanges()).append(" d5=").append(w.hasDeletions());
      System.out.println(line);
      w.close();
      String closed;
      try {
        w.hasDeletions();
        closed = "ok";
      } catch (Throwable t) {
        closed = t.getClass().getSimpleName();
      }
      System.out.println("closed=" + closed);
    }
    {
      ByteBuffersDirectory dir = new ByteBuffersDirectory();
      IndexWriterConfig pooling = config();
      pooling.setReaderPooling(true);
      // No merge may fold the deletion away before the question is asked.
      pooling.setMergePolicy(NoMergePolicy.INSTANCE);
      IndexWriter w = new IndexWriter(dir, pooling);
      w.addDocument(doc(0));
      w.addDocument(doc(1));
      w.commit();
      w.deleteDocuments(new Term("id", "0"));
      DirectoryReader reader = DirectoryReader.open(w);
      System.out.println("pooled=" + w.hasDeletions() + " numDocs=" + reader.numDocs());
      reader.close();
      w.rollback();
    }
    {
      ByteBuffersDirectory dir = new ByteBuffersDirectory();
      IndexWriter w = new IndexWriter(dir, config());
      w.addDocument(doc(0));
      w.addDocument(doc(1));
      w.commit();
      w.addDocument(doc(2));
      w.flush();
      w.deleteDocuments(new Term("id", "0"));
      w.flush();
      w.rollback();
      try (IndexWriter reopened = new IndexWriter(dir, config())) {
        System.out.println("reopen=" + reopened.hasUncommittedChanges());
      }
    }
    {
      ByteBuffersDirectory dir = new ByteBuffersDirectory();
      try (IndexWriter w = new IndexWriter(dir, config())) {
        w.addDocument(doc(0));
        w.commit();
        long v1 = SegmentInfos.readLatestCommit(dir).getVersion();
        StringBuilder line = new StringBuilder("live");
        w.setLiveCommitData(Map.of("k", "v").entrySet());
        line.append(" huc1=").append(w.hasUncommittedChanges());
        w.commit();
        SegmentInfos c2 = SegmentInfos.readLatestCommit(dir);
        line.append(" data1=").append(c2.getUserData()).append(" dv1=").append(c2.getVersion() - v1);
        line.append(" huc1c=").append(w.hasUncommittedChanges());
        w.setLiveCommitData(Map.of("k", "w").entrySet(), false);
        line.append(" huc2=").append(w.hasUncommittedChanges());
        w.commit();
        SegmentInfos c3 = SegmentInfos.readLatestCommit(dir);
        line.append(" data2=").append(c3.getUserData());
        line.append(" dv2=").append(c3.getVersion() - c2.getVersion());
        line.append(" gen=").append(c3.getGeneration());
        System.out.println(line);
      }
    }
  }
}
