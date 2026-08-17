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

import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.DoubleDocValuesField;
import org.apache.lucene.document.DoublePoint;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.document.FloatDocValuesField;
import org.apache.lucene.document.FloatPoint;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.index.VectorEncoding;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.Version;

/**
 * Writes a small, deterministic Apache Lucene Core 10.5.0 index for a given
 * document shape. The output directory is intended to be used as a reference
 * fixture for byte-for-byte portability tests against the Rucene Rust port.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... CodecIndexWriter &lt;output-dir&gt; &lt;shape&gt;
 * </pre>
 *
 * <p>Supported shapes:
 * <ul>
 *   <li>{@code text} - tokenized text fields plus a string doc-id field.</li>
 *   <li>{@code docvalues} - numeric, sorted, sorted-set and binary doc-values.</li>
 *   <li>{@code points} - int, long, float and double point fields.</li>
 *   <li>{@code vectors} - float KNN vector fields.</li>
 *   <li>{@code stored} - stored-only fields.</li>
 *   <li>{@code termvectors} - text fields with term vectors enabled.</li>
 *   <li>{@code postings} - deterministic tokenized body field with positions and offsets, exposed as separate postings files.</li>
 * </ul>
 */
public final class CodecIndexWriter {

  private CodecIndexWriter() {}

  public static void main(String[] args) {
    if (args.length != 2) {
      System.err.println("Usage: CodecIndexWriter <output-dir> <shape>");
      System.err.println("Supported shapes: text, docvalues, points, vectors, stored, termvectors, postings");
      System.exit(1);
    }

    Path outputDir = Paths.get(args[0]);
    String shape = args[1];

    try {
      Files.createDirectories(outputDir);

      Analyzer analyzer = shape.equals("postings") ? new WhitespaceAnalyzer() : new StandardAnalyzer();
      IndexWriterConfig config = new IndexWriterConfig(analyzer);
      config.setCodec(new Lucene104Codec());
      config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      // Disable the randomized merge policy so produced indexes are reproducible.
      config.setMergePolicy(org.apache.lucene.index.NoMergePolicy.INSTANCE);
      // The postings and points shapes must expose individual files for byte-for-byte
      // portability tests; compound files hide those extensions inside .cfs/.cfe.
      if (shape.equals("postings") || shape.equals("points")) {
        config.setUseCompoundFile(false);
      }

      try (FSDirectory dir = FSDirectory.open(outputDir);
           IndexWriter writer = new IndexWriter(dir, config)) {
        writeDocuments(writer, shape);
        writer.commit();
      }

      System.out.println("shape=" + shape);
      System.out.println("version=" + Version.LATEST);
      System.out.println("codec=Lucene104Codec");
      System.out.println("output_dir=" + outputDir.toAbsolutePath());
    } catch (Exception e) {
      e.printStackTrace();
      System.exit(2);
    }
  }

  private static void writeDocuments(IndexWriter writer, String shape) throws IOException {
    switch (shape) {
      case "text" -> writeTextDocuments(writer);
      case "docvalues" -> writeDocValuesDocuments(writer);
      case "points" -> writePointsDocuments(writer);
      case "vectors" -> writeVectorDocuments(writer);
      case "stored" -> writeStoredDocuments(writer);
      case "termvectors" -> writeTermVectorDocuments(writer);
      case "postings" -> writePostingsDocuments(writer);
      default -> throw new IllegalArgumentException("Unknown shape: " + shape);
    }
  }

  private static void writeTextDocuments(IndexWriter writer) throws IOException {
    String[] bodies = {
      "the quick brown fox jumps over the lazy dog",
      "lucene is a full text search library",
      "rust provides memory safety without garbage collection",
      "portability tests compare bytes produced by two implementations",
      "deterministic content is essential for reproducible fixtures"
    };
    for (int i = 0; i < bodies.length; i++) {
      Document doc = new Document();
      doc.add(new StringField("id", "text-" + i, Field.Store.YES));
      doc.add(new TextField("title", "Document " + i, Field.Store.YES));
      doc.add(new TextField("body", bodies[i], Field.Store.NO));
      writer.addDocument(doc);
    }
  }

  private static void writeDocValuesDocuments(IndexWriter writer) throws IOException {
    for (int i = 0; i < 7; i++) {
      Document doc = new Document();
      doc.add(new StringField("id", "dv-" + i, Field.Store.YES));
      doc.add(new NumericDocValuesField("numeric", 100L + i));
      doc.add(new SortedNumericDocValuesField("sorted_numeric", 10L * i));
      doc.add(new FloatDocValuesField("float", 1.0f + i * 0.1f));
      doc.add(new DoubleDocValuesField("double", 2.0d + i * 0.01d));
      doc.add(new SortedDocValuesField("sorted", new BytesRef("value-" + i)));
      doc.add(new SortedSetDocValuesField("sorted_set", new BytesRef("set-" + (i % 3))));
      doc.add(new BinaryDocValuesField("binary", new BytesRef("binary-" + i)));
      writer.addDocument(doc);
    }
  }

  private static void writePointsDocuments(IndexWriter writer) throws IOException {
    for (int i = 0; i < 6; i++) {
      Document doc = new Document();
      doc.add(new StringField("id", "point-" + i, Field.Store.YES));
      doc.add(new IntPoint("int_point", i, i * 2));
      doc.add(new LongPoint("long_point", 1000L + i, 2000L + i));
      doc.add(new FloatPoint("float_point", 0.5f * i, 1.5f * i));
      doc.add(new DoublePoint("double_point", 0.05d * i, 0.15d * i));
      writer.addDocument(doc);
    }
  }

  private static void writeVectorDocuments(IndexWriter writer) throws IOException {
    VectorFieldType vectorType = new VectorFieldType();
    for (int i = 0; i < 5; i++) {
      Document doc = new Document();
      doc.add(new StringField("id", "vec-" + i, Field.Store.YES));
      float f = i + 1;
      float[] vector = new float[] {0.1f * f, 0.2f * f, 0.3f * f, 0.4f * f};
      doc.add(new KnnFloatVectorField("vector", vector, vectorType));
      writer.addDocument(doc);
    }
  }

  private static void writeStoredDocuments(IndexWriter writer) throws IOException {
    String[] values = {
      "alpha", "beta", "gamma", "delta", "epsilon"
    };
    for (int i = 0; i < values.length; i++) {
      Document doc = new Document();
      doc.add(new StringField("id", "stored-" + i, Field.Store.YES));
      doc.add(new StoredField("value", values[i]));
      doc.add(new StoredField("number", i));
      writer.addDocument(doc);
    }
  }

  private static void writeTermVectorDocuments(IndexWriter writer) throws IOException {
    FieldType tvType = new FieldType(TextField.TYPE_STORED);
    tvType.setStoreTermVectors(true);
    tvType.setStoreTermVectorPositions(true);
    tvType.setStoreTermVectorOffsets(true);
    tvType.freeze();

    String[] bodies = {
      "term vectors store per document token information",
      "offsets positions and payloads support advanced analysis",
      "fixtures must exercise every enabled term vector flag"
    };
    for (int i = 0; i < bodies.length; i++) {
      Document doc = new Document();
      doc.add(new StringField("id", "tv-" + i, Field.Store.YES));
      doc.add(new Field("body", bodies[i], tvType));
      writer.addDocument(doc);
    }
  }

  private static void writePostingsDocuments(IndexWriter writer) throws IOException {
    FieldType bodyType = new FieldType();
    bodyType.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    bodyType.setTokenized(true);
    bodyType.setStored(false);
    bodyType.freeze();

    String[] bodies = {
      "a b c d e f g",
      "a a b b c c d",
      "x y z",
      "a b x y",
      "one two three four five"
    };
    for (int i = 0; i < bodies.length; i++) {
      Document doc = new Document();
      doc.add(new StringField("id", "postings-" + i, Field.Store.YES));
      doc.add(new Field("body", bodies[i], bodyType));
      writer.addDocument(doc);
    }
  }

  /** Reusable {@link FieldType} for KNN float vector fields. */
  private static final class VectorFieldType extends FieldType {
    VectorFieldType() {
      setVectorAttributes(4, VectorEncoding.FLOAT32, VectorSimilarityFunction.COSINE);
      freeze();
    }
  }
}
