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
package org.apache.lucene.index;

import java.io.IOException;
import java.nio.file.Path;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.BiConsumer;

import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.BinaryPoint;
import org.apache.lucene.document.DoubleDocValuesField;
import org.apache.lucene.document.FeatureField;
import org.apache.lucene.document.FloatDocValuesField;
import org.apache.lucene.document.InetAddressRange;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.KeywordField;
import org.apache.lucene.document.KnnByteVectorField;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.document.LateInteractionField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.InfoStream;
import org.apache.lucene.util.Version;

/**
 * Measures the {@code toString()} of the reader, enum, pending-deletes,
 * buffered-update and writer-config families straight out of
 * {@code lucene-core-10.5.0.jar}.
 *
 * <p>Lives in {@code org.apache.lucene.index} because most of the types
 * involved — {@code PendingDeletes}, {@code ReadersAndUpdates},
 * {@code BufferedUpdates}, {@code TermsEnumIndex},
 * {@code DocumentsWriterPerThread} — are package-private and unreachable from
 * anywhere else.
 */
public final class RuceneToStringContracts {
  private RuceneToStringContracts() {}

  /** Escapes a value so that a multi-line {@code toString()} survives the `key=value` protocol. */
  private static String escape(String value) {
    return value.replace("\\", "\\\\").replace("\n", "\\n");
  }

  /**
   * @param indexDir a directory this fixture fills with the index every
   *     reader-side measurement is taken from, so that the Rust side can open
   *     the very same files and compare against the strings measured here.
   */
  public static void emit(BiConsumer<String, String> out, Path indexDir) throws Exception {
    document(out);
    documentFields(out);
    writerConfig(out);
    try (Directory dir = FSDirectory.open(indexDir)) {
      buildIndex(dir);
      readers(out, dir);
      enums(out, dir);
      pendingDeletes(out, dir);
      bufferedUpdates(out, dir);
    }
  }

  // -------------------------------------------------------------------------
  // Document
  // -------------------------------------------------------------------------

  private static void document(BiConsumer<String, String> out) {
    out.accept("Document.toString/empty", new Document().toString());

    Document one = new Document();
    one.add(new StringField("id", "1", Field.Store.NO));
    out.accept("Document.toString/single-string-field", one.toString());

    Document two = new Document();
    two.add(new StringField("id", "1", Field.Store.NO));
    two.add(new IntPoint("count", 42));
    out.accept("Document.toString/string-and-point", two.toString());

    Document mixed = new Document();
    mixed.add(new TextField("body", "hello world", Field.Store.YES));
    mixed.add(new NumericDocValuesField("price", 17L));
    mixed.add(new SortedDocValuesField("sorted", new BytesRef("abc")));
    out.accept("Document.toString/mixed", mixed.toString());
  }

  /**
   * Every {@code IndexableField} implementation the port carries, rendered by
   * its own {@code toString()} — which for all of these is the inherited
   * {@code Field.toString()}, so what is being pinned is how each one's
   * {@code fieldsData} renders.
   */
  private static void documentFields(BiConsumer<String, String> out) throws Exception {
    out.accept("field.StringField.string", new StringField("id", "1", Field.Store.NO).toString());
    out.accept(
        "field.StringField.stored", new StringField("id", "1", Field.Store.YES).toString());
    out.accept(
        "field.StringField.bytes",
        new StringField("id", new BytesRef("abc"), Field.Store.NO).toString());
    out.accept(
        "field.TextField.string", new TextField("body", "hello world", Field.Store.NO).toString());
    out.accept(
        "field.TextField.stored", new TextField("body", "hello world", Field.Store.YES).toString());
    out.accept("field.StoredField.string", new StoredField("s", "text").toString());
    out.accept("field.StoredField.int", new StoredField("s", 42).toString());
    out.accept("field.StoredField.long", new StoredField("s", 42L).toString());
    out.accept("field.StoredField.float", new StoredField("s", 2.5f).toString());
    out.accept("field.StoredField.double", new StoredField("s", 2.5).toString());
    out.accept(
        "field.StoredField.bytes", new StoredField("s", new BytesRef("abc")).toString());
    out.accept(
        "field.KeywordField.string", new KeywordField("k", "v", Field.Store.NO).toString());
    out.accept(
        "field.KeywordField.stored", new KeywordField("k", "v", Field.Store.YES).toString());
    out.accept(
        "field.KeywordField.bytes",
        new KeywordField("k", new BytesRef("abc"), Field.Store.NO).toString());
    out.accept(
        "field.BinaryPoint",
        new BinaryPoint("b", new byte[][] {new byte[] {1, 2, 3}}).toString());
    out.accept("field.NumericDocValuesField", new NumericDocValuesField("n", 17L).toString());
    out.accept("field.FloatDocValuesField", new FloatDocValuesField("n", 2.5f).toString());
    out.accept("field.DoubleDocValuesField", new DoubleDocValuesField("n", 2.5).toString());
    out.accept(
        "field.SortedDocValuesField",
        new SortedDocValuesField("s", new BytesRef("abc")).toString());
    out.accept(
        "field.SortedSetDocValuesField",
        new SortedSetDocValuesField("s", new BytesRef("abc")).toString());
    out.accept(
        "field.SortedNumericDocValuesField",
        new SortedNumericDocValuesField("s", 17L).toString());
    out.accept(
        "field.BinaryDocValuesField",
        new BinaryDocValuesField("b", new BytesRef("abc")).toString());
    out.accept(
        "field.KnnFloatVectorField",
        new KnnFloatVectorField("v", new float[] {1f, 2f})
            .toString()
            .replaceAll("@[0-9a-f]+>$", "@<identity>>"));
    out.accept(
        "field.KnnByteVectorField",
        new KnnByteVectorField("v", new byte[] {1, 2})
            .toString()
            .replaceAll("@[0-9a-f]+>$", "@<identity>>"));
    out.accept(
        "field.FeatureField", new FeatureField("f", "name", 2.5f).toString());
    out.accept(
        "field.LateInteractionField",
        new LateInteractionField("li", new float[][] {{1f, 2f}}).toString());
    out.accept(
        "field.InetAddressRange",
        new InetAddressRange(
                "r",
                java.net.InetAddress.getByName("1.2.3.4"),
                java.net.InetAddress.getByName("1.2.3.9"))
            .toString());
    out.accept(
        "field.LatLonShapeDocValuesField",
        org.apache.lucene.document.LatLonShape.createDocValueField(
                "shape", new org.apache.lucene.geo.Line(new double[] {0, 1}, new double[] {2, 3}))
            .toString());
    out.accept(
        "field.XYShapeDocValuesField",
        org.apache.lucene.document.XYShape.createDocValueField(
                "shape",
                new org.apache.lucene.geo.XYLine(new float[] {0, 1}, new float[] {2, 3}))
            .toString());
    out.accept(
        "field.IntRangeDocValuesField",
        new org.apache.lucene.document.IntRangeDocValuesField(
                "r", new int[] {10}, new int[] {20})
            .toString());
    out.accept(
        "field.LatLonShape.triangle",
        org.apache.lucene.document.LatLonShape.createIndexableFields(
                "shape", new org.apache.lucene.geo.Line(new double[] {0, 1}, new double[] {2, 3}))[0]
            .toString());
  }

  // -------------------------------------------------------------------------
  // Writer configuration
  // -------------------------------------------------------------------------

  private static void writerConfig(BiConsumer<String, String> out) {
    IndexWriterConfig defaults = new IndexWriterConfig();
    out.accept("IndexWriterConfig.toString/default", escape(defaults.toString()));

    IndexWriterConfig tuned = new IndexWriterConfig(new StandardAnalyzer());
    tuned.setRAMBufferSizeMB(64.0);
    tuned.setMaxBufferedDocs(1000);
    tuned.setUseCompoundFile(false);
    tuned.setCommitOnClose(false);
    tuned.setSoftDeletesField("__soft_deletes");
    tuned.setIndexSort(new Sort(new SortField("price", SortField.Type.LONG)));
    tuned.setMergePolicy(NoMergePolicy.INSTANCE);
    tuned.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    out.accept("IndexWriterConfig.toString/tuned", escape(tuned.toString()));

    // `LiveIndexWriterConfig.toString()` is what `IndexWriterConfig.toString()`
    // starts from: the same text without the trailing `writer=` line.
    LiveIndexWriterConfig live = new IndexWriterConfig();
    out.accept("LiveIndexWriterConfig.toString/default", escape(liveToString(live)));
  }

  /**
   * Calls {@code LiveIndexWriterConfig.toString()} rather than the
   * {@code IndexWriterConfig} override, which is what {@code super.toString()}
   * reaches inside {@code IndexWriterConfig.toString()}.
   */
  private static String liveToString(LiveIndexWriterConfig config) {
    // A plain LiveIndexWriterConfig, built through the package-private
    // constructor, so `toString()` dispatches to LiveIndexWriterConfig's own.
    LiveIndexWriterConfig plain = new LiveIndexWriterConfig(config.getAnalyzer());
    return plain.toString();
  }

  // -------------------------------------------------------------------------
  // The index every reader-side measurement is taken from
  // -------------------------------------------------------------------------

  private static void buildIndex(Directory dir) throws IOException {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    config.setUseCompoundFile(false);
    try (IndexWriter writer = new IndexWriter(dir, config)) {
      for (int segment = 0; segment < 2; segment++) {
        for (int doc = 0; doc < 3; doc++) {
          Document d = new Document();
          d.add(new StringField("id", segment + "-" + doc, Field.Store.YES));
          d.add(new TextField("body", "alpha beta gamma", Field.Store.NO));
          d.add(new NumericDocValuesField("price", doc));
          writer.addDocument(d);
        }
        writer.flush();
      }
      writer.commit();
    }
  }

  // -------------------------------------------------------------------------
  // Readers
  // -------------------------------------------------------------------------

  private static void readers(BiConsumer<String, String> out, Directory dir) throws IOException {
    try (DirectoryReader reader = DirectoryReader.open(dir)) {
      out.accept("StandardDirectoryReader.toString/two-segments", reader.toString());

      LeafReader leaf0 = reader.leaves().get(0).reader();
      out.accept("SegmentReader.toString/segment-0", leaf0.toString());
      out.accept("LeafReaderContext.toString/leaf-0", reader.leaves().get(0).toString());
      out.accept("LeafReaderContext.toString/leaf-1", reader.leaves().get(1).toString());

      // CompositeReader.toString() — inherited by MultiReader, which does not
      // override it, so the string is produced by CompositeReader itself.
      MultiReader multi = new MultiReader(leaf0, reader.leaves().get(1).reader());
      out.accept("CompositeReader.toString/MultiReader-two-leaves", multi.toString());
      out.accept("CompositeReader.toString/MultiReader-empty", new MultiReader().toString());

      // FilterLeafReader hardcodes its own name, so a subclass that does not
      // override toString() still renders as `FilterLeafReader(...)`.
      FilterLeafReader filtered =
          new FilterLeafReader(leaf0) {
            @Override
            public CacheHelper getCoreCacheHelper() {
              return null;
            }

            @Override
            public CacheHelper getReaderCacheHelper() {
              return null;
            }
          };
      out.accept("FilterLeafReader.toString/over-segment-0", filtered.toString());

      ParallelLeafReader parallel =
          new ParallelLeafReader(false, leaf0, reader.leaves().get(1).reader());
      out.accept("ParallelLeafReader.toString/two-leaves", parallel.toString());
      out.accept(
          "ParallelLeafReader.toString/one-leaf",
          new ParallelLeafReader(false, leaf0).toString());
      out.accept("ParallelLeafReader.toString/no-leaves", new ParallelLeafReader(false).toString());

      CodecReader sorted =
          SortingCodecReader.wrap(
              (CodecReader) leaf0, new Sort(new SortField("price", SortField.Type.LONG)));
      out.accept("SortingCodecReader.toString/over-segment-0", sorted.toString());

      // The classes whose Lucene counterpart declares no toString() of its own:
      // Object's default applies, so what the port has to reproduce is the
      // binary class name and the `name@hex` shape.
      out.accept("FilterCodecReader.class/wrapLiveDocs",
          FilterCodecReader.wrapLiveDocs((CodecReader) leaf0, leaf0.getLiveDocs(), leaf0.numDocs())
              .getClass().getName());
      out.accept("FilterCodecReader.toString/shape",
          FilterCodecReader.wrapLiveDocs((CodecReader) leaf0, leaf0.getLiveDocs(), leaf0.numDocs())
              .toString().replaceAll("@[0-9a-f]+$", "@<identity>"));
      out.accept("SlowCompositeCodecReaderWrapper.toString/shape",
          SlowCompositeCodecReaderWrapper.wrap(
                  java.util.List.of((CodecReader) leaf0, (CodecReader) reader.leaves().get(1).reader()))
              .toString()
              .replaceAll("@[0-9a-f]+$", "@<identity>"));
      out.accept("SlowCodecReaderWrapper.toString/over-parallel",
          SlowCodecReaderWrapper.wrap(parallel).toString());

      try (DirectoryReader exitable =
          new ExitableDirectoryReader(reader, () -> false)) {
        out.accept("ExitableDirectoryReader.toString/two-segments", exitable.toString());
        // FilterDirectoryReader itself does not override toString(): it
        // inherits CompositeReader's, over the *wrapped* leaves.
        out.accept(
            "ExitableFilterAtomicReader.toString/leaf-0",
            exitable.leaves().get(0).reader().toString());
      }
    }
  }

  // -------------------------------------------------------------------------
  // Term and postings enumerations
  // -------------------------------------------------------------------------

  private static void enums(BiConsumer<String, String> out, Directory dir) throws IOException {
    try (DirectoryReader reader = DirectoryReader.open(dir)) {
      Terms terms = MultiTerms.getTerms(reader, "body");
      TermsEnum multiTermsEnum = terms.iterator();
      out.accept("MultiTermsEnum.class/body", multiTermsEnum.getClass().getSimpleName());
      out.accept("MultiTermsEnum.toString/body-unpositioned", multiTermsEnum.toString());
      multiTermsEnum.seekExact(new BytesRef("alpha"));
      out.accept("MultiTermsEnum.toString/body-alpha", multiTermsEnum.toString());

      PostingsEnum postings = MultiTerms.getTermPostingsEnum(reader, "body", new BytesRef("alpha"));
      out.accept("MultiPostingsEnum.class/body-alpha", postings.getClass().getSimpleName());
      out.accept("MultiPostingsEnum.toString/body-alpha", postings.toString());

      LeafReader leaf0 = reader.leaves().get(0).reader();
      TermsEnum leafEnum = leaf0.terms("body").iterator();
      TermsEnumIndex index = new TermsEnumIndex(leafEnum, 0);
      out.accept("TermsEnumIndex.toString/leaf-0-unpositioned", index.toString());
      out.accept("TermsEnumIndex.toString/wraps-its-termsEnum",
          Boolean.toString(index.toString().equals(leafEnum.toString())));
      TermsEnumIndex nullIndex = new TermsEnumIndex(null, 3);
      out.accept("TermsEnumIndex.toString/null-termsEnum", nullIndex.toString());
    }
  }

  // -------------------------------------------------------------------------
  // Pending deletes
  // -------------------------------------------------------------------------

  private static void pendingDeletes(BiConsumer<String, String> out, Directory dir)
      throws IOException {
    SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
    SegmentCommitInfo info = infos.info(0);

    PendingDeletes hard = new PendingDeletes(info);
    out.accept("PendingDeletes.toString/fresh", hard.toString());
    hard.getMutableBits();
    out.accept("PendingDeletes.toString/writeable", hard.toString());
    hard.delete(1);
    out.accept("PendingDeletes.toString/one-deleted", hard.toString());

    PendingSoftDeletes soft = new PendingSoftDeletes("__soft_deletes", info);
    out.accept("PendingSoftDeletes.toString/fresh", soft.toString());

    ReadersAndUpdates rau = new ReadersAndUpdates(Version.LATEST.major, info, new PendingDeletes(info));
    out.accept("ReadersAndUpdates.toString/fresh", rau.toString());

    ReadersAndUpdates softRau =
        new ReadersAndUpdates(
            Version.LATEST.major, info, new PendingSoftDeletes("__soft_deletes", info));
    out.accept("ReadersAndUpdates.toString/soft-deletes", softRau.toString());
  }

  // -------------------------------------------------------------------------
  // Buffered updates
  // -------------------------------------------------------------------------

  private static void bufferedUpdates(BiConsumer<String, String> out, Directory dir)
      throws IOException {
    SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
    SegmentCommitInfo info = infos.info(0);

    BufferedUpdates empty = new BufferedUpdates("_0");
    out.accept("BufferedUpdates.toString/empty", empty.toString());

    BufferedUpdates terms = new BufferedUpdates("_0");
    terms.addTerm(new Term("id", "0-0"), 5);
    terms.addTerm(new Term("id", "0-1"), 7);
    out.accept("BufferedUpdates.toString/two-terms", terms.toString());

    BufferedUpdates queries = new BufferedUpdates("_0");
    queries.addQuery(new MatchAllDocsQuery(), 3);
    out.accept("BufferedUpdates.toString/one-query", queries.toString());

    BufferedUpdates everything = new BufferedUpdates("_0");
    everything.addTerm(new Term("id", "0-0"), 5);
    everything.addQuery(new TermQuery(new Term("id", "0-2")), 9);
    everything.addNumericUpdate(
        new DocValuesUpdate.NumericDocValuesUpdate(new Term("id", "0-1"), "price", 33L), 4);
    everything.gen = 12;
    out.accept("BufferedUpdates.toString/terms-query-and-updates", everything.toString());

    FrozenBufferedUpdates frozenEmpty =
        new FrozenBufferedUpdates(InfoStream.NO_OUTPUT, new BufferedUpdates("_0"), null);
    out.accept("FrozenBufferedUpdates.toString/empty", frozenEmpty.toString());

    BufferedUpdates source = new BufferedUpdates("_0");
    source.addTerm(new Term("id", "0-0"), 5);
    source.addTerm(new Term("id", "0-1"), 7);
    source.addNumericUpdate(
        new DocValuesUpdate.NumericDocValuesUpdate(new Term("id", "0-1"), "price", 33L), 4);
    FrozenBufferedUpdates frozen =
        new FrozenBufferedUpdates(InfoStream.NO_OUTPUT, source, null);
    frozen.setDelGen(7);
    out.accept("FrozenBufferedUpdates.toString/terms-and-updates", frozen.toString());

    BufferedUpdates privateSource = new BufferedUpdates("_0");
    privateSource.addQuery(new MatchAllDocsQuery(), 3);
    FrozenBufferedUpdates privateFrozen =
        new FrozenBufferedUpdates(InfoStream.NO_OUTPUT, privateSource, info);
    out.accept("FrozenBufferedUpdates.toString/private-segment", privateFrozen.toString());

    out.accept(
        "DocumentsWriterDeleteQueue.toString/fresh",
        new DocumentsWriterDeleteQueue(InfoStream.NO_OUTPUT).toString());

    Analyzer analyzer = new StandardAnalyzer();
    IndexWriterConfig config = new IndexWriterConfig(analyzer);
    DocumentsWriterDeleteQueue deleteQueue = new DocumentsWriterDeleteQueue(InfoStream.NO_OUTPUT);
    FieldInfos.FieldNumbers numbers = new FieldInfos.FieldNumbers(null, null);
    DocumentsWriterPerThread dwpt =
        new DocumentsWriterPerThread(
            Version.LATEST.major,
            "_9",
            dir,
            dir,
            config,
            deleteQueue,
            new FieldInfos.Builder(numbers),
            new AtomicLong(),
            false);
    out.accept("DocumentsWriterPerThread.toString/fresh", dwpt.toString());
  }
}
