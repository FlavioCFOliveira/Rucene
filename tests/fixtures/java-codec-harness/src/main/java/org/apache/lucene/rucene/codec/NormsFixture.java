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

import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInvertState;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.search.CollectionStatistics;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.TermStatistics;
import org.apache.lucene.search.similarities.BM25Similarity;
import org.apache.lucene.search.similarities.Similarity;
import org.apache.lucene.store.FSDirectory;

import org.apache.lucene.rucene.codec.IndexingChainFixture.ScriptedTokenStream;
import org.apache.lucene.rucene.codec.IndexingChainFixture.Tok;

/**
 * Writes a single-segment Apache Lucene Core 10.5.0 index whose only interesting
 * content is norms, so that the resulting {@code .nvd} and {@code .nvm} files
 * depend only on {@code NormValuesWriter}, on {@code Similarity.computeNorm}
 * and on {@code Lucene90NormsFormat}.
 *
 * <p>Every field value is a fully scripted table of
 * {@code (term, positionIncrement, startOffset, endOffset)} tuples which the
 * Rust portability test mirrors exactly, so no analyzer takes part: a byte
 * difference can only come from the norms writer or from the norms codec. The
 * field order inside a document fixes the field numbers, which order the
 * {@code .nvm} entries.
 *
 * <p>The cases are chosen to span the shape dimensions of the format rather
 * than only its values:
 *
 * <ul>
 *   <li>{@code dense} — every document carries the field, so the metadata says
 *       "all documents" and no docs-with-field stream is written at all;
 *   <li>{@code sparse} — few enough documents carry it that {@code IndexedDISI}
 *       uses its SPARSE (short-per-doc) block encoding;
 *   <li>{@code disidense} — enough documents carry it, inside one 65536-document
 *       block, that {@code IndexedDISI} switches to its DENSE (bitmap plus rank
 *       table) encoding;
 *   <li>{@code disiall} — a whole 65536-document block carries it while the
 *       segment as a whole does not, which is the ALL encoding;
 *   <li>{@code omitnorms} — every field omits norms, so no norms file exists;
 *   <li>{@code mixedomit} — one field omits norms next to two that do not;
 *   <li>{@code docsonly} — {@code IndexOptions.DOCS}, where the norm counts
 *       unique terms instead of tokens;
 *   <li>{@code overlaps} — tokens with a position increment of zero, which the
 *       default {@code discountOverlaps} subtracts from the length;
 *   <li>{@code nodiscount} — the same tokens with {@code discountOverlaps}
 *       turned off;
 *   <li>{@code multivalue} — a multi-valued field whose length accumulates
 *       across its values;
 *   <li>{@code emptyvalue} — a field present in a document with no tokens at
 *       all, whose norm is zero;
 *   <li>{@code constant} — every document produces the same norm, so the value
 *       is stored once in the metadata and the data file holds none;
 *   <li>{@code wide2}, {@code wide4}, {@code wide8} — custom similarities whose
 *       norms need two, four and eight bytes each. Lucene's own
 *       {@code computeNorm} always returns a signed byte, so these three widths
 *       of the format are unreachable without one;
 *   <li>{@code cfs} — the same content bundled into a compound file.
 * </ul>
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... NormsFixture &lt;output-dir&gt; &lt;case&gt;
 * </pre>
 */
public final class NormsFixture {

  private NormsFixture() {}

  /** The settings of one field, fixed for a whole case. */
  record Spec(String name, IndexOptions options, boolean omitNorms) {}

  /** One value of one field of one document. */
  record Val(int spec, List<Tok> tokens) {}

  /** A similarity whose norm is the field length times a fixed factor. */
  static final class ScaledSimilarity extends Similarity {
    private final long factor;

    ScaledSimilarity(long factor) {
      this.factor = factor;
    }

    @Override
    public long computeNorm(FieldInvertState state) {
      return state.getLength() * factor;
    }

    @Override
    public SimScorer scorer(
        float boost, CollectionStatistics collectionStats, TermStatistics... termStats) {
      throw new UnsupportedOperationException("the fixture never scores");
    }

    @Override
    public String toString() {
      return "Scaled(" + factor + ")";
    }
  }

  public static void main(String[] args) {
    if (args.length != 2) {
      System.err.println("Usage: NormsFixture <output-dir> <case>");
      System.err.println(
          "Supported cases: dense, sparse, disidense, disiall, omitnorms, mixedomit, docsonly, "
              + "overlaps, nodiscount, multivalue, emptyvalue, constant, wide2, wide4, wide8, cfs");
      System.exit(1);
    }

    Path outputDir = Paths.get(args[0]);
    String caseName = args[1];

    try {
      Files.createDirectories(outputDir);

      List<Spec> specs = specs(caseName);
      List<FieldType> types = new ArrayList<>();
      for (Spec spec : specs) {
        FieldType type = new FieldType();
        type.setTokenized(true);
        type.setStored(false);
        type.setOmitNorms(spec.omitNorms());
        type.setIndexOptions(spec.options());
        type.setStoreTermVectors(false);
        type.freeze();
        types.add(type);
      }

      Analyzer analyzer = new WhitespaceAnalyzer();
      IndexWriterConfig config = new IndexWriterConfig(analyzer);
      config.setCodec(new Lucene104Codec());
      config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      config.setUseCompoundFile(caseName.equals("cfs"));
      // One segment, flushed once: the byte comparison needs a single, fully
      // deterministic norms stream.
      config.setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH);
      config.setRAMBufferSizeMB(512.0);
      config.setSimilarity(similarity(caseName));

      List<List<Val>> documents = documents(caseName);

      try (FSDirectory dir = FSDirectory.open(outputDir);
          IndexWriter writer = new IndexWriter(dir, config)) {
        for (List<Val> values : documents) {
          Document doc = new Document();
          for (Val value : values) {
            doc.add(
                new Field(
                    specs.get(value.spec()).name(),
                    new ScriptedTokenStream(value.tokens()),
                    types.get(value.spec())));
          }
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
        System.out.println("segment_id=" + IndexingChainFixture.hex(commit.info.getId()));
        System.out.println("max_doc=" + commit.info.maxDoc());
        System.out.println("compound=" + commit.info.getUseCompoundFile());

        try (DirectoryReader reader = DirectoryReader.open(dir)) {
          for (LeafReaderContext leaf : reader.leaves()) {
            for (FieldInfo fi : leaf.reader().getFieldInfos()) {
              System.out.println(
                  "fieldinfo="
                      + fi.number
                      + " "
                      + fi.name
                      + " omitNorms="
                      + fi.omitsNorms()
                      + " hasNorms="
                      + fi.hasNorms()
                      + " indexOptions="
                      + fi.getIndexOptions());
            }
            System.out.println(
                "hasnorms=" + leaf.reader().getFieldInfos().hasNorms());
            for (String line : dump(leaf.reader())) {
              System.out.println(line);
            }
          }
        }
        System.out.println("read_ok=true");
      }
    } catch (Exception e) {
      e.printStackTrace();
      System.exit(2);
    }
  }

  /**
   * Renders every norm of every field as one line, in field-number order and
   * then document order, so that the Rust side can compare plain strings.
   *
   * <p>A field with no norms at all prints {@code nonorms=<field>}; a field
   * whose norms exist but skip a document prints nothing for that document,
   * because "absent" is a value the format can express and the reader must
   * reproduce it.
   */
  static List<String> dump(LeafReader reader) throws IOException {
    List<String> lines = new ArrayList<>();
    for (FieldInfo fi : reader.getFieldInfos()) {
      NumericDocValues norms = reader.getNormValues(fi.name);
      if (norms == null) {
        lines.add("nonorms=" + fi.name);
        continue;
      }
      for (int doc = norms.nextDoc();
          doc != DocIdSetIterator.NO_MORE_DOCS;
          doc = norms.nextDoc()) {
        lines.add("norm=" + doc + " " + fi.name + " " + norms.longValue());
      }
    }
    return lines;
  }

  /** The similarity a case indexes with. */
  static Similarity similarity(String caseName) {
    return switch (caseName) {
      case "nodiscount" -> new BM25Similarity(false);
      case "wide2" -> new ScaledSimilarity(300L);
      case "wide4" -> new ScaledSimilarity(1_000_000L);
      case "wide8" -> new ScaledSimilarity(1_000_000_000_000L);
      default -> new BM25Similarity();
    };
  }

  static List<Spec> specs(String caseName) {
    return switch (caseName) {
      case "omitnorms" ->
          List.of(
              new Spec("body", IndexOptions.DOCS_AND_FREQS_AND_POSITIONS, true),
              new Spec("title", IndexOptions.DOCS_AND_FREQS, true));
      case "mixedomit" ->
          List.of(
              new Spec("body", IndexOptions.DOCS_AND_FREQS_AND_POSITIONS, false),
              new Spec("skipped", IndexOptions.DOCS_AND_FREQS, true),
              new Spec("title", IndexOptions.DOCS_AND_FREQS, false));
      case "docsonly" ->
          List.of(
              new Spec("body", IndexOptions.DOCS, false),
              new Spec("title", IndexOptions.DOCS_AND_FREQS, false));
      default ->
          List.of(
              new Spec("body", IndexOptions.DOCS_AND_FREQS_AND_POSITIONS, false),
              new Spec("title", IndexOptions.DOCS_AND_FREQS, false));
    };
  }

  private static Tok t(String term, int posIncr, int start, int end) {
    return Tok.of(term, posIncr, start, end);
  }

  /** A value of `spec` with `count` distinct single-character-ish terms. */
  private static Val words(int spec, String prefix, int count) {
    List<Tok> tokens = new ArrayList<>();
    for (int i = 0; i < count; i++) {
      String term = prefix + i;
      tokens.add(t(term, 1, i * 6, i * 6 + term.length()));
    }
    return new Val(spec, tokens);
  }

  static List<List<Val>> documents(String caseName) {
    List<List<Val>> documents = new ArrayList<>();
    switch (caseName) {
      case "dense", "cfs", "wide2", "wide4", "wide8" -> {
        // Both fields in every document, with lengths that differ enough for
        // the encoder to produce several distinct norms.
        for (int doc = 0; doc < 12; doc++) {
          documents.add(
              List.of(words(0, "a", 1 + doc * 3), words(1, "b", 1 + (doc % 4))));
        }
      }
      case "sparse" -> {
        // `body` in every third document, `title` in every document: one sparse
        // field beside one that is all-documents, inside the same segment.
        for (int doc = 0; doc < 40; doc++) {
          List<Val> values = new ArrayList<>();
          if (doc % 3 == 0) {
            values.add(words(0, "a", 1 + doc));
          }
          values.add(words(1, "b", 1 + (doc % 7)));
          documents.add(values);
        }
      }
      case "disidense" -> {
        // 5000 of 10000 documents carry `body`, which is more than the 4095
        // entries `IndexedDISI` will store as shorts, so its block switches to
        // the bitmap-plus-rank-table encoding.
        for (int doc = 0; doc < 10_000; doc++) {
          List<Val> values = new ArrayList<>();
          if (doc % 2 == 0) {
            values.add(words(0, "a", 1 + (doc % 11)));
          }
          values.add(words(1, "b", 1 + (doc % 5)));
          documents.add(values);
        }
      }
      case "disiall" -> {
        // Exactly one whole 65536-document block carries `body`, so that block
        // is written with the ALL encoding — no bitmap, no shorts — while the
        // field is still not an all-documents field for the segment.
        for (int doc = 0; doc < 65_536 + 64; doc++) {
          List<Val> values = new ArrayList<>();
          if (doc < 65_536) {
            values.add(words(0, "a", 1 + (doc % 3)));
          }
          values.add(words(1, "b", 1 + (doc % 2)));
          documents.add(values);
        }
      }
      case "omitnorms", "mixedomit" -> {
        for (int doc = 0; doc < 8; doc++) {
          List<Val> values = new ArrayList<>();
          values.add(words(0, "a", 1 + doc));
          values.add(words(1, "b", 1 + (doc % 3)));
          if (caseName.equals("mixedomit")) {
            values.add(words(2, "c", 1 + (doc % 5)));
          }
          documents.add(values);
        }
      }
      case "docsonly" -> {
        // Repeated terms: with `IndexOptions.DOCS` the norm counts unique terms,
        // so the repetition must not change it, while the `title` field beside
        // it counts every token.
        for (int doc = 0; doc < 10; doc++) {
          List<Tok> repeated = new ArrayList<>();
          for (int i = 0; i < 1 + doc; i++) {
            repeated.add(t("a" + (i % 3), 1, i * 4, i * 4 + 2));
          }
          documents.add(List.of(new Val(0, repeated), words(1, "b", 1 + doc)));
        }
      }
      case "overlaps", "nodiscount" -> {
        // Every other token is an overlap (position increment zero), the way a
        // synonym filter emits them.
        for (int doc = 0; doc < 10; doc++) {
          List<Tok> tokens = new ArrayList<>();
          int offset = 0;
          for (int i = 0; i < 1 + doc; i++) {
            tokens.add(t("a" + i, 1, offset, offset + 2));
            tokens.add(t("syn" + i, 0, offset, offset + 2));
            offset += 4;
          }
          documents.add(List.of(new Val(0, tokens), words(1, "b", 1 + doc)));
        }
      }
      case "multivalue" -> {
        // Three values of `body` per document: the length accumulates across
        // them, so the norm reflects the sum and not the last value.
        for (int doc = 0; doc < 10; doc++) {
          documents.add(
              List.of(
                  words(0, "a", 1 + doc),
                  words(0, "b", 2),
                  words(0, "c", 1 + (doc % 3)),
                  words(1, "d", 1 + (doc % 4))));
        }
      }
      case "emptyvalue" -> {
        // Documents 2, 5 and 8 carry `body` with no tokens at all: present but
        // empty, whose norm is zero and which must not be confused with absent.
        for (int doc = 0; doc < 10; doc++) {
          List<Val> values = new ArrayList<>();
          if (doc % 3 == 2) {
            values.add(new Val(0, List.of()));
          } else if (doc % 3 == 1) {
            values.add(words(0, "a", 1 + doc));
          }
          values.add(words(1, "b", 1 + (doc % 4)));
          documents.add(values);
        }
      }
      case "constant" -> {
        // Every document produces the same length, so every norm is equal and
        // the format stores the single value in the metadata.
        for (int doc = 0; doc < 10; doc++) {
          documents.add(List.of(words(0, "a", 4), words(1, "b", 4)));
        }
      }
      default -> throw new IllegalArgumentException("unknown case: " + caseName);
    }
    return documents;
  }
}
