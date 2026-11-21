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
import java.net.InetAddress;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.DoubleRange;
import org.apache.lucene.document.FeatureField;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.FloatRange;
import org.apache.lucene.document.InetAddressPoint;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.IntRange;
import org.apache.lucene.document.KnnFloatVectorField;
import org.apache.lucene.document.LongField;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.document.LongRange;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedNumericDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.StoredFields;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.VectorSimilarityFunction;
import org.apache.lucene.search.FieldDoc;
import org.apache.lucene.search.FieldExistsQuery;
import org.apache.lucene.search.IndexOrDocValuesQuery;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.IndexSortSortedNumericDocValuesRangeQuery;
import org.apache.lucene.search.MatchAllDocsQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.Sort;
import org.apache.lucene.search.SortField;
import org.apache.lucene.search.SortedNumericSelector;
import org.apache.lucene.search.SortedNumericSortField;
import org.apache.lucene.search.SortedSetSelector;
import org.apache.lucene.search.SortedSetSortField;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.NumericUtils;

/**
 * Oracle of {@code tests/portability/dv_range_feature_queries.rs} (rmp task #409).
 *
 * <p>The corpus is defined here and in the Rust test by the same formulas. Modes:
 *
 * <ul>
 *   <li>{@code write <root>}: writes the main corpus into {@code <root>/main} (three commits,
 *       deletions in the first and the last segment) and the index-sorted corpus into {@code
 *       <root>/sorted} (index sort on the {@code ns} numeric doc values, two commits).
 *   <li>{@code query <root>}: runs the doc-values, point-set, range-field, inet-address, feature,
 *       distance-feature and field-exists queries and the selector sorts on {@code main}, and the
 *       index-sort range queries on {@code sorted}, printing the hits with their score bits and
 *       sort values.
 * </ul>
 */
public final class DvRangeFeatureFixture {

  // ---------------------------------------------------------------------------------------------
  // Corpus
  // ---------------------------------------------------------------------------------------------

  /** Documents of each batch of the main corpus; a commit closes each batch. */
  static final int[][] BATCHES = {{0, 100}, {100, 200}, {200, 300}};

  /** Documents of each batch of the index-sorted corpus. */
  static final int[][] SORTED_BATCHES = {{0, 100}, {100, 200}};

  static final String[] WORDS = {
    "apple", "apply", "ample", "maple", "apricot", "banana", "bandana", "cabana", "cherry", "sherry",
    "berry", "merry", "date", "data", "gate", "grape", "graph", "form", "from", "zebra"
  };

  /** Tokenized text with frequencies, positions and norms. */
  static final FieldType TEXT_TYPE = new FieldType();

  static {
    TEXT_TYPE.setTokenized(true);
    TEXT_TYPE.setIndexOptions(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    TEXT_TYPE.freeze();
  }

  /** Base of the {@code ts} timestamps, in milliseconds. */
  static final long TS_BASE = 1_600_000_000_000L;

  private DvRangeFeatureFixture() {}

  static String id(int i) {
    return String.format("id-%04d", i);
  }

  static byte[] ipv4(int i) {
    return new byte[] {10, 0, (byte) (i / 100), (byte) ((i * 7) % 256)};
  }

  static byte[] ipv6(int i) {
    byte[] b = new byte[16];
    b[0] = 0x20;
    b[1] = 0x01;
    b[2] = 0x0d;
    b[3] = (byte) 0xb8;
    b[14] = (byte) (i / 256);
    b[15] = (byte) (i % 256);
    return b;
  }

  static InetAddress inet(byte[] b) {
    try {
      return InetAddress.getByAddress(b);
    } catch (java.net.UnknownHostException e) {
      throw new IllegalArgumentException(e);
    }
  }

  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", id(i), Field.Store.YES));
    if (i % 7 != 6) {
      d.add(new SortedNumericDocValuesField("sn", (i * 37) % 101 - 50));
    }
    if (i % 3 == 0) {
      d.add(new SortedNumericDocValuesField("sn", (i * 13) % 97));
    }
    if (i % 5 != 4) {
      d.add(
          new SortedNumericDocValuesField(
              "snf", NumericUtils.floatToSortableInt(((i * 17) % 89) / 4.0f - 10.0f)));
    }
    if (i % 4 == 0) {
      d.add(
          new SortedNumericDocValuesField(
              "snf", NumericUtils.floatToSortableInt(((i * 29) % 71) / 2.0f - 5.0f)));
    }
    if (i % 6 != 5) {
      d.add(
          new SortedNumericDocValuesField(
              "snd", NumericUtils.doubleToSortableLong(((i * 23) % 103) / 8.0 - 6.0)));
    }
    if (i % 4 == 1) {
      d.add(
          new SortedNumericDocValuesField(
              "snd", NumericUtils.doubleToSortableLong(((i * 19) % 61) / 3.0 - 9.0)));
    }
    if (i % 11 != 10) {
      d.add(new NumericDocValuesField("num", (i * 41) % 211));
    }
    if (i % 9 != 8) {
      d.add(new SortedSetDocValuesField("ss", new BytesRef(WORDS[i % 20])));
      d.add(new SortedSetDocValuesField("ss", new BytesRef(WORDS[(i * 7 + 3) % 20])));
      if (i % 5 == 0) {
        d.add(new SortedSetDocValuesField("ss", new BytesRef(WORDS[(i * 3 + 1) % 20])));
      }
    }
    if (i % 10 != 9) {
      d.add(new IntPoint("ip", (i * 31) % 500));
    }
    long lv = (i * 53) % 1000;
    d.add(new LongPoint("lv", lv));
    d.add(new SortedNumericDocValuesField("lv", lv));
    int irMin = (i * 7) % 100;
    d.add(new IntRange("ir", new int[] {irMin}, new int[] {irMin + i % 13}));
    long lrMin = (i * 11) % 1000 - 500;
    d.add(new LongRange("lr", new long[] {lrMin}, new long[] {lrMin + (i % 17) * 3}));
    float frMin = (i % 50) / 2.0f;
    d.add(new FloatRange("fr", new float[] {frMin}, new float[] {frMin + (i % 7) / 4.0f}));
    double a = (i * 3) % 40;
    double b = (i * 7) % 30;
    d.add(
        new DoubleRange(
            "dr", new double[] {a, b}, new double[] {a + (i % 5 + 1), b + (i % 4) + 0.5}));
    d.add(new InetAddressPoint("inet", inet(ipv4(i))));
    if (i % 10 == 0) {
      d.add(new InetAddressPoint("inet", inet(ipv6(i))));
    }
    if (i % 8 != 7) {
      d.add(new FeatureField("features", "pagerank", 1.0f + ((i * 13) % 50) / 2.0f));
    }
    if (i % 2 == 0) {
      d.add(new FeatureField("features", "recency", 0.5f + (i % 30)));
    }
    if (i % 12 != 11) {
      d.add(new LongField("ts", TS_BASE + ((i * 7919L) % 100_000) * 1000, Field.Store.NO));
    }
    if (i % 4 != 3) {
      d.add(new Field("body", "w" + (i % 10) + " w" + ((i * 3) % 10), TEXT_TYPE));
    }
    if (i % 3 != 2) {
      d.add(
          new KnnFloatVectorField(
              "vec", new float[] {i % 10, (i % 7) + 1}, VectorSimilarityFunction.EUCLIDEAN));
    }
    return d;
  }

  static Document sortedDoc(int i) {
    Document d = new Document();
    d.add(new StringField("id", id(i), Field.Store.YES));
    d.add(new NumericDocValuesField("ns", (i * 41) % 211));
    return d;
  }

  static IndexWriterConfig config() {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    return config;
  }

  static void write(String root) throws IOException {
    try (Directory dir = FSDirectory.open(Paths.get(root, "main"));
        IndexWriter w = new IndexWriter(dir, config())) {
      for (int k = 0; k < BATCHES.length; k++) {
        for (int i = BATCHES[k][0]; i < BATCHES[k][1]; i++) {
          w.addDocument(doc(i));
        }
        if (k == 0) {
          w.deleteDocuments(new Term("id", id(7)));
        } else if (k == 2) {
          w.deleteDocuments(new Term("id", id(250)));
        }
        w.commit();
      }
    }
    IndexWriterConfig sortedConfig = config();
    sortedConfig.setIndexSort(new Sort(new SortField("ns", SortField.Type.LONG)));
    try (Directory dir = FSDirectory.open(Paths.get(root, "sorted"));
        IndexWriter w = new IndexWriter(dir, sortedConfig)) {
      for (int[] batch : SORTED_BATCHES) {
        for (int i = batch[0]; i < batch[1]; i++) {
          w.addDocument(sortedDoc(i));
        }
        w.commit();
      }
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Query set
  // ---------------------------------------------------------------------------------------------

  /** Hits printed per query. */
  static final int TOP_N = 20;

  /** Hits printed per sort. */
  static final int TOP_SORT = 40;

  static String bits(float f) {
    return String.format("%08x", Float.floatToIntBits(f));
  }

  static String bits(double d) {
    return String.format("%016x", Double.doubleToLongBits(d));
  }

  static BytesRef br(String s) {
    return new BytesRef(s);
  }

  static List<Object[]> queries() {
    List<Object[]> q = new ArrayList<>();
    q.add(new Object[] {"sn slowRange [-10,20]", SortedNumericDocValuesField.newSlowRangeQuery("sn", -10, 20)});
    q.add(
        new Object[] {
          "sn slowSet {0,5,13,40,-3}", SortedNumericDocValuesField.newSlowSetQuery("sn", 0, 5, 13, 40, -3)
        });
    q.add(new Object[] {"num slowRange [50,150]", NumericDocValuesField.newSlowRangeQuery("num", 50, 150)});
    q.add(
        new Object[] {
          "ss slowRange [banana,date)",
          SortedSetDocValuesField.newSlowRangeQuery("ss", br("banana"), br("date"), true, false)
        });
    q.add(
        new Object[] {
          "ss slowRange [*,cherry]", SortedSetDocValuesField.newSlowRangeQuery("ss", null, br("cherry"), true, true)
        });
    q.add(
        new Object[] {
          "ss slowSet {apple,grape,zebra,nothere}",
          SortedSetDocValuesField.newSlowSetQuery(
              "ss", List.of(br("apple"), br("grape"), br("zebra"), br("nothere")))
        });
    q.add(
        new Object[] {
          "lv indexOrDocValues [100,300]",
          new IndexOrDocValuesQuery(
              LongPoint.newRangeQuery("lv", 100, 300),
              SortedNumericDocValuesField.newSlowRangeQuery("lv", 100, 300))
        });
    for (String field : new String[] {"num", "sn", "ss", "body", "vec", "ts", "lv"}) {
      q.add(new Object[] {"exists " + field, new FieldExistsQuery(field)});
    }
    q.add(new Object[] {"ip set {3,31,62,93,499,1000}", IntPoint.newSetQuery("ip", 3, 31, 62, 93, 499, 1000)});
    q.add(new Object[] {"lv set {0,53,106,500,999}", LongPoint.newSetQuery("lv", 0, 53, 106, 500, 999)});
    q.add(new Object[] {"ir intersects [10,20]", IntRange.newIntersectsQuery("ir", new int[] {10}, new int[] {20})});
    q.add(new Object[] {"ir contains [40,41]", IntRange.newContainsQuery("ir", new int[] {40}, new int[] {41})});
    q.add(new Object[] {"ir within [0,60]", IntRange.newWithinQuery("ir", new int[] {0}, new int[] {60})});
    q.add(new Object[] {"ir crosses [25,35]", IntRange.newCrossesQuery("ir", new int[] {25}, new int[] {35})});
    q.add(
        new Object[] {
          "lr intersects [-100,100]", LongRange.newIntersectsQuery("lr", new long[] {-100}, new long[] {100})
        });
    q.add(new Object[] {"lr contains [0,1]", LongRange.newContainsQuery("lr", new long[] {0}, new long[] {1})});
    q.add(new Object[] {"lr within [-500,0]", LongRange.newWithinQuery("lr", new long[] {-500}, new long[] {0})});
    q.add(new Object[] {"lr crosses [200,260]", LongRange.newCrossesQuery("lr", new long[] {200}, new long[] {260})});
    q.add(
        new Object[] {"fr intersects [5,6]", FloatRange.newIntersectsQuery("fr", new float[] {5f}, new float[] {6f})});
    q.add(
        new Object[] {
          "fr contains [10.1,10.2]", FloatRange.newContainsQuery("fr", new float[] {10.1f}, new float[] {10.2f})
        });
    q.add(new Object[] {"fr within [0,12]", FloatRange.newWithinQuery("fr", new float[] {0f}, new float[] {12f})});
    q.add(
        new Object[] {
          "fr crosses [3.3,3.6]", FloatRange.newCrossesQuery("fr", new float[] {3.3f}, new float[] {3.6f})
        });
    q.add(
        new Object[] {
          "dr intersects [5,5]-[10,10]",
          DoubleRange.newIntersectsQuery("dr", new double[] {5, 5}, new double[] {10, 10})
        });
    q.add(
        new Object[] {
          "dr contains [20,10]-[20.5,10.5]",
          DoubleRange.newContainsQuery("dr", new double[] {20, 10}, new double[] {20.5, 10.5})
        });
    q.add(
        new Object[] {
          "dr within [0,0]-[30,25]", DoubleRange.newWithinQuery("dr", new double[] {0, 0}, new double[] {30, 25})
        });
    q.add(
        new Object[] {
          "dr crosses [10,10]-[12,12]",
          DoubleRange.newCrossesQuery("dr", new double[] {10, 10}, new double[] {12, 12})
        });
    q.add(
        new Object[] {
          "inet exact 10.0.0.7", InetAddressPoint.newExactQuery("inet", inet(new byte[] {10, 0, 0, 7}))
        });
    q.add(
        new Object[] {
          "inet prefix 10.0.1.0/24", InetAddressPoint.newPrefixQuery("inet", inet(new byte[] {10, 0, 1, 0}), 24)
        });
    q.add(
        new Object[] {
          "inet range [10.0.0.0,10.0.0.127]",
          InetAddressPoint.newRangeQuery(
              "inet", inet(new byte[] {10, 0, 0, 0}), inet(new byte[] {10, 0, 0, 127}))
        });
    q.add(
        new Object[] {
          "inet set {10.0.0.14,10.0.2.0,2001:db8::a}",
          InetAddressPoint.newSetQuery(
              "inet", inet(new byte[] {10, 0, 0, 14}), inet(new byte[] {10, 0, 2, 0}), inet(ipv6(10)))
        });
    q.add(
        new Object[] {
          "feature saturation pagerank w=1 pivot=10",
          FeatureField.newSaturationQuery("features", "pagerank", 1f, 10f)
        });
    q.add(
        new Object[] {
          "feature saturation pagerank auto pivot", FeatureField.newSaturationQuery("features", "pagerank")
        });
    q.add(
        new Object[] {
          "feature log pagerank w=2 scaling=1.5", FeatureField.newLogQuery("features", "pagerank", 2f, 1.5f)
        });
    q.add(
        new Object[] {
          "feature sigmoid recency w=1 pivot=15 exp=0.6",
          FeatureField.newSigmoidQuery("features", "recency", 1f, 15f, 0.6f)
        });
    q.add(
        new Object[] {
          "feature linear pagerank w=0.5", FeatureField.newLinearQuery("features", "pagerank", 0.5f)
        });
    q.add(
        new Object[] {
          "ts distanceFeature w=2 origin=+50000s pivot=5000s",
          LongField.newDistanceFeatureQuery("ts", 2f, TS_BASE + 50_000_000L, 5_000_000L)
        });
    return q;
  }

  static final SortedNumericSelector.Type[] NUMERIC_SELECTORS = {
    SortedNumericSelector.Type.MIN, SortedNumericSelector.Type.MAX
  };

  static final SortedSetSelector.Type[] SET_SELECTORS = {
    SortedSetSelector.Type.MIN,
    SortedSetSelector.Type.MAX,
    SortedSetSelector.Type.MIDDLE_MIN,
    SortedSetSelector.Type.MIDDLE_MAX
  };

  /** The missing value that sorts a document without a value first or last. */
  static Object missing(SortField.Type type, boolean low) {
    return switch (type) {
      case LONG -> low ? Long.MIN_VALUE : Long.MAX_VALUE;
      case INT -> low ? Integer.MIN_VALUE : Integer.MAX_VALUE;
      case FLOAT -> low ? Float.NEGATIVE_INFINITY : Float.POSITIVE_INFINITY;
      case DOUBLE -> low ? Double.NEGATIVE_INFINITY : Double.POSITIVE_INFINITY;
      default -> throw new IllegalArgumentException(type.toString());
    };
  }

  /** The sorts, by name, in the order they are printed. */
  static List<Object[]> sorts() {
    List<Object[]> s = new ArrayList<>();
    Object[][] numeric = {
      {"sn", SortField.Type.LONG},
      {"sn", SortField.Type.INT},
      {"snf", SortField.Type.FLOAT},
      {"snd", SortField.Type.DOUBLE}
    };
    for (Object[] n : numeric) {
      String field = (String) n[0];
      SortField.Type type = (SortField.Type) n[1];
      for (SortedNumericSelector.Type selector : NUMERIC_SELECTORS) {
        for (boolean reverse : new boolean[] {false, true}) {
          for (boolean first : new boolean[] {true, false}) {
            SortedNumericSortField sf = new SortedNumericSortField(field, type, reverse, selector);
            // First when ascending is the lowest value, first when descending the highest.
            sf.setMissingValue(missing(type, first != reverse));
            s.add(
                new Object[] {
                  "sortedNumeric "
                      + field
                      + " "
                      + type
                      + " "
                      + selector
                      + " reverse="
                      + reverse
                      + " missing="
                      + (first ? "first" : "last"),
                  new Sort(sf)
                });
          }
        }
      }
    }
    for (SortedSetSelector.Type selector : SET_SELECTORS) {
      for (boolean reverse : new boolean[] {false, true}) {
        for (boolean first : new boolean[] {true, false}) {
          SortedSetSortField sf = new SortedSetSortField("ss", reverse, selector);
          sf.setMissingValue(first ? SortField.STRING_FIRST : SortField.STRING_LAST);
          s.add(
              new Object[] {
                "sortedSet ss "
                    + selector
                    + " reverse="
                    + reverse
                    + " missing="
                    + (first ? "STRING_FIRST" : "STRING_LAST"),
                new Sort(sf)
              });
        }
      }
    }
    return s;
  }

  static List<Object[]> sortedQueries() {
    List<Object[]> q = new ArrayList<>();
    q.add(
        new Object[] {
          "indexSort ns [50,120]",
          new IndexSortSortedNumericDocValuesRangeQuery(
              "ns", 50, 120, NumericDocValuesField.newSlowRangeQuery("ns", 50, 120))
        });
    q.add(
        new Object[] {
          "indexSort ns [0,10]",
          new IndexSortSortedNumericDocValuesRangeQuery(
              "ns", 0, 10, NumericDocValuesField.newSlowRangeQuery("ns", 0, 10))
        });
    return q;
  }

  static String sortValue(Object v) {
    if (v == null) {
      return "null";
    } else if (v instanceof Long l) {
      return "Long:" + l;
    } else if (v instanceof Integer n) {
      return "Integer:" + n;
    } else if (v instanceof Float f) {
      return "Float:" + bits(f);
    } else if (v instanceof Double d) {
      return "Double:" + bits(d);
    } else if (v instanceof BytesRef b) {
      return "bytes:" + b.utf8ToString();
    }
    return v.getClass().getName() + ":" + v;
  }

  static void printHits(StoredFields stored, String header, TopDocs top) throws IOException {
    System.out.println(header + " total=" + top.totalHits.value() + " " + top.totalHits.relation());
    for (ScoreDoc hit : top.scoreDocs) {
      StringBuilder line = new StringBuilder();
      line.append("  hit doc=").append(hit.doc).append(" score=").append(bits(hit.score));
      if (hit instanceof FieldDoc fd) {
        line.append(" sort=").append(sortValue(fd.fields[0]));
      }
      line.append(" id=").append(stored.document(hit.doc).get("id"));
      System.out.println(line);
    }
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

  static void query(String root) throws IOException {
    try (Directory dir = FSDirectory.open(Paths.get(root, "main"));
        DirectoryReader reader = DirectoryReader.open(dir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      StoredFields stored = searcher.storedFields();
      System.out.println("index main");
      printReader(reader);
      for (Object[] q : queries()) {
        printHits(stored, "query " + q[0], searcher.search((Query) q[1], TOP_N));
      }
      for (Object[] s : sorts()) {
        printHits(
            stored, "sort " + s[0], searcher.search(MatchAllDocsQuery.INSTANCE, TOP_SORT, (Sort) s[1]));
      }
    }
    try (Directory dir = FSDirectory.open(Paths.get(root, "sorted"));
        DirectoryReader reader = DirectoryReader.open(dir)) {
      IndexSearcher searcher = new IndexSearcher(reader);
      StoredFields stored = searcher.storedFields();
      System.out.println("index sorted");
      printReader(reader);
      for (Object[] q : sortedQueries()) {
        printHits(stored, "query " + q[0], searcher.search((Query) q[1], TOP_N));
      }
    }
  }

  public static void main(String[] args) throws Exception {
    switch (args[0]) {
      case "write" -> write(args[1]);
      case "query" -> query(args[1]);
      default -> throw new IllegalArgumentException(args[0]);
    }
  }
}
