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

import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.text.Collator;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import org.apache.lucene.collation.CollationDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.store.ByteBuffersDirectory;

/**
 * The Java side of the collation oracle of rmp task #472 (Rust counterpart {@code
 * crates/rucene-analysis-common/tests/analysis_oracle_hunspell_compound_morph_collation.rs}).
 *
 * <p><b>Spec file</b> (UTF-8, TAB-separated): {@code collator <tag> <strength> <decomposition>}
 * lines, {@code sort <tag> <strength> <decomposition>} lines and {@code input <text>} corpus
 * lines (escaped as {@code AnalysisOracle.escape}).
 *
 * <p><b>Dump</b>: per {@code collator}, the line itself, then per input {@code k <hex>} with the
 * bytes of {@code getCollationKey(input).toByteArray()} and {@code c <sign>} with the sign of
 * {@code compare(input, next input)}; per {@code sort}, the line itself and {@code d <doc>} per hit
 * of a {@code MatchAllDocsQuery} sorted by a {@code CollationDocValuesField} (one document per
 * input of at most 1000 units, in order, the input also stored), as {@code
 * TestCollationDocValuesField} sorts.
 */
public final class CollationOracle {

  private CollationOracle() {}

  static String unescape(String s) {
    StringBuilder out = new StringBuilder();
    int i = 0;
    while (i < s.length()) {
      char ch = s.charAt(i);
      if (ch == '\\') {
        out.append((char) Integer.parseInt(s.substring(i + 2, i + 6), 16));
        i += 6;
      } else {
        out.append(ch);
        i++;
      }
    }
    return out.toString();
  }

  static Collator collator(String[] p) {
    Collator c = Collator.getInstance(new Locale.Builder().setLanguageTag(p[1]).build());
    c.setStrength(Integer.parseInt(p[2]));
    c.setDecomposition(Integer.parseInt(p[3]));
    return c;
  }

  public static void main(String[] args) throws Exception {
    List<String[]> specs = new ArrayList<>();
    List<String> inputs = new ArrayList<>();
    for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
      String[] p = line.split("\t", -1);
      if (p[0].equals("input")) {
        inputs.add(unescape(p[1]));
      } else {
        specs.add(p);
      }
    }
    PrintWriter out =
        new PrintWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8), false);
    for (String[] p : specs) {
      out.println(String.join(" ", p));
      Collator c = collator(p);
      if (p[0].equals("collator")) {
        for (int i = 0; i < inputs.size(); i++) {
          StringBuilder hex = new StringBuilder();
          for (byte b : c.getCollationKey(inputs.get(i)).toByteArray()) {
            hex.append(String.format(Locale.ROOT, "%02x", b & 0xff));
          }
          out.println("k " + hex);
          if (i + 1 < inputs.size()) {
            out.println("c " + Integer.signum(c.compare(inputs.get(i), inputs.get(i + 1))));
          }
        }
      } else {
        // the inputs whose keys fit a doc value (at most 32766 bytes)
        List<String> docs = inputs.stream().filter(s -> s.length() <= 1000).toList();
        ByteBuffersDirectory dir = new ByteBuffersDirectory();
        try (IndexWriter w = new IndexWriter(dir, new IndexWriterConfig())) {
          Document doc = new Document();
          Field field = new StoredField("field", "");
          CollationDocValuesField collationField = new CollationDocValuesField("collated", c);
          doc.add(field);
          doc.add(collationField);
          for (String input : docs) {
            field.setStringValue(input);
            collationField.setStringValue(input);
            w.addDocument(doc);
          }
        }
        try (DirectoryReader r = DirectoryReader.open(dir)) {
          IndexSearcher s = new IndexSearcher(r);
          TopDocs td =
              s.search(
                  MatchAllDocsQuery.INSTANCE,
                  docs.size(),
                  new Sort(new SortField("collated", SortField.Type.STRING)));
          StoredFields stored = r.storedFields();
          for (ScoreDoc sd : td.scoreDocs) {
            String v = stored.document(sd.doc).get("field");
            out.println("d " + sd.doc + " " + (v.equals(docs.get(sd.doc)) ? "ok" : "MISMATCH"));
          }
        }
      }
    }
    out.flush();
  }
}
