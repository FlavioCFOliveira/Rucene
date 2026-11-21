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

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.CheckIndex;
import org.apache.lucene.index.CodecReader;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Oracle of {@code tests/portability/terms_dictionary.rs} (rmp task #406).
 *
 * <p>The corpus is defined here and in the Rust test by the same formulas: thousands of distinct
 * terms with shared prefixes per field, so that the block tree writes many blocks and a multi-level
 * trie. Modes:
 *
 * <ul>
 *   <li>{@code write <root>}: writes the corpus as one segment (one commit, no compound file) into
 *       {@code <root>/terms}; two source indexes {@code <root>/src1} (two commits, one deletion) and
 *       {@code <root>/src2} (one commit, one deletion); then {@code <root>/dirs} with {@code
 *       IndexWriter.addIndexes(Directory...)} and {@code <root>/readers} with {@code
 *       IndexWriter.addIndexes(CodecReader...)} of the two sources.
 *   <li>{@code query <root>}: dumps, per leaf and field of {@code terms}, the {@code Terms}
 *       statistics, every term with its document and total term frequency, and the {@code
 *       seekCeil} result of every probe; then, for {@code dirs} and {@code readers}, the per-leaf
 *       statistics and the hits of a term-query set.
 *   <li>{@code check <root>}: runs {@code CheckIndex} (level {@code MAX_VALUE}, one thread,
 *       verbose) on {@code terms}, {@code dirs} and {@code readers} and prints its status and its
 *       full text output.
 * </ul>
 */
public final class TermsDictFixture {

  // ---------------------------------------------------------------------------------------------
  // Corpus
  // ---------------------------------------------------------------------------------------------

  /** Documents of the single-segment terms corpus. */
  static final int DOCS = 3000;

  /** The two source indexes of the addIndexes cases: each batch is closed by a commit. */
  static final int[][][] SOURCES = {{{0, 200}, {200, 400}}, {{400, 700}}};

  /** The document each source deletes after its first batch. */
  static final int[] SOURCE_DELETES = {13, 450};

  /** The fields dumped, in the order they are printed. */
  static final String[] FIELDS = {"body", "id", "kw", "tag"};

  /** Tokenized text with frequencies and positions. */
  static final FieldType TEXT_TYPE = new FieldType();

  static {
    TEXT_TYPE.setTokenized(true);
    TEXT_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    TEXT_TYPE.freeze();
  }

  private TermsDictFixture() {}

  static String id(int i) {
    return String.format("id-%05d", i);
  }

  /** {@code n} as {@code len} letters {@code a..z}, most significant first. */
  static String base26(int n, int len) {
    char[] c = new char[len];
    for (int k = len - 1; k >= 0; k--) {
      c[k] = (char) ('a' + n % 26);
      n /= 26;
    }
    return new String(c);
  }

  static String kwA(int i) {
    return base26((i * 37) % 17576, 3) + (i % 3 == 0 ? "ing" : "") + (i % 7 == 0 ? "ed" : "");
  }

  static String kwB(int i) {
    return "pre" + base26(i % 676, 2) + (i % 7);
  }

  /** A non-ASCII keyword on every fiftieth document. */
  static String kwC(int i) {
    return "zé" + base26(i / 50, 2);
  }

  /** {@code i % 6 + 3} words; every third word repeats the word before it. */
  static String body(int i) {
    List<String> words = new ArrayList<>();
    for (int j = 0; j < i % 6 + 3; j++) {
      words.add(j % 3 == 2 ? words.get(j - 1) : "t" + base26((i * 31 + j * 17) % 2500, 3));
    }
    return String.join(" ", words);
  }

  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", id(i), Field.Store.YES));
    d.add(new StringField("kw", kwA(i), Field.Store.NO));
    d.add(new StringField("kw", kwB(i), Field.Store.NO));
    if (i % 50 == 0) {
      d.add(new StringField("kw", kwC(i), Field.Store.NO));
    }
    d.add(new Field("body", body(i), TEXT_TYPE));
    d.add(new StringField("tag", "g" + (i % 13), Field.Store.NO));
    return d;
  }

  static IndexWriterConfig config() {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    return config;
  }

  static void write(String root) throws IOException {
    // One commit, no compound file, flushing only on commit: a single deterministic segment.
    IndexWriterConfig termsConfig = config();
    termsConfig.setUseCompoundFile(false);
    termsConfig.setRAMBufferSizeMB(256);
    try (Directory dir = FSDirectory.open(Paths.get(root, "terms"));
        IndexWriter w = new IndexWriter(dir, termsConfig)) {
      for (int i = 0; i < DOCS; i++) {
        w.addDocument(doc(i));
      }
      w.commit();
    }
    for (int s = 0; s < SOURCES.length; s++) {
      try (Directory dir = FSDirectory.open(Paths.get(root, "src" + (s + 1)));
          IndexWriter w = new IndexWriter(dir, config())) {
        for (int b = 0; b < SOURCES[s].length; b++) {
          for (int i = SOURCES[s][b][0]; i < SOURCES[s][b][1]; i++) {
            w.addDocument(doc(i));
          }
          if (b == 0) {
            w.deleteDocuments(new Term("id", id(SOURCE_DELETES[s])));
          }
          w.commit();
        }
      }
    }
    try (Directory src1 = FSDirectory.open(Paths.get(root, "src1"));
        Directory src2 = FSDirectory.open(Paths.get(root, "src2"))) {
      try (Directory dir = FSDirectory.open(Paths.get(root, "dirs"));
          IndexWriter w = new IndexWriter(dir, config())) {
        w.addIndexes(src1, src2);
        w.commit();
      }
      try (DirectoryReader r1 = DirectoryReader.open(src1);
          DirectoryReader r2 = DirectoryReader.open(src2);
          Directory dir = FSDirectory.open(Paths.get(root, "readers"));
          IndexWriter w = new IndexWriter(dir, config())) {
        List<CodecReader> readers = new ArrayList<>();
        for (DirectoryReader r : new DirectoryReader[] {r1, r2}) {
          for (LeafReaderContext ctx : r.leaves()) {
            readers.add((CodecReader) ctx.reader());
          }
        }
        w.addIndexes(readers.toArray(new CodecReader[0]));
        w.commit();
      }
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Query set
  // ---------------------------------------------------------------------------------------------

  /** Hits printed per query. */
  static final int TOP_N = 20;

  static String bits(float f) {
    return String.format("%08x", Float.floatToIntBits(f));
  }

  static String text(BytesRef b) {
    return b == null ? "null" : b.utf8ToString();
  }

  /** The {@code seekCeil} probes, applied in order to one enum per field. */
  static List<String> probes() {
    List<String> p =
        new ArrayList<>(
            List.of(
                "", "a", "b", "g1", "g12", "g9", "id-0", "id-01234", "id-01234x", "id-02999",
                "id-3", "pre", "preab", "preab3", "preaz9", "prezz", "t", "taa", "tdzz", "tz",
                "tag", "z", "zé", "zz"));
    for (int k : new int[] {0, 1, 777, 2999}) {
      p.add(kwA(k));
      p.add(kwA(k) + "0");
      p.add(kwB(k));
      p.add(id(k) + "0");
    }
    return p;
  }

  static void printStats(String field, Terms terms) throws IOException {
    System.out.println(
        "field "
            + field
            + " size="
            + terms.size()
            + " sumDocFreq="
            + terms.getSumDocFreq()
            + " sumTotalTermFreq="
            + terms.getSumTotalTermFreq()
            + " docCount="
            + terms.getDocCount()
            + " min="
            + text(terms.getMin())
            + " max="
            + text(terms.getMax()));
  }

  static void printReader(DirectoryReader reader) {
    System.out.println("reader maxDoc=" + reader.maxDoc() + " numDocs=" + reader.numDocs());
    for (LeafReaderContext ctx : reader.leaves()) {
      System.out.println(
          "leaf ord="
              + ctx.ord
              + " docBase="
              + ctx.docBase
              + " maxDoc="
              + ctx.reader().maxDoc()
              + " numDocs="
              + ctx.reader().numDocs());
    }
  }

  static void dumpTerms(DirectoryReader reader) throws IOException {
    for (LeafReaderContext ctx : reader.leaves()) {
      for (String field : FIELDS) {
        Terms terms = ctx.reader().terms(field);
        if (terms == null) {
          System.out.println("field " + field + " leaf=" + ctx.ord + " none");
          continue;
        }
        printStats(field, terms);
        TermsEnum te = terms.iterator();
        for (BytesRef t = te.next(); t != null; t = te.next()) {
          System.out.println(
              "  term " + t.utf8ToString() + " docFreq=" + te.docFreq() + " ttf=" + te.totalTermFreq());
        }
        TermsEnum seeker = terms.iterator();
        for (String probe : probes()) {
          TermsEnum.SeekStatus status = seeker.seekCeil(new BytesRef(probe));
          System.out.println(
              "  seekCeil "
                  + probe
                  + " -> "
                  + status
                  + (status == TermsEnum.SeekStatus.END
                      ? ""
                      : " " + seeker.term().utf8ToString() + " docFreq=" + seeker.docFreq()));
        }
      }
    }
  }

  /** The term queries run on the addIndexes results. */
  static List<Term> queryTerms() {
    return List.of(
        new Term("kw", kwA(5)),
        new Term("kw", kwB(30)),
        // The first word of document 10: (10 * 31) % 2500.
        new Term("body", "t" + base26(310, 3)),
        new Term("tag", "g3"),
        new Term("id", id(12)),
        new Term("id", id(451)));
  }

  static void printAddIndexes(String root, String name) throws IOException {
    try (Directory dir = FSDirectory.open(Paths.get(root, name));
        DirectoryReader reader = DirectoryReader.open(dir)) {
      System.out.println("index " + name);
      printReader(reader);
      for (LeafReaderContext ctx : reader.leaves()) {
        for (String field : FIELDS) {
          Terms terms = ctx.reader().terms(field);
          System.out.print("leaf=" + ctx.ord + " ");
          if (terms == null) {
            System.out.println("field " + field + " none");
          } else {
            printStats(field, terms);
          }
        }
      }
      IndexSearcher searcher = new IndexSearcher(reader);
      StoredFields stored = searcher.storedFields();
      for (Term t : queryTerms()) {
        TopDocs top = searcher.search(new TermQuery(t), TOP_N);
        System.out.println(
            "query " + t + " total=" + top.totalHits.value() + " " + top.totalHits.relation());
        for (ScoreDoc hit : top.scoreDocs) {
          System.out.println(
              "  hit doc="
                  + hit.doc
                  + " score="
                  + bits(hit.score)
                  + " id="
                  + stored.document(hit.doc).get("id"));
        }
      }
    }
  }

  static void query(String root) throws IOException {
    try (Directory dir = FSDirectory.open(Paths.get(root, "terms"));
        DirectoryReader reader = DirectoryReader.open(dir)) {
      System.out.println("index terms");
      printReader(reader);
      dumpTerms(reader);
    }
    printAddIndexes(root, "dirs");
    printAddIndexes(root, "readers");
  }

  // ---------------------------------------------------------------------------------------------
  // CheckIndex
  // ---------------------------------------------------------------------------------------------

  /** The indexes {@code check} runs on. */
  static final String[] CHECKED = {"terms", "dirs", "readers"};

  static void check(String root) throws IOException {
    for (String name : CHECKED) {
      ByteArrayOutputStream bytes = new ByteArrayOutputStream();
      CheckIndex.Status status;
      try (Directory dir = FSDirectory.open(Paths.get(root, name));
          CheckIndex checker = new CheckIndex(dir);
          PrintStream out = new PrintStream(bytes, true, StandardCharsets.UTF_8)) {
        checker.setLevel(CheckIndex.Level.MAX_VALUE);
        checker.setThreadCount(1);
        checker.setInfoStream(out, true);
        status = checker.checkIndex();
      }
      System.out.println(
          "check "
              + name
              + " clean="
              + status.clean
              + " numSegments="
              + status.numSegments
              + " numBadSegments="
              + status.numBadSegments
              + " totLoseDocCount="
              + status.totLoseDocCount);
      bytes.toString(StandardCharsets.UTF_8).lines().forEach(l -> System.out.println("  | " + l));
    }
  }

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "write" -> write(args[1]);
      case "query" -> query(args[1]);
      case "check" -> check(args[1]);
      default -> throw new IllegalArgumentException(args[0]);
    }
  }
}
