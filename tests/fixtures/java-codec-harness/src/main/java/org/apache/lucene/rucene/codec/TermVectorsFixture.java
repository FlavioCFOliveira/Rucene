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
import org.apache.lucene.index.Fields;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.index.TermVectors;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

import org.apache.lucene.rucene.codec.IndexingChainFixture.ScriptedTokenStream;
import org.apache.lucene.rucene.codec.IndexingChainFixture.Tok;

/**
 * Writes a single-segment Apache Lucene Core 10.5.0 index whose only content is
 * term vectors, so that the resulting {@code .tvd}, {@code .tvx} and
 * {@code .tvm} files depend only on the term-vectors consumer and on the
 * term-vectors codec.
 *
 * <p>Every field value is a fully scripted table of
 * {@code (term, positionIncrement, startOffset, endOffset, payload)} tuples
 * which the Rust portability test mirrors exactly, so no analyzer takes part
 * and a byte difference can only come from the consumer or the codec. The field
 * order inside a document fixes the field numbers, which are written into the
 * term-vector chunks.
 *
 * <p>The tool prints the segment name and the hexadecimal segment id of the
 * committed segment — both are baked into the file headers — and then, for
 * every document, the term vectors Lucene's own reader decodes from the index
 * it has just written, so the Rust side can assert that it decodes exactly the
 * same values.
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... TermVectorsFixture &lt;output-dir&gt; &lt;case&gt;
 * </pre>
 *
 * <p>Supported cases: {@code basic}, {@code flags}, {@code payloads},
 * {@code missing}, {@code multivalue}, {@code chunks}, {@code empty},
 * {@code order} and {@code cfs}.
 */
public final class TermVectorsFixture {

  private TermVectorsFixture() {}

  /**
   * The scripted token and the token stream that replays it are shared with
   * {@link IndexingChainFixture}: both fixtures need to bypass analysis in
   * exactly the same way, and duplicating them would let the two drift.
   */
  static Tok tok(String term, int posIncr, int start, int end) {
    return Tok.of(term, posIncr, start, end);
  }

  /** The term-vector settings of one field, fixed for a whole case. */
  record Spec(
      String name,
      IndexOptions options,
      boolean vectors,
      boolean positions,
      boolean offsets,
      boolean payloads) {}

  /** One value of one field of one document. */
  record Val(int spec, List<Tok> tokens) {}

  public static void main(String[] args) {
    if (args.length != 2) {
      System.err.println("Usage: TermVectorsFixture <output-dir> <case>");
      System.err.println(
          "Supported cases: basic, flags, payloads, missing, multivalue, chunks, empty, order, immense, cfs");
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
        type.setOmitNorms(true);
        type.setIndexOptions(spec.options());
        type.setStoreTermVectors(spec.vectors());
        type.setStoreTermVectorPositions(spec.positions());
        type.setStoreTermVectorOffsets(spec.offsets());
        type.setStoreTermVectorPayloads(spec.payloads());
        type.freeze();
        types.add(type);
      }

      Analyzer analyzer = new WhitespaceAnalyzer();
      IndexWriterConfig config = new IndexWriterConfig(analyzer);
      config.setCodec(new Lucene104Codec());
      config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      // Only the `cfs` case bundles the segment; every other case compares the
      // loose `.tvd`/`.tvx`/`.tvm`.
      config.setUseCompoundFile(caseName.equals("cfs"));
      // One segment, flushed once: the byte comparison needs a single, fully
      // deterministic term-vectors stream.
      config.setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH);
      config.setRAMBufferSizeMB(512.0);

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
          try {
            writer.addDocument(doc);
          } catch (IllegalArgumentException e) {
            // A document-level failure: the document is dropped and indexing
            // continues, in both engines. The `immense` case relies on it, and
            // the message is printed so the Rust side can compare it.
            System.out.println("rejected=" + e.getMessage());
          }
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
        for (int i = 0; i < specs.size(); i++) {
          System.out.println("field=" + specs.get(i).name());
        }

        try (DirectoryReader reader = DirectoryReader.open(dir)) {
          for (org.apache.lucene.index.LeafReaderContext leaf : reader.leaves()) {
            for (org.apache.lucene.index.FieldInfo fi : leaf.reader().getFieldInfos()) {
              System.out.println(
                  "fieldinfo="
                      + fi.number
                      + " "
                      + fi.name
                      + " vectors="
                      + fi.hasTermVectors()
                      + " payloads="
                      + fi.hasPayloads());
            }
          }
          TermVectors termVectors = reader.termVectors();
          for (String line : dump(termVectors, commit.info.maxDoc())) {
            System.out.println(line);
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
   * Renders every term vector of every document as one line per term, plus one
   * line per document naming the fields in the order the reader yields them.
   *
   * <p>The Rust portability test produces byte-identical strings from its own
   * reader, so the two sides can be compared with a plain equality assertion.
   */
  static List<String> dump(TermVectors termVectors, int maxDoc) throws IOException {
    List<String> lines = new ArrayList<>();
    for (int docID = 0; docID < maxDoc; docID++) {
      Fields fields = termVectors.get(docID);
      if (fields == null) {
        lines.add("docnull " + docID);
        continue;
      }
      List<String> names = new ArrayList<>();
      for (String name : fields) {
        names.add(name);
      }
      lines.add("doc " + docID + " " + String.join("|", names));
      for (String name : names) {
        Terms terms = fields.terms(name);
        if (terms == null) {
          continue;
        }
        boolean hasPositions = terms.hasPositions();
        boolean hasOffsets = terms.hasOffsets();
        boolean hasPayloads = terms.hasPayloads();
        TermsEnum iterator = terms.iterator();
        BytesRef term;
        PostingsEnum postings = null;
        while ((term = iterator.next()) != null) {
          int freq = (int) iterator.totalTermFreq();
          postings = iterator.postings(postings, PostingsEnum.ALL);
          postings.nextDoc();
          List<String> positions = new ArrayList<>();
          List<String> offsets = new ArrayList<>();
          List<String> payloads = new ArrayList<>();
          if (hasPositions || hasOffsets) {
            for (int i = 0; i < freq; i++) {
              int position = postings.nextPosition();
              if (hasPositions) {
                positions.add(Integer.toString(position));
              }
              if (hasOffsets) {
                offsets.add(postings.startOffset() + ":" + postings.endOffset());
              }
              if (hasPayloads) {
                BytesRef payload = postings.getPayload();
                payloads.add(payload == null ? "." : hex(payload));
              }
            }
          }
          lines.add(
              "tv "
                  + docID
                  + " "
                  + name
                  + " P"
                  + (hasPositions ? 1 : 0)
                  + " O"
                  + (hasOffsets ? 1 : 0)
                  + " Y"
                  + (hasPayloads ? 1 : 0)
                  + " "
                  + term.utf8ToString()
                  + " "
                  + freq
                  + " "
                  + join(hasPositions, positions)
                  + " "
                  + join(hasOffsets, offsets)
                  + " "
                  + join(hasPayloads, payloads));
        }
      }
    }
    return lines;
  }

  private static String join(boolean present, List<String> values) {
    return present ? String.join(";", values) : "-";
  }

  private static String hex(BytesRef bytes) {
    StringBuilder builder = new StringBuilder(bytes.length * 2);
    for (int i = 0; i < bytes.length; i++) {
      builder.append(String.format("%02x", bytes.bytes[bytes.offset + i]));
    }
    return builder.toString();
  }

  // -------------------------------------------------------------------------
  // The scripts
  // -------------------------------------------------------------------------

  static final IndexOptions FULL = IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS;
  static final IndexOptions PROX = IndexOptions.DOCS_AND_FREQS_AND_POSITIONS;

  static List<Spec> specs(String caseName) {
    return switch (caseName) {
      case "basic", "missing", "cfs" ->
          List.of(
              new Spec("body", FULL, true, true, true, false),
              new Spec("title", PROX, true, false, false, false),
              new Spec("plain", PROX, false, false, false, false));
      case "flags" ->
          List.of(
              new Spec("a_none", PROX, true, false, false, false),
              new Spec("b_pos", PROX, true, true, false, false),
              new Spec("c_off", FULL, true, false, true, false),
              new Spec("d_posoff", FULL, true, true, true, false),
              new Spec("e_pospay", PROX, true, true, false, true),
              new Spec("f_all", FULL, true, true, true, true));
      case "payloads" -> List.of(new Spec("body", PROX, true, true, false, true));
      case "immense" ->
          List.of(
              new Spec("a", FULL, true, true, true, false),
              new Spec("b", FULL, true, true, true, true));
      case "multivalue", "empty", "chunks" ->
          List.of(new Spec("body", FULL, true, true, true, false));
      case "order" ->
          List.of(
              new Spec("zeta", FULL, true, true, true, false),
              new Spec("alpha", FULL, true, true, true, false),
              new Spec("mu", FULL, true, true, true, false));
      default -> throw new IllegalArgumentException("Unknown case: " + caseName);
    };
  }

  static List<List<Val>> documents(String caseName) {
    return switch (caseName) {
      case "basic", "cfs" -> basicDocuments();
      case "flags" -> flagDocuments();
      case "payloads" -> payloadDocuments();
      case "missing" -> missingDocuments();
      case "multivalue" -> multiValueDocuments();
      case "chunks" -> chunkDocuments();
      case "empty" -> emptyDocuments();
      case "order" -> orderDocuments();
      case "immense" -> immenseDocuments();
      default -> throw new IllegalArgumentException("Unknown case: " + caseName);
    };
  }

  /**
   * Four documents mixing a field with full vectors, a field with vectors but
   * no extras, and a field that asks for no vectors at all.
   */
  static List<List<Val>> basicDocuments() {
    List<List<Val>> docs = new ArrayList<>();
    docs.add(
        List.of(
            new Val(
                0,
                List.of(
                    Tok.of("alpha", 1, 0, 5),
                    Tok.of("beta", 1, 6, 10),
                    Tok.of("alpha", 1, 11, 16),
                    Tok.of("gamma", 1, 17, 22))),
            new Val(1, List.of(Tok.of("lucene", 1, 0, 6), Tok.of("rust", 1, 7, 11))),
            new Val(2, List.of(Tok.of("ignored", 1, 0, 7)))));
    docs.add(
        List.of(
            new Val(
                0,
                List.of(
                    Tok.of("gamma", 1, 0, 5),
                    Tok.of("gamma", 0, 0, 5),
                    Tok.of("epsilon", 2, 6, 13)))));
    docs.add(List.of(new Val(2, List.of(Tok.of("only", 1, 0, 4)))));
    docs.add(
        List.of(
            new Val(1, List.of(Tok.of("solo", 1, 0, 4))),
            new Val(
                0,
                List.of(
                    Tok.of("zeta", 1, 0, 4),
                    Tok.of("zeta", 1, 5, 9),
                    Tok.of("zeta", 1, 10, 14)))));
    return docs;
  }

  /** Two documents where every legal flag combination is exercised at once. */
  static List<List<Val>> flagDocuments() {
    List<List<Val>> docs = new ArrayList<>();
    for (int doc = 0; doc < 2; doc++) {
      List<Val> values = new ArrayList<>();
      for (int spec = 0; spec < 6; spec++) {
        values.add(
            new Val(
                spec,
                List.of(
                    new Tok("alpha", 1, 0, 5, doc == 0 ? new byte[] {1, 2, 3} : null),
                    Tok.of("beta", 2, 10, 14),
                    new Tok("alpha", 1, 20, 25, new byte[] {(byte) 0xFF}))));
      }
      docs.add(values);
    }
    return docs;
  }

  /** Payloads present, absent, empty and long, in the same field. */
  static List<List<Val>> payloadDocuments() {
    List<List<Val>> docs = new ArrayList<>();
    docs.add(
        List.of(
            new Val(
                0,
                List.of(
                    new Tok("alpha", 1, 0, 5, new byte[] {1}),
                    new Tok("beta", 1, 6, 10, null),
                    new Tok("alpha", 1, 11, 16, new byte[] {2, 3, 4})))));
    docs.add(
        List.of(
            new Val(
                0,
                List.of(
                    new Tok("beta", 1, 0, 4, new byte[] {(byte) 0xFF, 0x00, 0x7F}),
                    new Tok("gamma", 1, 5, 10, new byte[] {})))));
    docs.add(
        List.of(
            new Val(
                0,
                List.of(
                    new Tok("alpha", 1, 0, 5, IndexingChainFixture.longPayload(40)),
                    new Tok("gamma", 1, 6, 11, IndexingChainFixture.longPayload(7))))));
    // No payload at all: `hasPayloads` must come back false for this document.
    docs.add(
        List.of(new Val(0, List.of(Tok.of("delta", 1, 0, 5), Tok.of("delta", 1, 6, 11)))));
    return docs;
  }

  /**
   * Five documents where only the middle one carries vectors, so the writer has
   * to back-fill the documents before it and pad the ones after it.
   */
  static List<List<Val>> missingDocuments() {
    List<List<Val>> docs = new ArrayList<>();
    for (int doc = 0; doc < 5; doc++) {
      if (doc == 2) {
        docs.add(
            List.of(new Val(0, List.of(Tok.of("alpha", 1, 0, 5), Tok.of("beta", 1, 6, 10)))));
      } else {
        docs.add(List.of(new Val(2, List.of(Tok.of("plain", 1, 0, 5)))));
      }
    }
    return docs;
  }

  /** Three documents whose vector field is repeated, so the gaps apply. */
  static List<List<Val>> multiValueDocuments() {
    List<List<Val>> docs = new ArrayList<>();
    docs.add(
        List.of(
            new Val(0, List.of(Tok.of("alpha", 1, 0, 5), Tok.of("beta", 1, 6, 10))),
            new Val(0, List.of(Tok.of("alpha", 1, 0, 5), Tok.of("gamma", 1, 6, 11)))));
    docs.add(
        List.of(
            new Val(0, List.of(Tok.of("delta", 1, 0, 5))),
            new Val(0, List.of(Tok.of("delta", 1, 0, 5))),
            new Val(0, List.of(Tok.of("epsilon", 1, 0, 7)))));
    docs.add(List.of(new Val(0, List.of(Tok.of("beta", 1, 0, 4))), new Val(0, List.of())));
    return docs;
  }

  /**
   * Enough documents and long enough terms to cross both chunk triggers of the
   * compressing format: 4 KiB of term bytes and 128 documents.
   */
  static List<List<Val>> chunkDocuments() {
    List<List<Val>> docs = new ArrayList<>();
    for (int doc = 0; doc < 300; doc++) {
      List<Tok> tokens = new ArrayList<>();
      int offset = 0;
      for (int term = 0; term < 8; term++) {
        String text = String.format("term-%04d-%d-padding-padding", doc, term);
        tokens.add(Tok.of(text, 1, offset, offset + text.length()));
        offset += text.length() + 1;
      }
      docs.add(List.of(new Val(0, tokens)));
    }
    return docs;
  }

  /** Documents that mix empty values with real ones. */
  static List<List<Val>> emptyDocuments() {
    List<List<Val>> docs = new ArrayList<>();
    docs.add(List.of(new Val(0, List.of())));
    docs.add(List.of(new Val(0, List.of(Tok.of("solo", 1, 0, 4)))));
    docs.add(List.of());
    docs.add(List.of(new Val(0, List.of(Tok.of("solo", 1, 0, 4), Tok.of("solo", 1, 5, 9)))));
    docs.add(List.of(new Val(0, List.of())));
    return docs;
  }

  /**
   * Three documents, the middle of which has a good field {@code a} followed by
   * a field {@code b} whose third token exceeds {@code IndexWriter.MAX_TERM_LENGTH}.
   *
   * <p>The over-long term is a document-level failure: the document is dropped
   * and indexing continues. What it proves is that {@code b} contributes
   * *nothing* — no term vector, and no {@code storePayloads} flag — because
   * Lucene marks a field as indexed only after {@code invert} returns normally.
   * A port that marks it before would write {@code b}'s first two tokens into
   * the term vectors of a document nobody can read.
   */
  static List<List<Val>> immenseDocuments() {
    List<List<Val>> docs = new ArrayList<>();
    docs.add(List.of(new Val(0, List.of(Tok.of("alpha", 1, 0, 5)))));

    StringBuilder immense = new StringBuilder();
    for (int i = 0; i < 40_000; i++) {
      immense.append((char) ('a' + (i % 26)));
    }
    docs.add(
        List.of(
            new Val(0, List.of(Tok.of("beta", 1, 0, 4))),
            new Val(
                1,
                List.of(
                    new Tok("one", 1, 0, 3, new byte[] {7, 7}),
                    Tok.of("two", 1, 4, 7),
                    Tok.of(immense.toString(), 1, 8, 12)))));

    docs.add(List.of(new Val(0, List.of(Tok.of("gamma", 1, 0, 5)))));
    return docs;
  }

  /** One document whose three vector fields are added out of name order. */
  static List<List<Val>> orderDocuments() {
    List<List<Val>> docs = new ArrayList<>();
    docs.add(
        List.of(
            new Val(0, List.of(Tok.of("one", 1, 0, 3))),
            new Val(1, List.of(Tok.of("two", 1, 0, 3))),
            new Val(2, List.of(Tok.of("three", 1, 0, 5)))));
    docs.add(
        List.of(
            new Val(2, List.of(Tok.of("four", 1, 0, 4))),
            new Val(0, List.of(Tok.of("five", 1, 0, 4)))));
    return docs;
  }
}
