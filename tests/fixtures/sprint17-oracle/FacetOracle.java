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

import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.Random;
import java.util.ArrayList;
import java.util.List;
import java.util.Collections;
import java.util.Map;
import java.util.TreeMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.DoubleDocValuesField;
import org.apache.lucene.document.DoubleRangeDocValuesField;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.LongRangeDocValuesField;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.RangeFieldQuery;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.facet.DrillDownQuery;
import org.apache.lucene.facet.DrillSideways;
import org.apache.lucene.facet.FacetField;
import org.apache.lucene.facet.FacetResult;
import org.apache.lucene.facet.Facets;
import org.apache.lucene.facet.FacetsCollector;
import org.apache.lucene.facet.FacetsCollectorManager;
import org.apache.lucene.facet.FacetsConfig;
import org.apache.lucene.facet.facetset.DimRange;
import org.apache.lucene.facet.facetset.ExactFacetSetMatcher;
import org.apache.lucene.facet.facetset.FacetSetDecoder;
import org.apache.lucene.facet.facetset.FacetSetMatcher;
import org.apache.lucene.facet.facetset.FacetSetsField;
import org.apache.lucene.facet.facetset.LongFacetSet;
import org.apache.lucene.facet.facetset.MatchingFacetSetsCounts;
import org.apache.lucene.facet.facetset.RangeFacetSetMatcher;
import org.apache.lucene.facet.range.DoubleRange;
import org.apache.lucene.facet.range.DoubleRangeFacetCounts;
import org.apache.lucene.facet.range.LongRange;
import org.apache.lucene.facet.range.LongRangeFacetCounts;
import org.apache.lucene.facet.rangeonrange.DoubleRangeOnRangeFacetCounts;
import org.apache.lucene.facet.rangeonrange.LongRangeOnRangeFacetCounts;
import org.apache.lucene.facet.sortedset.ConcurrentSortedSetDocValuesFacetCounts;
import org.apache.lucene.facet.sortedset.DefaultSortedSetDocValuesReaderState;
import org.apache.lucene.facet.sortedset.SortedSetDocValuesFacetCounts;
import org.apache.lucene.facet.sortedset.SortedSetDocValuesFacetField;
import org.apache.lucene.facet.sortedset.SortedSetDocValuesReaderState;
import org.apache.lucene.facet.taxonomy.AssociationAggregationFunction;
import org.apache.lucene.facet.taxonomy.FacetLabel;
import org.apache.lucene.facet.taxonomy.FastTaxonomyFacetCounts;
import org.apache.lucene.facet.taxonomy.FloatAssociationFacetField;
import org.apache.lucene.facet.taxonomy.IntAssociationFacetField;
import org.apache.lucene.facet.taxonomy.ParallelTaxonomyArrays;
import org.apache.lucene.facet.taxonomy.TaxonomyFacetFloatAssociations;
import org.apache.lucene.facet.taxonomy.TaxonomyFacetIntAssociations;
import org.apache.lucene.facet.taxonomy.TaxonomyReader;
import org.apache.lucene.facet.taxonomy.directory.DirectoryTaxonomyReader;
import org.apache.lucene.facet.taxonomy.directory.DirectoryTaxonomyWriter;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;

/**
 * The Java side of the sprint-17 oracle harness for {@code lucene/facet} (rmp #628,
 * {@code tests/sprint17_oracle/mod.rs}): {@code main(seed, scenario, input, args...)} prints the
 * dump of the scenario, which Rucene's test computes from the same seed.
 */
public class FacetOracle {
  /** The seeded generators of the harness (`tests/sprint17_oracle/mod.rs`, `corpus`). */
  static final class Corpus {
    static final String[] VOCABULARY = {
      "the", "of", "and", "to", "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
      "hotel", "india", "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec",
      "romeo", "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu"
    };

    static String word(Random r) {
      return VOCABULARY[r.nextInt(VOCABULARY.length)];
    }

    static String text(Random r, int n) {
      StringBuilder sb = new StringBuilder();
      for (int i = 0; i < n; i++) {
        if (i > 0) sb.append(' ');
        sb.append(word(r));
      }
      return sb.toString();
    }
  }

  /** Printable ASCII except the backslash as is, every other UTF-16 unit as a u-escape. */
  static String escape(String s) {
    if (s == null) return "null";
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (c >= 0x20 && c <= 0x7e && c != '\\') sb.append(c);
      else sb.append(String.format("\\u%04X", (int) c));
    }
    return sb.toString();
  }

  static String bits(float f) {
    return Integer.toHexString(Float.floatToRawIntBits(f));
  }

  static String bits(double d) {
    return Long.toHexString(Double.doubleToRawLongBits(d));
  }

  static String input(String path) throws IOException {
    return path.equals("-") ? null : Files.readString(Path.of(path), StandardCharsets.UTF_8);
  }

  public static void main(String[] args) throws Exception {
    long seed = Long.parseLong(args[0]);
    String scenario = args[1];
    String input = input(args[2]);
    String[] rest = Arrays.copyOfRange(args, 3, args.length);
    PrintStream out =
        new PrintStream(new FileOutputStream(FileDescriptor.out), false, StandardCharsets.UTF_8);
    run(scenario, new Random(seed), input, rest, out);
    out.flush();
  }

  static IllegalArgumentException unknown(String scenario) {
    return new IllegalArgumentException("unknown scenario: " + scenario);
  }

  static void run(String scenario, Random r, String input, String[] args, PrintStream out)
      throws Exception {
    switch (scenario) {
      case "smoke" -> smoke(r, out);
      case "taxonomy" -> taxonomy(r, out);
      case "sortedset" -> sortedSet(r, out);
      case "range" -> range(r, out);
      case "drill" -> drill(r, out);
      case "taxo-write" -> taxoWrite(r, Path.of(args[0]), out);
      case "taxo-read" -> taxoRead(Path.of(args[0]), out);
      default -> throw unknown(scenario);
    }
  }

  /**
   * Twenty seeded paths (a dimension and one to four components, each a word into which the
   * delimiter and escape characters may be spliced): {@code FacetsConfig.pathToString} of each,
   * then {@code stringToPath} of the result.
   */
  static void smoke(Random r, PrintStream out) throws Exception {
    for (int i = 0; i < 20; i++) {
      List<String> path = new ArrayList<>();
      path.add("dim" + r.nextInt(3));
      int n = 1 + r.nextInt(4);
      for (int j = 0; j < n; j++) {
        StringBuilder component = new StringBuilder(Corpus.word(r));
        int splice = r.nextInt(4);
        if (splice == 1) component.insert(r.nextInt(component.length() + 1), FacetsConfig.DELIM_CHAR);
        if (splice == 2) component.insert(r.nextInt(component.length() + 1), '\u001E');
        path.add(component.toString());
      }
      String encoded = FacetsConfig.pathToString(path.toArray(new String[0]));
      out.println("path " + escape(String.join("/", path)));
      out.println("encoded " + escape(encoded));
      out.println("decoded " + escape(String.join("/", FacetsConfig.stringToPath(encoded))));
    }
  }

  // ---------------------------------------------------------------------------------------------
  // The facet corpus of the portability scenarios (rmp #630). Rucene's
  // `crates/rucene-facet/tests/facet_oracle.rs` draws the same values in the same order.
  // ---------------------------------------------------------------------------------------------

  static final int NUM_DOCS = 40;

  /** One generated document. */
  static final class Doc {
    String id;
    String body;
    String author;
    String[] date;
    List<String> tags = new ArrayList<>();
    List<String> intLabels = new ArrayList<>();
    List<Integer> intValues = new ArrayList<>();
    List<String> floatLabels = new ArrayList<>();
    List<Float> floatValues = new ArrayList<>();
    long price;
    double weight;
    List<Long> multi = new ArrayList<>();
    long lrMin, lrMax;
    double drMin, drMax;
    List<long[]> sets = new ArrayList<>();

    String describe() {
      StringBuilder sb = new StringBuilder("doc " + id + " body=" + body + " author=" + author);
      sb.append(" date=").append(String.join("/", date));
      sb.append(" tags=").append(tags);
      sb.append(" int=[");
      for (int i = 0; i < intLabels.size(); i++) {
        if (i > 0) sb.append(", ");
        sb.append(intLabels.get(i)).append(':').append(intValues.get(i));
      }
      sb.append("] float=[");
      for (int i = 0; i < floatLabels.size(); i++) {
        if (i > 0) sb.append(", ");
        sb.append(floatLabels.get(i)).append(':').append(bits(floatValues.get(i)));
      }
      sb.append("] price=").append(price).append(" weight=").append(bits(weight));
      sb.append(" multi=").append(multi);
      sb.append(" lr=").append(lrMin).append("..").append(lrMax);
      sb.append(" dr=").append(bits(drMin)).append("..").append(bits(drMax));
      sb.append(" fs=[");
      for (int i = 0; i < sets.size(); i++) {
        if (i > 0) sb.append(", ");
        sb.append('(').append(sets.get(i)[0]).append(',').append(sets.get(i)[1]).append(')');
      }
      return sb.append(']').toString();
    }
  }

  static String vocab(int i) {
    return Corpus.VOCABULARY[i];
  }

  static Doc gen(Random r, int i) {
    Doc d = new Doc();
    d.id = String.format("d%02d", i);
    d.body = "b" + r.nextInt(3);
    d.author = vocab(4 + r.nextInt(6));
    d.date =
        new String[] {
          Integer.toString(2010 + r.nextInt(4)),
          Integer.toString(1 + r.nextInt(12)),
          Integer.toString(1 + r.nextInt(28))
        };
    int nTags = r.nextInt(4);
    for (int t = 0; t < nTags; t++) {
      String tag = vocab(10 + r.nextInt(8));
      if (!d.tags.contains(tag)) d.tags.add(tag);
    }
    int nInt = 1 + r.nextInt(2);
    for (int t = 0; t < nInt; t++) {
      String label = vocab(18 + r.nextInt(4));
      int value = r.nextInt(100);
      if (!d.intLabels.contains(label)) {
        d.intLabels.add(label);
        d.intValues.add(value);
      }
    }
    int nFloat = 1 + r.nextInt(2);
    for (int t = 0; t < nFloat; t++) {
      String label = vocab(22 + r.nextInt(4));
      float value = r.nextFloat() * 10;
      if (!d.floatLabels.contains(label)) {
        d.floatLabels.add(label);
        d.floatValues.add(value);
      }
    }
    d.price = r.nextInt(1000) - 100;
    d.weight = r.nextDouble() * 100;
    int nMulti = r.nextInt(3);
    for (int t = 0; t < nMulti; t++) d.multi.add((long) r.nextInt(50));
    d.lrMin = r.nextInt(100);
    d.lrMax = d.lrMin + r.nextInt(50);
    d.drMin = r.nextDouble() * 100;
    d.drMax = d.drMin + r.nextDouble() * 30;
    int nSets = 1 + r.nextInt(2);
    for (int t = 0; t < nSets; t++) d.sets.add(new long[] {r.nextInt(5), r.nextInt(5)});
    return d;
  }

  static FacetsConfig config() {
    FacetsConfig c = new FacetsConfig();
    c.setHierarchical("Publish Date", true);
    c.setDrillDownTermsIndexing(
        "Publish Date", FacetsConfig.DrillDownTermsIndexing.ALL_PATHS_NO_DIM);
    c.setMultiValued("Tags", true);
    c.setRequireDimCount("Tags", true);
    c.setIndexFieldName("Tags", "$tags");
    c.setMultiValued("Int", true);
    c.setIndexFieldName("Int", "$int");
    c.setMultiValued("Float", true);
    c.setIndexFieldName("Float", "$float");
    c.setIndexFieldName("SAuthor", "$ssdv");
    c.setHierarchical("SDate", true);
    c.setIndexFieldName("SDate", "$ssdv");
    c.setMultiValued("STags", true);
    c.setIndexFieldName("STags", "$ssdv");
    return c;
  }

  static Document document(Doc d) throws IOException {
    Document doc = new Document();
    doc.add(new StringField("id", d.id, Field.Store.YES));
    doc.add(new StringField("body", d.body, Field.Store.NO));
    doc.add(new FacetField("Author", d.author));
    doc.add(new FacetField("Publish Date", d.date));
    for (String t : d.tags) doc.add(new FacetField("Tags", t));
    for (int i = 0; i < d.intLabels.size(); i++) {
      doc.add(new IntAssociationFacetField(d.intValues.get(i), "Int", d.intLabels.get(i)));
    }
    for (int i = 0; i < d.floatLabels.size(); i++) {
      doc.add(new FloatAssociationFacetField(d.floatValues.get(i), "Float", d.floatLabels.get(i)));
    }
    doc.add(new SortedSetDocValuesFacetField("SAuthor", d.author));
    doc.add(new SortedSetDocValuesFacetField("SDate", d.date));
    for (String t : d.tags) doc.add(new SortedSetDocValuesFacetField("STags", t));
    doc.add(new LongPoint("price", d.price));
    doc.add(new NumericDocValuesField("price", d.price));
    doc.add(new DoubleDocValuesField("weight", d.weight));
    for (long m : d.multi) doc.add(new SortedNumericDocValuesField("multi", m));
    doc.add(
        new org.apache.lucene.document.LongRange(
            "lr", new long[] {d.lrMin}, new long[] {d.lrMax}));
    doc.add(
        new org.apache.lucene.document.DoubleRange(
            "dr", new double[] {d.drMin}, new double[] {d.drMax}));
    doc.add(new LongRangeDocValuesField("lr", new long[] {d.lrMin}, new long[] {d.lrMax}));
    doc.add(new DoubleRangeDocValuesField("dr", new double[] {d.drMin}, new double[] {d.drMax}));
    LongFacetSet[] sets = new LongFacetSet[d.sets.size()];
    for (int i = 0; i < sets.length; i++) sets[i] = new LongFacetSet(d.sets.get(i));
    doc.add(FacetSetsField.create("fs", sets));
    return doc;
  }

  /**
   * Indexes {@link #NUM_DOCS} generated documents into {@code indexDir} and {@code taxoDir} (a
   * commit after the first half, so the main index has two segments) and prints one line per
   * document.
   */
  static FacetsConfig build(Random r, Directory indexDir, Directory taxoDir, PrintStream out)
      throws IOException {
    FacetsConfig config = config();
    try (IndexWriter w = new IndexWriter(indexDir, new IndexWriterConfig());
        DirectoryTaxonomyWriter tw = new DirectoryTaxonomyWriter(taxoDir)) {
      for (int i = 0; i < NUM_DOCS; i++) {
        Doc d = gen(r, i);
        out.println(d.describe());
        w.addDocument(config.build(tw, document(d)));
        if (i == NUM_DOCS / 2 - 1) w.commit();
      }
      tw.commit();
      w.commit();
    }
    return config;
  }

  static void print(PrintStream out, String what, FacetResult result) {
    out.print(what + " ");
    out.print(result == null ? "null\n" : result.toString());
  }

  static void printAll(PrintStream out, String what, List<FacetResult> results) {
    out.println(what + " " + results.size());
    for (FacetResult fr : results) out.print(fr.toString());
  }

  static void value(PrintStream out, String what, Facets facets, String dim, String... path)
      throws IOException {
    String v;
    try {
      v = String.valueOf(facets.getSpecificValue(dim, path));
    } catch (IllegalArgumentException | UnsupportedOperationException e) {
      v = e.toString();
    }
    out.println(what + " " + v);
  }

  static String hits(IndexSearcher searcher, TopDocs td) throws IOException {
    StoredFields stored = searcher.storedFields();
    List<String> ids = new ArrayList<>();
    for (ScoreDoc sd : td.scoreDocs) ids.add(stored.document(sd.doc).get("id"));
    Collections.sort(ids);
    return td.totalHits.value() + " " + ids;
  }

  static List<Query> queries() {
    return List.of(MatchAllDocsQuery.INSTANCE, new TermQuery(new Term("body", "b0")));
  }

  /** Taxonomy counts and associations over the corpus, for two queries. */
  static void taxonomy(Random r, PrintStream out) throws Exception {
    Directory indexDir = new ByteBuffersDirectory();
    Directory taxoDir = new ByteBuffersDirectory();
    FacetsConfig config = build(r, indexDir, taxoDir, out);
    try (DirectoryReader reader = DirectoryReader.open(indexDir);
        DirectoryTaxonomyReader taxo = new DirectoryTaxonomyReader(taxoDir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      out.println("taxonomy size " + taxo.getSize());
      for (Query q : queries()) {
        FacetsCollectorManager.FacetsResult res =
            FacetsCollectorManager.search(searcher, q, 10, new FacetsCollectorManager());
        FacetsCollector fc = res.facetsCollector();
        out.println("query " + q + " hits " + res.topDocs().totalHits.value());
        Facets counts = new FastTaxonomyFacetCounts(taxo, config, fc);
        printAll(out, "allDims", counts.getAllDims(10));
        print(out, "top3 date", counts.getTopChildren(3, "Publish Date"));
        print(out, "top5 date/2011", counts.getTopChildren(5, "Publish Date", "2011"));
        print(out, "top5 date/2099", counts.getTopChildren(5, "Publish Date", "2099"));
        print(out, "all author", counts.getAllChildren("Author"));
        value(out, "value author/alpha", counts, "Author", "alpha");
        value(out, "value author", counts, "Author");
        value(out, "value date/2012", counts, "Publish Date", "2012");
        printAll(out, "topDims 2 2", counts.getTopDims(2, 2));
        Facets tags = new FastTaxonomyFacetCounts("$tags", taxo, config, fc);
        printAll(out, "tags allDims", tags.getAllDims(10));
        print(out, "tags top2", tags.getTopChildren(2, "Tags"));
        value(out, "value tags", tags, "Tags");
        value(out, "value tags/golf", tags, "Tags", "golf");
        for (AssociationAggregationFunction f :
            List.of(AssociationAggregationFunction.SUM, AssociationAggregationFunction.MAX)) {
          String name = f == AssociationAggregationFunction.SUM ? "sum" : "max";
          Facets ints = new TaxonomyFacetIntAssociations("$int", taxo, config, fc, f);
          printAll(out, "int " + name + " allDims", ints.getAllDims(10));
          print(out, "int " + name + " top2", ints.getTopChildren(2, "Int"));
          value(out, "int " + name + " value/papa", ints, "Int", "papa");
          Facets floats = new TaxonomyFacetFloatAssociations("$float", taxo, config, fc, f);
          printAll(out, "float " + name + " allDims", floats.getAllDims(10));
          print(out, "float " + name + " top2", floats.getTopChildren(2, "Float"));
          value(out, "float " + name + " value/tango", floats, "Float", "tango");
        }
      }
      Facets all =
          new FastTaxonomyFacetCounts(FacetsConfig.DEFAULT_INDEX_FIELD_NAME, reader, taxo, config);
      printAll(out, "countAll allDims", all.getAllDims(10));
    }
  }

  /** Sorted-set doc-values counts (plain and concurrent) over the corpus. */
  static void sortedSet(Random r, PrintStream out) throws Exception {
    Directory indexDir = new ByteBuffersDirectory();
    Directory taxoDir = new ByteBuffersDirectory();
    FacetsConfig config = build(r, indexDir, taxoDir, out);
    ExecutorService exec = Executors.newFixedThreadPool(2);
    try (DirectoryReader reader = DirectoryReader.open(indexDir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      SortedSetDocValuesReaderState state =
          new DefaultSortedSetDocValuesReaderState(reader, "$ssdv", config);
      List<String> dims = new ArrayList<>();
      for (String dim : state.getDims()) dims.add(dim);
      out.println("state size " + state.getSize() + " dims " + dims);
      for (Query q : queries()) {
        FacetsCollector fc =
            FacetsCollectorManager.search(searcher, q, 10, new FacetsCollectorManager())
                .facetsCollector();
        out.println("query " + q);
        Facets ssdv = new SortedSetDocValuesFacetCounts(state, fc);
        printAll(out, "allDims", ssdv.getAllDims(10));
        print(out, "top3 date", ssdv.getTopChildren(3, "SDate"));
        print(out, "top10 date/2011", ssdv.getTopChildren(10, "SDate", "2011"));
        print(out, "all tags", ssdv.getAllChildren("STags"));
        print(out, "all date/2012/5", ssdv.getAllChildren("SDate", "2012", "5"));
        value(out, "value author/alpha", ssdv, "SAuthor", "alpha");
        value(out, "value date/2013", ssdv, "SDate", "2013");
        printAll(out, "topDims 2 3", ssdv.getTopDims(2, 3));
        Facets conc = new ConcurrentSortedSetDocValuesFacetCounts(state, fc, exec);
        printAll(out, "concurrent allDims", conc.getAllDims(10));
        print(out, "concurrent top3 date", conc.getTopChildren(3, "SDate"));
      }
      printAll(out, "countAll", new SortedSetDocValuesFacetCounts(state).getAllDims(10));
      printAll(
          out,
          "concurrent countAll",
          new ConcurrentSortedSetDocValuesFacetCounts(state, exec).getAllDims(10));
    } finally {
      exec.shutdown();
    }
  }

  /** Range, range-on-range and facet-set counts over the corpus. */
  static void range(Random r, PrintStream out) throws Exception {
    Directory indexDir = new ByteBuffersDirectory();
    Directory taxoDir = new ByteBuffersDirectory();
    build(r, indexDir, taxoDir, out);
    try (DirectoryReader reader = DirectoryReader.open(indexDir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      LongRange[] price = new LongRange[6];
      for (int i = 0; i < 5; i++) {
        long min = r.nextInt(1000) - 150;
        long max = min + 2 + r.nextInt(400);
        boolean minInc = r.nextBoolean(), maxInc = r.nextBoolean();
        price[i] = new LongRange("p" + i, min, minInc, max, maxInc);
        out.println("price range " + price[i]);
      }
      price[5] = new LongRange("all", Long.MIN_VALUE, true, Long.MAX_VALUE, true);
      LongRange[] multi = new LongRange[4];
      for (int i = 0; i < 4; i++) {
        long min = r.nextInt(50);
        long max = min + 1 + r.nextInt(20);
        multi[i] = new LongRange("m" + i, min, true, max, r.nextBoolean());
        out.println("multi range " + multi[i]);
      }
      DoubleRange[] weight = new DoubleRange[5];
      for (int i = 0; i < 5; i++) {
        double min = r.nextDouble() * 80;
        double max = min + r.nextDouble() * 40;
        weight[i] = new DoubleRange("w" + i, min, r.nextBoolean(), max, r.nextBoolean());
        out.println("weight range " + weight[i]);
      }
      org.apache.lucene.facet.rangeonrange.LongRange[] lr =
          new org.apache.lucene.facet.rangeonrange.LongRange[4];
      for (int i = 0; i < 4; i++) {
        long min = r.nextInt(120);
        long max = min + 2 + r.nextInt(60);
        lr[i] =
            new org.apache.lucene.facet.rangeonrange.LongRange(
                "lr" + i, min, r.nextBoolean(), max, r.nextBoolean());
        out.println("lr range " + lr[i]);
      }
      org.apache.lucene.facet.rangeonrange.DoubleRange[] dr =
          new org.apache.lucene.facet.rangeonrange.DoubleRange[4];
      for (int i = 0; i < 4; i++) {
        double min = r.nextDouble() * 100;
        double max = min + r.nextDouble() * 50;
        dr[i] =
            new org.apache.lucene.facet.rangeonrange.DoubleRange(
                "dr" + i, min, r.nextBoolean(), max, r.nextBoolean());
        out.println("dr range " + dr[i]);
      }
      List<FacetSetMatcher> matchers = new ArrayList<>();
      for (int i = 0; i < 3; i++) {
        long a = r.nextInt(5), b = r.nextInt(5);
        matchers.add(new ExactFacetSetMatcher("e" + i + "(" + a + "," + b + ")", new LongFacetSet(a, b)));
      }
      for (int i = 0; i < 2; i++) {
        long a = r.nextInt(4), b = r.nextInt(4);
        matchers.add(
            new RangeFacetSetMatcher(
                "r" + i + "(" + a + ".." + (a + 1) + "," + b + ".." + (b + 2) + ")",
                DimRange.fromLongs(a, true, a + 1, true),
                DimRange.fromLongs(b, true, b + 2, false)));
      }
      for (Query q : queries()) {
        FacetsCollector fc =
            FacetsCollectorManager.search(searcher, q, 10, new FacetsCollectorManager())
                .facetsCollector();
        out.println("query " + q);
        Facets p = new LongRangeFacetCounts("price", fc, price);
        print(out, "price top3", p.getTopChildren(3, "price"));
        print(out, "price all", p.getAllChildren("price"));
        printAll(out, "price allDims", p.getAllDims(10));
        Facets m = new LongRangeFacetCounts("multi", fc, multi);
        print(out, "multi all", m.getAllChildren("multi"));
        Facets w = new DoubleRangeFacetCounts("weight", fc, weight);
        print(out, "weight top10", w.getTopChildren(10, "weight"));
        print(out, "weight all", w.getAllChildren("weight"));
        for (RangeFieldQuery.QueryType type : RangeFieldQuery.QueryType.values()) {
          Facets l = new LongRangeOnRangeFacetCounts("lr", fc, type, lr);
          print(out, "lr " + type + " all", l.getAllChildren("lr"));
          Facets d = new DoubleRangeOnRangeFacetCounts("dr", fc, type, dr);
          print(out, "dr " + type + " all", d.getAllChildren("dr"));
        }
        Facets fs =
            new MatchingFacetSetsCounts(
                "fs", fc, FacetSetDecoder::decodeLongs, matchers.toArray(new FacetSetMatcher[0]));
        print(out, "fs top10", fs.getTopChildren(10, "fs"));
        print(out, "fs all", fs.getAllChildren("fs"));
        printAll(out, "fs allDims", fs.getAllDims(10));
      }
    }
  }

  static void sideways(PrintStream out, String what, IndexSearcher searcher, DrillSideways.DrillSidewaysResult res, String... dims)
      throws IOException {
    out.println(what + " hits " + hits(searcher, res.hits));
    for (String dim : dims) print(out, what + " " + dim, res.facets.getTopChildren(10, dim));
  }

  /** DrillDownQuery hits and DrillSideways results (taxonomy and sorted-set). */
  static void drill(Random r, PrintStream out) throws Exception {
    Directory indexDir = new ByteBuffersDirectory();
    Directory taxoDir = new ByteBuffersDirectory();
    FacetsConfig config = build(r, indexDir, taxoDir, out);
    try (DirectoryReader reader = DirectoryReader.open(indexDir);
        DirectoryTaxonomyReader taxo = new DirectoryTaxonomyReader(taxoDir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      SortedSetDocValuesReaderState state =
          new DefaultSortedSetDocValuesReaderState(reader, "$ssdv", config);
      for (int round = 0; round < 4; round++) {
        String a1 = vocab(4 + r.nextInt(6)), a2 = vocab(4 + r.nextInt(6));
        String year = Integer.toString(2010 + r.nextInt(4));
        String tag = vocab(10 + r.nextInt(8));
        boolean withBase = r.nextBoolean();
        Query base = withBase ? new TermQuery(new Term("body", "b" + r.nextInt(3))) : null;
        out.println("round " + round + " " + a1 + " " + a2 + " " + year + " " + tag + " " + base);

        DrillDownQuery ddq = new DrillDownQuery(config, base);
        ddq.add("Author", a1);
        ddq.add("Author", a2);
        ddq.add("Publish Date", year);
        out.println("ddq " + ddq);
        out.println("ddq hits " + hits(searcher, searcher.search(ddq, 100)));
        DrillDownQuery ddqTags = new DrillDownQuery(config, base);
        ddqTags.add("Tags", tag);
        out.println("ddq tags hits " + hits(searcher, searcher.search(ddqTags, 100)));

        DrillSideways ds = new DrillSideways(searcher, config, taxo);
        sideways(out, "ds", searcher, ds.search(ddq, 100), "Author", "Publish Date");

        DrillDownQuery sddq = new DrillDownQuery(config, base);
        sddq.add("SAuthor", a1);
        sddq.add("STags", tag);
        DrillSideways sds = new DrillSideways(searcher, config, state);
        sideways(out, "sds", searcher, sds.search(sddq, 100), "SAuthor", "STags", "SDate");
      }
    }
  }

  /** Writes the corpus into {@code dir}/index and {@code dir}/taxo, then prints {@link #taxoRead}. */
  static void taxoWrite(Random r, Path dir, PrintStream out) throws Exception {
    try (Directory indexDir = FSDirectory.open(dir.resolve("index"));
        Directory taxoDir = FSDirectory.open(dir.resolve("taxo"))) {
      build(r, indexDir, taxoDir, out);
    }
    taxoRead(dir, out);
  }

  /**
   * The content of the taxonomy and main index under {@code dir}: file names, every ordinal with
   * its parent, child, sibling and label, the commit data, and the facet counts of the main index.
   */
  static void taxoRead(Path dir, PrintStream out) throws Exception {
    FacetsConfig config = config();
    try (Directory indexDir = FSDirectory.open(dir.resolve("index"));
        Directory taxoDir = FSDirectory.open(dir.resolve("taxo"));
        DirectoryReader reader = DirectoryReader.open(indexDir);
        DirectoryTaxonomyReader taxo = new DirectoryTaxonomyReader(taxoDir)) {
      out.println("taxo files " + files(taxoDir));
      out.println("index files " + files(indexDir));
      out.println("taxo commit " + new TreeMap<>(taxo.getCommitUserData()));
      int size = taxo.getSize();
      out.println("taxo size " + size);
      ParallelTaxonomyArrays arrays = taxo.getParallelTaxonomyArrays();
      for (int ord = 0; ord < size; ord++) {
        FacetLabel label = taxo.getPath(ord);
        out.println(
            "ord "
                + ord
                + " parent "
                + arrays.parents().get(ord)
                + " child "
                + arrays.children().get(ord)
                + " sibling "
                + arrays.siblings().get(ord)
                + " "
                + label
                + " -> "
                + taxo.getOrdinal(label));
      }
      out.println("index maxDoc " + reader.maxDoc() + " numDocs " + reader.numDocs());
      for (LeafReaderContext leaf : reader.leaves()) {
        out.println("leaf " + leaf.ord + " maxDoc " + leaf.reader().maxDoc());
      }
      IndexSearcher searcher = new IndexSearcher(reader);
      out.println("ids " + hits(searcher, searcher.search(MatchAllDocsQuery.INSTANCE, 100)));
      printAll(
          out,
          "counts",
          new FastTaxonomyFacetCounts(FacetsConfig.DEFAULT_INDEX_FIELD_NAME, reader, taxo, config)
              .getAllDims(10));
      printAll(
          out,
          "tags counts",
          new FastTaxonomyFacetCounts("$tags", reader, taxo, config).getAllDims(10));
      FacetsCollector fc =
          FacetsCollectorManager.search(
                  searcher, MatchAllDocsQuery.INSTANCE, 10, new FacetsCollectorManager())
              .facetsCollector();
      printAll(
          out,
          "int sum",
          new TaxonomyFacetIntAssociations(
                  "$int", taxo, config, fc, AssociationAggregationFunction.SUM)
              .getAllDims(10));
      printAll(
          out,
          "float sum",
          new TaxonomyFacetFloatAssociations(
                  "$float", taxo, config, fc, AssociationAggregationFunction.SUM)
              .getAllDims(10));
      printAll(
          out,
          "ssdv counts",
          new SortedSetDocValuesFacetCounts(
                  new DefaultSortedSetDocValuesReaderState(reader, "$ssdv", config))
              .getAllDims(10));
    }
  }

  /** The sorted file names of {@code dir}, without the lock file. */
  static List<String> files(Directory dir) throws IOException {
    List<String> names = new ArrayList<>();
    for (String name : dir.listAll()) if (!name.equals("write.lock")) names.add(name);
    Collections.sort(names);
    return names;
  }
}
