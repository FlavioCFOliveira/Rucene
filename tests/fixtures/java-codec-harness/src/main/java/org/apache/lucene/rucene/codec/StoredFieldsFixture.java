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

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.DoubleField;
import org.apache.lucene.document.FloatField;
import org.apache.lucene.document.IntField;
import org.apache.lucene.document.KeywordField;
import org.apache.lucene.document.LongField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.StoredFieldVisitor;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Writes a single-segment Apache Lucene Core 10.5.0 index whose content is
 * entirely made of stored fields, so that the resulting {@code .fdt},
 * {@code .fdx} and {@code .fdm} files depend only on the stored-fields
 * consumer and on the stored-fields codec.
 *
 * <p>Every document is built from a deterministic script that the Rust
 * portability test mirrors exactly, field for field and value for value, in
 * the same order. Because the field order fixes the field numbers and the
 * value types fix the type bits written into the {@code .fdt} stream, any byte
 * difference between the two indexes can only come from the stored-fields
 * consumer or from the compressing stored-fields codec.
 *
 * <p>The tool prints the segment name and the hexadecimal segment id of the
 * committed segment, both of which are baked into the file headers, so that
 * the Rust side can reuse them and make a byte-for-byte comparison meaningful.
 * It also prints, for every document, the values a
 * {@link StoredFieldVisitor} sees when reading the index back, so the Rust
 * side can assert that it decodes the very same values Lucene does.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... StoredFieldsFixture &lt;output-dir&gt; &lt;case&gt;
 * </pre>
 *
 * <p>Supported cases: {@code strings}, {@code numbers}, {@code binary},
 * {@code mixed}, {@code empties} and {@code chunks}.
 */
public final class StoredFieldsFixture {

  private StoredFieldsFixture() {}

  /** The indexed-and-stored field the {@code mixed} case uses. */
  static final String INDEXED_FIELD = "body";

  public static void main(String[] args) {
    if (args.length < 2 || args.length > 3) {
      System.err.println("Usage: StoredFieldsFixture <output-dir> <case> [mode]");
      System.err.println(
          "Supported cases: strings, numbers, binary, mixed, empties, chunks, sliced, floats, types, cfs, redundant");
      System.err.println("Supported modes: BEST_SPEED (default), BEST_COMPRESSION");
      System.exit(1);
    }

    Path outputDir = Paths.get(args[0]);
    String caseName = args[1];
    Lucene104Codec.Mode mode =
        args.length == 3
            ? Lucene104Codec.Mode.valueOf(args[2])
            : Lucene104Codec.Mode.BEST_SPEED;

    try {
      Files.createDirectories(outputDir);

      Analyzer analyzer = new WhitespaceAnalyzer();
      IndexWriterConfig config = new IndexWriterConfig(analyzer);
      config.setCodec(new Lucene104Codec(mode));
      config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      // The `cfs` case is the only one that bundles the segment into a
      // compound file; every other case compares the loose `.fdt/.fdx/.fdm`.
      config.setUseCompoundFile(caseName.equals("cfs"));
      // One segment, flushed once: the byte comparison needs a single, fully
      // deterministic stored-fields stream.
      config.setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH);
      config.setRAMBufferSizeMB(512.0);

      List<Document> documents = documents(caseName, mode);

      try (FSDirectory dir = FSDirectory.open(outputDir);
          IndexWriter writer = new IndexWriter(dir, config)) {
        for (Document doc : documents) {
          writer.addDocument(doc);
        }
        writer.commit();
      }

      try (FSDirectory dir = FSDirectory.open(outputDir)) {
        SegmentInfos infos = SegmentInfos.readLatestCommit(dir);
        if (infos.size() != 1) {
          throw new IllegalStateException("expected exactly one segment, got " + infos.size());
        }
        SegmentCommitInfo commit = infos.info(0);
        System.out.println("case=" + caseName);
        System.out.println("segment=" + commit.info.name);
        System.out.println("segment_id=" + hex(commit.info.getId()));
        System.out.println("max_doc=" + commit.info.maxDoc());
        System.out.println("compound=" + commit.info.getUseCompoundFile());
        System.out.println("mode=" + mode);
        System.out.println("output_dir=" + outputDir.toAbsolutePath());
      }

      // Read the index back through a recording visitor and print what Lucene
      // decodes, one line per document.
      try (FSDirectory dir = FSDirectory.open(outputDir);
          org.apache.lucene.index.DirectoryReader reader =
              org.apache.lucene.index.DirectoryReader.open(dir)) {
        org.apache.lucene.index.StoredFields storedFields = reader.storedFields();
        for (int docID = 0; docID < reader.maxDoc(); docID++) {
          RecordingVisitor visitor = new RecordingVisitor();
          storedFields.document(docID, visitor);
          System.out.println("doc " + docID + " " + String.join("|", visitor.seen));
        }
      }
    } catch (Exception e) {
      e.printStackTrace();
      System.exit(2);
    }
  }

  /** Returns the scripted documents of a case. */
  static List<Document> documents(String caseName, Lucene104Codec.Mode mode) {
    return switch (caseName) {
      case "strings" -> stringDocuments();
      case "numbers" -> numberDocuments();
      case "binary" -> binaryDocuments();
      case "mixed" -> mixedDocuments();
      case "empties" -> emptyDocuments();
      case "chunks" -> chunkDocuments();
      case "sliced" -> slicedDocuments(mode);
      case "redundant" -> redundantDocuments();
      case "floats" -> floatDocuments();
      case "types", "cfs" -> typedDocuments();
      default -> throw new IllegalArgumentException("Unknown case: " + caseName);
    };
  }

  /**
   * Strings: a plain value, a single-field document, a document with no stored
   * field at all, an empty string, non-ASCII text including a surrogate pair,
   * and a multi-valued field.
   */
  static List<Document> stringDocuments() {
    List<Document> docs = new ArrayList<>();

    Document d0 = new Document();
    d0.add(new StoredField("title", "alpha"));
    d0.add(new StoredField("body", "the quick brown fox"));
    docs.add(d0);

    Document d1 = new Document();
    d1.add(new StoredField("title", "beta"));
    docs.add(d1);

    docs.add(new Document());

    Document d3 = new Document();
    d3.add(new StoredField("title", ""));
    d3.add(new StoredField("body", "ünïcödé ☃ 😀"));
    docs.add(d3);

    Document d4 = new Document();
    d4.add(new StoredField("title", "gamma"));
    d4.add(new StoredField("title", "delta"));
    d4.add(new StoredField("body", "epsilon"));
    docs.add(d4);

    return docs;
  }

  /**
   * Numbers: zero, the extremes of every numeric type, negative zero, and the
   * three timestamp encodings the {@code TLong} format special-cases (second,
   * hour and day precision).
   */
  static List<Document> numberDocuments() {
    List<Document> docs = new ArrayList<>();

    Document d0 = new Document();
    d0.add(new StoredField("i", 0));
    d0.add(new StoredField("l", 0L));
    d0.add(new StoredField("f", 0.0f));
    d0.add(new StoredField("d", 0.0d));
    docs.add(d0);

    Document d1 = new Document();
    d1.add(new StoredField("i", Integer.MAX_VALUE));
    d1.add(new StoredField("l", Long.MAX_VALUE));
    d1.add(new StoredField("f", Float.MAX_VALUE));
    d1.add(new StoredField("d", Double.MAX_VALUE));
    docs.add(d1);

    Document d2 = new Document();
    d2.add(new StoredField("i", Integer.MIN_VALUE));
    d2.add(new StoredField("l", Long.MIN_VALUE));
    d2.add(new StoredField("f", -0.0f));
    d2.add(new StoredField("d", -0.0d));
    docs.add(d2);

    Document d3 = new Document();
    d3.add(new StoredField("i", -1));
    d3.add(new StoredField("l", 86_400_000L));
    d3.add(new StoredField("f", (float) Math.PI));
    d3.add(new StoredField("d", Math.E));
    docs.add(d3);

    Document d4 = new Document();
    d4.add(new StoredField("l", 1_000L));
    d4.add(new StoredField("l", 3_600_000L));
    d4.add(new StoredField("l", 1L));
    d4.add(new StoredField("l", 4_611_686_018_427_387_904L));
    d4.add(new StoredField("l", -4_611_686_018_427_387_904L));
    d4.add(new StoredField("i", 125));
    d4.add(new StoredField("f", 1.0f));
    d4.add(new StoredField("d", 1.0d));
    docs.add(d4);

    return docs;
  }

  /** Binary: an empty value, every byte value, several values per document. */
  static List<Document> binaryDocuments() {
    List<Document> docs = new ArrayList<>();

    Document d0 = new Document();
    d0.add(new StoredField("blob", new byte[0]));
    docs.add(d0);

    byte[] allBytes = new byte[256];
    for (int i = 0; i < allBytes.length; i++) {
      allBytes[i] = (byte) i;
    }
    Document d1 = new Document();
    d1.add(new StoredField("blob", allBytes));
    docs.add(d1);

    Document d2 = new Document();
    d2.add(new StoredField("blob", new byte[] {1, 2, 3}));
    d2.add(new StoredField("blob", new BytesRef(new byte[] {4, 5})));
    docs.add(d2);

    docs.add(new Document());

    return docs;
  }

  /** Mixed: an indexed-and-stored field next to stored-only fields of every type. */
  static List<Document> mixedDocuments() {
    FieldType indexedAndStored = new FieldType();
    indexedAndStored.setStored(true);
    indexedAndStored.setTokenized(true);
    indexedAndStored.setOmitNorms(true);
    indexedAndStored.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    indexedAndStored.freeze();

    List<Document> docs = new ArrayList<>();

    Document d0 = new Document();
    d0.add(new Field(INDEXED_FIELD, "alpha beta gamma", indexedAndStored));
    d0.add(new StoredField("count", 3));
    d0.add(new StoredField("blob", new byte[] {9, 9, 9}));
    docs.add(d0);

    Document d1 = new Document();
    d1.add(new StoredField("count", -3));
    docs.add(d1);

    Document d2 = new Document();
    d2.add(new Field(INDEXED_FIELD, "delta", indexedAndStored));
    d2.add(new StoredField("ratio", 0.125d));
    d2.add(new StoredField("when", 1_700_000_000_000L));
    docs.add(d2);

    docs.add(new Document());

    Document d4 = new Document();
    d4.add(new Field(INDEXED_FIELD, "alpha delta", indexedAndStored));
    d4.add(new StoredField("blob", new byte[] {0}));
    d4.add(new StoredField("count", 1));
    d4.add(new StoredField("ratio", -1.5f));
    docs.add(d4);

    return docs;
  }

  /** Only empty documents: every frame of the segment is empty. */
  static List<Document> emptyDocuments() {
    List<Document> docs = new ArrayList<>();
    for (int i = 0; i < 7; i++) {
      docs.add(new Document());
    }
    return docs;
  }

  /**
   * Enough documents and bytes to push the stored-fields stream past a single
   * chunk, so the chunk framing, the block-shift index and the dirty-chunk
   * bookkeeping all take part in the comparison.
   */
  static List<Document> chunkDocuments() {
    List<Document> docs = new ArrayList<>();
    for (int i = 0; i < 1500; i++) {
      Document doc = new Document();
      StringBuilder text = new StringBuilder();
      for (int word = 0; word < 12; word++) {
        text.append("word").append((i + word) % 97).append(' ');
      }
      doc.add(new StoredField("text", text.toString()));
      doc.add(new StoredField("ord", i));
      if (i % 5 == 0) {
        doc.add(new StoredField("payload", ("payload-" + i).getBytes(StandardCharsets.UTF_8)));
      }
      docs.add(doc);
    }
    return docs;
  }

  /**
   * One document large enough to force a *sliced* chunk <b>in {@code mode}</b>.
   *
   * <p>The writer slices a chunk whenever the buffered bytes reach twice the
   * chunk size, and the chunk size is mode-dependent: {@code 10 * 8 * 1024} for
   * {@code BEST_SPEED} but {@code 10 * 48 * 1024} for {@code BEST_COMPRESSION}.
   * A payload sized for the former leaves the latter unsliced, so the size
   * follows the mode. Each slice is compressed independently and framed with
   * its own uncompressed length, which is what the reader has to honour.
   *
   * <p>A small document is written first so that the huge one starts at a
   * non-zero offset inside the chunk, and another small one follows so that a
   * second, unsliced chunk exists too.
   */
  static List<Document> slicedDocuments(Lucene104Codec.Mode mode) {
    int target = mode == Lucene104Codec.Mode.BEST_COMPRESSION ? 1_100_000 : 243_000;
    List<Document> docs = new ArrayList<>();

    Document before = new Document();
    before.add(new StoredField("tag", "before"));
    docs.add(before);

    StringBuilder huge = new StringBuilder(target);
    // Compressible but not trivially so, and long enough to span slices.
    for (int i = 0; huge.length() < target; i++) {
      huge.append("chunk").append(i % 1009).append('-').append((char) ('a' + (i % 26))).append(' ');
    }
    Document big = new Document();
    big.add(new StoredField("payload", huge.toString()));
    big.add(new StoredField("size", huge.length()));
    docs.add(big);

    Document after = new Document();
    after.add(new StoredField("tag", "after"));
    docs.add(after);

    return docs;
  }

  /**
   * Every boundary value of the {@code ZFloat} and {@code ZDouble} encodings.
   *
   * <p>The single-byte small-integer form covers {@code [-1..125]} for floats
   * and {@code [-1..124]} for doubles; {@code -1} is the one whose header byte
   * is exactly {@code 0x80}. Negative zero is excluded from that form on
   * purpose, and NaN and the infinities take the wide form.
   */
  static List<Document> floatDocuments() {
    List<Document> docs = new ArrayList<>();

    Document d0 = new Document();
    d0.add(new StoredField("f", -1.0f));
    d0.add(new StoredField("d", -1.0d));
    docs.add(d0);

    Document d1 = new Document();
    for (float value :
        new float[] {0.0f, -0.0f, 1.0f, 125.0f, 126.0f, -2.0f, Float.MIN_VALUE, Float.MIN_NORMAL}) {
      d1.add(new StoredField("f", value));
    }
    docs.add(d1);

    Document d2 = new Document();
    for (double value :
        new double[] {
          0.0d, -0.0d, 1.0d, 124.0d, 125.0d, -2.0d, Double.MIN_VALUE, Double.MIN_NORMAL
        }) {
      d2.add(new StoredField("d", value));
    }
    docs.add(d2);

    Document d3 = new Document();
    d3.add(new StoredField("f", Float.NaN));
    d3.add(new StoredField("f", Float.POSITIVE_INFINITY));
    d3.add(new StoredField("f", Float.NEGATIVE_INFINITY));
    d3.add(new StoredField("d", Double.NaN));
    d3.add(new StoredField("d", Double.POSITIVE_INFINITY));
    d3.add(new StoredField("d", Double.NEGATIVE_INFINITY));
    docs.add(d3);

    return docs;
  }

  /**
   * Highly redundant prose: the input {@code BEST_COMPRESSION} exists for.
   *
   * <p>`Lucene90StoredFieldsFormat` describes that mode as trading speed for
   * ratio, so a port whose ratio is materially worse than Lucene's has
   * regressed the only thing the mode offers. This corpus makes the difference
   * measurable: the same paragraph repeated compresses several hundred fold,
   * and any weakness in the match finder shows up immediately.
   */
  static List<Document> redundantDocuments() {
    String paragraph =
        "Apache Lucene is a high-performance, full-featured search engine library written entirely in Java. ";
    StringBuilder text = new StringBuilder(620_016);
    while (text.length() < 620_000) {
      text.append(paragraph);
    }
    List<Document> docs = new ArrayList<>();
    Document doc = new Document();
    doc.add(new StoredField("prose", text.toString()));
    docs.add(doc);
    return docs;
  }

  /**
   * One document per stored field class, so that the *type byte* each of them
   * writes into the `.fdt` stream is compared against Lucene's.
   */
  static List<Document> typedDocuments() {
    List<Document> docs = new ArrayList<>();

    Document d0 = new Document();
    d0.add(new StringField("s_string", "keyword-value", Field.Store.YES));
    d0.add(new TextField("s_text", "some analysed text", Field.Store.YES));
    d0.add(new KeywordField("s_kw_string", "kw-string", Field.Store.YES));
    d0.add(new KeywordField("s_kw_bytes", new BytesRef(new byte[] {1, 2, 3}), Field.Store.YES));
    d0.add(new IntField("s_int", 42, Field.Store.YES));
    d0.add(new LongField("s_long", 1_234_567_890_123L, Field.Store.YES));
    d0.add(new FloatField("s_float", 2.5f, Field.Store.YES));
    d0.add(new DoubleField("s_double", -2.5d, Field.Store.YES));
    d0.add(new StoredField("s_bytes", new byte[] {(byte) 0xF0, 0x0D}));
    d0.add(new StoredField("s_only", "stored-only"));
    docs.add(d0);

    Document d1 = new Document();
    d1.add(new StringField("s_string", "second", Field.Store.YES));
    d1.add(new IntField("s_int", -1, Field.Store.YES));
    docs.add(d1);

    return docs;
  }

  /**
   * Records what a visitor is handed, in the format the Rust test expects.
   *
   * <p>Values longer than {@link #DIGEST_THRESHOLD} UTF-8 bytes are reduced to
   * a length plus an FNV-1a 64 digest, so that a 243 KB stored field does not
   * turn the harness transcript into megabytes. FNV-1a is specified precisely
   * enough that both sides compute the same number with no shared library.
   */
  static final class RecordingVisitor extends StoredFieldVisitor {
    /** Values at or below this many UTF-8 bytes are printed verbatim. */
    static final int DIGEST_THRESHOLD = 64;

    final List<String> seen = new ArrayList<>();

    @Override
    public void binaryField(FieldInfo fieldInfo, byte[] value) {
      seen.add(fieldInfo.name + "=bin" + render(value));
    }

    @Override
    public void stringField(FieldInfo fieldInfo, String value) {
      seen.add(fieldInfo.name + "=str" + render(value.getBytes(StandardCharsets.UTF_8)));
    }

    @Override
    public void intField(FieldInfo fieldInfo, int value) {
      seen.add(fieldInfo.name + "=i32:" + value);
    }

    @Override
    public void longField(FieldInfo fieldInfo, long value) {
      seen.add(fieldInfo.name + "=i64:" + value);
    }

    @Override
    public void floatField(FieldInfo fieldInfo, float value) {
      seen.add(fieldInfo.name + "=f32:" + Integer.toHexString(Float.floatToRawIntBits(value)));
    }

    @Override
    public void doubleField(FieldInfo fieldInfo, double value) {
      seen.add(fieldInfo.name + "=f64:" + Long.toHexString(Double.doubleToRawLongBits(value)));
    }

    @Override
    public Status needsField(FieldInfo fieldInfo) {
      return Status.YES;
    }

    /** Either {@code :<hex bytes>} or {@code #<length>:<fnv1a64>}. */
    static String render(byte[] value) {
      if (value.length <= DIGEST_THRESHOLD) {
        return ":" + hex(value);
      }
      return "#" + value.length + ":" + String.format("%016x", fnv1a64(value));
    }

    /** FNV-1a, 64-bit, over the raw bytes. */
    static long fnv1a64(byte[] value) {
      long hash = 0xcbf29ce484222325L;
      for (byte b : value) {
        hash ^= (b & 0xFFL);
        hash *= 0x100000001b3L;
      }
      return hash;
    }
  }

  static String hex(byte[] bytes) {
    StringBuilder builder = new StringBuilder(bytes.length * 2);
    for (byte b : bytes) {
      builder.append(String.format("%02x", b));
    }
    return builder.toString();
  }
}
