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
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.KnnByteVectorField;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.index.ByteVectorValues;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FloatVectorValues;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.KnnVectorValues;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.FSDirectory;

/**
 * Writes a single-segment Apache Lucene Core 10.5.0 index whose only content is
 * KNN vector values, so that the resulting {@code .vec}, {@code .vemf},
 * {@code .vex} and {@code .vem} files depend only on
 * {@code VectorValuesConsumer}, on {@code PerFieldKnnVectorsFormat} and on the
 * {@code Lucene99HnswVectorsWriter}/{@code Lucene99FlatVectorsWriter} pair
 * underneath it.
 *
 * <p>Every case is a fixed, algorithmically generated table of documents that
 * the Rust portability test mirrors exactly, so no analyzer takes part: a byte
 * difference can only come from the vectors consumer or from the vectors codec.
 * The order in which each field is first seen fixes both the field numbers and
 * — unlike doc values and points, which flush in field-hash order — the order
 * of the per-field entries inside the {@code .vemf} and the {@code .vem},
 * because {@code IndexingChain.initializeFieldInfo} asks the consumer for the
 * field writer during the first pass over the document's fields.
 *
 * <p>The cases span the shape dimensions of the format rather than only its
 * values:
 *
 * <ul>
 *   <li>both encodings, {@code FLOAT32} and {@code BYTE}, whose vector data is
 *       aligned to 64 and to 4 bytes respectively;
 *   <li>all four similarity functions, which are written as an ordinal into the
 *       per-field metadata;
 *   <li>dimension counts at the edges: 1, 16 (exactly one 64-byte float
 *       alignment unit), and 1024, the maximum {@code Lucene99HnswVectorsFormat}
 *       accepts;
 *   <li>dense and sparse fields, the second of which makes
 *       {@code DocsWithFieldSet} switch from "every document" to a bit set and
 *       adds the {@code DirectMonotonicWriter} ordinal-to-doc mapping to the
 *       metadata;
 *   <li>corpus sizes on both sides of the tiny-segment threshold, which is what
 *       decides whether an HNSW graph is built at all;
 *   <li>several vector fields in one segment, and a segment with none.
 * </ul>
 *
 * <p>Command-line usage: {@code java ... VectorsFixture <dir> <case>}.
 */
public final class VectorsFixture {

  private VectorsFixture() {}

  /** One vector field of one document. */
  record Vec(String field, float[] floats, byte[] bytes, VectorSimilarityFunction similarity) {
    static Vec floats(String field, VectorSimilarityFunction similarity, float[] values) {
      return new Vec(field, values, null, similarity);
    }

    static Vec bytes(String field, VectorSimilarityFunction similarity, byte[] values) {
      return new Vec(field, null, values, similarity);
    }
  }

  /**
   * A deterministic float vector: never all-zero, so it is legal under
   * {@code COSINE}, and never NaN or infinite.
   */
  static float[] floatVector(int doc, int dim, int salt) {
    float[] value = new float[dim];
    for (int i = 0; i < dim; i++) {
      // A mix that varies with the document, the dimension and the field, and
      // whose values are exactly representable in binary32 so that the Rust
      // side can produce the same bits from the same integers.
      int raw = ((doc * 7919) + (i * 104729) + (salt * 15485863)) % 2003;
      value[i] = (raw - 1000) / 8.0f;
    }
    // Guarantee a non-zero vector even when the mix lands on all zeroes.
    value[0] += 1.0f;
    return value;
  }

  /** A deterministic byte vector that is never all-zero. */
  static byte[] byteVector(int doc, int dim, int salt) {
    byte[] value = new byte[dim];
    for (int i = 0; i < dim; i++) {
      int raw = ((doc * 31) + (i * 17) + (salt * 7)) % 255;
      value[i] = (byte) (raw - 127);
    }
    value[0] = (byte) (value[0] == 0 ? 1 : value[0]);
    return value;
  }

  /** The documents of one case, each a list of vector fields. */
  static List<List<Vec>> documents(String shape) {
    List<List<Vec>> docs = new ArrayList<>();
    switch (shape) {
      case "f32tiny" -> {
        // Eight vectors: far below the tiny-segment threshold, so no HNSW graph
        // is built and the `.vex` holds nothing for this field.
        for (int doc = 0; doc < 8; doc++) {
          docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.EUCLIDEAN,
              floatVector(doc, 3, 0))));
        }
      }
      case "f32dense" -> {
        // Well past the threshold, so the graph is built and written.
        for (int doc = 0; doc < 300; doc++) {
          docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.DOT_PRODUCT,
              floatVector(doc, 4, 0))));
        }
      }
      case "f32sparse" -> {
        // Every third document has no vector, so `DocsWithFieldSet` stops being
        // dense and the metadata gains the ordinal-to-doc mapping.
        for (int doc = 0; doc < 400; doc++) {
          if (doc % 3 == 0) {
            docs.add(List.of());
          } else {
            docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.COSINE,
                floatVector(doc, 2, 0))));
          }
        }
      }
      case "f32cosine" -> {
        for (int doc = 0; doc < 150; doc++) {
          docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.COSINE,
              floatVector(doc, 5, 3))));
        }
      }
      case "f32mip" -> {
        for (int doc = 0; doc < 150; doc++) {
          docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.MAXIMUM_INNER_PRODUCT,
              floatVector(doc, 6, 5))));
        }
      }
      case "dim1" -> {
        for (int doc = 0; doc < 200; doc++) {
          docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.EUCLIDEAN,
              floatVector(doc, 1, 0))));
        }
      }
      case "dim16" -> {
        // 16 floats is exactly the 64-byte unit `alignOutput` pads the vector
        // data to, so this is the one dimension count that needs no padding.
        for (int doc = 0; doc < 200; doc++) {
          docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.EUCLIDEAN,
              floatVector(doc, 16, 0))));
        }
      }
      case "dim1024" -> {
        // The maximum `Lucene99HnswVectorsFormat.getMaxDimensions` allows.
        for (int doc = 0; doc < 12; doc++) {
          docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.EUCLIDEAN,
              floatVector(doc, 1024, 0))));
        }
      }
      case "bytetiny" -> {
        for (int doc = 0; doc < 8; doc++) {
          docs.add(List.of(Vec.bytes("v", VectorSimilarityFunction.EUCLIDEAN,
              byteVector(doc, 3, 0))));
        }
      }
      case "bytedense" -> {
        for (int doc = 0; doc < 300; doc++) {
          docs.add(List.of(Vec.bytes("v", VectorSimilarityFunction.EUCLIDEAN,
              byteVector(doc, 8, 0))));
        }
      }
      case "bytesparse" -> {
        for (int doc = 0; doc < 250; doc++) {
          if (doc % 4 == 1) {
            docs.add(List.of());
          } else {
            docs.add(List.of(Vec.bytes("v", VectorSimilarityFunction.DOT_PRODUCT,
                byteVector(doc, 4, 1))));
          }
        }
      }
      case "bytecosine" -> {
        for (int doc = 0; doc < 120; doc++) {
          docs.add(List.of(Vec.bytes("v", VectorSimilarityFunction.COSINE,
              byteVector(doc, 7, 2))));
        }
      }
      case "bytemip" -> {
        for (int doc = 0; doc < 120; doc++) {
          docs.add(List.of(Vec.bytes("v", VectorSimilarityFunction.MAXIMUM_INNER_PRODUCT,
              byteVector(doc, 5, 4))));
        }
      }
      case "multi" -> {
        // Three fields in one segment, whose Java string hash codes put them in
        // a field-hash order that is neither the order they are first seen nor
        // their field-number order. The `.vemf` and `.vem` entries must follow
        // the first-seen order, which is what separates this format's field
        // ordering from that of doc values and points.
        for (int doc = 0; doc < 140; doc++) {
          List<Vec> fields = new ArrayList<>();
          fields.add(Vec.floats("zeta", VectorSimilarityFunction.EUCLIDEAN,
              floatVector(doc, 3, 1)));
          fields.add(Vec.bytes("alpha", VectorSimilarityFunction.DOT_PRODUCT,
              byteVector(doc, 4, 2)));
          fields.add(Vec.floats("mid", VectorSimilarityFunction.COSINE,
              floatVector(doc, 2, 3)));
          docs.add(fields);
        }
      }
      case "multisparse" -> {
        // Two fields with disjoint document sets, so each field's ordinal space
        // and its ordinal-to-doc mapping differ from the other's.
        for (int doc = 0; doc < 200; doc++) {
          List<Vec> fields = new ArrayList<>();
          if (doc % 2 == 0) {
            fields.add(Vec.floats("even", VectorSimilarityFunction.EUCLIDEAN,
                floatVector(doc, 3, 6)));
          }
          if (doc % 3 == 0) {
            fields.add(Vec.bytes("third", VectorSimilarityFunction.EUCLIDEAN,
                byteVector(doc, 3, 7)));
          }
          docs.add(fields);
        }
      }
      case "fieldorder" -> {
        // The first document introduces only `b`; the second introduces `a`
        // before `b`. The consumer sees `b` first, so `b` must be the first
        // entry in the metadata even though `a` sorts and hashes ahead of it.
        for (int doc = 0; doc < 120; doc++) {
          List<Vec> fields = new ArrayList<>();
          if (doc > 0) {
            fields.add(Vec.floats("a", VectorSimilarityFunction.EUCLIDEAN,
                floatVector(doc, 2, 8)));
          }
          fields.add(Vec.floats("b", VectorSimilarityFunction.EUCLIDEAN,
              floatVector(doc, 2, 9)));
          docs.add(fields);
        }
      }
      case "one" -> {
        // A single vector in the whole segment: the smallest graph-less shape,
        // and the one where the ordinal-to-doc mapping has a single entry.
        docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.EUCLIDEAN,
            floatVector(0, 4, 0))));
      }
      case "lastonly" -> {
        // Only the final document carries a vector, so the field is sparse with
        // one value at the very end of the doc-id space.
        for (int doc = 0; doc < 130; doc++) {
          if (doc == 129) {
            docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.EUCLIDEAN,
                floatVector(doc, 3, 0))));
          } else {
            docs.add(List.of());
          }
        }
      }
      case "novec" -> {
        // No document carries a vector; the segment must have no vector file at
        // all, because the consumer never creates its writer.
        for (int doc = 0; doc < 20; doc++) {
          docs.add(List.of());
        }
      }
      default -> {
        // The threshold sweep: `thresholdN` builds a segment of exactly N
        // vectors, which is how the tiny-segment cut-off is located rather than
        // guessed.
        if (shape.startsWith("threshold")) {
          int count = Integer.parseInt(shape.substring("threshold".length()));
          for (int doc = 0; doc < count; doc++) {
            docs.add(List.of(Vec.floats("v", VectorSimilarityFunction.EUCLIDEAN,
                floatVector(doc, 4, 0))));
          }
        } else {
          throw new IllegalArgumentException("unknown shape: " + shape);
        }
      }
    }
    return docs;
  }

  public static void main(String[] args) throws IOException {
    if (args.length != 2) {
      System.err.println("Usage: VectorsFixture <dir> <case>");
      System.exit(1);
    }
    Path dirPath = Paths.get(args[0]);
    String shape = args[1];
    List<List<Vec>> documents = documents(shape);

    try (FSDirectory dir = FSDirectory.open(dirPath)) {
      IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
      config.setCodec(new Lucene104Codec());
      config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      config.setUseCompoundFile(false);
      config.setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH);

      try (IndexWriter writer = new IndexWriter(dir, config)) {
        for (List<Vec> vectors : documents) {
          Document document = new Document();
          for (Vec vec : vectors) {
            if (vec.floats() != null) {
              document.add(new KnnFloatVectorField(vec.field(), vec.floats(), vec.similarity()));
            } else {
              document.add(new KnnByteVectorField(vec.field(), vec.bytes(), vec.similarity()));
            }
          }
          writer.addDocument(document);
        }
        writer.commit();
      }

      SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
      if (infos.size() != 1) {
        throw new IllegalStateException("expected exactly one segment, got " + infos.size());
      }
      SegmentCommitInfo commit = infos.info(0);
      System.out.println("fixture=VectorsFixture");
      System.out.println("case=" + shape);
      System.out.println("segment=" + commit.info.name);
      System.out.println("segment_id=" + IndexingChainFixture.hex(commit.info.getId()));
      System.out.println("max_doc=" + commit.info.maxDoc());
      System.out.println("compound=" + commit.info.getUseCompoundFile());

      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        if (reader.leaves().size() != 1) {
          throw new IllegalStateException("expected one leaf, got " + reader.leaves().size());
        }
        LeafReaderContext leaf = reader.leaves().get(0);
        for (FieldInfo fi : leaf.reader().getFieldInfos()) {
          System.out.println(
              "fieldinfo="
                  + fi.name
                  + ",number="
                  + fi.number
                  + ",dim="
                  + fi.getVectorDimension()
                  + ",encoding="
                  + fi.getVectorEncoding()
                  + ",similarity="
                  + fi.getVectorSimilarityFunction());
        }
        for (String line : dump(leaf, documents)) {
          System.out.println(line);
        }
      }
      System.out.println("read_ok=true");
    }
  }

  /**
   * The query each case searches with, and how many neighbours it asks for.
   *
   * <p>The query is a vector of the same shape as the corpus but built from a
   * document number no case uses, so it is close to the corpus without being a
   * member of it.
   */
  static float[] floatQuery(int dim, int salt) {
    return floatVector(9973, dim, salt);
  }

  /** The byte counterpart of {@link #floatQuery}. */
  static byte[] byteQuery(int dim, int salt) {
    return byteVector(9973, dim, salt);
  }

  /** Dumps every vector of every field, in ordinal order, as Lucene reads them. */
  static List<String> dump(LeafReaderContext leaf, List<List<Vec>> documents) throws IOException {
    List<String> lines = new ArrayList<>();
    for (FieldInfo fi : leaf.reader().getFieldInfos()) {
      if (fi.getVectorDimension() == 0) {
        continue;
      }
      if (fi.getVectorEncoding() == VectorEncoding.FLOAT32) {
        FloatVectorValues values = leaf.reader().getFloatVectorValues(fi.name);
        if (values == null) {
          continue;
        }
        KnnVectorValues.DocIndexIterator iter = values.iterator();
        for (int doc = iter.nextDoc();
            doc != DocIdSetIterator.NO_MORE_DOCS;
            doc = iter.nextDoc()) {
          float[] value = values.vectorValue(iter.index());
          StringBuilder sb = new StringBuilder();
          sb.append("vec=").append(fi.name).append(",doc=").append(doc).append(",ord=")
              .append(iter.index()).append(",value=");
          for (int i = 0; i < value.length; i++) {
            if (i > 0) {
              sb.append(':');
            }
            // The raw bits, so the comparison cannot be blurred by formatting.
            sb.append(Integer.toHexString(Float.floatToRawIntBits(value[i])));
          }
          lines.add(sb.toString());
        }
      } else {
        ByteVectorValues values = leaf.reader().getByteVectorValues(fi.name);
        if (values == null) {
          continue;
        }
        KnnVectorValues.DocIndexIterator iter = values.iterator();
        for (int doc = iter.nextDoc();
            doc != DocIdSetIterator.NO_MORE_DOCS;
            doc = iter.nextDoc()) {
          byte[] value = values.vectorValue(iter.index());
          StringBuilder sb = new StringBuilder();
          sb.append("vec=").append(fi.name).append(",doc=").append(doc).append(",ord=")
              .append(iter.index()).append(",value=");
          for (byte b : value) {
            sb.append(String.format("%02x", b));
          }
          lines.add(sb.toString());
        }
      }
    }
    return lines;
  }
}
