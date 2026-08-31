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
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.analysis.tokenattributes.OffsetAttribute;
import org.apache.lucene.analysis.tokenattributes.PayloadAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.FieldInvertState;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.search.CollectionStatistics;
import org.apache.lucene.search.TermStatistics;
import org.apache.lucene.search.similarities.Similarity;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Writes a single-segment Apache Lucene Core 10.5.0 index from a fully scripted
 * token stream, so that the resulting postings files depend only on the
 * indexing chain and never on an analyzer.
 *
 * <p>Every document is built from a deterministic table of
 * {@code (term, positionIncrement, startOffset, endOffset, payload)} tuples
 * which the Rust portability test mirrors exactly. Because the tuples bypass
 * analysis entirely, a byte difference between the two indexes can only come
 * from the indexing chain or from the postings codec.
 *
 * <p>The tool prints the segment name and the hexadecimal segment id of the
 * committed segment. Those two values are baked into the file headers of the
 * postings files, so the Rust side reuses them to make a byte-for-byte
 * comparison meaningful.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... IndexingChainFixture &lt;output-dir&gt; &lt;case&gt;
 * </pre>
 *
 * <p>Supported cases: {@code docs}, {@code freqs}, {@code positions},
 * {@code offsets}, {@code payloads}, {@code multivalue}, {@code manyterms},
 * {@code emptyvalue}, {@code stats} and {@code statsmulti}.
 *
 * <p>The two {@code stats*} cases index with norms enabled and a recording
 * {@link Similarity}, so that every value Lucene's own indexing chain puts in
 * {@link FieldInvertState} is printed on stdout and can be compared with what
 * Rucene's chain computes. Their index files are not meant for byte
 * comparison.
 */
public final class IndexingChainFixture {

  private IndexingChainFixture() {}

  /** The name of the single indexed field of every case. */
  static final String FIELD = "body";

  /** One scripted token. */
  record Tok(String term, int posIncr, int start, int end, byte[] payload) {
    static Tok of(String term, int posIncr, int start, int end) {
      return new Tok(term, posIncr, start, end, null);
    }
  }

  public static void main(String[] args) {
    if (args.length != 2) {
      System.err.println("Usage: IndexingChainFixture <output-dir> <case>");
      System.err.println(
          "Supported cases: docs, freqs, positions, offsets, payloads, multivalue, manyterms, emptyvalue, stats, statsmulti");
      System.exit(1);
    }

    Path outputDir = Paths.get(args[0]);
    String caseName = args[1];

    try {
      Files.createDirectories(outputDir);

      boolean recordsStats = caseName.startsWith("stats");
      IndexOptions options = indexOptions(caseName);
      FieldType fieldType = new FieldType();
      fieldType.setTokenized(true);
      fieldType.setStored(false);
      // `Similarity.computeNorm(FieldInvertState)` is the only hook Lucene
      // offers onto the inversion statistics, and it only runs when the field
      // stores norms.
      fieldType.setOmitNorms(!recordsStats);
      fieldType.setIndexOptions(options);
      fieldType.freeze();

      Analyzer analyzer = new WhitespaceAnalyzer();
      IndexWriterConfig config = new IndexWriterConfig(analyzer);
      RecordingSimilarity similarity = new RecordingSimilarity();
      if (recordsStats) {
        config.setSimilarity(similarity);
      }
      config.setCodec(new Lucene104Codec());
      config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      config.setUseCompoundFile(false);

      List<List<List<Tok>>> documents = documents(caseName);

      try (FSDirectory dir = FSDirectory.open(outputDir);
          IndexWriter writer = new IndexWriter(dir, config)) {
        for (List<List<Tok>> values : documents) {
          Document doc = new Document();
          for (List<Tok> tokens : values) {
            doc.add(new Field(FIELD, new ScriptedTokenStream(tokens), fieldType));
          }
          writer.addDocument(doc);
        }
        writer.commit();
      }

      for (String record : similarity.records) {
        System.out.println(record);
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
        System.out.println("index_options=" + options);
        System.out.println("output_dir=" + outputDir.toAbsolutePath());
      }
    } catch (Exception e) {
      e.printStackTrace();
      System.exit(2);
    }
  }

  static IndexOptions indexOptions(String caseName) {
    return switch (caseName) {
      case "docs" -> IndexOptions.DOCS;
      case "freqs" -> IndexOptions.DOCS_AND_FREQS;
      case "positions", "payloads", "manyterms", "emptyvalue" ->
          IndexOptions.DOCS_AND_FREQS_AND_POSITIONS;
      case "offsets", "multivalue", "stats", "statsmulti" ->
          IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS;
      default -> throw new IllegalArgumentException("Unknown case: " + caseName);
    };
  }

  /** Returns the scripted documents of a case: document, then field value, then tokens. */
  static List<List<List<Tok>>> documents(String caseName) {
    return switch (caseName) {
      case "docs", "freqs", "positions", "offsets", "stats" -> baseDocuments();
      case "statsmulti" -> multiValueDocuments();
      case "payloads" -> payloadDocuments();
      case "multivalue" -> multiValueDocuments();
      case "manyterms" -> manyTermsDocuments();
      case "emptyvalue" -> emptyValueDocuments();
      default -> throw new IllegalArgumentException("Unknown case: " + caseName);
    };
  }

  /**
   * Six documents covering: a repeated term inside one document, a term present
   * in every document, terms unique to one document, a zero-increment overlap,
   * a position gap, and a document with no tokens at all.
   */
  static List<List<List<Tok>>> baseDocuments() {
    List<List<List<Tok>>> docs = new ArrayList<>();
    docs.add(List.of(List.of(
        Tok.of("alpha", 1, 0, 5),
        Tok.of("beta", 1, 6, 10),
        Tok.of("alpha", 1, 11, 16),
        Tok.of("gamma", 1, 17, 22))));
    docs.add(List.of(List.of(
        Tok.of("beta", 1, 0, 4),
        Tok.of("delta", 3, 10, 15),
        Tok.of("alpha", 1, 16, 21))));
    docs.add(List.of(List.of(
        Tok.of("gamma", 1, 0, 5),
        Tok.of("gamma", 0, 0, 5),
        Tok.of("epsilon", 2, 6, 13))));
    docs.add(List.of(List.of()));
    docs.add(List.of(List.of(
        Tok.of("alpha", 1, 0, 5),
        Tok.of("alpha", 1, 6, 11),
        Tok.of("alpha", 1, 12, 17),
        Tok.of("zeta", 1, 18, 22))));
    docs.add(List.of(List.of(
        Tok.of("beta", 1, 0, 4),
        Tok.of("gamma", 1, 5, 10),
        Tok.of("delta", 1, 11, 16),
        Tok.of("epsilon", 1, 17, 24),
        Tok.of("zeta", 1, 25, 29))));
    return docs;
  }

  /** Four documents where some tokens carry a payload and others do not. */
  static List<List<List<Tok>>> payloadDocuments() {
    List<List<List<Tok>>> docs = new ArrayList<>();
    docs.add(List.of(List.of(
        new Tok("alpha", 1, 0, 5, new byte[] {1}),
        new Tok("beta", 1, 6, 10, null),
        new Tok("alpha", 1, 11, 16, new byte[] {2, 3, 4}))));
    docs.add(List.of(List.of(
        new Tok("beta", 1, 0, 4, new byte[] {(byte) 0xFF, 0x00, 0x7F}),
        new Tok("gamma", 1, 5, 10, new byte[] {}))));
    docs.add(List.of(List.of(
        new Tok("alpha", 1, 0, 5, longPayload(40)),
        new Tok("gamma", 1, 6, 11, longPayload(7)))));
    docs.add(List.of(List.of(
        new Tok("delta", 1, 0, 5, null),
        new Tok("delta", 1, 6, 11, new byte[] {9, 9}))));
    return docs;
  }

  /** Three documents whose field is repeated, so the offset and position gaps apply. */
  static List<List<List<Tok>>> multiValueDocuments() {
    List<List<List<Tok>>> docs = new ArrayList<>();
    docs.add(List.of(
        List.of(Tok.of("alpha", 1, 0, 5), Tok.of("beta", 1, 6, 10)),
        List.of(Tok.of("alpha", 1, 0, 5), Tok.of("gamma", 1, 6, 11))));
    docs.add(List.of(
        List.of(Tok.of("delta", 1, 0, 5)),
        List.of(Tok.of("delta", 1, 0, 5)),
        List.of(Tok.of("epsilon", 1, 0, 7))));
    docs.add(List.of(
        List.of(Tok.of("beta", 1, 0, 4)),
        List.of()));
    return docs;
  }

  /**
   * Enough documents and terms to push the terms dictionary past a single block
   * and the postings past the 128-document block size.
   */
  static List<List<List<Tok>>> manyTermsDocuments() {
    List<List<List<Tok>>> docs = new ArrayList<>();
    for (int docId = 0; docId < 200; docId++) {
      List<Tok> tokens = new ArrayList<>();
      int offset = 0;
      for (int term = 0; term < 60; term++) {
        if ((docId + term) % 3 != 0) {
          continue;
        }
        String text = String.format("term%04d", term);
        int repeats = (term % 3) + 1;
        for (int r = 0; r < repeats; r++) {
          tokens.add(Tok.of(text, 1, offset, offset + text.length()));
          offset += text.length() + 1;
        }
      }
      docs.add(List.of(tokens));
    }
    return docs;
  }

  /** Documents that mix empty values, a single term and a repeated term. */
  static List<List<List<Tok>>> emptyValueDocuments() {
    List<List<List<Tok>>> docs = new ArrayList<>();
    docs.add(List.of(List.of()));
    docs.add(List.of(List.of(Tok.of("solo", 1, 0, 4))));
    docs.add(List.of(List.of()));
    docs.add(List.of(List.of(Tok.of("solo", 1, 0, 4), Tok.of("solo", 1, 5, 9))));
    docs.add(List.of(List.of()));
    return docs;
  }

  static byte[] longPayload(int length) {
    byte[] payload = new byte[length];
    for (int i = 0; i < length; i++) {
      payload[i] = (byte) (i * 7 + 1);
    }
    return payload;
  }

  static String hex(byte[] bytes) {
    StringBuilder builder = new StringBuilder(bytes.length * 2);
    for (byte b : bytes) {
      builder.append(String.format("%02x", b));
    }
    return builder.toString();
  }

  /**
   * Records the inversion statistics of every field instance Lucene norms.
   *
   * `computeNorm` runs once per document per indexed field with norms, right
   * after every value of that field has been inverted, which is exactly the
   * point at which Rucene's chain exposes its own `FieldInvertState`.
   */
  static final class RecordingSimilarity extends Similarity {
    final List<String> records = new ArrayList<>();

    @Override
    public long computeNorm(FieldInvertState state) {
      records.add(
          String.format(
              "invert_state field=%s length=%d numOverlap=%d uniqueTermCount=%d maxTermFrequency=%d position=%d offset=%d",
              state.getName(),
              state.getLength(),
              state.getNumOverlap(),
              state.getUniqueTermCount(),
              state.getMaxTermFrequency(),
              state.getPosition(),
              state.getOffset()));
      return 1L;
    }

    @Override
    public SimScorer scorer(
        float boost, CollectionStatistics collectionStats, TermStatistics... termStats) {
      throw new UnsupportedOperationException("the fixture never scores");
    }
  }

  /** Emits a fixed list of tokens, bypassing analysis completely. */
  static final class ScriptedTokenStream extends TokenStream {
    private final List<Tok> tokens;
    private final CharTermAttribute termAtt = addAttribute(CharTermAttribute.class);
    private final OffsetAttribute offsetAtt = addAttribute(OffsetAttribute.class);
    private final PositionIncrementAttribute posIncrAtt =
        addAttribute(PositionIncrementAttribute.class);
    private final PayloadAttribute payloadAtt = addAttribute(PayloadAttribute.class);
    private int upto;
    private int finalOffset;

    ScriptedTokenStream(List<Tok> tokens) {
      this.tokens = tokens;
    }

    @Override
    public boolean incrementToken() {
      if (upto == tokens.size()) {
        return false;
      }
      clearAttributes();
      Tok token = tokens.get(upto++);
      termAtt.append(token.term());
      posIncrAtt.setPositionIncrement(token.posIncr());
      offsetAtt.setOffset(token.start(), token.end());
      payloadAtt.setPayload(token.payload() == null ? null : new BytesRef(token.payload()));
      finalOffset = token.end();
      return true;
    }

    @Override
    public void reset() throws IOException {
      super.reset();
      upto = 0;
      finalOffset = 0;
    }

    @Override
    public void end() throws IOException {
      super.end();
      offsetAtt.setOffset(finalOffset, finalOffset);
    }
  }
}
