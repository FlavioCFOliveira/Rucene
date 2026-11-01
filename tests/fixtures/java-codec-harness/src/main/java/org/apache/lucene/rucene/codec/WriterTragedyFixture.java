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
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.BooleanSupplier;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.CodecReader;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LogDocMergePolicy;
import org.apache.lucene.index.MergePolicy;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.NoMergeScheduler;
import org.apache.lucene.index.OneMergeWrappingMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FilterDirectory;
import org.apache.lucene.store.FilterIndexInput;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.store.Lock;
import org.apache.lucene.util.InfoStream;

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
 *   <li>{@code pit-rollback}: commit merges on (the default {@code maxFullFlushMergeWaitMillis}),
 *       a scheduler that runs nothing, and a {@code LogDocMergePolicy} of merge factor 2 whose
 *       merges' {@code mergeFinished} fails for the merge of {@code _0}. Four committed documents
 *       and a {@code forceMerge(1, false)} leave the commit merges pending, each wrapped by
 *       {@code preparePointInTimeMerge}, whose {@code mergeFinished} calls the wrapped one's
 *       ({@code IndexWriter.java:4006}); {@code rollback()} aborts them. The case prints the
 *       outcome of the rollback, the merges finished, whether merges are pending, the merging
 *       segments and whether the writer is open.
 *   <li>{@code merge-close-failure <hook>}: a {@code SerialMergeScheduler} whose {@code
 *       wrapForMerge} fails, under a policy whose merges' {@code mergeFinished} fails too when
 *       {@code <hook>} is {@code throw}. {@code merge}'s {@code finally} closes the merge readers
 *       ({@code IndexWriter.java:4866-4872}); a failure there replaces the merge's own and skips
 *       {@code mergeFinish}, and the tragedy's rollback then waits for ever on the merge thread.
 *       The case prints the outcome of {@code forceMerge(1)} — {@code HUNG} when it does not
 *       return within eight seconds — the merges finished, the tragedy and whether the writer is
 *       open.
 *   <li>{@code merge-middle-failure <hook>}: a merge whose {@code OneMerge.wrapForMerge} fails
 *       inside {@code mergeMiddle}, and whose {@code mergeFinished} fails too when {@code <hook>}
 *       is {@code throw}; {@code mergeMiddle}'s {@code finally} closes the readers ({@code
 *       IndexWriter.java:5588-5594}). The case prints the same lines as {@code
 *       merge-close-failure}.
 *   <li>{@code register-failure}: two committed segments without compound files, then {@code
 *       _0.fdt} is deleted and {@code maybeMerge} asks a {@code LogDocMergePolicy} of merge factor
 *       2 for merges; {@code registerMerge} fails sizing the merge ({@code IndexWriter.java:
 *       5022}), and {@code updatePendingMerges} propagates it ({@code IndexWriter.java:
 *       2471-2477}). The case prints the outcome of the private {@code maybeMerge(MergePolicy,
 *       MergeTrigger, int)}, reached by reflection, the {@code IW} messages of {@code
 *       registerMerge} ({@code IndexWriter.java:4984-5017}), whether merges are pending, the
 *       merging segments, and the outcome of {@code rollback()}: the merge is pending but not
 *       registered, so its segments stay merging and, under {@code -ea}, the rollback trips
 *       {@code assert mergingSegments.isEmpty()} ({@code IndexWriter.java:2582-2583}).
 *   <li>{@code merging-after-rollback}: one committed and two flushed segments, the flushes
 *       registering the merge of {@code _0} and {@code _1} under a scheduler that runs nothing
 *       and a policy whose merges' {@code mergeFinished} fails. The case prints the merging
 *       segments before the rollback, its outcome, whether the writer is open, and the merging
 *       segments after it — which {@code getMergingSegments()} returns without checking the
 *       writer or the live infos ({@code IndexWriter.java:2488-2490}).
 *   <li>{@code cfs-rollback}: a {@code forceMerge(1)} of three segments into a compound file
 *       whose {@code .cfs} cannot be created until another thread's {@code rollback()} aborted
 *       the merge. The aborted merge returns 0 and the {@code finally} deletes the merged
 *       segment's non-compound files on the merge thread ({@code IndexWriter.java:5480-5503}).
 *       The case prints the outcome of the {@code forceMerge} and of the rollback, the
 *       compound-file {@code IW} messages, and the files the merge thread deletes after the failure.
 *   <li>{@code warmer-rollback <keep|drop-fails>}: reader pooling and a merged-segment warmer
 *       ({@code IndexWriter.java:5567-5578}) that waits until another thread's {@code
 *       rollback()} aborted the merge; {@code commitMerge} then skips it and returns false
 *       ({@code IndexWriter.java:4692-4708}), and {@code mergeMiddle}'s {@code finally} closes
 *       the readers, finishing the merge with {@code success=false}. With {@code drop-fails}
 *       the merged segment's {@code _3.fdt}, which the pooled reader holds, fails to close, so {@code readerPool.drop(merge.info)}
 *       ({@code IndexWriter.java:4705}) throws out of {@code commitMerge}. The case prints the
 *       outcome of the {@code forceMerge} and of the rollback, the merges finished, the tragedy
 *       and whether the writer is open.
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

  /**
   * A {@code LogDocMergePolicy} of merge factor {@code mergeFactor} whose merges record {@code
   * mergeFinished} in {@code finished} and fail it when {@code fails} names the merge's first
   * segment, or always when {@code fails} is {@code "*"}; their {@code wrapForMerge} fails when
   * {@code failWrap}.
   */
  static MergePolicy hookedPolicy(
      List<String> finished, String fails, boolean failWrap, int mergeFactor) {
    LogDocMergePolicy inner = new LogDocMergePolicy();
    inner.setMergeFactor(mergeFactor);
    return new OneMergeWrappingMergePolicy(
        inner,
        merge ->
            new MergePolicy.OneMerge(merge.segments) {
              @Override
              public CodecReader wrapForMerge(CodecReader reader) throws IOException {
                if (failWrap) {
                  throw new IOException("wrapForMerge failure");
                }
                return reader;
              }

              @Override
              public void mergeFinished(boolean success, boolean segmentDropped)
                  throws IOException {
                List<String> names = new ArrayList<>();
                for (SegmentCommitInfo info : segments) {
                  names.add(info.info.name);
                }
                String joined = String.join(",", names);
                finished.add(joined + " success=" + success);
                if (fails != null && (fails.equals("*") || fails.equals(names.get(0)))) {
                  throw new IOException("injected failure finishing " + joined);
                }
              }
            });
  }

  static void pitRollback() throws Exception {
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergeScheduler(NoMergeScheduler.INSTANCE);
    List<String> finished = Collections.synchronizedList(new ArrayList<>());
    config.setMergePolicy(hookedPolicy(finished, "_0", false, 2));
    IndexWriter writer = new IndexWriter(new ByteBuffersDirectory(), config);
    for (int i = 0; i < 4; i++) {
      writer.addDocument(idDoc(i));
      writer.commit();
    }
    writer.forceMerge(1, false);
    System.out.println("pending-before=" + writer.hasPendingMerges());
    System.out.println("rollback=" + described(writer::rollback));
    System.out.println("finished=" + finished);
    System.out.println("pending-after=" + writer.hasPendingMerges());
    System.out.println("merging=" + sortedNames(writer.getMergingSegments()));
    System.out.println("open=" + writer.isOpen());
  }

  static String sortedNames(java.util.Collection<SegmentCommitInfo> infos) {
    List<String> names = new ArrayList<>();
    for (SegmentCommitInfo info : infos) {
      names.add(info.toString());
    }
    Collections.sort(names);
    return String.join(" ", names);
  }

  /** Runs {@code forceMerge(1)} on its own thread, reporting {@code HUNG} after eight seconds. */
  static String forceMergeOrHang(IndexWriter writer) throws Exception {
    ExecutorService executor =
        Executors.newSingleThreadExecutor(
            r -> {
              Thread t = new Thread(r);
              t.setDaemon(true);
              return t;
            });
    Future<String> merged = executor.submit(() -> described(() -> writer.forceMerge(1)));
    try {
      return merged.get(8, TimeUnit.SECONDS);
    } catch (TimeoutException e) {
      return "HUNG";
    } finally {
      executor.shutdown();
    }
  }

  static void mergeFailure(boolean inMiddle, boolean hookThrows) throws Exception {
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMaxFullFlushMergeWaitMillis(0);
    if (inMiddle) {
      config.setMergeScheduler(new SerialMergeScheduler());
    } else {
      config.setMergeScheduler(
          new SerialMergeScheduler() {
            @Override
            public Directory wrapForMerge(MergePolicy.OneMerge merge, Directory in) {
              throw new IllegalStateException("wrap failure");
            }
          });
    }
    List<String> finished = Collections.synchronizedList(new ArrayList<>());
    // The default merge factor (10): only the forceMerge merges.
    config.setMergePolicy(hookedPolicy(finished, hookThrows ? "*" : null, inMiddle, 10));
    IndexWriter writer = new IndexWriter(new ByteBuffersDirectory(), config);
    for (int i = 0; i < 3; i++) {
      writer.addDocument(idDoc(i));
      writer.commit();
    }
    System.out.println("forceMerge=" + forceMergeOrHang(writer));
    System.out.println("finished=" + finished);
    Throwable tragedy = writer.getTragicException();
    System.out.println("tragic=" + (tragedy == null ? "none" : describe(tragedy)));
    System.out.println("open=" + writer.isOpen());
  }

  static void registerFailure() throws Exception {
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    IndexWriterConfig config = new IndexWriterConfig();
    List<String> registerMessages = Collections.synchronizedList(new ArrayList<>());
    config.setInfoStream(
        recordingInfoStream(
            registerMessages, "add merge to pendingMerges", "registerMerge"));
    config.setMaxFullFlushMergeWaitMillis(0);
    config.setMergeScheduler(NoMergeScheduler.INSTANCE);
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    config.setUseCompoundFile(false);
    IndexWriter writer = new IndexWriter(dir, config);
    for (int i = 0; i < 2; i++) {
      writer.addDocument(idDoc(i));
      writer.commit();
    }
    dir.deleteFile("_0.fdt");
    LogDocMergePolicy policy = new LogDocMergePolicy();
    policy.setMergeFactor(2);
    java.lang.reflect.Method maybeMerge =
        IndexWriter.class.getDeclaredMethod(
            "maybeMerge",
            MergePolicy.class,
            org.apache.lucene.index.MergeTrigger.class,
            int.class);
    maybeMerge.setAccessible(true);
    System.out.println(
        "maybeMerge="
            + described(
                () -> {
                  try {
                    maybeMerge.invoke(
                        writer, policy, org.apache.lucene.index.MergeTrigger.EXPLICIT, -1);
                  } catch (java.lang.reflect.InvocationTargetException e) {
                    if (e.getCause() instanceof Exception cause) {
                      throw cause;
                    }
                    throw e;
                  }
                }));
    for (String message : registerMessages) {
      System.out.println("IW " + message);
    }
    System.out.println("pending=" + writer.hasPendingMerges());
    System.out.println("merging=" + sortedNames(writer.getMergingSegments()));
    System.out.println("rollback=" + described(writer::rollback));
  }

  static void mergingAfterRollback() throws Exception {
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMaxFullFlushMergeWaitMillis(0);
    config.setMergeScheduler(NoMergeScheduler.INSTANCE);
    List<String> finished = Collections.synchronizedList(new ArrayList<>());
    config.setMergePolicy(hookedPolicy(finished, "*", false, 2));
    IndexWriter writer = new IndexWriter(new ByteBuffersDirectory(), config);
    writer.addDocument(idDoc(0));
    writer.commit();
    writer.addDocument(idDoc(1));
    writer.flush();
    writer.addDocument(idDoc(2));
    writer.flush();
    System.out.println("merging-before=" + sortedNames(writer.getMergingSegments()));
    System.out.println("rollback=" + described(writer::rollback));
    System.out.println("open=" + writer.isOpen());
    System.out.println("merging-after=" + sortedNames(writer.getMergingSegments()));
  }

  /** An {@link InfoStream} keeping the {@code IW} messages that start with one of {@code prefixes}. */
  static InfoStream recordingInfoStream(List<String> sink, String... prefixes) {
    return new InfoStream() {
      @Override
      public void message(String component, String message) {
        for (String prefix : prefixes) {
          if (message.startsWith(prefix)) {
            sink.add(message);
            return;
          }
        }
      }

      @Override
      public boolean isEnabled(String component) {
        return component.equals("IW");
      }

      @Override
      public void close() {}
    };
  }

  /**
   * Opens the input named {@code failingName} so that its {@code close()} fails, and records the
   * files deleted on {@code recordedThread}.
   */
  static class CloseFailingDirectory extends FilterDirectory {
    final AtomicReference<String> failingName = new AtomicReference<>();
    final AtomicReference<Thread> recordedThread = new AtomicReference<>();
    final List<String> deletedOnRecordedThread = Collections.synchronizedList(new ArrayList<>());

    CloseFailingDirectory(Directory in) {
      super(in);
    }

    @Override
    public IndexInput openInput(String name, IOContext context) throws IOException {
      IndexInput input = super.openInput(name, context);
      if (name.equals(failingName.get()) == false) {
        return input;
      }
      return new FilterIndexInput("CloseFailing(" + input + ")", input) {
        @Override
        public void close() throws IOException {
          in.close();
          throw new IOException("injected failure closing an input");
        }
      };
    }

    @Override
    public void deleteFile(String name) throws IOException {
      if (Thread.currentThread() == recordedThread.get()) {
        deletedOnRecordedThread.add(name);
      }
      super.deleteFile(name);
    }
  }

  /** Waits up to thirty seconds for {@code condition}, failing if it never holds. */
  static void await(BooleanSupplier condition, String what) throws InterruptedException {
    long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(30);
    while (condition.getAsBoolean() == false) {
      if (System.nanoTime() > deadline) {
        throw new IllegalStateException("timed out waiting for " + what);
      }
      Thread.sleep(5);
    }
  }

  /** Starts {@code writer.rollback()} on its own thread, which records its outcome. */
  static Thread startRollback(IndexWriter writer, AtomicReference<String> outcome) {
    Thread thread = new Thread(() -> outcome.set(described(writer::rollback)));
    thread.setDaemon(true);
    thread.start();
    return thread;
  }

  static void cfsRollback() throws Exception {
    CloseFailingDirectory dir;
    AtomicReference<IndexWriter> writerRef = new AtomicReference<>();
    AtomicReference<MergePolicy.OneMerge> captured = new AtomicReference<>();
    AtomicBoolean armed = new AtomicBoolean();
    AtomicReference<String> rollbackOutcome = new AtomicReference<>();
    AtomicReference<Thread> rollbackThread = new AtomicReference<>();
    dir =
        new CloseFailingDirectory(new ByteBuffersDirectory()) {
          @Override
          public IndexOutput createOutput(String name, IOContext context) throws IOException {
            if (name.endsWith(".cfs") && armed.compareAndSet(true, false)) {
              rollbackThread.set(startRollback(writerRef.get(), rollbackOutcome));
              try {
                await(() -> captured.get().isAborted(), "the merge to abort");
              } catch (InterruptedException e) {
                throw new IOException(e);
              }
              // Record what the merge thread deletes from here on.
              recordedThread.set(Thread.currentThread());
              throw new IOException("injected failure creating " + name);
            }
            return super.createOutput(name, context);
          }
        };
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMaxFullFlushMergeWaitMillis(0);
    config.setMergeScheduler(new SerialMergeScheduler());
    // Flushed segments without compound files: only the merge builds one.
    config.setUseCompoundFile(false);
    List<String> messages = Collections.synchronizedList(new ArrayList<>());
    config.setInfoStream(
        recordingInfoStream(
            messages,
            "hit merge abort exception creating compound file",
            "hit exception creating compound file"));
    LogDocMergePolicy inner = new LogDocMergePolicy();
    inner.setNoCFSRatio(1.0);
    config.setMergePolicy(
        new OneMergeWrappingMergePolicy(
            inner,
            merge -> {
              captured.set(merge);
              return merge;
            }));
    IndexWriter writer = new IndexWriter(dir, config);
    writerRef.set(writer);
    for (int i = 0; i < 3; i++) {
      writer.addDocument(idDoc(i));
      writer.commit();
    }
    armed.set(true);
    System.out.println("forceMerge=" + described(() -> writer.forceMerge(1)));
    dir.recordedThread.set(null);
    Thread rollback = rollbackThread.get();
    rollback.join(TimeUnit.SECONDS.toMillis(30));
    System.out.println("rollback=" + (rollback.isAlive() ? "HUNG" : rollbackOutcome.get()));
    System.out.println("messages=" + messages);
    List<String> deleted = new ArrayList<>(dir.deletedOnRecordedThread);
    Collections.sort(deleted);
    System.out.println("deleted-on-merge-thread-after-failure=" + deleted);
    System.out.println("open=" + writer.isOpen());
  }

  /**
   * {@code cfs-failure}: a merge cannot create its compound file, and nothing aborted it. Under the
   * writer's lock, {@code mergeMiddle}'s {@code catch} hands the failure to {@code
   * handleMergeException}, which logs it, sets it on the merge and rethrows it; the {@code finally}
   * then reports it and deletes the merged segment's non-compound files ({@code
   * IndexWriter.java:5481-5503}). {@code merge} hands it to {@code handleMergeException} again,
   * logs {@code hit exception during merge} and records the tragedy ({@code
   * IndexWriter.java:4850-4892}). The merge thread's deletions are recorded from the failure to
   * that log line, before the tragedy's rollback.
   */
  static void cfsFailure() throws Exception {
    AtomicBoolean armed = new AtomicBoolean();
    CloseFailingDirectory dir =
        new CloseFailingDirectory(new ByteBuffersDirectory()) {
          @Override
          public IndexOutput createOutput(String name, IOContext context) throws IOException {
            if (name.endsWith(".cfs") && armed.compareAndSet(true, false)) {
              // Record what the merge thread deletes from here on.
              recordedThread.set(Thread.currentThread());
              throw new IOException("injected failure creating " + name);
            }
            return super.createOutput(name, context);
          }
        };
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMaxFullFlushMergeWaitMillis(0);
    config.setMergeScheduler(new SerialMergeScheduler());
    // Flushed segments without compound files: only the merge builds one.
    config.setUseCompoundFile(false);
    List<String> messages = Collections.synchronizedList(new ArrayList<>());
    InfoStream recording =
        recordingInfoStream(
            messages,
            "handleMergeException",
            "hit merge abort exception creating compound file",
            "hit exception creating compound file",
            "hit exception during merge");
    config.setInfoStream(
        new InfoStream() {
          @Override
          public void message(String component, String message) {
            if (message.equals("hit exception during merge")) {
              dir.recordedThread.set(null);
            }
            recording.message(component, message);
          }

          @Override
          public boolean isEnabled(String component) {
            return recording.isEnabled(component);
          }

          @Override
          public void close() {}
        });
    LogDocMergePolicy policy = new LogDocMergePolicy();
    policy.setNoCFSRatio(1.0);
    config.setMergePolicy(policy);
    IndexWriter writer = new IndexWriter(dir, config);
    for (int i = 0; i < 3; i++) {
      writer.addDocument(idDoc(i));
      writer.commit();
    }
    armed.set(true);
    System.out.println("forceMerge=" + described(() -> writer.forceMerge(1)));
    dir.recordedThread.set(null);
    System.out.println("messages=" + messages);
    List<String> deleted = new ArrayList<>(dir.deletedOnRecordedThread);
    Collections.sort(deleted);
    System.out.println("deleted-on-merge-thread-before-merge-finished=" + deleted);
    Throwable tragedy = writer.getTragicException();
    System.out.println("tragic=" + (tragedy == null ? "none" : describe(tragedy)));
    System.out.println("open=" + writer.isOpen());
  }

  static void warmerRollback(boolean dropFails) throws Exception {
    CloseFailingDirectory dir = new CloseFailingDirectory(new ByteBuffersDirectory());
    AtomicReference<IndexWriter> writerRef = new AtomicReference<>();
    AtomicReference<MergePolicy.OneMerge> captured = new AtomicReference<>();
    AtomicReference<String> rollbackOutcome = new AtomicReference<>();
    AtomicReference<Thread> rollbackThread = new AtomicReference<>();
    List<String> finished = Collections.synchronizedList(new ArrayList<>());
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMaxFullFlushMergeWaitMillis(0);
    config.setMergeScheduler(new SerialMergeScheduler());
    config.setUseCompoundFile(false);
    config.setReaderPooling(true);
    config.setMergedSegmentWarmer(
        reader -> {
          rollbackThread.set(startRollback(writerRef.get(), rollbackOutcome));
          try {
            await(() -> captured.get().isAborted(), "the merge to abort");
          } catch (InterruptedException e) {
            throw new IOException(e);
          }
        });
    LogDocMergePolicy inner = new LogDocMergePolicy();
    inner.setNoCFSRatio(0.0);
    config.setMergePolicy(
        new OneMergeWrappingMergePolicy(
            inner,
            merge -> {
              MergePolicy.OneMerge wrapped =
                  new MergePolicy.OneMerge(merge.segments) {
                    @Override
                    public void mergeFinished(boolean success, boolean segmentDropped) {
                      List<String> names = new ArrayList<>();
                      for (SegmentCommitInfo info : segments) {
                        names.add(info.info.name);
                      }
                      finished.add(String.join(",", names) + " success=" + success);
                    }
                  };
              captured.set(wrapped);
              return wrapped;
            }));
    IndexWriter writer = new IndexWriter(dir, config);
    writerRef.set(writer);
    for (int i = 0; i < 3; i++) {
      writer.addDocument(idDoc(i));
      writer.commit();
    }
    if (dropFails) {
      // The stored-fields data of the merged segment, which the warmer's pooled reader keeps
      // open until `readerPool.drop` closes it.
      dir.failingName.set("_3.fdt");
    }
    System.out.println("forceMerge=" + described(() -> writer.forceMerge(1)));
    Thread rollback = rollbackThread.get();
    rollback.join(TimeUnit.SECONDS.toMillis(30));
    System.out.println("rollback=" + (rollback.isAlive() ? "HUNG" : rollbackOutcome.get()));
    System.out.println("finished=" + finished);
    Throwable tragedy = writer.getTragicException();
    System.out.println("tragic=" + (tragedy == null ? "none" : describe(tragedy)));
    System.out.println("open=" + writer.isOpen());
  }

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "merge-index-failure" -> mergeIndexFailure(args[1]);
      case "flush-failure" -> flushFailure();
      case "auto-flush-failure" -> autoFlushFailure();
      case "flush-next-buffer-failure" -> flushNextBufferFailure();
      case "add-indexes-release-failure" -> addIndexesReleaseFailure();
      case "oversize-delete" -> oversizeDelete(args[1], Integer.parseInt(args[2]));
      case "pit-rollback" -> pitRollback();
      case "merge-close-failure" -> mergeFailure(false, args[1].equals("throw"));
      case "merge-middle-failure" -> mergeFailure(true, args[1].equals("throw"));
      case "cfs-rollback" -> cfsRollback();
      case "cfs-failure" -> cfsFailure();
      case "warmer-rollback" -> warmerRollback(args[1].equals("drop-fails"));
      case "register-failure" -> registerFailure();
      case "merging-after-rollback" -> mergingAfterRollback();
      default -> throw new IllegalArgumentException("unknown case " + args[0]);
    }
    // A case that left a merge thread blocked must still end the JVM.
    System.exit(0);
  }
}
