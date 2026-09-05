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
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

import org.apache.lucene.codecs.CodecUtil;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.index.DocValuesSkipIndexType;
import org.apache.lucene.index.DocValuesType;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.store.TrackingDirectoryWrapper;
import org.apache.lucene.util.Version;

/**
 * Reference writer for the three files that seal a flushed segment: the
 * {@code .fnm} that {@code IndexingChain.flush} writes last, and the
 * {@code .si} and compound file that
 * {@code DocumentsWriterPerThread.sealFlushedSegment} writes.
 *
 * <p>Unlike {@link FlushFilesFixture}, which runs a whole {@code IndexWriter}
 * and can therefore only be compared by file <em>name</em>, this tool drives the
 * three codec writers directly with every input pinned — segment name, segment
 * id, versions, field metadata, diagnostics — so the files it produces can be
 * compared with Rucene's <strong>byte for byte</strong>. A segment written by a
 * real writer never can be: Lucene stamps each segment with
 * {@code StringHelper.randomId()} and stamps the {@code .si} diagnostics with a
 * wall-clock timestamp, so two runs of Lucene itself differ.
 *
 * <p>Command-line usage:
 *
 * <pre>
 *   java ... SealBytesFixture &lt;output-dir&gt; &lt;case&gt;
 * </pre>
 *
 * <p>Supported cases: {@code fnm}, {@code fnm_multi_attribute}, {@code si},
 * {@code si_multi_file}, {@code cfs}.
 *
 * <p>Every map and set written by these cases holds at most one entry. Both
 * {@code writeMapOfStrings} and {@code writeSetOfStrings} serialise their
 * argument in iteration order, and the iteration order of a
 * {@link java.util.HashMap} is a property of the JDK's hashing rather than of
 * the Lucene format; pinning the collections to a single entry keeps the
 * comparison about the format itself. See the Rust side for the multi-entry
 * case and what it can and cannot assert.
 */
public final class SealBytesFixture {

  private SealBytesFixture() {}

  /** The segment name every case writes. */
  public static final String SEGMENT = "_9";

  /** The segment id every case writes, chosen so no run has to guess it. */
  public static final byte[] SEGMENT_ID = {
    0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78,
    (byte) 0x87, (byte) 0x96, (byte) 0xa5, (byte) 0xb4,
    (byte) 0xc3, (byte) 0xd2, (byte) 0xe1, (byte) 0xf0
  };

  /** Runs one case. */
  public static void main(String[] args) throws IOException {
    if (args.length < 2) {
      throw new IllegalArgumentException("usage: SealBytesFixture <output-dir> <case>");
    }
    Path out = Paths.get(args[0]);
    String shape = args[1];
    System.out.println("version=" + Version.LATEST);

    Files.createDirectories(out);
    try (FSDirectory dir = FSDirectory.open(out)) {
      for (String name : dir.listAll()) {
        Files.deleteIfExists(out.resolve(name));
      }
      switch (shape) {
        case "fnm" -> writeFieldInfos(dir, false);
        case "fnm_multi_attribute" -> writeFieldInfos(dir, true);
        case "si" -> writeSegmentInfo(dir, false);
        case "si_multi_file" -> writeSegmentInfo(dir, true);
        case "cfs" -> writeCompoundFile(dir);
        default -> throw new IllegalArgumentException("unknown case: " + shape);
      }
      List<String> files = new ArrayList<>(Arrays.asList(dir.listAll()));
      files.sort(null);
      System.out.println("files=" + String.join(",", files));
    }
  }

  /** A `SegmentInfo` with everything pinned; `files` is set by the caller. */
  private static SegmentInfo segmentInfo(FSDirectory dir, boolean compound) {
    Map<String, String> diagnostics = new HashMap<>();
    diagnostics.put("source", "flush");
    Map<String, String> attributes = new HashMap<>();
    attributes.put("Lucene90StoredFieldsFormat.mode", "BEST_SPEED");
    return new SegmentInfo(
        dir,
        Version.LATEST,
        Version.LATEST,
        SEGMENT,
        7,
        compound,
        false,
        new Lucene104Codec(),
        diagnostics,
        SEGMENT_ID,
        attributes,
        null);
  }

  /**
   * Writes the `.fnm` of a segment whose fields span the shapes the format
   * encodes: postings options, norms, payloads, term vectors, doc values with
   * and without a skip index, points, vectors, the soft-deletes field and the
   * parent field.
   */
  private static void writeFieldInfos(FSDirectory dir, boolean multiAttribute) throws IOException {
    Map<String, String> one = new HashMap<>();
    one.put("PerFieldPostingsFormat.format", "Lucene104");
    Map<String, String> two = new HashMap<>();
    two.put("PerFieldPostingsFormat.format", "Lucene104");
    two.put("PerFieldPostingsFormat.suffix", "0");

    List<FieldInfo> infos = new ArrayList<>();
    infos.add(
        field(
            "body",
            0,
            true,
            false,
            true,
            IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
            DocValuesType.NONE,
            DocValuesSkipIndexType.NONE,
            -1,
            multiAttribute ? two : Map.of(),
            0,
            0,
            0,
            0,
            VectorEncoding.FLOAT32,
            VectorSimilarityFunction.EUCLIDEAN,
            false,
            false));
    infos.add(
        field(
            "id",
            1,
            false,
            true,
            false,
            IndexOptions.DOCS,
            DocValuesType.NONE,
            DocValuesSkipIndexType.NONE,
            -1,
            multiAttribute ? two : one,
            0,
            0,
            0,
            0,
            VectorEncoding.FLOAT32,
            VectorSimilarityFunction.EUCLIDEAN,
            false,
            false));
    infos.add(
        field(
            "price",
            2,
            false,
            true,
            false,
            IndexOptions.NONE,
            DocValuesType.SORTED_NUMERIC,
            DocValuesSkipIndexType.RANGE,
            -1,
            Map.of(),
            0,
            0,
            0,
            0,
            VectorEncoding.FLOAT32,
            VectorSimilarityFunction.EUCLIDEAN,
            false,
            false));
    infos.add(
        field(
            "point",
            3,
            false,
            true,
            false,
            IndexOptions.NONE,
            DocValuesType.NONE,
            DocValuesSkipIndexType.NONE,
            -1,
            Map.of(),
            2,
            1,
            4,
            0,
            VectorEncoding.FLOAT32,
            VectorSimilarityFunction.EUCLIDEAN,
            false,
            false));
    infos.add(
        field(
            "vector",
            4,
            false,
            true,
            false,
            IndexOptions.NONE,
            DocValuesType.NONE,
            DocValuesSkipIndexType.NONE,
            -1,
            Map.of(),
            0,
            0,
            0,
            3,
            VectorEncoding.BYTE,
            VectorSimilarityFunction.DOT_PRODUCT,
            false,
            false));
    infos.add(
        field(
            "__soft_deletes",
            5,
            false,
            true,
            false,
            IndexOptions.NONE,
            DocValuesType.NUMERIC,
            DocValuesSkipIndexType.NONE,
            -1,
            Map.of(),
            0,
            0,
            0,
            0,
            VectorEncoding.FLOAT32,
            VectorSimilarityFunction.EUCLIDEAN,
            true,
            false));
    infos.add(
        field(
            "_parent",
            6,
            false,
            true,
            false,
            IndexOptions.NONE,
            DocValuesType.NUMERIC,
            DocValuesSkipIndexType.NONE,
            -1,
            Map.of(),
            0,
            0,
            0,
            0,
            VectorEncoding.FLOAT32,
            VectorSimilarityFunction.EUCLIDEAN,
            false,
            true));

    SegmentInfo si = segmentInfo(dir, false);
    FieldInfos fieldInfos = new FieldInfos(infos.toArray(new FieldInfo[0]));
    new Lucene104Codec().fieldInfosFormat().write(dir, si, "", fieldInfos, IOContext.DEFAULT);
  }

  private static FieldInfo field(
      String name,
      int number,
      boolean storeTermVector,
      boolean omitNorms,
      boolean storePayloads,
      IndexOptions indexOptions,
      DocValuesType docValues,
      DocValuesSkipIndexType docValuesSkipIndex,
      long dvGen,
      Map<String, String> attributes,
      int pointDimensionCount,
      int pointIndexDimensionCount,
      int pointNumBytes,
      int vectorDimension,
      VectorEncoding vectorEncoding,
      VectorSimilarityFunction vectorSimilarity,
      boolean softDeletes,
      boolean isParent) {
    return new FieldInfo(
        name,
        number,
        storeTermVector,
        omitNorms,
        storePayloads,
        indexOptions,
        docValues,
        docValuesSkipIndex,
        dvGen,
        new HashMap<>(attributes),
        pointDimensionCount,
        pointIndexDimensionCount,
        pointNumBytes,
        vectorDimension,
        vectorEncoding,
        vectorSimilarity,
        softDeletes,
        isParent);
  }

  /** The extra files the multi-file `.si` case lists. */
  public static final String[] SI_EXTRA_FILES = {".fdt", ".fdx", ".nvd"};

  /**
   * Writes a `.si` with every field of the format pinned.
   *
   * <p>With {@code multiFile} false the segment starts with no files at all, so
   * the only entry the format writes is the {@code .si} itself, which
   * {@code Lucene99SegmentInfoFormat.write} adds before serialising
   * (`Lucene99SegmentInfoFormat.java:175`). A one-entry set has one possible
   * encoding, which makes the byte comparison exact.
   *
   * <p>With {@code multiFile} true the segment lists several files, which is
   * the shape a real segment has and whose encoding order
   * {@code writeSetOfStrings} takes from a {@link HashSet}.
   */
  private static void writeSegmentInfo(FSDirectory dir, boolean multiFile) throws IOException {
    SegmentInfo si = segmentInfo(dir, false);
    Set<String> files = new HashSet<>();
    if (multiFile) {
      for (String extension : SI_EXTRA_FILES) {
        files.add(SEGMENT + extension);
      }
    }
    si.setFiles(files);
    new Lucene104Codec().segmentInfoFormat().write(dir, si, IOContext.DEFAULT);
  }

  /**
   * Writes three sub-files of pairwise distinct lengths and packs them into a
   * compound file.
   *
   * <p>The lengths must differ: {@code Lucene90CompoundFormat} orders the
   * entries by ascending file length through a priority queue
   * (`Lucene90CompoundFormat.java:126-131`), and only distinct lengths make that
   * order independent of the iteration order of {@code si.files()}, which is a
   * {@link HashSet}.
   */
  private static void writeCompoundFile(FSDirectory dir) throws IOException {
    Set<String> files = new HashSet<>();
    for (SubFile sub : SUB_FILES) {
      String name = SEGMENT + sub.extension;
      try (IndexOutput out = dir.createOutput(name, IOContext.DEFAULT)) {
        CodecUtil.writeIndexHeader(out, sub.codec, sub.version, SEGMENT_ID, "");
        for (int i = 0; i < sub.bodyLength; i++) {
          out.writeByte((byte) (i * 7 + sub.bodyLength));
        }
        CodecUtil.writeFooter(out);
      }
      files.add(name);
    }
    SegmentInfo si = segmentInfo(dir, false);
    si.setFiles(files);
    // `IndexWriter.createCompoundFile` is package private, so this reproduces
    // its body (`IndexWriter.java:5979-6009`): the clean-tracking-dir check, the
    // one call that writes the files, and the `setFiles` that replaces the
    // segment's files with the `.cfs`/`.cfe` pair. Only the error path, which
    // deletes what the failed write created, is left out — nothing here fails.
    TrackingDirectoryWrapper tracking = new TrackingDirectoryWrapper(dir);
    si.getCodec().compoundFormat().write(tracking, si, IOContext.DEFAULT);
    si.setFiles(new HashSet<>(tracking.getCreatedFiles()));
  }

  /** One sub-file of the compound-file case. */
  private record SubFile(String extension, String codec, int version, int bodyLength) {}

  /** The sub-files of the compound-file case, in the order they are created. */
  public static final SubFile[] SUB_FILES = {
    new SubFile(".fdt", "RuceneSealTestA", 3, 700),
    new SubFile(".nvd", "RuceneSealTestB", 0, 61),
    new SubFile(".tim", "RuceneSealTestC", 11, 313),
  };
}
