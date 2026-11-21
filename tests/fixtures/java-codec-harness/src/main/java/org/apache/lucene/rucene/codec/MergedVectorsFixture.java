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
import java.util.Arrays;
import java.util.HexFormat;
import java.util.List;
import java.util.TreeSet;
import java.util.zip.CRC32;
import org.apache.lucene.codecs.KnnVectorsFormat;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.lucene104.Lucene104HnswScalarQuantizedVectorsFormat;
import org.apache.lucene.codecs.lucene104.Lucene104ScalarQuantizedVectorsFormat;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.KnnByteVectorField;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.FieldExistsQuery;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.KnnByteVectorQuery;
import org.apache.lucene.search.KnnFloatVectorQuery;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.store.FSDirectory;

/**
 * Reference tool for the vector merge: two segments of vector documents are
 * written with a real Apache Lucene Core 10.5.0 {@link IndexWriter}, optionally
 * some documents are deleted, and the two are force-merged into one. The tool
 * prints the merged segment's name and id and every byte of its vector files.
 *
 * <p>This is the output Rucene's {@code KnnVectorsWriter.merge} and
 * {@code KnnVectorsWriter.MergedVectorValues} are judged against
 * ({@code tests/index_writer_merge.rs}). Every document stores {@code id} and
 * has a two-dimensional float vector {@code f} and a two-dimensional byte
 * vector {@code b}; the even documents also have a float vector {@code s}, so
 * one field is sparse; with deletions, documents 1 and 4 also have a float
 * vector {@code del}, and a {@link FieldExistsQuery} on it deletes exactly
 * them, so the merged segment holds a vector field whose every document is
 * gone.
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... MergedVectorsFixture &lt;output-dir&gt; &lt;default|sq&gt; &lt;true|false&gt;
 * </pre>
 *
 * <p>The second argument selects the codec: {@code default} is
 * {@link Lucene104Codec} as shipped, whose vectors format is
 * {@code Lucene99HnswVectorsFormat}; {@code sq} gives every field one shared
 * {@link Lucene104ScalarQuantizedVectorsFormat}; {@code hnswsq} one shared
 * {@link Lucene104HnswScalarQuantizedVectorsFormat}; {@code sq-then-hnswsq}
 * writes the source segments with the first and merges them with the second, so
 * the merge reads quantized sources and builds its graph through
 * {@code QuantizedVectorsReader.getRandomVectorScorerSupplierForMerge}. The third says whether to
 * delete, or is {@code graph}: two segments of {@value #GRAPH_DOCS_PER_SEGMENT}
 * documents of {@link #graphDoc(int)}, each too small for a graph of its own
 * while the merged segment is large enough to get one, or {@code wide-graph}:
 * the same with {@link #wideGraphDoc(int)}, whose float vectors have
 * {@value #WIDE_DIMENSIONS} dimensions, enough for the quantized scores that
 * build a graph through the quantized reader to differ from the raw ones. For
 * those two cases the tool prints, instead of every byte, each vector file's
 * length and the CRC-32 of its bytes with the segment id and the footer
 * checksum zeroed, followed by the hits of one kNN query per field on the
 * merged segment.
 */
public final class MergedVectorsFixture {

  private MergedVectorsFixture() {}

  static Document doc(int i, boolean deletable) {
    Document d = new Document();
    d.add(new StoredField("id", "doc-" + i));
    d.add(
        new KnnFloatVectorField(
            "f", new float[] {i, 1f + i * 0.5f}, VectorSimilarityFunction.EUCLIDEAN));
    d.add(
        new KnnByteVectorField(
            "b", new byte[] {(byte) i, (byte) (200 - i)}, VectorSimilarityFunction.EUCLIDEAN));
    if (i % 2 == 0) {
      d.add(
          new KnnFloatVectorField(
              "s", new float[] {i * 2f, -i}, VectorSimilarityFunction.EUCLIDEAN));
    }
    if (deletable && (i == 1 || i == 4)) {
      d.add(new KnnFloatVectorField("del", new float[] {1f}, VectorSimilarityFunction.EUCLIDEAN));
    }
    return d;
  }

  /** Documents per source segment in the {@code graph} case. */
  static final int GRAPH_DOCS_PER_SEGMENT = 400;

  /**
   * Document {@code i} of the {@code graph} case: integer arithmetic only, so a
   * port reproduces every vector exactly.
   */
  static Document graphDoc(int i) {
    Document d = new Document();
    d.add(new StoredField("id", "doc-" + i));
    d.add(
        new KnnFloatVectorField(
            "f",
            new float[] {((i * 7919) % 1009) / 7f, ((i * 104729) % 1013) / 11f},
            VectorSimilarityFunction.EUCLIDEAN));
    d.add(
        new KnnByteVectorField(
            "b",
            new byte[] {(byte) ((i * 31) % 251), (byte) ((i * 17) % 241)},
            VectorSimilarityFunction.EUCLIDEAN));
    return d;
  }

  /** Dimensions of the float vectors of the {@code wide-graph} case. */
  static final int WIDE_DIMENSIONS = 16;

  /**
   * Document {@code i} of the {@code wide-graph} case: {@link #graphDoc(int)} with
   * a float vector of {@value #WIDE_DIMENSIONS} dimensions, integer arithmetic
   * only.
   */
  static Document wideGraphDoc(int i) {
    Document d = new Document();
    d.add(new StoredField("id", "doc-" + i));
    float[] f = new float[WIDE_DIMENSIONS];
    for (int j = 0; j < WIDE_DIMENSIONS; j++) {
      f[j] = ((i * 7919 + j * 104729) % 1009) / 7f;
    }
    d.add(new KnnFloatVectorField("f", f, VectorSimilarityFunction.EUCLIDEAN));
    d.add(
        new KnnByteVectorField(
            "b",
            new byte[] {(byte) ((i * 31) % 251), (byte) ((i * 17) % 241)},
            VectorSimilarityFunction.EUCLIDEAN));
    return d;
  }

  /** CRC-32 of {@code file} with the header's segment id and the footer checksum zeroed. */
  static long maskedCrc(byte[] file) {
    byte[] masked = file.clone();
    int id = 4 + 1 + masked[4] + 4;
    Arrays.fill(masked, id, id + 16, (byte) 0);
    Arrays.fill(masked, masked.length - 8, masked.length, (byte) 0);
    CRC32 crc = new CRC32();
    crc.update(masked);
    return crc.getValue();
  }

  /** Runs one case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 3) {
      throw new IllegalArgumentException(
          "usage: MergedVectorsFixture <output-dir> <default|sq|hnswsq|sq-then-hnswsq>"
              + " <true|false|graph|wide-graph>");
    }
    Path out = Paths.get(args[0]);
    boolean sq = args[1].equals("sq");
    boolean hnswsq = args[1].equals("hnswsq");
    boolean sqThenHnswsq = args[1].equals("sq-then-hnswsq");
    boolean wide = args[2].equals("wide-graph");
    boolean graph = wide || args[2].equals("graph");
    boolean deletes = Boolean.parseBoolean(args[2]);
    Files.createDirectories(out);
    try (var listing = Files.list(out)) {
      for (Path file : listing.toList()) {
        Files.delete(file);
      }
    }
    System.out.println("version=" + org.apache.lucene.util.Version.LATEST);

    IndexWriterConfig config = new IndexWriterConfig();
    final boolean[] merging = {false};
    if (sqThenHnswsq) {
      final KnnVectorsFormat flat = new Lucene104ScalarQuantizedVectorsFormat();
      final KnnVectorsFormat graphFormat = new Lucene104HnswScalarQuantizedVectorsFormat();
      config.setCodec(
          new Lucene104Codec() {
            @Override
            public KnnVectorsFormat getKnnVectorsFormatForField(String field) {
              return merging[0] ? graphFormat : flat;
            }
          });
    } else if (sq || hnswsq) {
      final KnnVectorsFormat format =
          sq
              ? new Lucene104ScalarQuantizedVectorsFormat()
              : new Lucene104HnswScalarQuantizedVectorsFormat();
      config.setCodec(
          new Lucene104Codec() {
            @Override
            public KnnVectorsFormat getKnnVectorsFormatForField(String field) {
              return format;
            }
          });
    }
    try (FSDirectory dir = FSDirectory.open(out)) {
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        if (graph) {
          for (int i = 0; i < GRAPH_DOCS_PER_SEGMENT; i++) {
            writer.addDocument(wide ? wideGraphDoc(i) : graphDoc(i));
          }
          writer.commit();
          for (int i = GRAPH_DOCS_PER_SEGMENT; i < 2 * GRAPH_DOCS_PER_SEGMENT; i++) {
            writer.addDocument(wide ? wideGraphDoc(i) : graphDoc(i));
          }
          writer.commit();
        } else {
          for (int i = 0; i < 3; i++) {
            writer.addDocument(doc(i, deletes));
          }
          writer.commit();
          for (int i = 3; i < 6; i++) {
            writer.addDocument(doc(i, deletes));
          }
          writer.commit();
        }
        if (deletes) {
          writer.deleteDocuments(new FieldExistsQuery("del"));
          writer.commit();
        }
        merging[0] = true;
        writer.forceMerge(1);
        writer.commit();
      }
      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      SegmentCommitInfo merged = infos.info(0);
      System.out.println("segments=" + infos.size());
      System.out.println("name=" + merged.info.name);
      System.out.println("id=" + HexFormat.of().formatHex(merged.info.getId()));
      System.out.println("maxDoc=" + merged.info.maxDoc() + " delCount=" + merged.getDelCount());
      List<String> files = List.copyOf(new TreeSet<>(merged.files()));
      for (String file : files) {
        if (file.contains("VectorsFormat")) {
          byte[] bytes = Files.readAllBytes(out.resolve(file));
          if (graph) {
            System.out.println("file " + file + " " + bytes.length + " " + maskedCrc(bytes));
          } else {
            System.out.println("file " + file + " " + HexFormat.of().formatHex(bytes));
          }
        }
      }
      if (graph) {
        try (DirectoryReader reader = DirectoryReader.open(dir)) {
          IndexSearcher searcher = new IndexSearcher(reader);
          float[] target = new float[] {50f, 40f};
          if (wide) {
            target = new float[WIDE_DIMENSIONS];
            for (int j = 0; j < WIDE_DIMENSIONS; j++) {
              target[j] = 50f + j;
            }
          }
          print("knn f", searcher.search(new KnnFloatVectorQuery("f", target, 10), 10));
          print(
              "knn b",
              searcher.search(new KnnByteVectorQuery("b", new byte[] {(byte) 100, (byte) 50}, 10), 10));
        }
      }
    }
  }

  static void print(String label, TopDocs topDocs) {
    StringBuilder hits = new StringBuilder();
    for (ScoreDoc scoreDoc : topDocs.scoreDocs) {
      hits.append(' ').append(scoreDoc.doc).append(':').append(Float.floatToIntBits(scoreDoc.score));
    }
    System.out.println(label + hits);
  }
}
