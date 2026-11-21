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
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.FeatureField;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.BinaryDocValues;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexableField;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.PointValues;
import org.apache.lucene.index.IndexReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.MergePolicy;
import org.apache.lucene.index.MergeTrigger;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.NoMergeScheduler;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.SoftDeletesDirectoryReaderWrapper;
import org.apache.lucene.index.SortedDocValues;
import org.apache.lucene.index.SortedNumericDocValues;
import org.apache.lucene.index.SortedSetDocValues;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.index.TermVectors;
import org.apache.lucene.index.TieredMergePolicy;
import org.apache.lucene.search.BinarySortField;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.FieldDoc;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.search.TopFieldCollector;
import org.apache.lucene.search.TopFieldCollectorManager;
import org.apache.lucene.search.TopFieldDocs;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.util.Bits;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.InfoStream;
import org.apache.lucene.util.StringHelper;
import org.apache.lucene.util.Version;

/**
 * Reference tool for the ten nested types of Sprint 8's last port effort: every case drives a real
 * Apache Lucene Core 10.5.0 through a public path that reaches one of those types and prints what
 * it observes, for {@code tests/nested_types_portability.rs} to reproduce with Rucene.
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... NestedTypesFixture.java &lt;output-dir&gt; &lt;case&gt; [&lt;variant&gt;]
 * </pre>
 *
 * <p>Cases:
 *
 * <ul>
 *   <li>{@code sorted-flush <asc|desc|last|desc-last>}: 600 documents flushed into one segment
 *       under a {@link BinarySortField} index sort ({@code IndexSorter.BinarySorter}); the
 *       postings are written through {@code FreqProxTermsWriter.SortingTerms}. Prints the
 *       segment's name, id and files, and each {@code id} term with the document it landed on.
 *   <li>{@code sorted-private-delete}: the same documents with a segment-private
 *       delete-by-query whose limit is in the pre-sort doc space.
 *   <li>{@code feature-sort}: three segments searched under {@code FeatureField.newFeatureSort}
 *       ({@code FeatureSortField.FeatureComparator}), first page and a {@code searchAfter} page,
 *       with each hit's sort value and populated score as float bits.
 *   <li>{@code merge-score}: {@link TieredMergePolicy#findMerges} over synthetic segments of
 *       chosen sizes and deletions; prints every {@code "  maybe="} line, which carries {@code
 *       TieredMergePolicy.MergeScore}'s score and explanation, then the specification.
 *   <li>{@code dv-update multi-field}: five numeric and one binary doc-values field updated in
 *       one commit, so that each field's generation follows {@code pendingDVUpdates}' iteration
 *       order.
 *   <li>{@code dv-update <single|single-reset|reset-first>}: numeric doc-values updates of one
 *       value ({@code NumericDocValuesFieldUpdates.SingleValueNumericDocValuesFieldUpdates}),
 *       with a reset after or before; prints the segment's id and files after the update commit.
 *   <li>{@code merge-observer <none|pending|wait>}: {@link IndexWriter#forceMergeDeletes(boolean)}
 *       and the {@code MergePolicy.MergeObserver} it returns.
 *   <li>{@code cache-helper}: the {@code FilterDirectoryReader.DelegatingCacheHelper} of {@link
 *       SoftDeletesDirectoryReaderWrapper} and of its wrapped leaf.
 *   <li>{@code sorted-full <numeric|string|binary>}: one segment flushed under a numeric, string
 *       or binary index sort, with norms, stored fields, points, term vectors, every doc-values
 *       type and buffered term deletes; prints the segment's id and files, then one line per
 *       document in doc-id order with everything a reader sees for it.
 *   <li>{@code sorted-merge <numeric|string|binary>}: two such segments merged into one; prints
 *       the same per-document lines.
 * </ul>
 */
public final class NestedTypesFixture {

  private NestedTypesFixture() {}

  static final int SORTED_DOCS = 600;

  /** {@code body}: offsets, no norms. */
  static final FieldType BODY_TYPE = fieldType(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);

  /** {@code freq}: frequencies, no norms. */
  static final FieldType FREQ_TYPE = fieldType(IndexOptions.DOCS_AND_FREQS);

  /** {@code pos}: positions, no norms. */
  static final FieldType POS_TYPE = fieldType(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);

  static FieldType fieldType(IndexOptions options) {
    FieldType type = new FieldType();
    type.setTokenized(true);
    type.setOmitNorms(true);
    type.setIndexOptions(options);
    type.freeze();
    return type;
  }

  /**
   * The binary sort key of document {@code i}: missing, empty, or one byte above or below 128
   * followed by nothing, a zero byte, or {@code 0xFF} and one more byte — so that unsigned order,
   * prefixes, empty-versus-missing and ties are all exercised.
   */
  static byte[] key(int i) {
    if (i % 7 == 3) {
      return null;
    }
    if (i % 11 == 5) {
      return new byte[0];
    }
    byte a = (byte) ((i * 131) % 256);
    return switch (i % 3) {
      case 0 -> new byte[] {a};
      case 1 -> new byte[] {a, 0};
      default -> new byte[] {a, (byte) 0xFF, (byte) (i % 2)};
    };
  }

  static Document sortedDoc(int i) {
    Document d = new Document();
    d.add(new StringField("id", String.format("id%04d", i), Field.Store.NO));
    byte[] key = key(i);
    if (key != null) {
      d.add(new BinaryDocValuesField("key", new BytesRef(key)));
    }
    d.add(new StringField("tag", "t" + (i % 17), Field.Store.NO));
    StringBuilder body = new StringBuilder("all w").append(i % 31).append(" x").append(i);
    for (int r = 0; r <= i % 4; r++) {
      body.append(" rep");
    }
    d.add(new Field("body", body.toString(), BODY_TYPE));
    d.add(new Field("freq", "f" + (i % 5) + " f" + (i % 5) + " g" + (i % 9), FREQ_TYPE));
    d.add(new Field("pos", "p" + (i % 3) + " q p" + (i % 3), POS_TYPE));
    return d;
  }

  static SortField binarySortField(String variant) {
    return switch (variant) {
      case "asc" -> new BinarySortField("key", false);
      case "desc" -> new BinarySortField("key", true);
      case "last" -> new BinarySortField("key", false, SortField.STRING_LAST);
      case "desc-last" -> new BinarySortField("key", true, SortField.STRING_LAST);
      default -> throw new IllegalArgumentException("unknown variant: " + variant);
    };
  }

  static void printSegment(Path out, SegmentCommitInfo info) throws IOException {
    System.out.println("name=" + info.info.name);
    System.out.println("id=" + HexFormat.of().formatHex(info.info.getId()));
    for (String file : new TreeSet<>(info.files())) {
      System.out.println("file " + file + " " + Files.size(out.resolve(file)));
    }
  }

  static void sortedFlush(Path out, String variant) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig iwc = new IndexWriterConfig();
      iwc.setUseCompoundFile(false);
      iwc.setMergePolicy(NoMergePolicy.INSTANCE);
      iwc.setIndexSort(new Sort(binarySortField(variant)));
      try (IndexWriter writer = new IndexWriter(dir, iwc)) {
        for (int i = 0; i < SORTED_DOCS; i++) {
          writer.addDocument(sortedDoc(i));
        }
        writer.commit();
      }
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      System.out.println("segments=" + infos.size());
      System.out.println("indexSort=" + infos.info(0).info.getIndexSort());
      printSegment(out, infos.info(0));
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        LeafReader leaf = reader.leaves().get(0).reader();
        TermsEnum terms = leaf.terms("id").iterator();
        TreeMap<Integer, String> order = new TreeMap<>();
        PostingsEnum postings = null;
        for (BytesRef term = terms.next(); term != null; term = terms.next()) {
          postings = terms.postings(postings, PostingsEnum.NONE);
          order.put(postings.nextDoc(), term.utf8ToString());
        }
        for (var entry : order.entrySet()) {
          System.out.println("doc " + entry.getKey() + " " + entry.getValue());
        }
      }
    }
  }

  /**
   * {@code sorted-private-delete}: the {@code sorted-flush} documents under the ascending binary
   * sort, with a delete-by-query issued after document 40 is added: it is private to the
   * in-memory segment and limited to the 41 documents added before it, a limit in the pre-sort
   * doc space ({@code FrozenBufferedUpdates.applyQueryDeletes} with {@code rld.sortMap}).
   */
  static void sortedPrivateDelete(Path out) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig iwc = new IndexWriterConfig();
      iwc.setUseCompoundFile(false);
      iwc.setMergePolicy(NoMergePolicy.INSTANCE);
      iwc.setIndexSort(new Sort(binarySortField("asc")));
      try (IndexWriter writer = new IndexWriter(dir, iwc)) {
        for (int i = 0; i < 120; i++) {
          writer.addDocument(sortedDoc(i));
          if (i == 40) {
            writer.deleteDocuments(
                new org.apache.lucene.search.TermQuery(new Term("tag", "t3")));
          }
        }
        writer.commit();
      }
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        LeafReader leaf = reader.leaves().get(0).reader();
        var liveDocs = leaf.getLiveDocs();
        TermsEnum terms = leaf.terms("id").iterator();
        TreeMap<Integer, String> order = new TreeMap<>();
        PostingsEnum postings = null;
        for (BytesRef term = terms.next(); term != null; term = terms.next()) {
          postings = terms.postings(postings, PostingsEnum.NONE);
          int doc = postings.nextDoc();
          boolean live = liveDocs == null || liveDocs.get(doc);
          order.put(doc, term.utf8ToString() + (live ? " live" : " deleted"));
        }
        System.out.println("numDocs=" + leaf.numDocs());
        for (var entry : order.entrySet()) {
          System.out.println("doc " + entry.getKey() + " " + entry.getValue());
        }
      }
    }
  }

  /** The feature value of document {@code i}, or 0 when it has none. */
  static float featureValue(int i) {
    float[] values = {1.5f, 0.25f, 8f, 0f, 8f, 3.75f, 100f, 0.001f, 8.000001f, 42f};
    return values[i % values.length];
  }

  static Document featureDoc(int i, int segment) {
    Document d = new Document();
    d.add(new StringField("id", "f" + i, Field.Store.NO));
    if (segment == 0) {
      if (i == 3) {
        d.add(new FeatureField("features", "other", 2f));
      } else {
        d.add(new FeatureField("features", "pagerank", featureValue(i)));
      }
    } else if (segment == 2) {
      d.add(new FeatureField("features", "other", 1f + i));
      if (i % 3 == 0) {
        d.add(new FeatureField("features", "pagerank", 2.5f + i));
      }
    }
    return d;
  }

  static void printHits(String page, TopFieldDocs top) {
    for (ScoreDoc hit : top.scoreDocs) {
      FieldDoc fd = (FieldDoc) hit;
      System.out.println(
          page
              + " doc="
              + fd.doc
              + " value="
              + Integer.toHexString(Float.floatToRawIntBits((Float) fd.fields[0]))
              + " score="
              + Integer.toHexString(Float.floatToRawIntBits(fd.score)));
    }
  }

  static void featureSort(Path out) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig iwc = new IndexWriterConfig();
      iwc.setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(dir, iwc)) {
        int i = 0;
        int[] sizes = {10, 5, 7};
        for (int segment = 0; segment < sizes.length; segment++) {
          for (int k = 0; k < sizes[segment]; k++, i++) {
            writer.addDocument(featureDoc(i, segment));
          }
          writer.commit();
        }
      }
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        System.out.println("leaves=" + reader.leaves().size());
        IndexSearcher searcher = new IndexSearcher(reader);
        SortField sortField = FeatureField.newFeatureSort("features", "pagerank");
        System.out.println("sortField=" + sortField);
        Sort sort = new Sort(sortField);
        Query query = MatchAllDocsQuery.INSTANCE;
        TopFieldDocs first =
            searcher.search(query, new TopFieldCollectorManager(sort, 6, null, Integer.MAX_VALUE));
        TopFieldCollector.populateScores(first.scoreDocs, searcher, query);
        printHits("first", first);
        FieldDoc after = (FieldDoc) first.scoreDocs[first.scoreDocs.length - 1];
        TopFieldDocs second =
            searcher.search(
                query, new TopFieldCollectorManager(sort, 30, after, Integer.MAX_VALUE));
        TopFieldCollector.populateScores(second.scoreDocs, searcher, query);
        printHits("second", second);
      }
    }
  }

  /** Synthetic segments for {@code merge-score}: maxDoc, delCount and byte size. */
  static final int[][] SCORE_SEGMENTS = {
    {300, 0, 30000},
    {280, 28, 28000},
    {250, 0, 25000},
    {90, 9, 9000},
    {80, 0, 8000},
    {75, 30, 7500},
    {70, 21, 7000},
    {30, 0, 3000},
    {29, 3, 2900},
    {28, 0, 2800},
    {12, 6, 1200},
    {11, 0, 1100},
  };

  static void mergeScore() throws IOException {
    Directory dir = new ByteBuffersDirectory();
    SegmentInfos infos = new SegmentInfos(Version.LATEST.major);
    for (int k = 0; k < SCORE_SEGMENTS.length; k++) {
      int[] seg = SCORE_SEGMENTS[k];
      String name = "_" + Integer.toString(k, Character.MAX_RADIX);
      SegmentInfo si =
          new SegmentInfo(
              dir,
              Version.LATEST,
              Version.LATEST,
              name,
              seg[0],
              false,
              false,
              Codec.getDefault(),
              Map.of(),
              StringHelper.randomId(),
              Map.of(),
              null);
      try (IndexOutput output = dir.createOutput(name + ".bin", IOContext.DEFAULT)) {
        output.writeBytes(new byte[seg[2]], seg[2]);
      }
      si.setFiles(Set.of(name + ".bin"));
      infos.add(new SegmentCommitInfo(si, seg[1], 0, -1, -1, -1, StringHelper.randomId()));
    }
    TieredMergePolicy tmp = new TieredMergePolicy();
    tmp.setFloorSegmentMB(0.002);
    tmp.setMaxMergedSegmentMB(0.06);
    tmp.setSegmentsPerTier(3);
    tmp.setMaxMergeAtOnce(4);
    List<String> lines = new ArrayList<>();
    InfoStream infoStream =
        new InfoStream() {
          @Override
          public void message(String component, String message) {
            if ("MP".equals(component)) {
              lines.add(message);
            }
          }

          @Override
          public boolean isEnabled(String component) {
            return "MP".equals(component);
          }

          @Override
          public void close() {}
        };
    MergePolicy.MergeContext context =
        new MergePolicy.MergeContext() {
          @Override
          public int numDeletesToMerge(SegmentCommitInfo info) {
            return info.getDelCount();
          }

          @Override
          public int numDeletedDocs(SegmentCommitInfo info) {
            return info.getDelCount();
          }

          @Override
          public InfoStream getInfoStream() {
            return infoStream;
          }

          @Override
          public Set<SegmentCommitInfo> getMergingSegments() {
            return Set.of();
          }
        };
    MergePolicy.MergeSpecification spec = tmp.findMerges(MergeTrigger.EXPLICIT, infos, context);
    for (String line : lines) {
      if (line.startsWith("  maybe=")) {
        System.out.println("maybe " + line);
      }
    }
    System.out.println("spec " + (spec == null ? "null" : spec.toString().replace("\n", "|")));
  }

  /** The numeric fields of {@code dv-update multi-field}, in the order they are updated. */
  static final String[] MULTI_NUMERIC = {"weight", "views", "stock", "rating", "price"};

  static void dvUpdateMultiField(Path out) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig iwc = new IndexWriterConfig();
      iwc.setUseCompoundFile(false);
      iwc.setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(dir, iwc)) {
        for (int i = 0; i < 50; i++) {
          Document d = new Document();
          d.add(new StringField("id", "d" + i, Field.Store.NO));
          d.add(new StringField("group", "g" + (i % 5), Field.Store.NO));
          for (int f = 0; f < MULTI_NUMERIC.length; f++) {
            d.add(new NumericDocValuesField(MULTI_NUMERIC[f], i * (f + 2L)));
          }
          d.add(new BinaryDocValuesField("label", new BytesRef("l" + i)));
          writer.addDocument(d);
        }
        writer.commit();
        for (int f = 0; f < MULTI_NUMERIC.length; f++) {
          writer.updateNumericDocValue(new Term("group", "g" + f), MULTI_NUMERIC[f], 100L + f);
        }
        writer.updateBinaryDocValue(new Term("group", "g2"), "label", new BytesRef("updated"));
        writer.commit();
      }
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      SegmentCommitInfo info = infos.info(0);
      System.out.println("dvGen=" + info.getDocValuesGen());
      printSegment(out, info);
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        LeafReader leaf = reader.leaves().get(0).reader();
        for (FieldInfo fi : leaf.getFieldInfos()) {
          System.out.println("gen " + fi.name + " " + fi.getDocValuesGen());
        }
        for (String field : MULTI_NUMERIC) {
          var values = leaf.getNumericDocValues(field);
          StringBuilder b = new StringBuilder("values " + field);
          for (int doc = values.nextDoc();
              doc != DocIdSetIterator.NO_MORE_DOCS;
              doc = values.nextDoc()) {
            b.append(' ').append(doc).append('=').append(values.longValue());
          }
          System.out.println(b);
        }
        BinaryDocValues labels = leaf.getBinaryDocValues("label");
        StringBuilder b = new StringBuilder("values label");
        for (int doc = labels.nextDoc();
            doc != DocIdSetIterator.NO_MORE_DOCS;
            doc = labels.nextDoc()) {
          b.append(' ').append(doc).append('=').append(labels.binaryValue().utf8ToString());
        }
        System.out.println(b);
      }
    }
  }

  static void dvUpdate(Path out, String variant) throws IOException {
    if (variant.equals("multi-field")) {
      dvUpdateMultiField(out);
      return;
    }
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig iwc = new IndexWriterConfig();
      iwc.setUseCompoundFile(false);
      iwc.setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(dir, iwc)) {
        for (int i = 0; i < 50; i++) {
          Document d = new Document();
          d.add(new StringField("id", "d" + i, Field.Store.NO));
          d.add(new StringField("group", "g" + (i % 5), Field.Store.NO));
          d.add(new NumericDocValuesField("price", i * 3L));
          writer.addDocument(d);
        }
        writer.commit();
        switch (variant) {
          case "single" -> {
            writer.updateNumericDocValue(new Term("group", "g1"), "price", 42L);
            writer.updateNumericDocValue(new Term("group", "g3"), "price", 42L);
          }
          case "single-reset" -> {
            writer.updateNumericDocValue(new Term("group", "g1"), "price", 42L);
            writer.updateDocValues(
                new Term("group", "g3"), new NumericDocValuesField("price", (Long) null));
          }
          case "reset-first" -> {
            writer.updateDocValues(
                new Term("group", "g1"), new NumericDocValuesField("price", (Long) null));
            writer.updateNumericDocValue(new Term("group", "g3"), "price", 42L);
          }
          default -> throw new IllegalArgumentException("unknown variant: " + variant);
        }
        writer.commit();
      }
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      SegmentCommitInfo info = infos.info(0);
      System.out.println("dvGen=" + info.getDocValuesGen());
      printSegment(out, info);
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        LeafReader leaf = reader.leaves().get(0).reader();
        var values = leaf.getNumericDocValues("price");
        StringBuilder b = new StringBuilder("values");
        for (int doc = values.nextDoc();
            doc != DocIdSetIterator.NO_MORE_DOCS;
            doc = values.nextDoc()) {
          b.append(' ').append(doc).append('=').append(values.longValue());
        }
        System.out.println(b);
      }
    }
  }

  static final int FULL_DOCS = 300;

  /** {@code tv}: term vectors with positions and offsets. */
  static final FieldType TV_TYPE = termVectorType();

  static FieldType termVectorType() {
    FieldType type = new FieldType(TextField.TYPE_NOT_STORED);
    type.setStoreTermVectors(true);
    type.setStoreTermVectorPositions(true);
    type.setStoreTermVectorOffsets(true);
    type.freeze();
    return type;
  }

  /** Document {@code i} of {@code sorted-full}: every kind of data a flush writes. */
  static Document fullDoc(int i) {
    Document d = new Document();
    d.add(new StringField("id", String.format("id%04d", i), Field.Store.YES));
    d.add(new StringField("tag", "t" + (i % 17), Field.Store.NO));
    StringBuilder body = new StringBuilder("all w").append(i % 31);
    for (int r = 0; r <= (i * 7) % 9; r++) {
      body.append(" rep").append(r % 3);
    }
    d.add(new TextField("body", body.toString(), Field.Store.NO));
    if (i % 4 != 1) {
      d.add(new StoredField("sv", "value-" + (i * 13 % 97)));
    }
    d.add(new StoredField("sn", i * 3));
    d.add(new IntPoint("ip", (i * 37) % 101));
    d.add(new IntPoint("p2", i % 23, (i * 11) % 29));
    if (i % 3 != 2) {
      d.add(new Field("tv", "tv" + (i % 5) + " common tv" + (i % 5), TV_TYPE));
    }
    if (i % 13 != 4) {
      d.add(new NumericDocValuesField("rank", (i * 37) % 101));
    }
    if (i % 7 != 3) {
      d.add(new SortedDocValuesField("skey", new BytesRef("s" + ((i * 53) % 71))));
    }
    byte[] key = key(i);
    if (key != null) {
      d.add(new BinaryDocValuesField("key", new BytesRef(key)));
    }
    for (int v = 0; v < i % 3; v++) {
      d.add(new SortedNumericDocValuesField("snum", (i * (v + 5)) % 19 - 7));
    }
    for (int v = 0; v < i % 4; v++) {
      d.add(new SortedSetDocValuesField("sset", new BytesRef("ss" + ((i + v * 7) % 11))));
    }
    return d;
  }

  static SortField fullSortField(String variant) {
    return switch (variant) {
      case "numeric" -> new SortField("rank", SortField.Type.LONG);
      case "string" -> new SortField("skey", SortField.Type.STRING);
      case "binary" -> new BinarySortField("key", false);
      default -> throw new IllegalArgumentException("unknown variant: " + variant);
    };
  }

  /** Adds documents {@code from} to {@code to}, with two buffered term deletes among them. */
  static void addFullDocs(IndexWriter writer, int from, int to) throws IOException {
    for (int i = from; i < to; i++) {
      writer.addDocument(fullDoc(i));
      if (i == from + (to - from) / 2) {
        // Limited to the documents added so far.
        writer.deleteDocuments(new Term("tag", "t3"));
      }
    }
    writer.deleteDocuments(new Term("tag", "t5"));
  }

  /** The points of {@code field}, by document. */
  static TreeMap<Integer, String> points(LeafReader leaf, String field) throws IOException {
    TreeMap<Integer, String> byDoc = new TreeMap<>();
    PointValues values = leaf.getPointValues(field);
    int dims = values.getNumDimensions();
    values.intersect(
        new PointValues.IntersectVisitor() {
          @Override
          public void visit(int docID) {
            throw new AssertionError("every cell crosses");
          }

          @Override
          public void visit(int docID, byte[] packedValue) {
            StringBuilder b = new StringBuilder();
            for (int dim = 0; dim < dims; dim++) {
              if (dim > 0) {
                b.append(',');
              }
              b.append(IntPoint.decodeDimension(packedValue, dim * Integer.BYTES));
            }
            byDoc.merge(docID, b.toString(), (a, c) -> a + ";" + c);
          }

          @Override
          public PointValues.Relation compare(byte[] minPackedValue, byte[] maxPackedValue) {
            return PointValues.Relation.CELL_CROSSES_QUERY;
          }
        });
    return byDoc;
  }

  /** One line per document: everything a reader sees for it, in doc-id order. */
  static void printDocuments(LeafReader leaf) throws IOException {
    TreeMap<Integer, String> ids = new TreeMap<>();
    TermsEnum terms = leaf.terms("id").iterator();
    PostingsEnum postings = null;
    for (BytesRef term = terms.next(); term != null; term = terms.next()) {
      postings = terms.postings(postings, PostingsEnum.NONE);
      for (int doc = postings.nextDoc();
          doc != DocIdSetIterator.NO_MORE_DOCS;
          doc = postings.nextDoc()) {
        ids.merge(doc, term.utf8ToString(), (a, c) -> a + ";" + c);
      }
    }
    TreeMap<Integer, String> ip = points(leaf, "ip");
    TreeMap<Integer, String> p2 = points(leaf, "p2");
    Bits live = leaf.getLiveDocs();
    StoredFields stored = leaf.storedFields();
    TermVectors vectors = leaf.termVectors();
    NumericDocValues rank = leaf.getNumericDocValues("rank");
    SortedDocValues skey = leaf.getSortedDocValues("skey");
    BinaryDocValues key = leaf.getBinaryDocValues("key");
    SortedNumericDocValues snum = leaf.getSortedNumericDocValues("snum");
    SortedSetDocValues sset = leaf.getSortedSetDocValues("sset");
    NumericDocValues norms = leaf.getNormValues("body");
    for (int doc = 0; doc < leaf.maxDoc(); doc++) {
      StringBuilder b = new StringBuilder("doc ").append(doc);
      b.append(" live=").append(live == null || live.get(doc));
      b.append(" id=").append(ids.getOrDefault(doc, "-"));
      b.append(" stored=[");
      boolean first = true;
      for (IndexableField f : stored.document(doc)) {
        if (first == false) {
          b.append(',');
        }
        first = false;
        b.append(f.name()).append('=');
        b.append(f.numericValue() != null ? f.numericValue() : f.stringValue());
      }
      b.append(']');
      b.append(" rank=").append(rank.advanceExact(doc) ? Long.toString(rank.longValue()) : "-");
      b.append(" skey=")
          .append(skey.advanceExact(doc) ? skey.lookupOrd(skey.ordValue()).utf8ToString() : "-");
      b.append(" key=")
          .append(
              key.advanceExact(doc)
                  ? HexFormat.of().formatHex(BytesRef.deepCopyOf(key.binaryValue()).bytes)
                  : "-");
      b.append(" snum=[");
      if (snum.advanceExact(doc)) {
        for (int v = 0; v < snum.docValueCount(); v++) {
          if (v > 0) {
            b.append(',');
          }
          b.append(snum.nextValue());
        }
      }
      b.append(']');
      b.append(" sset=[");
      if (sset.advanceExact(doc)) {
        for (int v = 0; v < sset.docValueCount(); v++) {
          if (v > 0) {
            b.append(',');
          }
          b.append(sset.lookupOrd(sset.nextOrd()).utf8ToString());
        }
      }
      b.append(']');
      b.append(" norm=").append(norms.advanceExact(doc) ? Long.toString(norms.longValue()) : "-");
      b.append(" ip=").append(ip.getOrDefault(doc, "-"));
      b.append(" p2=").append(p2.getOrDefault(doc, "-"));
      b.append(" tv=[");
      Terms tv = vectors.get(doc, "tv");
      if (tv != null) {
        TermsEnum te = tv.iterator();
        boolean firstTerm = true;
        for (BytesRef term = te.next(); term != null; term = te.next()) {
          if (firstTerm == false) {
            b.append(',');
          }
          firstTerm = false;
          PostingsEnum pe = te.postings(null, PostingsEnum.ALL);
          pe.nextDoc();
          b.append(term.utf8ToString()).append(':').append(pe.freq());
          for (int k = 0; k < pe.freq(); k++) {
            int position = pe.nextPosition();
            b.append('@')
                .append(position)
                .append('[')
                .append(pe.startOffset())
                .append('-')
                .append(pe.endOffset())
                .append(']');
          }
        }
      }
      b.append(']');
      System.out.println(b);
    }
  }

  static void sortedFull(Path out, String variant, boolean merge) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig iwc = new IndexWriterConfig();
      iwc.setUseCompoundFile(false);
      iwc.setMergePolicy(merge ? new TieredMergePolicy() : NoMergePolicy.INSTANCE);
      iwc.setIndexSort(new Sort(fullSortField(variant)));
      try (IndexWriter writer = new IndexWriter(dir, iwc)) {
        if (merge) {
          addFullDocs(writer, 0, FULL_DOCS / 2);
          writer.commit();
          addFullDocs(writer, FULL_DOCS / 2, FULL_DOCS);
          writer.commit();
          writer.forceMerge(1);
        } else {
          addFullDocs(writer, 0, FULL_DOCS);
        }
        writer.commit();
      }
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      System.out.println("segments=" + infos.size());
      System.out.println("indexSort=" + infos.info(0).info.getIndexSort());
      System.out.println("delCount=" + infos.info(0).getDelCount());
      printSegment(out, infos.info(0));
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        printDocuments(reader.leaves().get(0).reader());
      }
    }
  }

  static void mergeObserver(Path out, String variant) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig iwc = new IndexWriterConfig();
      iwc.setMergeScheduler(
          variant.equals("pending") ? NoMergeScheduler.INSTANCE : new SerialMergeScheduler());
      try (IndexWriter writer = new IndexWriter(dir, iwc)) {
        for (int segment = 0; segment < 2; segment++) {
          for (int k = 0; k < 20; k++) {
            Document d = new Document();
            d.add(new StringField("id", "s" + segment + "d" + k, Field.Store.NO));
            // 15% and 10% deletions: above the 10% `forceMergeDeletesPctAllowed`
            // for the first segment only, and below the 20% `deletesPctAllowed`
            // that would trigger a natural merge first.
            boolean deleted = variant.equals("none") == false && k < 3 - segment;
            d.add(new StringField("del", deleted ? "yes" : "no", Field.Store.NO));
            writer.addDocument(d);
          }
          writer.commit();
        }
        writer.deleteDocuments(new Term("del", "yes"));
        MergePolicy.MergeObserver observer = writer.forceMergeDeletes(variant.equals("wait"));
        System.out.println("numMerges=" + observer.numMerges());
        System.out.println("numCompletedMerges=" + observer.numCompletedMerges());
        System.out.println("awaitTimeout=" + observer.await(10, TimeUnit.MILLISECONDS));
        System.out.println("awaitAsyncDone=" + observer.awaitAsync().isDone());
        if (variant.equals("wait")) {
          System.out.println("await=" + observer.await());
        }
        // After the wait, each merge renders the merged segment's name too.
        System.out.println("toString=" + observer.toString().replace("\n", "|"));
        writer.rollback();
      }
    }
  }

  static void cacheHelper(Path out) throws IOException {
    try (FSDirectory dir = FSDirectory.open(out)) {
      IndexWriterConfig iwc = new IndexWriterConfig();
      iwc.setSoftDeletesField("soft");
      iwc.setMergePolicy(NoMergePolicy.INSTANCE);
      try (IndexWriter writer = new IndexWriter(dir, iwc)) {
        // Document c3 carries the soft-deletes field, so the first segment has
        // one soft deletion; the second segment has none.
        for (int i = 0; i < 10; i++) {
          Document d = new Document();
          d.add(new StringField("id", "c" + i, Field.Store.NO));
          if (i == 3) {
            d.add(new NumericDocValuesField("soft", 1));
          }
          writer.addDocument(d);
        }
        writer.commit();
        Document last = new Document();
        last.add(new StringField("id", "c10", Field.Store.NO));
        writer.addDocument(last);
        writer.commit();
      }
      DirectoryReader in = DirectoryReader.open(dir);
      DirectoryReader wrapped = new SoftDeletesDirectoryReaderWrapper(in, "soft");
      IndexReader.CacheKey inKey = in.getReaderCacheHelper().getKey();
      IndexReader.CacheKey wrappedKey = wrapped.getReaderCacheHelper().getKey();
      System.out.println("dirDistinct=" + (inKey != wrappedKey));
      System.out.println("dirStable=" + (wrapped.getReaderCacheHelper().getKey() == wrappedKey));
      for (int i = 0; i < wrapped.leaves().size(); i++) {
        LeafReaderContext w = wrapped.leaves().get(i);
        LeafReaderContext u = in.leaves().get(i);
        IndexReader.CacheKey wk = w.reader().getReaderCacheHelper().getKey();
        IndexReader.CacheKey uk = u.reader().getReaderCacheHelper().getKey();
        System.out.println(
            "leaf "
                + i
                + " numDocs="
                + w.reader().numDocs()
                + " readerDistinct="
                + (wk != uk)
                + " readerStable="
                + (w.reader().getReaderCacheHelper().getKey() == wk)
                + " coreShared="
                + (w.reader().getCoreCacheHelper().getKey()
                    == u.reader().getCoreCacheHelper().getKey()));
      }
      AtomicInteger dirCalls = new AtomicInteger();
      AtomicInteger dirKeyMatches = new AtomicInteger();
      wrapped
          .getReaderCacheHelper()
          .addClosedListener(
              key -> {
                dirCalls.incrementAndGet();
                if (key == wrappedKey) {
                  dirKeyMatches.incrementAndGet();
                }
              });
      AtomicInteger leafCalls = new AtomicInteger();
      wrapped
          .leaves()
          .get(0)
          .reader()
          .getReaderCacheHelper()
          .addClosedListener(key -> leafCalls.incrementAndGet());
      wrapped.close();
      System.out.println("dirListenerCalls=" + dirCalls.get());
      System.out.println("dirListenerKeyIsWrapper=" + (dirKeyMatches.get() == dirCalls.get()));
      System.out.println("leafListenerCalls=" + leafCalls.get());
    }
  }

  /** Runs one case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 2) {
      throw new IllegalArgumentException(
          "usage: NestedTypesFixture <output-dir> <case> [<variant>]");
    }
    Path out = Paths.get(args[0]);
    String kase = args[1];
    String variant = args.length > 2 ? args[2] : "";
    Files.createDirectories(out);
    try (var listing = Files.list(out)) {
      for (Path file : listing.toList()) {
        Files.delete(file);
      }
    }
    System.out.println("version=" + Version.LATEST);
    switch (kase) {
      case "sorted-flush" -> sortedFlush(out, variant);
      case "sorted-private-delete" -> sortedPrivateDelete(out);
      case "feature-sort" -> featureSort(out);
      case "merge-score" -> mergeScore();
      case "dv-update" -> dvUpdate(out, variant);
      case "merge-observer" -> mergeObserver(out, variant);
      case "cache-helper" -> cacheHelper(out);
      case "sorted-full" -> sortedFull(out, variant, false);
      case "sorted-merge" -> sortedFull(out, variant, true);
      default -> throw new IllegalArgumentException("unknown case: " + kase);
    }
  }
}
