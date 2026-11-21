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

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.util.BytesRef;

/**
 * Reference tool for the RAM Apache Lucene Core 10.5.0 charges for buffered deletes and doc-values
 * updates, which {@code FlushByRamOrCountsPolicy.onChange} reads to apply the deletes and to pick a
 * per-thread writer to flush ({@code FlushByRamOrCountsPolicy.java:43-55}). Each case prints what
 * {@code tests/buffered_updates_ram.rs} compares Rucene against:
 *
 * <ul>
 *   <li>{@code trace}: term deletes, query deletes, numeric and binary doc-values updates on a
 *       writer with no buffered document; after each one, {@code IndexWriter.ramBytesUsed()} and,
 *       read by reflection, {@code DocumentsWriterFlushControl.getDeleteBytesUsed()}.
 *   <li>{@code files}: adds, document updates, term deletes and query deletes of numeric-only
 *       documents under a 0.0665 MB RAM buffer; the committed files and, per segment, its size,
 *       deletions and deletion generation. With the environment variable {@code TRACE} set, it also
 *       prints the two figures above before every operation.
 * </ul>
 */
public final class DeleteRamFixture {
  static final boolean TRACE = System.getenv("TRACE") != null;

  interface Op {
    void run(IndexWriter w) throws Exception;
  }

  static Document doc(int id) {
    Document d = new Document();
    d.add(new StringField("id", "d" + id, org.apache.lucene.document.Field.Store.NO));
    d.add(new StringField("grp", "g" + (id % 3), org.apache.lucene.document.Field.Store.NO));
    d.add(new NumericDocValuesField("num", id));
    d.add(new BinaryDocValuesField("bin", new BytesRef("b" + id)));
    return d;
  }

  /** A document whose only field is a numeric doc value. */
  static Document valueDoc(int id) {
    Document d = new Document();
    d.add(new NumericDocValuesField("num", id));
    return d;
  }

  static long deleteBytes(IndexWriter w) throws Exception {
    Field dw = IndexWriter.class.getDeclaredField("docWriter");
    dw.setAccessible(true);
    Object docWriter = dw.get(w);
    Field fc = docWriter.getClass().getDeclaredField("flushControl");
    fc.setAccessible(true);
    Object control = fc.get(docWriter);
    Method m = control.getClass().getDeclaredMethod("getDeleteBytesUsed");
    m.setAccessible(true);
    return (long) m.invoke(control);
  }

  static void trace() throws Exception {
    Directory dir = new ByteBuffersDirectory();
    IndexWriterConfig c = new IndexWriterConfig();
    c.setMergeScheduler(new SerialMergeScheduler());
    IndexWriter w = new IndexWriter(dir, c);
    for (int i = 0; i < 4; i++) w.addDocument(doc(i));
    w.commit();
    List<String> names = new ArrayList<>();
    List<Op> ops = new ArrayList<>();
    for (int i = 0; i < 20; i++) {
      final int k = i;
      names.add("term id x" + k);
      ops.add(x -> x.deleteDocuments(new Term("id", "x" + k)));
    }
    names.add("term id x3 again");
    ops.add(x -> x.deleteDocuments(new Term("id", "x3")));
    names.add("term grp new field");
    ops.add(x -> x.deleteDocuments(new Term("grp", "zz")));
    for (int i = 0; i < 3; i++) {
      final int k = i;
      names.add("query id q" + k);
      ops.add(x -> x.deleteDocuments(new TermQuery(new Term("id", "q" + k))));
    }
    names.add("query id q0 again");
    ops.add(x -> x.deleteDocuments(new TermQuery(new Term("id", "q0"))));
    for (int i = 0; i < 3; i++) {
      final int k = i;
      names.add("numeric d" + k);
      ops.add(x -> x.updateNumericDocValue(new Term("id", "d" + k), "num", 100 + k));
    }
    names.add("numeric grp g1");
    ops.add(x -> x.updateNumericDocValue(new Term("grp", "g1"), "num", 7));
    for (int i = 0; i < 3; i++) {
      final int k = i;
      names.add("binary d" + k);
      ops.add(x -> x.updateBinaryDocValue(new Term("id", "d" + k), "bin", new BytesRef("v" + k)));
    }
    names.add("binary grp g2 long");
    ops.add(x -> x.updateBinaryDocValue(new Term("grp", "g2"), "bin", new BytesRef("a much longer binary value")));
    System.out.println("start ram=" + w.ramBytesUsed() + " del=" + deleteBytes(w));
    for (int i = 0; i < ops.size(); i++) {
      ops.get(i).run(w);
      System.out.println(names.get(i) + " ram=" + w.ramBytesUsed() + " del=" + deleteBytes(w));
    }
    w.rollback();
  }

  static void files() throws Exception {
    Directory dir = new ByteBuffersDirectory();
    IndexWriterConfig c = new IndexWriterConfig();
    c.setMergeScheduler(new SerialMergeScheduler());
    c.setMergePolicy(NoMergePolicy.INSTANCE);
    c.setRAMBufferSizeMB(0.0665);
    c.setMaxBufferedDocs(16);
    IndexWriter w = new IndexWriter(dir, c);
    for (int i = 0; i < 400; i++) {
      if (TRACE) {
        System.out.println("op " + i + " ram=" + w.ramBytesUsed() + " del=" + deleteBytes(w));
      }
      switch (i % 5) {
        case 0, 1 -> w.addDocument(valueDoc(i));
        case 2 -> w.updateDocument(new Term("id", "d" + (i - 2)), valueDoc(i));
        case 3 -> w.deleteDocuments(new Term("id", "d" + (i - 2)));
        default -> {
          w.deleteDocuments(new TermQuery(new Term("id", "nomatch" + i)));
          w.deleteDocuments(new Term("grp", "zz" + i));
        }
      }
    }
    w.commit();
    List<String> l = new ArrayList<>();
    for (String f : dir.listAll()) if (!f.equals("write.lock")) l.add(f);
    Collections.sort(l);
    System.out.println("files=" + String.join(",", l));
    SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
    for (SegmentCommitInfo info : infos) {
      System.out.println(
          info.info.name + " maxDoc=" + info.info.maxDoc() + " del=" + info.getDelCount()
              + " delGen=" + info.getDelGen());
    }
    w.close();
  }

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "trace" -> trace();
      case "files" -> files();
      default -> throw new IllegalArgumentException(args[0]);
    }
  }
}
