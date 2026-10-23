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
import java.util.HexFormat;
import java.util.List;
import java.util.TreeSet;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.LongPoint;
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
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.Term;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Reference tool for the merge of postings, points, doc values and deletions:
 * the documents of one case are written as committed segments by a real Apache
 * Lucene Core 10.5.0 {@link IndexWriter} with the default codec, optionally
 * some are deleted, and the index is force-merged into one segment. The index
 * is left in the output directory; the tool prints the merged segment's name,
 * id and document counts, and the name and length of every file it is made
 * of.
 *
 * <p>This is the output Rucene's {@code FieldsConsumer.merge}, {@code
 * PointsWriter.merge}, {@code PerFieldMergeState.restrictFields} and {@code
 * PendingDeletes.getLiveDocs} are judged against ({@code
 * tests/index_writer_merge_fields.rs}), which builds the same documents with
 * {@link #doc(int, int, int)}'s Rust twin and compares every merged file byte
 * for byte.
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... MergedFieldsFixture &lt;output-dir&gt; &lt;case&gt;
 * </pre>
 *
 * <p>Cases, each chosen at a boundary of the formats it exercises:
 *
 * <ul>
 *   <li>{@code postings}: two segments of 4200 documents, so the merged
 *       segment's most common term is in more than {@code LEVEL1_NUM_DOCS}
 *       (8192) documents, every {@code id} is a distinct term of a block tree
 *       well past {@code maxItemsInBlock} (48), and a field indexes offsets.
 *   <li>{@code points}: two segments of 700 documents, more points than one
 *       leaf holds (512), in one- and two-dimensional fields, one of them
 *       multi-valued and one sparse.
 *   <li>{@code points-default}: the {@code points} documents plus a third
 *       segment with no point field, whose null points reader sends {@code
 *       Lucene90PointsWriter.merge} down the default {@code PointsWriter.merge}.
 *   <li>{@code docvalues}: two segments of 9000 documents, a sparse field,
 *       sorted terms past one LZ4 block (64) and sorted-set terms past one
 *       reverse-index interval (1024). Field {@code n} holds 18000 values,
 *       more than one numeric block (16384), but per-block widths would not
 *       save the 10% Lucene requires, so Lucene writes it as one block: the
 *       multi-block numeric encoding is not exercised.
 *   <li>{@code deletes}: two segments of 500 documents and a third of 50, one
 *       document in six of the first two and every document of the third
 *       deleted, and an indexed field, a point field and a doc-values field
 *       of every type that only deleted documents have: the merged segment
 *       keeps their field infos and no terms, point or value. The third segment is dropped when its deletions are
 *       committed; the other two keep fewer deletions than {@code
 *       TieredMergePolicy}'s default {@code deletesPctAllowed} (20%), so no
 *       natural merge runs before {@code forceMerge}.
 *   <li>{@code combined}: every field above, in two segments of 6500 documents
 *       and a third of 50, with the deletions of {@code deletes}.
 * </ul>
 */
public final class MergedFieldsFixture {

  private MergedFieldsFixture() {}

  /** Field type of {@code off}: a tokenized field that indexes offsets. */
  static final FieldType OFFSETS_TYPE = new FieldType();

  static {
    OFFSETS_TYPE.setTokenized(true);
    OFFSETS_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    OFFSETS_TYPE.freeze();
  }

  static final int POSTINGS = 1;
  static final int POINTS = 2;
  static final int DOC_VALUES = 4;
  static final int DELETES = 8;

  /**
   * Document {@code i} of segment {@code seg}, with the fields of {@code
   * features}. Integer arithmetic only, so a port reproduces every value.
   */
  static Document doc(int features, int seg, int i) {
    Document d = new Document();
    if ((features & POSTINGS) != 0) {
      d.add(new StringField("id", String.format("id%05d", i), Field.Store.NO));
      StringBuilder body = new StringBuilder("all m").append(i % 3);
      body.append(" w").append(i % 97).append(" x").append(i);
      for (int r = 0; r <= i % 5; r++) {
        body.append(" rep");
      }
      d.add(new TextField("body", body.toString(), Field.Store.NO));
      d.add(new StringField("tag", "t" + (i % 50), Field.Store.NO));
      d.add(new Field("off", "alpha beta" + (i % 13) + " alpha", OFFSETS_TYPE));
    }
    if ((features & POINTS) != 0 && seg < 2) {
      d.add(new IntPoint("i1", ((i * 7919) % 2003) - 1000));
      d.add(new LongPoint("l1", ((long) i * 1000000007L) % 4000000000L - 2000000000L));
      d.add(new IntPoint("p2", (i * 31) % 257, (i * 17) % 263));
      if (i % 2 == 0) {
        d.add(new IntPoint("p2", (i * 13) % 251, (i * 7) % 241));
      }
      if (i % 10 == 0) {
        d.add(new IntPoint("sparse", i));
      }
    }
    if ((features & DOC_VALUES) != 0) {
      long n = seg == 0 ? i % 8 : ((long) i * 1000003L) % 1000000000L;
      d.add(new NumericDocValuesField("n", n));
      if (i % 7 == 0) {
        d.add(new NumericDocValuesField("nsparse", i * 3L));
      }
      d.add(new BinaryDocValuesField("b", new BytesRef("b" + i)));
      d.add(new SortedDocValuesField("s", new BytesRef("s" + (i % 300))));
      d.add(new SortedSetDocValuesField("ss", new BytesRef("a" + (i % 40))));
      d.add(new SortedSetDocValuesField("ss", new BytesRef("b" + (i % 1100))));
      d.add(new SortedNumericDocValuesField("sn", i % 5));
      d.add(new SortedNumericDocValuesField("sn", -i));
    }
    if ((features & DELETES) != 0) {
      boolean deleted = i % 6 == 0 || seg == 2;
      d.add(new StringField("del", deleted ? "yes" : "no", Field.Store.NO));
      d.add(new StoredField("sv", "v" + i));
      if (deleted && seg < 2) {
        d.add(new StringField("qt", "qt" + (i % 4), Field.Store.NO));
        d.add(new IntPoint("q", i));
        d.add(new NumericDocValuesField("qdv", i));
        d.add(new BinaryDocValuesField("qb", new BytesRef("qb" + i)));
        d.add(new SortedDocValuesField("qs", new BytesRef("qs" + i)));
        d.add(new SortedSetDocValuesField("qss", new BytesRef("qss" + i)));
        d.add(new SortedNumericDocValuesField("qsn", i));
      }
    }
    if (features == POINTS && seg == 2) {
      d.add(new StringField("id", String.format("id%05d", i), Field.Store.NO));
    }
    return d;
  }

  /** The features of {@code kase}. */
  static int features(String kase) {
    return switch (kase) {
      case "postings" -> POSTINGS;
      case "points", "points-default" -> POINTS;
      case "docvalues" -> DOC_VALUES;
      case "deletes" -> POSTINGS | DELETES;
      case "combined" -> POSTINGS | POINTS | DOC_VALUES | DELETES;
      default -> throw new IllegalArgumentException("unknown case: " + kase);
    };
  }

  /** The number of documents of each segment of {@code kase}. */
  static int[] segmentSizes(String kase) {
    return switch (kase) {
      case "postings" -> new int[] {4200, 4200};
      case "points" -> new int[] {700, 700};
      case "points-default" -> new int[] {700, 700, 50};
      case "docvalues" -> new int[] {9000, 9000};
      case "deletes" -> new int[] {500, 500, 50};
      case "combined" -> new int[] {6500, 6500, 50};
      default -> throw new IllegalArgumentException("unknown case: " + kase);
    };
  }

  /** Runs one case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 2) {
      throw new IllegalArgumentException("usage: MergedFieldsFixture <output-dir> <case>");
    }
    Path out = Paths.get(args[0]);
    String kase = args[1];
    int features = features(kase);
    int[] sizes = segmentSizes(kase);
    Files.createDirectories(out);
    try (var listing = Files.list(out)) {
      for (Path file : listing.toList()) {
        Files.delete(file);
      }
    }
    System.out.println("version=" + org.apache.lucene.util.Version.LATEST);

    try (FSDirectory dir = FSDirectory.open(out)) {
      try (IndexWriter writer = new IndexWriter(dir, new IndexWriterConfig())) {
        int i = 0;
        for (int seg = 0; seg < sizes.length; seg++) {
          for (int k = 0; k < sizes[seg]; k++, i++) {
            writer.addDocument(doc(features, seg, i));
          }
          writer.commit();
        }
        if ((features & DELETES) != 0) {
          writer.deleteDocuments(new Term("del", "yes"));
          writer.commit();
        }
        System.out.println("segmentsBeforeMerge=" + SegmentInfos.readLatestCommit(dir).size());
        writer.forceMerge(1);
        writer.commit();
      }
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      SegmentCommitInfo merged = infos.info(0);
      System.out.println("segments=" + infos.size());
      System.out.println("name=" + merged.info.name);
      System.out.println("id=" + HexFormat.of().formatHex(merged.info.getId()));
      System.out.println("compound=" + merged.info.getUseCompoundFile());
      System.out.println("maxDoc=" + merged.info.maxDoc() + " delCount=" + merged.getDelCount());
      List<String> files = List.copyOf(new TreeSet<>(merged.files()));
      for (String file : files) {
        System.out.println("file " + file + " " + Files.size(out.resolve(file)));
      }
    }
  }
}
