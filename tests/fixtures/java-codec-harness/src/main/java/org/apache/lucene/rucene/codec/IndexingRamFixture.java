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

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.Term;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.InfoStream;

/**
 * Reference tool for the RAM Apache Lucene Core 10.5.0 charges while indexing, which {@code
 * FlushByRamOrCountsPolicy} compares against the RAM buffer to decide where segments end, and for
 * where those decisions cut segments. Each case prints what {@code tests/indexing_ram.rs} compares
 * Rucene against:
 *
 * <ul>
 *   <li>{@code trace <kind> <docs>}: {@code <docs>} documents of one kind (see {@link #doc}) added
 *       to a writer whose buffer never flushes; {@code IndexWriter.ramBytesUsed()} after each,
 *       which with a document limit set is the per-thread writer's exact {@code ramBytesUsed()}.
 *   <li>{@code dv-updates <type> <maxDoc> <count> <stride>}: a {@code NumericDocValuesFieldUpdates}
 *       ({@code numeric}), its min/max form ({@code numeric-range}) or a {@code
 *       BinaryDocValuesFieldUpdates} ({@code binary}) of a segment of {@code <maxDoc>} documents,
 *       built by reflection as {@code FrozenBufferedUpdates} and {@code IndexWriter} build them;
 *       {@code ramBytesUsed()} after each of {@code <count>} updates of documents {@code i *
 *       <stride> % maxDoc}, then after {@code finish()}.
 *   <li>{@code flushing <docs> <maxBufferedDocs>}: mixed documents flushed every {@code
 *       <maxBufferedDocs>}; while each per-thread writer flushes, {@code ramBytesUsed()} and {@code
 *       getFlushingBytes()}, read from the {@code InfoStream} message that {@code
 *       DocumentsWriterPerThread.flush} logs first ({@code DocumentsWriterPerThread.java:479-483}).
 *       The flushing writer's bytes are part of {@code ramBytesUsed()}, which is {@code
 *       getDeleteBytesUsed() + netBytes()} ({@code DocumentsWriterFlushControl.java:100-102,
 *       499-502}).
 *   <li>{@code files <ramMB> <ops> <seed>}: {@code <ops>} operations drawn from a 64-bit LCG seeded
 *       with {@code <seed>} (adds and updates of mixed documents, term deletes, numeric and binary
 *       doc-values updates by id and by group) under a {@code <ramMB>} RAM buffer, then a commit;
 *       the committed files and, per segment, its size, deletions and deletion, field-infos and
 *       doc-values generations. With {@code TRACE} set, {@code ramBytesUsed()} after every
 *       operation too; with {@code PERTURB=<n>} set, {@code n} identity hash codes are drawn
 *       first (see {@link #files}).
 * </ul>
 */
public final class IndexingRamFixture {

  static final FieldType TV_TYPE = new FieldType(TextField.TYPE_NOT_STORED);

  static {
    TV_TYPE.setStoreTermVectors(true);
    TV_TYPE.setStoreTermVectorPositions(true);
    TV_TYPE.setStoreTermVectorOffsets(true);
    TV_TYPE.freeze();
  }

  static final FieldType DOCS_FREQS_TYPE = new FieldType(TextField.TYPE_NOT_STORED);

  static {
    DOCS_FREQS_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS);
    DOCS_FREQS_TYPE.freeze();
  }

  /** Space-separated words {@code w<k>} drawn deterministically for document {@code id}. */
  static String text(int id) {
    StringBuilder b = new StringBuilder();
    int words = id % 7 + 1;
    for (int j = 0; j < words; j++) {
      if (j > 0) {
        b.append(' ');
      }
      b.append('w').append((id * 31 + j * 17) % 50);
    }
    return b.toString();
  }

  /** One document of {@code kind} with id {@code id}. */
  static Document doc(String kind, int id) {
    Document d = new Document();
    switch (kind) {
      case "string" -> d.add(new StringField("id", "d" + id, Field.Store.NO));
      case "text" -> d.add(new TextField("body", text(id), Field.Store.NO));
      case "freqs" -> d.add(new Field("freqs", text(id), DOCS_FREQS_TYPE));
      case "tv" -> d.add(new Field("tv", text(id), TV_TYPE));
      case "stored" -> {
        d.add(new StoredField("s", "v" + id));
        d.add(new StoredField("n", id));
      }
      case "numeric" -> d.add(new NumericDocValuesField("num", id * 1000L));
      case "binary" -> d.add(new BinaryDocValuesField("bin", new BytesRef("b" + id)));
      case "sorted" -> d.add(new SortedDocValuesField("sorted", new BytesRef("s" + (id % 13))));
      case "sortedset" -> {
        for (int j = 0; j <= id % 3; j++) {
          d.add(new SortedSetDocValuesField("sset", new BytesRef("t" + ((id + j) % 11))));
        }
      }
      case "sortednumeric" -> {
        for (int j = 0; j <= id % 3; j++) {
          d.add(new SortedNumericDocValuesField("snum", (long) id * j));
        }
      }
      case "mixed" -> {
        d.add(new StringField("id", "d" + id, Field.Store.YES));
        d.add(new StringField("grp", "g" + (id % 5), Field.Store.NO));
        d.add(new TextField("body", text(id), Field.Store.NO));
        d.add(new NumericDocValuesField("num", id));
        d.add(new BinaryDocValuesField("bin", new BytesRef("b" + id)));
        d.add(new SortedDocValuesField("sorted", new BytesRef("s" + (id % 13))));
      }
      default -> throw new IllegalArgumentException(kind);
    }
    return d;
  }

  static void trace(String kind, int docs) throws Exception {
    Directory dir = new ByteBuffersDirectory();
    IndexWriterConfig c = new IndexWriterConfig();
    c.setMergeScheduler(new SerialMergeScheduler());
    c.setRAMBufferSizeMB(256);
    // A document limit that is never reached: with one set, {@code
    // DocumentsWriterFlushControl.doAfterDocument} commits the per-thread RAM after every
    // document instead of only once it grew by {@code ramBufferGranularity()}
    // ({@code DocumentsWriterFlushControl.java:205-213}).
    c.setMaxBufferedDocs(1 << 30);
    IndexWriter w = new IndexWriter(dir, c);
    System.out.println("start ram=" + w.ramBytesUsed());
    for (int i = 0; i < docs; i++) {
      w.addDocument(doc(kind, i));
      System.out.println(i + " ram=" + w.ramBytesUsed());
    }
    w.rollback();
  }

  static void dvUpdates(String type, int maxDoc, int count, int stride) throws Exception {
    String pkg = "org.apache.lucene.index.";
    Object updates;
    switch (type) {
      case "numeric" -> {
        Constructor<?> k =
            Class.forName(pkg + "NumericDocValuesFieldUpdates")
                .getDeclaredConstructor(long.class, String.class, int.class);
        k.setAccessible(true);
        updates = k.newInstance(7L, "f", maxDoc);
      }
      case "numeric-range" -> {
        Constructor<?> k =
            Class.forName(pkg + "NumericDocValuesFieldUpdates")
                .getDeclaredConstructor(
                    long.class, String.class, long.class, long.class, int.class);
        k.setAccessible(true);
        updates = k.newInstance(7L, "f", 5L, 5L + (long) count * 977, maxDoc);
      }
      case "binary" -> {
        Constructor<?> k =
            Class.forName(pkg + "BinaryDocValuesFieldUpdates")
                .getDeclaredConstructor(long.class, String.class, int.class);
        k.setAccessible(true);
        updates = k.newInstance(7L, "f", maxDoc);
      }
      default -> throw new IllegalArgumentException(type);
    }
    Class<?> base = Class.forName(pkg + "DocValuesFieldUpdates");
    Method ram = updates.getClass().getMethod("ramBytesUsed");
    ram.setAccessible(true);
    Method addLong = updates.getClass().getDeclaredMethod("add", int.class, long.class);
    addLong.setAccessible(true);
    Method addBytes = updates.getClass().getDeclaredMethod("add", int.class, BytesRef.class);
    addBytes.setAccessible(true);
    Method finish = base.getDeclaredMethod("finish");
    finish.setAccessible(true);
    System.out.println("start ram=" + ram.invoke(updates));
    for (int i = 0; i < count; i++) {
      int doc = (int) ((long) i * stride % maxDoc);
      if (type.equals("binary")) {
        addBytes.invoke(updates, doc, new BytesRef("value" + i));
      } else {
        addLong.invoke(updates, doc, 5L + (long) i * 977);
      }
      System.out.println(i + " ram=" + ram.invoke(updates));
    }
    finish.invoke(updates);
    System.out.println("finish ram=" + ram.invoke(updates));
  }

  static void flushing(int docs, int maxBufferedDocs) throws Exception {
    Directory dir = new ByteBuffersDirectory();
    IndexWriterConfig c = new IndexWriterConfig();
    c.setMergeScheduler(new SerialMergeScheduler());
    c.setMergePolicy(NoMergePolicy.INSTANCE);
    c.setMaxBufferedDocs(maxBufferedDocs);
    final IndexWriter[] writer = new IndexWriter[1];
    c.setInfoStream(
        new InfoStream() {
          @Override
          public void message(String component, String message) {
            if (component.equals("DWPT") && message.startsWith("flush postings as segment")) {
              try {
                System.out.println(
                    message
                        + " ram="
                        + writer[0].ramBytesUsed()
                        + " flushing="
                        + writer[0].getFlushingBytes());
              } catch (Exception e) {
                throw new RuntimeException(e);
              }
            }
          }

          @Override
          public boolean isEnabled(String component) {
            return component.equals("DWPT");
          }

          @Override
          public void close() {}
        });
    writer[0] = new IndexWriter(dir, c);
    for (int i = 0; i < docs; i++) {
      writer[0].addDocument(doc("mixed", i));
      if (i % 5 == 4) {
        writer[0].deleteDocuments(new Term("id", "d" + (i - 2)));
      }
    }
    writer[0].commit();
    writer[0].close();
  }

  static void files(double ramMB, int ops, long seed) throws Exception {
    boolean trace = System.getenv("TRACE") != null;
    // PERTURB=<n> draws n identity hash codes first, which shifts every later one. The writer's
    // ReaderPool keys its readerMap by SegmentCommitInfo, whose hashCode is the identity hash, and
    // getReadersByRam breaks ties between equal-RAM segments in that map's order (ReaderPool.java
    // :47, 395-422), so a case whose output changes with PERTURB depends on JVM identity hashes.
    String perturb = System.getenv("PERTURB");
    if (perturb != null) {
      for (int i = 0, n = Integer.parseInt(perturb); i < n; i++) {
        new Object().hashCode();
      }
    }
    Directory dir = new ByteBuffersDirectory();
    IndexWriterConfig c = new IndexWriterConfig();
    c.setMergeScheduler(new SerialMergeScheduler());
    c.setMergePolicy(NoMergePolicy.INSTANCE);
    c.setRAMBufferSizeMB(ramMB);
    IndexWriter w = new IndexWriter(dir, c);
    long x = seed;
    int next = 0;
    for (int op = 0; op < ops; op++) {
      x = x * 6364136223846793005L + 1442695040888963407L;
      int r = (int) ((x >>> 33) % 100);
      int target = next == 0 ? 0 : (int) ((x >>> 17) % next);
      String what;
      if (r < 55 || next == 0) {
        w.addDocument(doc("mixed", next));
        what = "add " + next;
        next++;
      } else if (r < 65) {
        w.updateDocument(new Term("id", "d" + target), doc("mixed", next));
        what = "update d" + target + " " + next;
        next++;
      } else if (r < 72) {
        w.deleteDocuments(new Term("id", "d" + target));
        what = "delete d" + target;
      } else if (r < 84) {
        w.updateNumericDocValue(new Term("id", "d" + target), "num", (long) op * 31);
        what = "numeric d" + target;
      } else if (r < 94) {
        w.updateBinaryDocValue(new Term("id", "d" + target), "bin", new BytesRef("u" + op));
        what = "binary d" + target;
      } else {
        w.updateNumericDocValue(new Term("grp", "g" + (target % 5)), "num", (long) op);
        what = "numeric g" + (target % 5);
      }
      if (trace) {
        System.out.println("op " + op + " " + what + " ram=" + w.ramBytesUsed());
      }
    }
    w.commit();
    List<String> l = new ArrayList<>();
    for (String f : dir.listAll()) {
      if (!f.equals("write.lock")) {
        l.add(f);
      }
    }
    Collections.sort(l);
    System.out.println("files=" + String.join(",", l));
    SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
    for (SegmentCommitInfo info : infos) {
      System.out.println(
          info.info.name
              + " maxDoc="
              + info.info.maxDoc()
              + " del="
              + info.getDelCount()
              + " delGen="
              + info.getDelGen()
              + " fieldInfosGen="
              + info.getFieldInfosGen()
              + " dvGen="
              + info.getDocValuesGen());
    }
    w.close();
  }

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "trace" -> trace(args[1], Integer.parseInt(args[2]));
      case "dv-updates" ->
          dvUpdates(
              args[1], Integer.parseInt(args[2]), Integer.parseInt(args[3]), Integer.parseInt(args[4]));
      case "flushing" -> flushing(Integer.parseInt(args[1]), Integer.parseInt(args[2]));
      case "files" -> files(Double.parseDouble(args[1]), Integer.parseInt(args[2]), Long.parseLong(args[3]));
      default -> throw new IllegalArgumentException(args[0]);
    }
  }
}
