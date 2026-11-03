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
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.SegmentCommitInfo;
import org.apache.lucene.index.SegmentInfos;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Writes a single-segment Apache Lucene Core 10.5.0 index whose only interesting
 * content is doc values, so that the resulting {@code .dvd} and {@code .dvm}
 * files depend only on the doc-values writers and on the doc-values codec.
 *
 * <p>Every case is a fixed table of documents and values that the Rust
 * portability test mirrors exactly, so no analyzer takes part: a byte
 * difference can only come from a doc-values writer or from the doc-values
 * codec. The field order inside the first document that uses a field fixes
 * the field numbers, which order the {@code .dvm} entries.
 *
 * <p>The cases are chosen to span the shape dimensions of the format rather
 * than only its values:
 *
 * <ul>
 *   <li>{@code numeric} — three numeric fields: one whose values span a wide
 *       signed range (which also exercises the GCD and the unique-value
 *       table), one that repeats a small set, and one constant, which is
 *       stored once in the metadata while the data file holds none;
 *   <li>{@code sparse} — a field present in every third document only, which
 *       {@code IndexedDISI} writes with its SPARSE (short-per-doc) block
 *       encoding, beside a field every document carries;
 *   <li>{@code dense} — 10000 documents, half of which carry the field: more
 *       than the 4095 entries {@code IndexedDISI} stores as shorts, so its
 *       block switches to the DENSE bitmap-plus-rank encoding;
 *   <li>{@code binary} — a dense binary field whose lengths run from zero
 *       upward (including the empty value) beside a sparse one;
 *   <li>{@code sorted} — a single-valued sorted field whose first-seen term
 *       order disagrees with the sorted order, so a writer that keeps
 *       insertion-order ordinals diverges;
 *   <li>{@code sortednumeric} — a sparse multi-valued numeric field with one,
 *       two and three values per document, including within-document
 *       duplicates, which the format must keep (it is a list, not a set);
 *   <li>{@code sortedsetsingle} — a sorted-set field in which every document
 *       carries exactly one value, which Lucene writes through the
 *       single-valued route whose metadata byte is the {@code SORTED} one;
 *   <li>{@code sortedsetmulti} — a genuinely multi-valued sorted set with
 *       in-document duplicates and cross-document repeats, which drives the
 *       multi-valued addresses route;
 *   <li>{@code sortedlongterms} — 200 distinct 400-byte terms in a single
 *       sorted field. The terms dictionary writes 64 terms per LZ4 block
 *       ({@code TERMS_DICT_BLOCK_LZ4_SHIFT}), so each block buffers roughly
 *       25 KB — past the 16384 bytes {@code Lucene90DocValuesConsumer}
 *       allocates for {@code termsDictBuffer}, which forces
 *       {@code maybeGrowBuffer} to grow it. Every other case in this suite
 *       keeps its terms far below that, so this is the only one that reaches
 *       the growth path, the multi-block path, and LZ4 blocks large enough to
 *       have anything to compress;
 *   <li>{@code mixed} — one document stream carrying all five types at once,
 *       with gaps in several of them;
 *   <li>{@code blocks} — 34000 documents whose numeric values change range
 *       from one 16384-value block to the next, so {@code writeValues} takes
 *       its per-block layout ({@code doBlocks},
 *       {@code Lucene90DocValuesConsumer.java:485-497}): a dense numeric field,
 *       a sparse one, and a two-values-per-document sorted-numeric field, with
 *       a constant block and a partial last block;
 *   <li>{@code sortedoneterm} — a sorted field and a single-valued sorted-set
 *       field whose only term is the same in every document, so every ordinal
 *       is 0 and the ordinals' GCD stays 0
 *       ({@code Lucene90DocValuesConsumer.java:390-401});
 *   <li>{@code ordblocks} — sorted and sorted-set ordinals whose ranges change
 *       from block to block, so the ordinals themselves take the per-block
 *       layout;
 *   <li>{@code extreme}, {@code extremeblocks} — {@code Long.MIN_VALUE} and
 *       {@code Long.MAX_VALUE} beside small values, where the consumer's
 *       {@code long} arithmetic wraps;
 *   <li>{@code boundary09}, {@code boundary095} — per-block savings on either
 *       side of the 10% threshold that decides {@code doBlocks}.
 * </ul>
 *
 * <p>Command-line usage:
 * <pre>
 *   java ... DocValuesFixture &lt;output-dir&gt; &lt;case&gt;
 * </pre>
 */
public final class DocValuesFixture {

  private DocValuesFixture() {}

  public static void main(String[] args) {
    if (args.length != 2) {
      System.err.println("Usage: DocValuesFixture <output-dir> <case>");
      System.err.println(
          "Supported cases: numeric, sparse, dense, binary, sorted, sortednumeric, "
              + "sortedsetsingle, sortedsetmulti, sortedlongterms, mixed, blocks, sortedoneterm");
      System.exit(1);
    }

    Path outputDir = Paths.get(args[0]);
    String caseName = args[1];

    try {
      Files.createDirectories(outputDir);

      IndexWriterConfig config = new IndexWriterConfig(new WhitespaceAnalyzer());
      config.setCodec(new Lucene104Codec());
      config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
      config.setMergePolicy(NoMergePolicy.INSTANCE);
      config.setUseCompoundFile(false);
      // One segment, flushed once: the byte comparison needs a single, fully
      // deterministic doc-values stream.
      config.setMaxBufferedDocs(IndexWriterConfig.DISABLE_AUTO_FLUSH);
      config.setRAMBufferSizeMB(512.0);

      try (FSDirectory dir = FSDirectory.open(outputDir);
          IndexWriter writer = new IndexWriter(dir, config)) {
        for (Document document : documents(caseName)) {
          writer.addDocument(document);
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
                  "fieldinfo=" + fi.number + " " + fi.name + " dv=" + fi.getDocValuesType());
            }
            for (String line : DocValuesReaderFixture.dumpLeaf(leaf.reader())) {
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

  // ---------------------------------------------------------------------------
  // The cases
  // ---------------------------------------------------------------------------

  /** Twelve documents over three numeric fields: wide, repeating, constant. */
  private static List<Document> numeric() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 12; doc++) {
      Document document = new Document();
      document.add(new NumericDocValuesField("num", (doc - 6) * 1_000_003L));
      document.add(new NumericDocValuesField("gcd", doc % 4));
      document.add(new NumericDocValuesField("konst", 42));
      documents.add(document);
    }
    return documents;
  }

  /** Forty documents; `sparse` every third, `all` in every document. */
  private static List<Document> sparse() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 40; doc++) {
      Document document = new Document();
      if (doc % 3 == 0) {
        document.add(new NumericDocValuesField("sparse", doc * 13 - 40));
      }
      document.add(new NumericDocValuesField("all", doc % 9));
      documents.add(document);
    }
    return documents;
  }

  /** Ten thousand documents, half of which carry the field: a DENSE block. */
  private static List<Document> dense() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 10_000; doc++) {
      Document document = new Document();
      if (doc % 2 == 0) {
        document.add(new NumericDocValuesField("dense", 1 + doc % 11));
      }
      document.add(new NumericDocValuesField("every", doc));
      documents.add(document);
    }
    return documents;
  }

  /**
   * Twelve documents over two binary fields: one dense, with lengths from
   * zero (the empty value) upward; one sparse, one of whose values is the
   * empty array, which the format must carry without confusing it with
   * "absent".
   */
  private static List<Document> binary() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 12; doc++) {
      Document document = new Document();
      document.add(new BinaryDocValuesField("bin", new BytesRef(binaryFor(doc))));
      if (doc % 5 == 2) {
        document.add(
            new BinaryDocValuesField("sbin", new BytesRef(doc == 7 ? "" : "s" + doc)));
      }
      documents.add(document);
    }
    return documents;
  }

  private static byte[] binaryFor(int doc) {
    return doc == 4 ? new byte[0] : ("bin" + doc + "payload" + doc).getBytes(StandardCharsets.UTF_8);
  }

  /**
   * Ten documents whose first-seen order ("zz" before "apple") disagrees
   * with sorted order, so a writer that keeps insertion-order ordinals
   * diverges.
   */
  private static List<Document> sorted() {
    List<Document> documents = new ArrayList<>();
    String[] dict = {"zz", "apple", "mm", "bee"};
    for (int doc = 0; doc < 10; doc++) {
      Document document = new Document();
      document.add(new SortedDocValuesField("sort", new BytesRef(dict[doc % 4])));
      documents.add(document);
    }
    return documents;
  }

  private static List<Document> sortedNumeric() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 20; doc++) {
      Document document = new Document();
      if (doc % 2 == 0) {
        // One, two and three values per document; SORTED_NUMERIC is a
        // list, not a set, so duplicates must survive the flush.
        int count = 1 + doc % 3;
        for (int i = 0; i < count; i++) {
          document.add(new SortedNumericDocValuesField("snum", (doc * 31 - 9) * (i + 1)));
        }
      }
      documents.add(document);
    }
    return documents;
  }

  private static List<Document> sortedSetSingle() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 10; doc++) {
      Document document = new Document();
      if (doc % 3 == 0) {
        document.add(new SortedSetDocValuesField("ss", new BytesRef(SINGLE[doc % 6])));
      }
      documents.add(document);
    }
    return documents;
  }

  private static List<Document> sortedSetMulti() {
    List<Document> documents = new ArrayList<>();
    // Unordered on purpose, with an empty term: in-document duplicates are
    // deduplicated, and first-seen order disagrees with sorted order.
    String[] dict = {"bee", "ant", "cow", "ant", "emu", "bee", "dog", "fox"};
    for (int doc = 0; doc < 12; doc++) {
      Document document = new Document();
      if (doc % 2 == 0) {
        int count = 1 + doc % 3;
        for (int i = 0; i < count; i++) {
          document.add(new SortedSetDocValuesField("ss", new BytesRef(dict[(doc * 2 + i) % 8])));
        }
      }
      documents.add(document);
    }
    return documents;
  }

  /**
   * Builds term {@code i}: a zero-padded ordinal prefix followed by a rotating
   * alphabet, padded to exactly 400 ASCII bytes. Sorted order and insertion
   * order agree here on purpose — this case is about the buffer, not the
   * ordinals — and consecutive terms share a prefix, so the suffix each block
   * stores is long enough to take the {@code writeVInt(suffixLength - 16)}
   * branch.
   */
  private static String longTerm(int i) {
    StringBuilder term = new StringBuilder(String.format(java.util.Locale.ROOT, "t%04d-", i));
    for (int j = 0; term.length() < 400; j++) {
      term.append((char) ('a' + ((i + j) % 26)));
    }
    return term.toString();
  }

  private static List<Document> sortedLongTerms() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 200; doc++) {
      Document document = new Document();
      document.add(
          new SortedDocValuesField(
              "lterm", new BytesRef(longTerm(doc).getBytes(StandardCharsets.UTF_8))));
      documents.add(document);
    }
    return documents;
  }

  private static List<Document> mixed() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 12; doc++) {
      Document document = new Document();
      if (doc % 2 == 0) {
        document.add(new NumericDocValuesField("mnum", (doc - 6) * 77));
      }
      if (doc % 3 == 0) {
        document.add(new BinaryDocValuesField("mbin", new BytesRef(("mb" + doc).getBytes(StandardCharsets.UTF_8))));
      }
      if (doc % 3 != 1) {
        document.add(new SortedDocValuesField("msort", new BytesRef(DICTIONARY[doc % 5])));
      }
      if (doc % 4 != 0) {
        document.add(new SortedNumericDocValuesField("msnum", doc - 5));
        if (doc % 4 == 3) {
          document.add(new SortedNumericDocValuesField("msnum", doc * 13));
        }
      }
      if (doc % 2 == 1) {
        document.add(new SortedSetDocValuesField("mss", new BytesRef(SET[doc % 4])));
      }
      documents.add(document);
    }
    return documents;
  }

  /** The value of {@code vbpv} in document {@code doc}: three ranges, one per block. */
  static long blockValue(int doc) {
    if (doc < 16384) {
      return doc % 2;
    } else if (doc < 32768) {
      return doc * 1_000_003L % 1_099_511_627_776L;
    }
    return 7;
  }

  /** The value of {@code svbpv} for its {@code i}-th value (document {@code 2 * i}). */
  static long sparseBlockValue(int i) {
    return i < 16384 ? i % 4 : i * 7919L;
  }

  private static List<Document> blocks() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 34_000; doc++) {
      Document document = new Document();
      document.add(new NumericDocValuesField("vbpv", blockValue(doc)));
      if (doc % 2 == 0) {
        document.add(new NumericDocValuesField("svbpv", sparseBlockValue(doc / 2)));
      }
      if (doc < 8192) {
        document.add(new SortedNumericDocValuesField("snvbpv", doc % 3));
        document.add(new SortedNumericDocValuesField("snvbpv", 1 + doc % 3));
      } else {
        document.add(new SortedNumericDocValuesField("snvbpv", doc * 104_729L));
        document.add(new SortedNumericDocValuesField("snvbpv", doc * 104_729L + 5));
      }
      documents.add(document);
    }
    return documents;
  }

  private static List<Document> sortedOneTerm() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 10; doc++) {
      Document document = new Document();
      document.add(new SortedDocValuesField("one", new BytesRef("only")));
      if (doc % 2 == 0) {
        document.add(new SortedSetDocValuesField("oneset", new BytesRef("only")));
      }
      documents.add(document);
    }
    return documents;
  }

  private static List<Document> ordBlocks() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 40_000; doc++) {
      Document document = new Document();
      String term = doc < 16384 ? (doc % 2 == 0 ? "a" : "b") : String.format("t%05d", doc);
      document.add(new SortedDocValuesField("ob", new BytesRef(term)));
      if (doc < 16384) {
        document.add(new SortedSetDocValuesField("obs", new BytesRef("a")));
      } else {
        document.add(new SortedSetDocValuesField("obs", new BytesRef(String.format("t%05d", doc))));
        document.add(new SortedSetDocValuesField("obs", new BytesRef(String.format("u%05d", doc))));
      }
      documents.add(document);
    }
    return documents;
  }

  static long extremeValue(int doc) {
    if (doc < 16384) return doc % 2;
    if (doc < 32768) return doc % 2 == 0 ? Long.MIN_VALUE : Long.MAX_VALUE;
    return -1;
  }

  private static List<Document> extreme() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 34_000; doc++) {
      Document document = new Document();
      document.add(new NumericDocValuesField("ex", extremeValue(doc)));
      document.add(new SortedNumericDocValuesField("exs", Long.MIN_VALUE));
      document.add(new SortedNumericDocValuesField("exs", Long.MAX_VALUE));
      document.add(new NumericDocValuesField("exw", doc % 3 == 0 ? Long.MIN_VALUE : (doc % 3 == 1 ? Long.MAX_VALUE : (long) doc)));
      documents.add(document);
    }
    return documents;
  }

  private static List<Document> extremeBlocks() {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 34_000; doc++) {
      Document document = new Document();
      long v = doc < 16384 ? doc % 2 : (doc < 32768 ? (doc % 3 == 0 ? Long.MIN_VALUE : (doc % 3 == 1 ? Long.MAX_VALUE : (long) doc)) : -1L);
      document.add(new NumericDocValuesField("xb", v));
      documents.add(document);
    }
    return documents;
  }

  private static List<Document> boundary(int lowBits) {
    List<Document> documents = new ArrayList<>();
    for (int doc = 0; doc < 32_768; doc++) {
      Document document = new Document();
      long v = doc < 16384 ? doc % (1 << lowBits) : (doc * 7L) % 1024;
      document.add(new NumericDocValuesField("bd", v));
      documents.add(document);
    }
    return documents;
  }

  static final String[] DICTIONARY = {"zz", "apple", "mm", "bee", "kiwi"};
  static final String[] SINGLE = {"ant", "bee", "cow", "dog", "emu", "fox"};
  static final String[] SET = {"bee", "ant", "cow", "dog"};

  /** The documents of one case, in order. */
  static List<Document> documents(String caseName) {
    return switch (caseName) {
      case "numeric" -> numeric();
      case "sparse" -> sparse();
      case "dense" -> dense();
      case "binary" -> binary();
      case "sorted" -> sorted();
      case "sortednumeric" -> sortedNumeric();
      case "sortedsetsingle" -> sortedSetSingle();
      case "sortedsetmulti" -> sortedSetMulti();
      case "sortedlongterms" -> sortedLongTerms();
      case "mixed" -> mixed();
      case "blocks" -> blocks();
      case "sortedoneterm" -> sortedOneTerm();
      case "ordblocks" -> ordBlocks();
      case "extreme" -> extreme();
      case "extremeblocks" -> extremeBlocks();
      case "boundary09" -> boundary(8);
      case "boundary095" -> boundary(9);
      default -> throw new IllegalArgumentException("unknown case: " + caseName);
    };
  }
}