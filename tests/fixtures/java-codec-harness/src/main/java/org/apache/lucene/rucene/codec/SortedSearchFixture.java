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
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.DoubleDocValuesField;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FloatDocValuesField;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopFieldCollectorManager;
import org.apache.lucene.search.TopFieldDocs;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Apache Lucene Core 10.5.0 oracle for sorted searches through {@code TopFieldCollectorManager}
 * (rmp #242).
 *
 * <p>Usage: {@code java -cp lucene-core-10.5.0.jar SortedSearchFixture.java <index-dir>}.
 *
 * <p>Writes {@value #DOCS} documents in {@value #SEGMENTS} segments (so that an executor-driven
 * search has several slices) and prints, for every sort of {@link #sorts()} and both queries, the
 * total hits and the top 10 document ids of {@code searcher.search(query, new
 * TopFieldCollectorManager(sort, 10, null, 1000))}. The document fields are described by {@link
 * #doc(int)}.
 */
public final class SortedSearchFixture {

  static final int DOCS = 1200;
  static final int SEGMENTS = 12;

  private SortedSearchFixture() {}

  /** Document {@code i}. Every field but {@code body} is missing from some documents. */
  static Document doc(int i) {
    Document doc = new Document();
    StringBuilder body = new StringBuilder();
    for (int k = 0; k < 1 + i % 5; k++) {
      body.append("a ");
    }
    if (i % 3 == 0) {
      body.append("b");
    }
    doc.add(new TextField("body", body.toString(), Field.Store.NO));
    if (i % 17 != 0) {
      BytesRef value = new BytesRef("v" + (i * 37 % 200));
      doc.add(new SortedDocValuesField("s", value));
      doc.add(new BinaryDocValuesField("sv", value));
    }
    if (i % 13 != 0) {
      long l = (i * 7919L % 1000) - 500;
      doc.add(new NumericDocValuesField("l", l));
      doc.add(new LongPoint("l", l));
    }
    if (i % 11 != 0) {
      int v = i * 104729 % 50;
      doc.add(new NumericDocValuesField("i", v));
      doc.add(new IntPoint("i", v));
    }
    doc.add(new DoubleDocValuesField("d", (i % 23) * 0.5));
    doc.add(new FloatDocValuesField("f", (i % 11) * 0.25f));
    return doc;
  }

  static SortField missing(SortField field, Object value) {
    field.setMissingValue(value);
    return field;
  }

  /** The sorts, by name. */
  static Map<String, Sort> sorts() {
    Map<String, Sort> sorts = new LinkedHashMap<>();
    sorts.put("s", new Sort(new SortField("s", SortField.Type.STRING)));
    sorts.put("s_rev", new Sort(new SortField("s", SortField.Type.STRING, true)));
    sorts.put(
        "s_last",
        new Sort(missing(new SortField("s", SortField.Type.STRING), SortField.STRING_LAST)));
    sorts.put("l", new Sort(new SortField("l", SortField.Type.LONG)));
    sorts.put("l_rev", new Sort(new SortField("l", SortField.Type.LONG, true)));
    sorts.put("l_missing", new Sort(missing(new SortField("l", SortField.Type.LONG), 1000L)));
    sorts.put("i", new Sort(new SortField("i", SortField.Type.INT)));
    sorts.put("d_rev", new Sort(new SortField("d", SortField.Type.DOUBLE, true)));
    sorts.put("f", new Sort(new SortField("f", SortField.Type.FLOAT)));
    sorts.put("score", new Sort(SortField.FIELD_SCORE));
    sorts.put("doc", new Sort(SortField.FIELD_DOC));
    sorts.put("sv", new Sort(new SortField("sv", SortField.Type.STRING_VAL)));
    sorts.put(
        "s_l_rev",
        new Sort(
            new SortField("s", SortField.Type.STRING),
            new SortField("l", SortField.Type.LONG, true)));
    sorts.put("score_doc", new Sort(SortField.FIELD_SCORE, SortField.FIELD_DOC));
    sorts.put(
        "f_i_s",
        new Sort(
            new SortField("f", SortField.Type.FLOAT),
            new SortField("i", SortField.Type.INT, true),
            new SortField("s", SortField.Type.STRING)));
    return sorts;
  }

  public static void main(String[] args) throws IOException {
    Path path = Paths.get(args[0]);
    Files.createDirectories(path);
    IndexWriterConfig config = new IndexWriterConfig();
    config.setMergePolicy(NoMergePolicy.INSTANCE);
    try (FSDirectory dir = FSDirectory.open(path);
        IndexWriter writer = new IndexWriter(dir, config)) {
      for (int i = 0; i < DOCS; i++) {
        writer.addDocument(doc(i));
        if ((i + 1) % (DOCS / SEGMENTS) == 0) {
          writer.commit();
        }
      }
    }
    try (FSDirectory dir = FSDirectory.open(path);
        DirectoryReader reader = DirectoryReader.open(dir)) {
      System.out.println("leaves=" + reader.leaves().size());
      ExecutorService executor = Executors.newFixedThreadPool(4);
      try {
        // Without an executor every slice is collected on this thread, in order,
        // so the total hit counts are reproducible; with one, the slices race and
        // only the top documents are.
        print("hits", new IndexSearcher(reader), true);
        print("xhits", new IndexSearcher(reader, executor), false);
      } finally {
        executor.shutdown();
      }
    }
  }

  static void print(String prefix, IndexSearcher searcher, boolean withTotals)
      throws IOException {
    System.out.println(prefix + "_slices=" + searcher.getSlices().length);
    {
      Map<String, Query> queries = new LinkedHashMap<>();
      queries.put("a", new TermQuery(new Term("body", "a")));
      queries.put("all", new MatchAllDocsQuery());
      for (Map.Entry<String, Sort> sort : sorts().entrySet()) {
        for (Map.Entry<String, Query> query : queries.entrySet()) {
          TopFieldDocs top =
              searcher.search(
                  query.getValue(), new TopFieldCollectorManager(sort.getValue(), 10, null, 1000));
          StringBuilder docs = new StringBuilder();
          for (ScoreDoc hit : top.scoreDocs) {
            if (docs.length() > 0) {
              docs.append(',');
            }
            docs.append(hit.doc);
          }
          String totals =
              withTotals ? top.totalHits.value() + " " + top.totalHits.relation() + " " : "";
          System.out.println(
              prefix + " " + sort.getKey() + " " + query.getKey() + " " + totals + docs);
        }
      }
    }
  }
}
