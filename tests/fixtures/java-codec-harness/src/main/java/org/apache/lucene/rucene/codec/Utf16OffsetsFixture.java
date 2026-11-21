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
import java.nio.file.Paths;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.Fields;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Offsets in UTF-16 code units through the index (rmp #357 block 3): documents of supplementary
 * text analyzed by {@code StandardAnalyzer} into a field whose postings and term vectors carry
 * offsets.
 *
 * <p>{@code write <dir>} writes the documents into a new index; {@code dump <dir>} prints, for
 * the index at {@code dir}, every posting of the field (term, doc, positions and offsets) and every
 * term vector. The Rust side is {@code tests/portability/utf16_offsets.rs}, which keeps the same
 * documents.
 */
public final class Utf16OffsetsFixture {
  private Utf16OffsetsFixture() {}

  /**
   * The texts: those of {@code AnalysisMocksOracle.U16} that Rucene's {@code StandardTokenizer}
   * segments as Lucene does (the emoji sequences are left out).
   */
  static final String[] TEXTS = {
    "x𐐀y 𝄞𝄞 zz 𐐨",
    "İSTANBUL ΣΑΣ ÀÖø一鿌",
    "𩬅艱鍟䏹愯瀛",
    "💩 💩💩",
    "poo💩poo",
    "💩中國💩",
    "12345678𐌀",
    "ab cd toolong xy z",
    "A bc defg hiJklmn opqrstuv wxy z ",
    "  \t\r\n  leading whitespace",
  };

  static FieldType bodyType() {
    FieldType t = new FieldType();
    t.setTokenized(true);
    t.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
    t.setStoreTermVectors(true);
    t.setStoreTermVectorPositions(true);
    t.setStoreTermVectorOffsets(true);
    t.freeze();
    return t;
  }

  /**
   * Document {@code i}: one value per text, then one document holding two values (the offset gap
   * between values of a field) and one holding every text.
   */
  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", Integer.toString(i), Field.Store.YES));
    if (i < TEXTS.length) {
      d.add(new Field("body", TEXTS[i], bodyType()));
    } else if (i == TEXTS.length) {
      d.add(new Field("body", TEXTS[0], bodyType()));
      d.add(new Field("body", TEXTS[3], bodyType()));
    } else {
      for (String text : TEXTS) {
        d.add(new Field("body", text, bodyType()));
      }
    }
    return d;
  }

  static final int DOCS = TEXTS.length + 2;

  static void write(String path) throws IOException {
    try (Directory dir = FSDirectory.open(Paths.get(path));
        IndexWriter w =
            new IndexWriter(
                dir,
                new IndexWriterConfig(new StandardAnalyzer())
                    .setOpenMode(IndexWriterConfig.OpenMode.CREATE))) {
      for (int i = 0; i < DOCS; i++) {
        w.addDocument(doc(i));
      }
      w.commit();
    }
  }

  static String hex(BytesRef b) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < b.length; i++) {
      sb.append(String.format("%02x", b.bytes[b.offset + i] & 0xff));
    }
    return sb.toString();
  }

  static String positions(PostingsEnum pe) throws IOException {
    StringBuilder sb = new StringBuilder();
    int freq = pe.freq();
    sb.append(" freq=").append(freq);
    for (int k = 0; k < freq; k++) {
      int pos = pe.nextPosition();
      sb.append(" [")
          .append(pos)
          .append(',')
          .append(pe.startOffset())
          .append(',')
          .append(pe.endOffset())
          .append(']');
    }
    return sb.toString();
  }

  static void dump(String path) throws IOException {
    try (Directory dir = FSDirectory.open(Paths.get(path));
        DirectoryReader reader = DirectoryReader.open(dir)) {
      StoredFields stored = reader.storedFields();
      for (LeafReaderContext ctx : reader.leaves()) {
        LeafReader leaf = ctx.reader();
        Terms terms = leaf.terms("body");
        if (terms == null) {
          continue;
        }
        TermsEnum te = terms.iterator();
        BytesRef text;
        while ((text = te.next()) != null) {
          PostingsEnum pe = te.postings(null, PostingsEnum.ALL);
          for (int d = pe.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = pe.nextDoc()) {
            String id = stored.document(ctx.docBase + d).get("id");
            System.out.println("post " + hex(text) + " id=" + id + positions(pe));
          }
        }
      }
      for (int d = 0; d < reader.maxDoc(); d++) {
        String id = stored.document(d).get("id");
        Fields fields = reader.termVectors().get(d);
        Terms terms = fields == null ? null : fields.terms("body");
        if (terms == null) {
          System.out.println("tv id=" + id + " none");
          continue;
        }
        System.out.println(
            "tv id=" + id + " offsets=" + terms.hasOffsets() + " positions=" + terms.hasPositions());
        TermsEnum te = terms.iterator();
        BytesRef text;
        while ((text = te.next()) != null) {
          PostingsEnum pe = te.postings(null, PostingsEnum.ALL);
          pe.nextDoc();
          System.out.println(" " + hex(text) + positions(pe));
        }
      }
    }
  }

  public static void main(String[] args) throws IOException {
    switch (args[0]) {
      case "write" -> write(args[1]);
      case "dump" -> dump(args[1]);
      default -> throw new IllegalArgumentException("unknown mode " + args[0]);
    }
  }
}
