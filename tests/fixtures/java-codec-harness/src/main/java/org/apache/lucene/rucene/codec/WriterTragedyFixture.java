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
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.atomic.AtomicBoolean;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FilterDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.store.Lock;

/**
 * Reference tool for how a real Apache Lucene Core 10.5.0 {@link IndexWriter}
 * reacts to a tragedy. Each case prints what {@code
 * tests/index_writer_tragedy.rs} compares Rucene against:
 *
 * <ul>
 *   <li>{@code flush-failure}: one committed document, then two acknowledged
 *       documents whose flush fails because the directory refuses to create
 *       any file but {@code segments_N} / {@code pending_segments_N}. The
 *       failure is an aborting exception of the per-thread writer
 *       ({@code DocumentsWriterPerThread.java:594-604}), which {@code
 *       IndexWriter.this.onTragicEvent} records ({@code IndexWriter.java:446-447})
 *       and {@code doFlush}'s {@code maybeCloseOnTragicEvent} turns into a
 *       close ({@code IndexWriter.java:4415-4421}). The case prints the
 *       outcome of the flush, whether the writer has a tragedy and is open,
 *       the outcome of the following commit, and the documents of the last
 *       commit on disk.
 *   <li>{@code auto-flush-failure}: with {@code maxBufferedDocs=2}, the second
 *       buffered document triggers a flush inside {@code addDocument} that the
 *       directory refuses; the tragedy closes the writer, so the next {@code
 *       addDocument}, the {@code commit} and a further {@code addDocument} fail.
 *       The case prints each outcome, the documents of the last commit and the
 *       files left in the directory.
 *   <li>{@code flush-next-buffer-failure}: {@code flushNextBuffer()} fails on
 *       the refused directory; the case prints its outcome, whether the writer
 *       has a tragedy and is open, the outcome of {@code rollback()} and the
 *       files left in the directory.
 *   <li>{@code merge-index-failure <extension>}: two committed segments whose documents have
 *       stored fields and term vectors, then a {@code forceMerge(1)} during which the directory
 *       refuses to create the one file ending in {@code <extension>} ({@code .fdx} or {@code
 *       .tvx}), the fields index that {@code FieldsIndexWriter.finish} writes. {@code
 *       SegmentMerger} closes the stored-fields and term-vectors writers in try-with-resources
 *       ({@code SegmentMerger.java:246-267}), so {@code FieldsIndexWriter.close()} deletes its
 *       temporary files ({@code FieldsIndexWriter.java:183-201}). The case prints whether the
 *       merge failed, whether the writer has a tragedy and is open, and the files left in the
 *       directory.
 *   <li>{@code oversize-delete <op> <docs>}: {@code <docs>} buffered documents, then a delete term
 *       of 32767 bytes,
 *       which {@code BytesRefHash.add} rejects ({@code BytesRefHash.java:279-284}) after the delete
 *       queue recorded it, so the full flush of the following {@code <op>} ({@code commit}, {@code
 *       flush}, {@code reader}, an NRT {@code DirectoryReader.open(IndexWriter)}, or {@code
 *       add-indexes}, {@code addIndexes(Directory...)} of a one-document index, which flushes
 *       first ({@code IndexWriter.java:3099})) cannot close
 *       the queue ({@code DocumentsWriterDeleteQueue.java:316-325}). Without {@code -ea} it throws
 *       that {@code IllegalStateException} and the writer stays usable. With {@code -ea}, {@code
 *       finishFullFlush}'s {@code assert setFlushingDeleteQueue(null)} throws {@code
 *       AssertionError} ({@code DocumentsWriter.java:645-651, 732}), which the caller records as
 *       the tragedy and closes the writer on ({@code IndexWriter.java:713-715, 3858-3863,
 *       4412-4414}); the close's own {@code assert} on {@code pendingNumDocs} ({@code
 *       IndexWriter.java:2545-2556}) may then replace the thrown error. The case prints the outcome
 *       of the delete, of {@code <op>} twice and of an {@code addDocument}, each with its cause,
 *       then the tragedy, whether the writer is open, and the files in the directory.
 *   <li>{@code add-indexes-release-failure}: {@code addIndexes(Directory...)} of a one-document
 *       index whose write lock cannot be released. On success {@code IOUtils.close(locks)}
 *       throws that failure ({@code IndexWriter.java:3205-3211}); the copied segment stays in
 *       the writer. The case prints the outcome, whether the writer is open and has a tragedy,
 *       and the documents of the following commit.
 * </ul>
 */
public final class WriterTragedyFixture {

  /** Refuses to create any file but a commit point while {@code armed}. */
  static final class FailingDirectory extends FilterDirectory {
    final AtomicBoolean armed = new AtomicBoolean();

    FailingDirectory(Directory in) {
      super(in);
    }

    @Override
    public IndexOutput createOutput(String name, IOContext context) throws IOException {
      if (armed.get() && !name.startsWith("segments") && !name.startsWith("pending")) {
        throw new IOException("disk full creating " + name);
      }
      return super.createOutput(name, context);
    }

    @Override
    public IndexOutput createTempOutput(String prefix, String suffix, IOContext context)
        throws IOException {
      if (armed.get()) {
        throw new IOException("disk full (temp)");
      }
      return super.createTempOutput(prefix, suffix, context);
    }
  }

  static Document idDoc(int id) {
    Document doc = new Document();
    doc.add(new StringField("id", Integer.toString(id), Field.Store.YES));
    doc.add(new TextField("content", "a b c", Field.Store.NO));
    return doc;
  }

  static String outcome(ThrowingRunnable body) {
    try {
      body.run();
      return "ok";
    } catch (Throwable t) {
      return t.getClass().getSimpleName();
    }
  }

  interface ThrowingRunnable {
    void run() throws Exception;
  }

  static void flushFailure() throws Exception {
    ByteBuffersDirectory ram = new ByteBuffersDirectory();
    FailingDirectory dir = new FailingDirectory(ram);
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergeScheduler(new SerialMergeScheduler());
    IndexWriter writer = new IndexWriter(dir, config);
    writer.addDocument(idDoc(0));
    writer.commit();
    writer.addDocument(idDoc(1));
    writer.addDocument(idDoc(2));
    dir.armed.set(true);
    String flushed = outcome(writer::flush);
    dir.armed.set(false);
    System.out.println("flush=" + flushed);
    System.out.println("tragic=" + (writer.getTragicException() != null));
    System.out.println("open=" + writer.isOpen());
    System.out.println("commit=" + outcome(writer::commit));
    System.out.println("committed_max_doc=" + SegmentInfos.readLatestCommit(ram).totalMaxDoc());
  }

  /** The files of {@code dir} but {@code write.lock}, sorted, comma-separated. */
  static String files(Directory dir) throws IOException {
    List<String> names = new ArrayList<>();
    for (String name : dir.listAll()) {
      if (!name.equals("write.lock")) {
        names.add(name);
      }
    }
    Collections.sort(names);
    return String.join(",", names);
  }

  static void autoFlushFailure() throws Exception {
    ByteBuffersDirectory ram = new ByteBuffersDirectory();
    FailingDirectory dir = new FailingDirectory(ram);
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergeScheduler(new SerialMergeScheduler());
    config.setMaxBufferedDocs(2);
    IndexWriter writer = new IndexWriter(dir, config);
    writer.addDocument(idDoc(0));
    writer.commit();
    dir.armed.set(true);
    String first = outcome(() -> writer.addDocument(idDoc(1)));
    String second = outcome(() -> writer.addDocument(idDoc(2)));
    dir.armed.set(false);
    System.out.println("add1=" + first);
    System.out.println("add2=" + second);
    System.out.println("tragic=" + (writer.getTragicException() != null));
    System.out.println("open=" + writer.isOpen());
    System.out.println("commit=" + outcome(writer::commit));
    System.out.println("add3=" + outcome(() -> writer.addDocument(idDoc(3))));
    System.out.println("committed_max_doc=" + SegmentInfos.readLatestCommit(ram).totalMaxDoc());
    System.out.println("files=" + files(ram));
  }

  static void flushNextBufferFailure() throws Exception {
    ByteBuffersDirectory ram = new ByteBuffersDirectory();
    FailingDirectory dir = new FailingDirectory(ram);
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergeScheduler(new SerialMergeScheduler());
    IndexWriter writer = new IndexWriter(dir, config);
    writer.addDocument(idDoc(0));
    writer.commit();
    writer.addDocument(idDoc(1));
    dir.armed.set(true);
    String flushed = outcome(writer::flushNextBuffer);
    dir.armed.set(false);
    System.out.println("flush_next_buffer=" + flushed);
    System.out.println("tragic=" + (writer.getTragicException() != null));
    System.out.println("open=" + writer.isOpen());
    System.out.println("rollback=" + outcome(writer::rollback));
    System.out.println("files=" + files(ram));
  }

  /** Refuses to create the one file ending in {@code extension} while {@code armed}. */
  static final class RefusingDirectory extends FilterDirectory {
    final AtomicBoolean armed = new AtomicBoolean();
    final String extension;

    RefusingDirectory(Directory in, String extension) {
      super(in);
      this.extension = extension;
    }

    @Override
    public IndexOutput createOutput(String name, IOContext context) throws IOException {
      if (armed.get() && name.endsWith(extension)) {
        throw new IOException("disk full creating " + name);
      }
      return super.createOutput(name, context);
    }
  }

  /** {@code TextField.TYPE_NOT_STORED} with term vectors. */
  static final FieldType TV_TYPE = new FieldType();

  static {
    TV_TYPE.setTokenized(true);
    TV_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    TV_TYPE.setStoreTermVectors(true);
    TV_TYPE.freeze();
  }

  static Document tvDoc(int id) {
    Document doc = new Document();
    doc.add(new StringField("id", Integer.toString(id), Field.Store.YES));
    doc.add(new Field("tv", "a b c", TV_TYPE));
    return doc;
  }

  static void mergeIndexFailure(String extension) throws Exception {
    ByteBuffersDirectory ram = new ByteBuffersDirectory();
    RefusingDirectory dir = new RefusingDirectory(ram, extension);
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergeScheduler(new SerialMergeScheduler());
    IndexWriter writer = new IndexWriter(dir, config);
    writer.addDocument(tvDoc(0));
    writer.commit();
    writer.addDocument(tvDoc(1));
    writer.commit();
    dir.armed.set(true);
    String merged;
    try {
      writer.forceMerge(1);
      merged = "ok";
    } catch (Throwable t) {
      // IndexWriter.handleMergeException rethrows the merge's own exception,
      // so its class and message reach the caller unwrapped.
      merged = t.getClass().getSimpleName() + ": " + t.getMessage();
    }
    dir.armed.set(false);
    System.out.println("force_merge=" + merged);
    System.out.println("tragic=" + (writer.getTragicException() != null));
    System.out.println("open=" + writer.isOpen());
    System.out.println("files=" + files(ram));
    writer.rollback();
  }

  /** {@code <SimpleName>: <message>} of the throwable {@code body} raised, then its cause. */
  static String described(ThrowingRunnable body) {
    try {
      body.run();
      return "ok";
    } catch (Throwable t) {
      return describe(t) + " cause=" + (t.getCause() == null ? "none" : describe(t.getCause()));
    }
  }

  static String describe(Throwable t) {
    return t.getClass().getSimpleName() + ": " + t.getMessage();
  }

  /** A directory holding one committed document, for {@code addIndexes(Directory...)}. */
  static Directory committedIndex() throws IOException {
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergeScheduler(new SerialMergeScheduler());
    try (IndexWriter source = new IndexWriter(dir, config)) {
      source.addDocument(idDoc(100));
    }
    return dir;
  }

  /** A directory whose write lock fails to release, after releasing it. */
  static final class UnreleasableLockDirectory extends FilterDirectory {
    UnreleasableLockDirectory(Directory in) {
      super(in);
    }

    @Override
    public Lock obtainLock(String name) throws IOException {
      Lock lock = super.obtainLock(name);
      return new Lock() {
        @Override
        public void close() throws IOException {
          lock.close();
          throw new IOException("cannot release " + name);
        }

        @Override
        public void ensureValid() throws IOException {
          lock.ensureValid();
        }
      };
    }
  }

  static void addIndexesReleaseFailure() throws Exception {
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergeScheduler(new SerialMergeScheduler());
    IndexWriter writer = new IndexWriter(dir, config);
    writer.addDocument(idDoc(0));
    System.out.println(
        "addIndexes="
            + described(() -> writer.addIndexes(new UnreleasableLockDirectory(committedIndex()))));
    System.out.println("open=" + writer.isOpen());
    System.out.println("tragic=" + (writer.getTragicException() != null));
    writer.commit();
    System.out.println("committed_max_doc=" + SegmentInfos.readLatestCommit(dir).totalMaxDoc());
    writer.close();
  }

  static void oversizeDelete(String op, int docs) throws Exception {
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergeScheduler(new SerialMergeScheduler());
    // The "hit tragic <SimpleName> inside <location>" lines onTragicEvent logs
    // (IndexWriter.java:5857-5860).
    List<String> tragicMessages = new ArrayList<>();
    config.setInfoStream(
        new org.apache.lucene.util.InfoStream() {
          @Override
          public void message(String component, String message) {
            if (message.startsWith("hit tragic ")) {
              tragicMessages.add(component + ": " + message);
            }
          }

          @Override
          public boolean isEnabled(String component) {
            return component.equals("IW");
          }

          @Override
          public void close() {}
        });
    IndexWriter writer = new IndexWriter(dir, config);
    for (int i = 0; i < docs; i++) {
      writer.addDocument(idDoc(i));
    }
    byte[] big = new byte[32767];
    java.util.Arrays.fill(big, (byte) 'x');
    System.out.println(
        "delete="
            + described(
                () ->
                    writer.deleteDocuments(
                        new org.apache.lucene.index.Term(
                            "id", new org.apache.lucene.util.BytesRef(big)))));
    ThrowingRunnable run =
        switch (op) {
          case "commit" -> writer::commit;
          case "flush" -> writer::flush;
          case "reader" -> () -> org.apache.lucene.index.DirectoryReader.open(writer).close();
          case "add-indexes" -> () -> writer.addIndexes(committedIndex());
          default -> throw new IllegalArgumentException(op);
        };
    System.out.println(op + "1=" + described(run));
    System.out.println(op + "2=" + described(run));
    System.out.println("add=" + described(() -> writer.addDocument(idDoc(docs))));
    Throwable tragedy = writer.getTragicException();
    System.out.println("tragic=" + (tragedy == null ? "none" : describe(tragedy)));
    System.out.println("open=" + writer.isOpen());
    System.out.println("files=" + files(dir));
    System.out.println("tragic-messages=" + String.join(" | ", tragicMessages));
    writer.rollback();
  }

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "merge-index-failure" -> mergeIndexFailure(args[1]);
      case "flush-failure" -> flushFailure();
      case "auto-flush-failure" -> autoFlushFailure();
      case "flush-next-buffer-failure" -> flushNextBufferFailure();
      case "add-indexes-release-failure" -> addIndexesReleaseFailure();
      case "oversize-delete" -> oversizeDelete(args[1], Integer.parseInt(args[2]));
      default -> throw new IllegalArgumentException("unknown case " + args[0]);
    }
  }
}
