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

import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.Field;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FloatVectorValues;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.KnnVectorValues;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.Version;

/**
 * Captures the exact {@code (docID, index())} sequences that the three
 * {@link KnnVectorValues.DocIndexIterator} factories produce, plus the metadata
 * a {@link KnnVectorValues} exposes.
 *
 * <p>The three factories ({@code createDenseIterator}, {@code createSparseIterator}
 * and {@code fromDISI}) are {@code protected} on {@link KnnVectorValues}, so
 * the fixture reaches them through a subclass rather than through a codec. That
 * is deliberate: it pins the behaviour of the abstraction itself, including the
 * detail that {@code fromDISI}'s {@code advance} leaves the ordinal stale.
 *
 * <p>A dense and a sparse {@code KnnFloatVectorField} are also indexed and read
 * back, so the metadata reported through a real reader is captured too.
 *
 * <p>Usage:
 * <pre>
 *   mvn -q -f tests/fixtures/java-codec-harness/pom.xml \
 *       exec:java -Dexec.mainClass=org.apache.lucene.rucene.codec.VectorValuesIteratorFixture \
 *       -Dexec.args="/tmp/rucene-vector-fixtures ITERATORS"
 * </pre>
 * where CASE is ITERATORS or INDEXED.
 */
public final class VectorValuesIteratorFixture {

  private VectorValuesIteratorFixture() {}

  private static final String DENSE_FIELD = "dense";
  private static final String SPARSE_FIELD = "sparse";

  public static void main(String[] args) throws IOException {
    if (args.length != 2) {
      System.err.println("Usage: VectorValuesIteratorFixture <output-dir> <case>");
      System.err.println("Cases: ITERATORS, INDEXED");
      System.exit(1);
    }
    Path outputDir = Paths.get(args[0]);
    String testCase = args[1];

    System.out.println("fixture=VectorValuesIteratorFixture");
    System.out.println("version=" + Version.LATEST);
    System.out.println("case=" + testCase);

    switch (testCase) {
      case "ITERATORS" -> emitIterators();
      case "INDEXED" -> emitIndexed(outputDir);
      default -> {
        System.err.println("unknown case: " + testCase);
        System.exit(1);
      }
    }
  }

  // ---------------------------------------------------------------------------
  // The three DocIndexIterator factories
  // ---------------------------------------------------------------------------

  private static void emitIterators() throws IOException {
    ProbeValues dense = new ProbeValues(5, new int[] {0, 1, 2, 3, 4});
    System.out.println("values name=dense size=" + dense.size() + " dimension=" + dense.dimension()
        + " byte_length=" + dense.getVectorByteLength()
        + " encoding_ordinal=" + dense.getEncoding().ordinal());
    emitSequential("dense", dense.dense());
    emitAdvances("dense", dense, true, new int[] {0, 3, 4, 5});

    int[] sparseOrdToDoc = {2, 4, 9, 15};
    ProbeValues sparse = new ProbeValues(sparseOrdToDoc.length, sparseOrdToDoc);
    for (int ord = 0; ord < sparseOrdToDoc.length; ord++) {
      System.out.println("ord_to_doc name=sparse ord=" + ord + " doc=" + sparse.ordToDoc(ord));
    }
    System.out.println("values name=sparse size=" + sparse.size() + " dimension="
        + sparse.dimension() + " byte_length=" + sparse.getVectorByteLength()
        + " encoding_ordinal=" + sparse.getEncoding().ordinal());
    emitSequential("sparse", sparse.sparse());
    emitAdvances("sparse", sparse, false, new int[] {0, 4, 5, 16});

    int[] disiDocs = {0, 5, 9, 12};
    emitSequential("from_disi", ProbeValues.fromDocs(new ArrayDocs(disiDocs)));
    // The interesting run: interleave advance with nextDoc and report index()
    // after every step. Java never touches the ordinal in advance.
    KnnVectorValues.DocIndexIterator mixed = ProbeValues.fromDocs(new ArrayDocs(disiDocs));
    System.out.println("step from_disi op=start doc=" + mixed.docID() + " index=" + mixed.index());
    System.out.println(
        "step from_disi op=next doc=" + mixed.nextDoc() + " index=" + mixed.index());
    System.out.println(
        "step from_disi op=next doc=" + mixed.nextDoc() + " index=" + mixed.index());
    System.out.println(
        "step from_disi op=advance(12) doc=" + mixed.advance(12) + " index=" + mixed.index());
    System.out.println(
        "step from_disi op=next doc=" + mixed.nextDoc() + " index=" + mixed.index());
  }

  private static void emitSequential(String name, KnnVectorValues.DocIndexIterator it)
      throws IOException {
    System.out.println("seq " + name + " start doc=" + it.docID() + " index=" + it.index());
    int doc;
    while ((doc = it.nextDoc()) != DocIdSetIterator.NO_MORE_DOCS) {
      System.out.println(
          "seq " + name + " next doc=" + doc + " index=" + it.index() + " run_end="
              + it.docIDRunEnd());
    }
    System.out.println("seq " + name + " end doc=" + it.docID() + " index=" + it.index());
  }

  private static void emitAdvances(String name, ProbeValues values, boolean dense, int[] targets)
      throws IOException {
    for (int target : targets) {
      KnnVectorValues.DocIndexIterator it = dense ? values.dense() : values.sparse();
      int doc = it.advance(target);
      System.out.println(
          "advance " + name + " target=" + target + " doc=" + doc + " index=" + it.index());
    }
  }

  /** A {@link KnnVectorValues} with no values, used only to reach the iterator factories. */
  private static final class ProbeValues extends KnnVectorValues {
    private final int size;
    private final int[] ordToDoc;

    ProbeValues(int size, int[] ordToDoc) {
      this.size = size;
      this.ordToDoc = ordToDoc;
    }

    @Override
    public int dimension() {
      return 3;
    }

    @Override
    public int size() {
      return size;
    }

    @Override
    public int ordToDoc(int ord) {
      return ordToDoc[ord];
    }

    @Override
    public KnnVectorValues copy() {
      return this;
    }

    @Override
    public VectorEncoding getEncoding() {
      return VectorEncoding.FLOAT32;
    }

    DocIndexIterator dense() {
      return createDenseIterator();
    }

    DocIndexIterator sparse() {
      return createSparseIterator();
    }

    static DocIndexIterator fromDocs(DocIdSetIterator docs) {
      return fromDISI(docs);
    }
  }

  /** A {@link DocIdSetIterator} over a fixed, increasing array of doc IDs. */
  private static final class ArrayDocs extends DocIdSetIterator {
    private final int[] docs;
    private int position = -1;

    ArrayDocs(int[] docs) {
      this.docs = docs;
    }

    @Override
    public int docID() {
      if (position < 0) {
        return -1;
      }
      if (position >= docs.length) {
        return NO_MORE_DOCS;
      }
      return docs[position];
    }

    @Override
    public int nextDoc() {
      position++;
      return docID();
    }

    @Override
    public int advance(int target) {
      int doc;
      do {
        doc = nextDoc();
      } while (doc < target);
      return doc;
    }

    @Override
    public long cost() {
      return docs.length;
    }
  }

  // ---------------------------------------------------------------------------
  // A real index
  // ---------------------------------------------------------------------------

  private static void emitIndexed(Path outputDir) throws IOException {
    Files.createDirectories(outputDir);
    IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
    config.setCodec(new Lucene104Codec());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    config.setUseCompoundFile(false);

    try (FSDirectory dir = FSDirectory.open(outputDir)) {
      try (IndexWriter writer = new IndexWriter(dir, config)) {
        for (int doc = 0; doc < 6; doc++) {
          Document document = new Document();
          document.add(new StringField("id", "d" + doc, Field.Store.NO));
          document.add(
              new KnnFloatVectorField(
                  DENSE_FIELD,
                  new float[] {doc, doc + 0.5f, doc - 0.5f},
                  VectorSimilarityFunction.DOT_PRODUCT));
          // Only every other document carries the sparse vector.
          if (doc % 2 == 0) {
            document.add(
                new KnnFloatVectorField(
                    SPARSE_FIELD,
                    new float[] {doc, -doc, 1.0f},
                    VectorSimilarityFunction.EUCLIDEAN));
          }
          writer.addDocument(document);
        }
        writer.commit();
      }

      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        LeafReader leaf = reader.leaves().get(0).reader();
        emitIndexedField(leaf, DENSE_FIELD);
        emitIndexedField(leaf, SPARSE_FIELD);
      }
    }
  }

  private static void emitIndexedField(LeafReader leaf, String field) throws IOException {
    FloatVectorValues values = leaf.getFloatVectorValues(field);
    if (values == null) {
      throw new IllegalStateException("no vector values for field " + field);
    }
    System.out.println(
        "indexed field="
            + field
            + " size="
            + values.size()
            + " dimension="
            + values.dimension()
            + " byte_length="
            + values.getVectorByteLength()
            + " encoding_ordinal="
            + values.getEncoding().ordinal());
    for (int ord = 0; ord < values.size(); ord++) {
      System.out.println("indexed_ord field=" + field + " ord=" + ord + " doc="
          + values.ordToDoc(ord));
    }
    KnnVectorValues.DocIndexIterator it = values.iterator();
    int doc;
    while ((doc = it.nextDoc()) != DocIdSetIterator.NO_MORE_DOCS) {
      System.out.println(
          "indexed_seq field=" + field + " doc=" + doc + " index=" + it.index() + " run_end="
              + it.docIDRunEnd());
    }
  }
}
