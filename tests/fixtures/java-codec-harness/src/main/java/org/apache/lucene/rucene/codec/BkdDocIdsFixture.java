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
import java.util.List;

import org.apache.lucene.document.IntPoint;
import org.apache.lucene.index.PointValues;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.util.IORunnable;
import org.apache.lucene.util.bkd.BKDConfig;
import org.apache.lucene.util.bkd.BKDReader;
import org.apache.lucene.util.bkd.BKDWriter;

/**
 * Writes and reads a BKD points index whose four fields exercise the four
 * doc-id encodings of {@code DocIdsWriter}: BITSET_IDS, DELTA_BPV_16, BPV_21
 * and BPV_24.
 *
 * <p>Each field is a single leaf of 512 points with distinct values, so the
 * doc-id encoding is chosen purely by the doc-id layout:
 *
 * <ul>
 *   <li>{@code bitset}: docs 64, 66, ..., 1086 (span 1023 &le; 512&lt;&lt;4) &rarr; BITSET_IDS;
 *   <li>{@code delta}: docs 2000, 2128, ..., 67408 (span 65409 &gt; 512&lt;&lt;4, &le; 0xFFFF)
 *       &rarr; DELTA_BPV_16;
 *   <li>{@code bpv21}: docs 100000, 100256, ..., 230816 (max &le; 0x1FFFFF) &rarr; BPV_21;
 *   <li>{@code bpv24}: docs 1966336, 1966592, ..., 2097152 (max &gt; 0x1FFFFF, &le; 0xFFFFFF)
 *       &rarr; BPV_24.
 * </ul>
 *
 * <p>Usage:
 * <pre>
 *   mvn -q -f tests/fixtures/java-codec-harness/pom.xml \
 *       exec:java -Dexec.mainClass=org.apache.lucene.rucene.codec.BkdDocIdsFixture \
 *       -Dexec.args="&lt;dir&gt; write"
 *   mvn -q -f tests/fixtures/java-codec-harness/pom.xml \
 *       exec:java -Dexec.mainClass=org.apache.lucene.rucene.codec.BkdDocIdsFixture \
 *       -Dexec.args="&lt;dir&gt; read"
 * </pre>
 *
 * <p>{@code write} produces {@code &lt;field&gt;.kdm}, {@code &lt;field&gt;.kdi}
 * and {@code &lt;field&gt;.kdd} for each field. {@code read} opens those files
 * with {@link BKDReader} and prints every doc id produced by a full-range
 * {@code intersect}.
 */
public final class BkdDocIdsFixture {

  private BkdDocIdsFixture() {}

  /** One more than the largest doc id used (2_097_152). */
  private static final int MAX_DOC = 2_097_153;

  private static final int POINTS_PER_FIELD = 512;
  private static final int BYTES_PER_DIM = 4;

  private record FieldSpec(String name, int firstDoc, int step) {
    int docId(int i) {
      return firstDoc + step * i;
    }
  }

  private static final List<FieldSpec> FIELDS =
      List.of(
          new FieldSpec("bitset", 64, 2),
          new FieldSpec("delta", 2000, 128),
          new FieldSpec("bpv21", 100000, 256),
          new FieldSpec("bpv24", 1966336, 256));

  public static void main(String[] args) throws IOException {
    if (args.length != 2) {
      System.err.println("Usage: BkdDocIdsFixture <dir> <write|read>");
      System.exit(1);
    }
    Path dirPath = Paths.get(args[0]);
    Files.createDirectories(dirPath);
    try (Directory dir = FSDirectory.open(dirPath)) {
      switch (args[1]) {
        case "write" -> write(dir);
        case "read" -> read(dir);
        default -> {
          System.err.println("unknown mode: " + args[1]);
          System.exit(1);
        }
      }
    }
  }

  private static void write(Directory dir) throws IOException {
    for (FieldSpec spec : FIELDS) {
      BKDConfig config =
          new BKDConfig(1, 1, BYTES_PER_DIM, BKDConfig.DEFAULT_MAX_POINTS_IN_LEAF_NODE);
      try (BKDWriter writer =
          new BKDWriter(MAX_DOC, dir, spec.name(), config, 16.0, POINTS_PER_FIELD)) {
        for (int i = 0; i < POINTS_PER_FIELD; i++) {
          byte[] packed = new byte[BYTES_PER_DIM];
          IntPoint.encodeDimension(i, packed, 0);
          writer.add(packed, spec.docId(i));
        }
        try (IndexOutput metaOut = dir.createOutput(spec.name() + ".kdm", IOContext.DEFAULT);
            IndexOutput indexOut = dir.createOutput(spec.name() + ".kdi", IOContext.DEFAULT);
            IndexOutput dataOut = dir.createOutput(spec.name() + ".kdd", IOContext.DEFAULT)) {
          IORunnable finalizer = writer.finish(metaOut, indexOut, dataOut);
          finalizer.run();
        }
      }
    }
    System.out.println("fixture=BkdDocIdsFixture");
    System.out.println("mode=write");
    System.out.println("max_doc=" + MAX_DOC);
    System.out.println("points_per_field=" + POINTS_PER_FIELD);
    for (FieldSpec spec : FIELDS) {
      System.out.println(
          "field " + spec.name() + " first_doc=" + spec.firstDoc() + " step=" + spec.step());
    }
  }

  private static void read(Directory dir) throws IOException {
    System.out.println("fixture=BkdDocIdsFixture");
    System.out.println("mode=read");
    for (FieldSpec spec : FIELDS) {
      try (IndexInput metaIn = dir.openInput(spec.name() + ".kdm", IOContext.READONCE);
          IndexInput indexIn = dir.openInput(spec.name() + ".kdi", IOContext.READONCE);
          IndexInput dataIn = dir.openInput(spec.name() + ".kdd", IOContext.READONCE)) {
        BKDReader reader = new BKDReader(metaIn, indexIn, dataIn);
        DocIdCollector collector = new DocIdCollector();
        reader.intersect(collector);
        System.out.println("field " + spec.name() + " doc_ids=" + join(collector.docIds));
      }
    }
  }

  private static String join(List<Integer> ids) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < ids.size(); i++) {
      if (i > 0) {
        sb.append(',');
      }
      sb.append(ids.get(i));
    }
    return sb.toString();
  }

  /**
   * Records every doc id the traversal produces. Only the single-document
   * callbacks are overridden; the bulk ones keep their interface defaults,
   * which fan out to the single-document ones. That matters: {@code
   * DocIdsWriter} picks between {@code visit(IntsRef)}, {@code
   * visit(DocIdSetIterator)} and {@code visit(int)} according to how the doc
   * ids of a leaf were encoded, so only the fanned-out trace is a property of
   * the traversal algorithm rather than of the doc-id codec.
   */
  private static final class DocIdCollector implements PointValues.IntersectVisitor {
    final List<Integer> docIds = new ArrayList<>();

    @Override
    public void visit(int docID) {
      docIds.add(docID);
    }

    @Override
    public void visit(int docID, byte[] packedValue) {
      docIds.add(docID);
    }

    @Override
    public PointValues.Relation compare(byte[] minPackedValue, byte[] maxPackedValue) {
      return PointValues.Relation.CELL_CROSSES_QUERY;
    }
  }
}
