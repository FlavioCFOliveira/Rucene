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

import java.net.InetAddress;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.DoublePoint;
import org.apache.lucene.document.FeatureField;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.FloatPoint;
import org.apache.lucene.document.InetAddressPoint;
import org.apache.lucene.document.InetAddressRange;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.LatLonDocValuesField;
import org.apache.lucene.document.LatLonPoint;
import org.apache.lucene.document.LatLonShape;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.document.XYDocValuesField;
import org.apache.lucene.document.XYPointField;
import org.apache.lucene.document.XYShape;
import org.apache.lucene.index.BinaryDocValues;
import org.apache.lucene.index.CheckIndex;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.DocValues;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.IndexableField;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.SortedDocValues;
import org.apache.lucene.index.SortedNumericDocValues;
import org.apache.lucene.index.SortedSetDocValues;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.search.SortedNumericSortField;
import org.apache.lucene.search.SortedSetSortField;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;

/**
 * Reference tool for documents that carry a stored field and a doc-values field under one name
 * ({@code IndexingChain.FieldSchema}): every doc-values type, with and without an index sort.
 *
 * <ul>
 *   <li>{@code write <root>} writes one index per (type, sort) case under {@code root}.
 *   <li>{@code read <root>} opens every case under {@code root} and prints, per document, the
 *       stored values and the doc values of the field, the leaf's index sort, and whether {@code
 *       CheckIndex} finds the index clean.
 *   <li>{@code schema} indexes the documents of the schema-validation cases into memory and prints
 *       whether each one is accepted, or the exception it raises.
 *   <li>{@code flush} commits, under an index sort, documents whose sort field carries no doc
 *       values ({@code IndexingChain.java:243}, {@code DocValues.java:206-222}) or has no {@code
 *       FieldInfo} because its only document failed validation ({@code IndexingChain.java:187}),
 *       and prints the outcome of the commit and whether the writer is open.
 * </ul>
 *
 * <p>{@code tests/stored_field_and_doc_values.rs} writes the same cases with Rucene, has this tool
 * read both, and replays the schema cases.
 *
 * <p>Command-line usage: {@code java -cp lucene-core-10.5.0.jar StoredDocValuesFixture.java
 * <mode> [root]}.
 */
public final class StoredDocValuesFixture {

  private StoredDocValuesFixture() {}

  static final String[] KINDS = {"NUMERIC", "BINARY", "SORTED", "SORTED_NUMERIC", "SORTED_SET"};

  static final int DOCS = 3;

  /** The doc-values field of document {@code i} for {@code kind}: value {@code 2 - i}. */
  static List<IndexableField> docValues(String kind, int i) {
    int v = 2 - i;
    List<IndexableField> out = new ArrayList<>();
    switch (kind) {
      case "NUMERIC" -> out.add(new NumericDocValuesField("f", v));
      case "BINARY" -> out.add(new BinaryDocValuesField("f", new BytesRef("b" + v)));
      case "SORTED" -> out.add(new SortedDocValuesField("f", new BytesRef("s" + v)));
      case "SORTED_NUMERIC" -> {
        out.add(new SortedNumericDocValuesField("f", v));
        out.add(new SortedNumericDocValuesField("f", 10 + v));
      }
      case "SORTED_SET" -> {
        out.add(new SortedSetDocValuesField("f", new BytesRef("t" + v)));
        out.add(new SortedSetDocValuesField("f", new BytesRef("u" + v)));
      }
      default -> throw new AssertionError(kind);
    }
    return out;
  }

  /** The index sort of the sorted case: the field itself where core can sort by its type. */
  static Sort sort(String kind) {
    return switch (kind) {
      case "NUMERIC" -> new Sort(new SortField("f", SortField.Type.LONG));
      case "BINARY" -> new Sort(new SortField("key", SortField.Type.LONG));
      case "SORTED" -> new Sort(new SortField("f", SortField.Type.STRING));
      case "SORTED_NUMERIC" -> new Sort(new SortedNumericSortField("f", SortField.Type.LONG));
      case "SORTED_SET" -> new Sort(new SortedSetSortField("f", false));
      default -> throw new AssertionError(kind);
    };
  }

  /**
   * Document {@code i}: a stored field and the doc values under the name {@code f}, the stored
   * field first in even documents and last in odd ones, and a {@code key} numeric field.
   */
  static Document doc(String kind, int i) {
    Document d = new Document();
    StoredField stored = new StoredField("f", "v" + i);
    if (i % 2 == 0) {
      d.add(stored);
    }
    for (IndexableField f : docValues(kind, i)) {
      d.add(f);
    }
    if (i % 2 == 1) {
      d.add(stored);
    }
    d.add(new NumericDocValuesField("key", 2 - i));
    return d;
  }

  static void write(Path root) throws Exception {
    for (String kind : KINDS) {
      for (boolean sorted : new boolean[] {false, true}) {
        Path path = root.resolve(kind + (sorted ? "-sorted" : "-unsorted"));
        Files.createDirectories(path);
        try (Directory dir = FSDirectory.open(path)) {
          IndexWriterConfig config = new IndexWriterConfig();
          if (sorted) {
            config.setIndexSort(sort(kind));
          }
          try (IndexWriter w = new IndexWriter(dir, config)) {
            for (int i = 0; i < DOCS; i++) {
              w.addDocument(doc(kind, i));
            }
          }
        }
      }
    }
  }

  static String docValuesOf(LeafReader leaf, String kind, int doc) throws Exception {
    StringBuilder sb = new StringBuilder();
    switch (kind) {
      case "NUMERIC" -> {
        NumericDocValues v = DocValues.getNumeric(leaf, "f");
        if (v.advanceExact(doc)) sb.append(v.longValue());
      }
      case "BINARY" -> {
        BinaryDocValues v = DocValues.getBinary(leaf, "f");
        if (v.advanceExact(doc)) sb.append(v.binaryValue().utf8ToString());
      }
      case "SORTED" -> {
        SortedDocValues v = DocValues.getSorted(leaf, "f");
        if (v.advanceExact(doc)) {
          sb.append(v.ordValue()).append('=').append(v.lookupOrd(v.ordValue()).utf8ToString());
        }
      }
      case "SORTED_NUMERIC" -> {
        SortedNumericDocValues v = DocValues.getSortedNumeric(leaf, "f");
        if (v.advanceExact(doc)) {
          for (int k = 0; k < v.docValueCount(); k++) sb.append(k == 0 ? "" : ",").append(v.nextValue());
        }
      }
      case "SORTED_SET" -> {
        SortedSetDocValues v = DocValues.getSortedSet(leaf, "f");
        if (v.advanceExact(doc)) {
          for (int k = 0; k < v.docValueCount(); k++) {
            long ord = v.nextOrd();
            sb.append(k == 0 ? "" : ",").append(ord).append('=').append(v.lookupOrd(ord).utf8ToString());
          }
        }
      }
      default -> throw new AssertionError(kind);
    }
    return sb.toString();
  }

  static void read(Path root) throws Exception {
    for (String kind : KINDS) {
      for (boolean sorted : new boolean[] {false, true}) {
        String name = kind + (sorted ? "-sorted" : "-unsorted");
        System.out.println("case " + name);
        try (Directory dir = FSDirectory.open(root.resolve(name));
            DirectoryReader reader = DirectoryReader.open(dir)) {
          for (LeafReaderContext ctx : reader.leaves()) {
            LeafReader leaf = ctx.reader();
            System.out.println(
                "  leaf maxDoc=" + leaf.maxDoc() + " sort=" + leaf.getMetaData().sort());
            StoredFields stored = leaf.storedFields();
            for (int doc = 0; doc < leaf.maxDoc(); doc++) {
              System.out.println(
                  "  doc "
                      + doc
                      + " stored="
                      + String.join(",", stored.document(doc).getValues("f"))
                      + " dv="
                      + docValuesOf(leaf, kind, doc));
            }
          }
          try (CheckIndex check = new CheckIndex(dir)) {
            System.out.println("  clean=" + check.checkIndex().clean);
          }
        }
      }
    }
  }

  static void add(IndexWriter w, String label, IndexableField... fields) {
    Document d = new Document();
    for (IndexableField f : fields) {
      d.add(f);
    }
    try {
      w.addDocument(d);
      System.out.println(label + ": ok");
    } catch (Exception e) {
      System.out.println(label + ": " + e.getClass().getSimpleName() + ": " + e.getMessage());
    }
  }

  static IndexWriter memoryWriter(Sort sort) throws Exception {
    IndexWriterConfig config = new IndexWriterConfig();
    if (sort != null) {
      config.setIndexSort(sort);
    }
    return new IndexWriter(new ByteBuffersDirectory(), config);
  }

  static void schema() throws Exception {
    // The frozen-type shortcut: a document whose field reuses the frozen type validated last is
    // not validated again (IndexingChain.java:625-643).
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "Q1 doc0 [stored, sdv]", new StoredField("id", "a"), new SortedDocValuesField("id", new BytesRef("a")));
      add(w, "Q1 doc1 [stored]", new StoredField("id", "b"));
      add(w, "Q1 doc2 [sdv]", new SortedDocValuesField("id", new BytesRef("c")));
      add(w, "Q1 doc3 [stored]", new StoredField("id", "d"));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "Q2 doc0 [sdv, stored]", new SortedDocValuesField("id", new BytesRef("a")), new StoredField("id", "a"));
      add(w, "Q2 doc1 [stored]", new StoredField("id", "b"));
      add(w, "Q2 doc2 [sdv]", new SortedDocValuesField("id", new BytesRef("c")));
    }
    try (IndexWriter w = memoryWriter(null)) {
      FieldType ft = new FieldType();
      ft.setStored(true);
      add(w, "Q3 doc0 [stored(unfrozen), sdv]", new Field("id", "a", ft), new SortedDocValuesField("id", new BytesRef("a")));
      add(w, "Q3 doc1 [stored(unfrozen)]", new Field("id", "b", ft));
    }
    try (IndexWriter w = memoryWriter(null)) {
      FieldType t1 = new FieldType();
      t1.setStored(true);
      t1.freeze();
      FieldType t2 = new FieldType();
      t2.setStored(true);
      t2.freeze();
      add(w, "Q4 doc0 [t1, sdv]", new Field("id", "a", t1), new SortedDocValuesField("id", new BytesRef("a")));
      add(w, "Q4 doc1 [t2]", new Field("id", "b", t2));
      add(w, "Q4 doc2 [t1]", new Field("id", "c", t1));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "Q5 doc0 [stored]", new StoredField("id", "a"));
      add(w, "Q5 doc1 [stored, sdv]", new StoredField("id", "b"), new SortedDocValuesField("id", new BytesRef("b")));
      add(w, "Q5 doc2 [stored]", new StoredField("id", "c"));
    }
    // Changing the schema across documents, within a segment and across segments.
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "X1 doc0 [sdv]", new SortedDocValuesField("id", new BytesRef("a")));
      add(w, "X1 doc1 [ndv]", new NumericDocValuesField("id", 1));
      add(w, "X1 doc2 [stored, ndv]", new StoredField("id", "x"), new NumericDocValuesField("id", 1));
      w.commit();
      add(w, "X1 doc3 next segment [ndv]", new NumericDocValuesField("id", 1));
      add(w, "X1 doc4 next segment [stored, ndv]", new StoredField("id", "x"), new NumericDocValuesField("id", 1));
    }
    // Contradicting instances within one document, and index options.
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "X2 doc0 [sdv, ndv]", new SortedDocValuesField("id", new BytesRef("a")), new NumericDocValuesField("id", 1));
      add(w, "X2 doc1 [string, sdv]", new StringField("id", "a", Field.Store.YES), new SortedDocValuesField("id", new BytesRef("a")));
      add(w, "X2 doc2 [text]", new TextField("id", "a b", Field.Store.NO));
    }
    // An index sort on the field itself.
    try (IndexWriter w = memoryWriter(new Sort(new SortField("id", SortField.Type.STRING)))) {
      add(w, "X3 sort-by-id [stored, ndv]", new StoredField("id", "a"), new NumericDocValuesField("id", 1));
      add(w, "X3 sort-by-id [stored, sdv]", new StoredField("id", "a"), new SortedDocValuesField("id", new BytesRef("a")));
    }
    // The shortcut compares types by reference (IndexingChain.java:1803-1825): a point builds a
    // new type per instance (IntPoint.java:48-53 and its siblings), so a later point is
    // validated; the geo, address and shape fields share one static type per class, so a later
    // field of the same class is not; FeatureField's two types are never frozen.
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "P1 doc0 [intpoint, ndv]", new IntPoint("p", 1), new NumericDocValuesField("p", 1));
      add(w, "P1 doc1 [intpoint]", new IntPoint("p", 2));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "P2 doc0 [longpoint, ndv]", new LongPoint("p", 1L), new NumericDocValuesField("p", 1));
      add(w, "P2 doc1 [longpoint]", new LongPoint("p", 2L));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "P3 doc0 [floatpoint, ndv]", new FloatPoint("p", 1f), new NumericDocValuesField("p", 1));
      add(w, "P3 doc1 [floatpoint]", new FloatPoint("p", 2f));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "P4 doc0 [doublepoint, ndv]", new DoublePoint("p", 1d), new NumericDocValuesField("p", 1));
      add(w, "P4 doc1 [doublepoint]", new DoublePoint("p", 2d));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "G1 doc0 [latlonpoint, latlondv]", new LatLonPoint("g", 1, 2), new LatLonDocValuesField("g", 1, 2));
      add(w, "G1 doc1 [latlonpoint]", new LatLonPoint("g", 3, 4));
      add(w, "G1 doc2 [xypoint]", new XYPointField("g", 3, 4));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "G2 doc0 [xypoint, xydv]", new XYPointField("g", 1, 2), new XYDocValuesField("g", 1, 2));
      add(w, "G2 doc1 [xypoint]", new XYPointField("g", 3, 4));
      add(w, "G2 doc2 [latlonpoint]", new LatLonPoint("g", 3, 4));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "D1 doc0 [latlondv, latlonpoint]", new LatLonDocValuesField("g", 1, 2), new LatLonPoint("g", 1, 2));
      add(w, "D1 doc1 [latlondv]", new LatLonDocValuesField("g", 3, 4));
      add(w, "D1 doc2 [xydv]", new XYDocValuesField("g", 3, 4));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "D2 doc0 [xydv, xypoint]", new XYDocValuesField("g", 1, 2), new XYPointField("g", 1, 2));
      add(w, "D2 doc1 [xydv]", new XYDocValuesField("g", 3, 4));
      add(w, "D2 doc2 [latlondv]", new LatLonDocValuesField("g", 3, 4));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "I1 doc0 [inetpoint, ndv]", new InetAddressPoint("a", address(1)), new NumericDocValuesField("a", 1));
      add(w, "I1 doc1 [inetpoint]", new InetAddressPoint("a", address(2)));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "R1 doc0 [inetrange, ndv]", new InetAddressRange("r", address(1), address(2)), new NumericDocValuesField("r", 1));
      add(w, "R1 doc1 [inetrange]", new InetAddressRange("r", address(3), address(4)));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "T1 doc0 [latlonshape, ndv]", concat(LatLonShape.createIndexableFields("s", 1, 2), new NumericDocValuesField("s", 1)));
      add(w, "T1 doc1 [xyshape]", XYShape.createIndexableFields("s", 3, 4));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "S1 doc0 [latlonshapedv, intpoint]", LatLonShape.createDocValueField("s", 1, 2), new IntPoint("s", 1));
      add(w, "S1 doc1 [xyshapedv]", XYShape.createDocValueField("s", 3, 4));
    }
    try (IndexWriter w = memoryWriter(null)) {
      add(w, "F1 doc0 [feature, ndv]", new FeatureField("f", "a", 1f), new NumericDocValuesField("f", 1));
      add(w, "F1 doc1 [feature]", new FeatureField("f", "b", 2f));
    }
    System.out.println(
        "F0 frozen="
            + ((FieldType) new FeatureField("f", "a", 1f).fieldType()).isFrozen()
            + " with-term-vectors="
            + ((FieldType) new FeatureField("f", "a", 1f, true).fieldType()).isFrozen());
  }

  static InetAddress address(int last) throws Exception {
    return InetAddress.getByAddress(new byte[] {10, 0, 0, (byte) last});
  }

  static IndexableField[] concat(IndexableField[] fields, IndexableField last) {
    IndexableField[] all = java.util.Arrays.copyOf(fields, fields.length + 1);
    all[fields.length] = last;
    return all;
  }

  /** Commits a writer, printing the outcome and whether it is still open. */
  static void commit(IndexWriter w, String label) {
    try {
      w.commit();
      System.out.println(label + " commit: ok");
    } catch (Throwable e) {
      System.out.println(label + " commit: " + e.getClass().getSimpleName() + ": " + e.getMessage());
    }
    System.out.println(label + " open=" + w.isOpen());
  }

  static void flush() throws Exception {
    // A sort over a field indexed only as a StringField: the buffered reader answers no doc
    // values and getFieldInfos() the segment's field infos, so DocValues.checkField fails the
    // flush, which aborts the writer.
    Sort[] sorts = {
      new Sort(new SortField("f", SortField.Type.LONG)),
      new Sort(new SortedSetSortField("f", false)),
      new Sort(new SortedNumericSortField("f", SortField.Type.LONG)),
      new Sort(new SortField("f", SortField.Type.STRING))
    };
    for (Sort sort : sorts) {
      IndexWriter w = memoryWriter(sort);
      add(w, "N " + sort + " doc0", new StringField("f", "x", Field.Store.NO));
      add(w, "N " + sort + " doc1", new StringField("f", "y", Field.Store.NO));
      commit(w, "N " + sort);
    }
    // The sort field's only document failed validation, so its PerField has no FieldInfo, which
    // the buffered reader dereferences.
    IndexWriter w = memoryWriter(new Sort(new SortField("f", SortField.Type.LONG)));
    add(w, "M doc0 [sdv]", new SortedDocValuesField("f", new BytesRef("x")));
    add(w, "M doc1 [ndv key]", new NumericDocValuesField("key", 1));
    commit(w, "M");
  }

  /** Runs the mode named by {@code args[0]}. */
  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "write" -> write(Paths.get(args[1]));
      case "read" -> read(Paths.get(args[1]));
      case "schema" -> schema();
      case "flush" -> flush();
      default -> throw new IllegalArgumentException("unknown mode " + args[0]);
    }
  }
}
