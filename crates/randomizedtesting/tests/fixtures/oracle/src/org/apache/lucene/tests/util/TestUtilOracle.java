package org.apache.lucene.tests.util;

import com.carrotsearch.randomizedtesting.OracleSupport;
import java.io.File;
import java.io.PrintWriter;
import java.math.BigInteger;
import java.util.Random;
import org.apache.lucene.document.Document;
import org.apache.lucene.index.IndexableField;
import org.apache.lucene.index.IndexableFieldType;
import org.apache.lucene.store.ByteBuffersDataOutput;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.tests.util.automaton.AutomatonTestUtil;
import org.apache.lucene.tests.util.fst.FSTTester;
import org.apache.lucene.util.fst.FST;
import org.apache.lucene.util.fst.PositiveIntOutputs;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.IntsRef;
import org.apache.lucene.util.automaton.Automaton;
import org.apache.lucene.util.automaton.Operations;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.lucene90.Lucene90DocValuesFormat;
import org.apache.lucene.codecs.simpletext.SimpleTextCodec;
import org.apache.lucene.document.BinaryDocValuesField;
import org.apache.lucene.document.BinaryPoint;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.IntField;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.KeywordField;
import org.apache.lucene.document.LongField;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.SortedSetDocValuesField;
import org.apache.lucene.document.StoredField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.ConcurrentMergeScheduler;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FilterMergePolicy;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LogByteSizeMergePolicy;
import org.apache.lucene.index.LogDocMergePolicy;
import org.apache.lucene.index.LogMergePolicy;
import org.apache.lucene.index.MergePolicy;
import org.apache.lucene.index.MergeScheduler;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.TieredMergePolicy;
import org.apache.lucene.search.FieldDoc;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.TotalHits;
import org.apache.lucene.tests.codecs.asserting.AssertingCodec;
import org.apache.lucene.tests.codecs.blockterms.LuceneFixedGap;
import org.apache.lucene.tests.codecs.cheapbastard.CheapBastardCodec;

/**
 * The oracle of the {@code org.apache.lucene.tests.util} helpers of rmp #342, for {@code
 * crates/rucene-test-framework/tests/test_util_oracle.rs}. Writes into {@code
 * <out>/rucene-test-framework/}:
 *
 * <ul>
 *   <li>{@code testutil_english.tsv}: {@code English.intToEnglish} of every int in [-1000, 1000],
 *       {@code Integer.MIN_VALUE} and {@code Integer.MAX_VALUE}; {@code English.longToEnglish} of
 *       {@code ±10^k} and {@code 10^k - 1} for k in [0, 18], {@code Long.MIN_VALUE}, {@code
 *       Long.MAX_VALUE}, and 500 longs of {@code new Random(0)} (each also divided by {@code
 *       10^(i % 19)}).
 *   <li>{@code testutil_generators.tsv}: for every {@code TestUtil} random generator, each of its
 *       parameter sets and each seed of {@code OracleSupport.seeds()}: the FNV-1a 64-bit digest
 *       of {@value #DRAWS} consecutive results drawn from one {@code new Random(seed)} (a string as
 *       its length and UTF-16 code units, a {@code byte[]} or {@code BytesRef} as its length and
 *       bytes, a number as its little-endian bytes, all little-endian), the {@code nextLong()} the
 *       generator leaves next, and, for the first seed, the first 200 characters of the first result
 *       ({@code units}).
 *   <li>{@code testutil_patterns.tsv}: per seed, the patterns of {@value #DRAWS} {@code
 *       randomPattern} calls on one {@code new Random(seed)}.
 *   <li>{@code testutil_linefiledocs.tsv}: {@code new LineFileDocs(new Random(seed),
 *       "europarl.lines.txt.gz")} for the first 8 seeds of {@code OracleSupport.seeds()} (30
 *       documents each) and for the first seed {@code s >= 0} whose {@code .seek} draw is the last
 *       skip point (1000 documents, past the end of the file and its rewind; every document of
 *       the first 30 and every 50th): per document its index, title, date (a line's fields hold no tab), the digest
 *       of its body, and its other fields ({@code name=value}, sparse fields with their
 *       class); then one row per field name (the first document) with its field type's properties.
 *       ({@code nextDoc} draws its sparse fields into a {@code Document} it drops: the returned
 *       documents never hold them.)
 *   <li>{@code testutil_automaton.tsv}: as {@code testutil_generators.tsv}, for {@code
 *       AutomatonTestUtil.randomRegexp} (40 draws), {@code randomSingleAutomaton} (private,
 *       by reflection) and {@code randomAutomaton} (10 draws, as {@code toDot()}), and {@code
 *       operations} (5 draws): of a {@code randomAutomaton}, its state count and {@code
 *       isDeterministicSlow}, {@code determinizeSimple(a).toDot()}, {@code minimizeSimple} of
 *       that without dead states ({@code d}), {@code isFinite(d)}, {@code sameLanguage(d, min)},
 *       {@code subsetOf(min, d)}, the sorted {@code getFiniteStringsRecursive(d, 50)} of a non-empty finite
 *       {@code d}, and 10 {@code RandomAcceptedStrings(d)} strings, as one string.
 *   <li>{@code testutil_fsttester.tsv}: for input modes 0 and 1 and the first 10 seeds, the
 *       pairs of {@code r.nextInt(300)} {@code FSTTester.getRandomString(r)} terms (none for the
 *       first seed), deduplicated and sorted as {@code IntsRef}s, each with a {@code
 *       PositiveIntOutputs} output {@code r.nextInt(1000)}; {@code new FSTTester(new
 *       Random(r.nextLong()), new ByteBuffersDirectory(), mode, pairs, outputs).doTest()}: the
 *       node and arc counts, the length and digest of the FST's {@code save(out, out)} bytes, and
 *       the tester random's next {@code nextLong()}.
 *   <li>{@code testutil_compare.tsv}: {@code STRING_CODEPOINT_COMPARATOR}, {@code
 *       bytesRefToString}, {@code floatUlpEquals} and {@code doubleUlpEquals} on fixed inputs.
 *   <li>{@code testutil_members.tsv}: {@code reduceOpenFiles} (then {@code
 *       syncConcurrentMerges}) on a live writer per merge policy and scheduler (the policy's
 *       {@code noCFSRatio} and factors, the scheduler's merge and thread counts after); {@code
 *       getPostingsFormat}, {@code getDocValuesFormat} per codec and field, their one-argument
 *       forms and {@code fieldSupportsHugeBinaryDocValues}; {@code cloneDocument} (the {@code
 *       toString()} of every field of the clone, or the error per single-field document); {@code
 *       addIndexesSlowly} (segments and documents after); {@code assertConsistent} per case
 *       ({@code ok} or the error).
 *   <li>{@code testutil_ramusage.tsv}: {@code RamUsageTester.ramUsed} and {@code humanSizeOf} of
 *       primitive arrays, strings, {@code BytesRef}s, reference arrays (shared and {@code null}
 *       elements), nested arrays, an {@code ArrayList} and a {@code HashMap} of strings, and
 *       {@code ramUsed} with an {@code Accumulator} that does not follow array elements.
 * </ul>
 */
public final class TestUtilOracle {
  private TestUtilOracle() {}

  /** The number of results each generator row digests. */
  static final int DRAWS = 40;

  interface Gen {
    Object draw(Random r) throws Exception;
  }

  static final class Digest {
    long h = 0xcbf29ce484222325L;

    void b(int v) {
      h ^= (v & 0xff);
      h *= 0x100000001b3L;
    }

    void i(int v) {
      for (int k = 0; k < 4; k++) b(v >>> (8 * k));
    }

    void add(Object o) {
      if (o instanceof String s) {
        i(s.length());
        for (int k = 0; k < s.length(); k++) {
          b(s.charAt(k));
          b(s.charAt(k) >>> 8);
        }
      } else if (o instanceof char[] c) {
        add(new String(c));
      } else if (o instanceof byte[] a) {
        i(a.length);
        for (byte x : a) b(x);
      } else if (o instanceof BytesRef br) {
        // The whole array (randomBinaryTerm fills it) and the length.
        add(br.bytes);
        i(br.offset);
        i(br.length);
      } else if (o instanceof BigInteger bi) {
        add(bi.toByteArray());
      } else if (o instanceof Long l) {
        for (int k = 0; k < 8; k++) b((int) (l >>> (8 * k)));
      } else if (o instanceof Integer n) {
        i(n);
      } else {
        throw new AssertionError(o);
      }
    }
  }

  static String show(Object o) {
    if (o instanceof String s) return OracleSupport.units(s);
    if (o instanceof char[] c) return OracleSupport.units(new String(c));
    if (o instanceof BigInteger bi) return hex(bi.toByteArray());
    if (o instanceof byte[] a) return hex(a);
    if (o instanceof BytesRef br) return hex(br.bytes) + "/" + br.length;
    return String.valueOf(o);
  }

  static String hex(byte[] a) {
    StringBuilder sb = new StringBuilder();
    for (byte x : a) sb.append(String.format("%02x", x & 0xff));
    return sb.length() == 0 ? "-" : sb.toString();
  }

  static void row(PrintWriter w, String gen, String params, Gen g) throws Exception {
    row(w, gen, params, DRAWS, g);
  }

  static void row(PrintWriter w, String gen, String params, int draws, Gen g) throws Exception {
    long[] seeds = OracleSupport.seeds();
    for (int si = 0; si < seeds.length; si++) {
      Random r = new Random(seeds[si]);
      Digest d = new Digest();
      String first = null;
      for (int k = 0; k < draws; k++) {
        Object o = g.draw(r);
        if (first == null) {
          first = show(o);
          first = first.substring(0, Math.min(first.length(), 200));
        }
        d.add(o);
      }
      w.println(
          gen
              + "\t"
              + params
              + "\t"
              + seeds[si]
              + "\t"
              + String.format("%016x", d.h)
              + "\t"
              + r.nextLong()
              + "\t"
              + (si == 0 ? first : "-"));
    }
  }

  static void generators(PrintWriter w) throws Exception {
    w.println("# generator\tparams\tseed\tdigest\tnextLong after\tfirst result (first seed)");
    long[][] longRanges = {
      {0, 10},
      {-5, 5},
      {0, Integer.MAX_VALUE - 1},
      {0, Integer.MAX_VALUE},
      {-(1L << 40), 1L << 40},
      {Long.MIN_VALUE, -1},
      {0, Long.MAX_VALUE},
      {Long.MIN_VALUE, Long.MAX_VALUE},
    };
    for (long[] range : longRanges) {
      row(w, "nextLong", range[0] + "," + range[1], r -> TestUtil.nextLong(r, range[0], range[1]));
    }
    for (int max : new int[] {1, 4, 20}) {
      row(w, "nextBigInteger", "" + max, r -> TestUtil.nextBigInteger(r, max));
    }
    row(w, "randomSimpleString", "-", r -> TestUtil.randomSimpleString(r));
    for (int max : new int[] {0, 1, 5, 50}) {
      row(w, "randomSimpleString", "" + max, r -> TestUtil.randomSimpleString(r, max));
    }
    for (int[] mm : new int[][] {{3, 3}, {2, 10}}) {
      row(
          w,
          "randomSimpleString",
          mm[0] + "," + mm[1],
          r -> TestUtil.randomSimpleString(r, mm[0], mm[1]));
    }
    for (int[] range : new int[][] {{'a', 'c', 10}, {0x20, 0x7e, 30}, {0xe000, 0xffff, 5}}) {
      row(
          w,
          "randomSimpleStringRange",
          range[0] + "," + range[1] + "," + range[2],
          r -> TestUtil.randomSimpleStringRange(r, (char) range[0], (char) range[1], range[2]));
    }
    row(w, "randomUnicodeString", "-", r -> TestUtil.randomUnicodeString(r));
    for (int max : new int[] {1, 2, 3, 50}) {
      row(w, "randomUnicodeString", "" + max, r -> TestUtil.randomUnicodeString(r, max));
    }
    for (int[] ol : new int[][] {{10, 0, 10}, {10, 3, 5}, {2, 0, 2}}) {
      row(
          w,
          "randomFixedLengthUnicodeString",
          ol[0] + "," + ol[1] + "," + ol[2],
          r -> {
            char[] chars = new char[ol[0]];
            TestUtil.randomFixedLengthUnicodeString(r, chars, ol[1], ol[2]);
            return chars;
          });
    }
    row(w, "randomRegexpishString", "-", r -> TestUtil.randomRegexpishString(r));
    for (int max : new int[] {0, 5, 100}) {
      row(w, "randomRegexpishString", "" + max, r -> TestUtil.randomRegexpishString(r, max));
    }
    for (int n : new int[] {0, 1, 5, 30}) {
      row(w, "randomHtmlishString", "" + n, r -> TestUtil.randomHtmlishString(r, n));
    }
    String[] recase = {
      "script", "Straße ǅ İı ﬀ", "Ǆǅǆ Σσςΐϐϑᲀẞ",
      "𐐨𐐀 µᾳ"
    };
    for (String s : recase) {
      row(
          w,
          "randomlyRecaseCodePoints",
          OracleSupport.units(s),
          r -> TestUtil.randomlyRecaseCodePoints(r, s));
    }
    row(w, "randomRealisticUnicodeString", "-", r -> TestUtil.randomRealisticUnicodeString(r));
    row(
        w,
        "randomRealisticUnicodeString",
        "5",
        r -> TestUtil.randomRealisticUnicodeString(r, 5));
    row(
        w,
        "randomRealisticUnicodeString",
        "3,8",
        r -> TestUtil.randomRealisticUnicodeString(r, 3, 8));
    for (int len : new int[] {0, 1, 2, 3, 4, 10, 40}) {
      row(
          w,
          "randomFixedByteLengthUnicodeString",
          "" + len,
          r -> TestUtil.randomFixedByteLengthUnicodeString(r, len));
    }
    row(w, "randomBinaryTerm", "-", r -> TestUtil.randomBinaryTerm(r));
    for (int len : new int[] {0, 7}) {
      row(w, "randomBinaryTerm", "" + len, r -> TestUtil.randomBinaryTerm(r, len));
    }
    for (int max : new int[] {1, 10, 50, 200}) {
      for (boolean simple : new boolean[] {true, false}) {
        row(
            w,
            "randomAnalysisString",
            max + "," + simple,
            r -> TestUtil.randomAnalysisString(r, max, simple));
      }
    }
    for (int len : new int[] {0, 1, 5, 30}) {
      for (boolean simple : new boolean[] {true, false}) {
        row(
            w,
            "randomSubString",
            len + "," + simple,
            r -> TestUtil.randomSubString(r, len, simple));
      }
    }
    row(
        w,
        "stringToCharSequence",
        "-",
        r -> TestUtil.stringToCharSequence("aé𐐀", r).toString());
  }

  static void patterns(PrintWriter w) {
    w.println("# seed\tthe patterns of 40 randomPattern(r) calls on one new Random(seed)");
    for (long seed : OracleSupport.seeds()) {
      Random r = new Random(seed);
      StringBuilder sb = new StringBuilder().append(seed);
      for (int k = 0; k < DRAWS; k++) {
        sb.append('\t').append(TestUtil.randomPattern(r).pattern());
      }
      w.println(sb);
    }
  }

  static String fieldValue(IndexableField f) {
    if (f.numericValue() != null) return f.numericValue().toString();
    if (f.stringValue() != null) return f.stringValue();
    return hex(BytesRef.deepCopyOf(f.binaryValue()).bytes);
  }

  static String fieldType(IndexableFieldType t) {
    return t.stored()
        + "," + t.tokenized()
        + "," + t.indexOptions()
        + "," + t.storeTermVectors()
        + "," + t.storeTermVectorOffsets()
        + "," + t.storeTermVectorPositions()
        + "," + t.storeTermVectorPayloads()
        + "," + t.omitNorms()
        + "," + t.docValuesType()
        + "," + t.pointDimensionCount()
        + "," + t.pointIndexDimensionCount()
        + "," + t.pointNumBytes();
  }

  static String docRow(long seed, int k, Document doc) {
    Digest body = new Digest();
    StringBuilder others = new StringBuilder();
    for (IndexableField f : doc) {
      switch (f.name()) {
        case "title", "titleTokenized", "date" -> {}
        case "body" -> body.add(f.stringValue());
        default -> {
          if (others.length() > 0) others.append(';');
          others.append(f.name()).append('=').append(fieldValue(f));
          if (f.name().startsWith("docLength") || f.name().startsWith("docTitleLength")) {
            others.append('(').append(f.getClass().getSimpleName()).append(')');
          }
        }
      }
    }
    return seed
        + "\t" + k
        + "\t" + doc.get("titleTokenized")
        + "\t" + doc.get("date")
        + "\t" + String.format("%016x", body.h)
        + "\t" + doc.getFields().size()
        + "\t" + others;
  }

  static void lineFileDocs(PrintWriter w) throws Exception {
    w.println("# seed\tdoc\ttitle\tdate\tbody digest\tfield count\tother fields");
    long[] seeds = OracleSupport.seeds();
    Document first = null;
    for (int si = 0; si < 8; si++) {
      try (LineFileDocs docs = new LineFileDocs(new Random(seeds[si]), "europarl.lines.txt.gz")) {
        for (int k = 0; k < 30; k++) {
          Document doc = docs.nextDoc();
          w.println(docRow(seeds[si], k, doc));
          if (first == null) {
            first = new Document();
            for (IndexableField f : doc) first.add(f);
          }
        }
      }
    }
    long last = 0;
    while (new Random(new Random(last).nextLong()).nextInt(20) != 19) last++;
    try (LineFileDocs docs = new LineFileDocs(new Random(last), "europarl.lines.txt.gz")) {
      for (int k = 0; k < 1000; k++) {
        Document doc = docs.nextDoc();
        if (k < 30 || k % 50 == 0) w.println(docRow(last, k, doc));
      }
    }
    for (IndexableField f : first) {
      w.println("fieldType\t" + f.name() + "\t" + fieldType(f.fieldType()));
    }
  }

  static Automaton randomSingleAutomaton(Random r) throws Exception {
    java.lang.reflect.Method m =
        AutomatonTestUtil.class.getDeclaredMethod("randomSingleAutomaton", Random.class);
    m.setAccessible(true);
    return (Automaton) m.invoke(null, r);
  }

  static void automata(PrintWriter w) throws Exception {
    w.println("# generator\tparams\tseed\tdigest\tnextLong after\tfirst result (first seed)");
    row(w, "randomRegexp", "-", r -> AutomatonTestUtil.randomRegexp(r));
    row(w, "randomSingleAutomaton", "-", 10, r -> randomSingleAutomaton(r).toDot());
    row(w, "randomAutomaton", "-", 10, r -> AutomatonTestUtil.randomAutomaton(r).toDot());
    // The slow operations on one random automaton per draw, and random accepted strings.
    row(
        w,
        "operations",
        "-",
        5,
        r -> {
          Automaton a = AutomatonTestUtil.randomAutomaton(r);
          StringBuilder sb = new StringBuilder();
          sb.append(a.getNumStates()).append(' ').append(AutomatonTestUtil.isDeterministicSlow(a));
          Automaton ds = AutomatonTestUtil.determinizeSimple(a);
          sb.append('\n').append(ds.toDot());
          Automaton d = Operations.removeDeadStates(ds);
          Automaton min = AutomatonTestUtil.minimizeSimple(d);
          sb.append('\n').append(min.toDot());
          sb.append('\n').append(AutomatonTestUtil.isFinite(d));
          sb.append(' ').append(AutomatonTestUtil.sameLanguage(d, min));
          sb.append(' ').append(AutomatonTestUtil.subsetOf(min, d));
          if (d.getNumStates() > 0 && AutomatonTestUtil.isFinite(d)) {
            java.util.List<String> fs = new java.util.ArrayList<>();
            for (IntsRef ir : AutomatonTestUtil.getFiniteStringsRecursive(d, 50)) {
              fs.add(java.util.Arrays.toString(java.util.Arrays.copyOfRange(ir.ints, ir.offset, ir.offset + ir.length)));
            }
            java.util.Collections.sort(fs);
            sb.append('\n').append(fs);
          }
          if (d.getNumStates() > 0) {
            AutomatonTestUtil.RandomAcceptedStrings ras =
                new AutomatonTestUtil.RandomAcceptedStrings(d);
            for (int k = 0; k < 10; k++) {
              sb.append('\n').append(java.util.Arrays.toString(ras.getRandomAcceptedString(r)));
            }
          }
          return sb.toString();
        });
  }

  static void fstTester(PrintWriter w) throws Exception {
    w.println("# seed\tinputMode\tpairs\tnodeCount\tarcCount\tFST bytes (meta+data) length\tdigest\ttester nextLong after");
    long[] seeds = OracleSupport.seeds();
    for (int mode = 0; mode < 2; mode++) {
      for (int si = 0; si < 10; si++) {
        Random r = new Random(seeds[si]);
        int numWords = si == 0 ? 0 : r.nextInt(300);
        java.util.TreeSet<IntsRef> terms = new java.util.TreeSet<>();
        for (int i = 0; i < numWords; i++) {
          terms.add(FSTTester.toIntsRef(FSTTester.getRandomString(r), mode));
        }
        PositiveIntOutputs outputs = PositiveIntOutputs.getSingleton();
        java.util.List<FSTTester.InputOutput<Long>> pairs = new java.util.ArrayList<>();
        for (IntsRef term : terms) {
          pairs.add(new FSTTester.InputOutput<>(term, (long) r.nextInt(1000)));
        }
        Random testerRandom = new Random(r.nextLong());
        FSTTester<Long> tester =
            new FSTTester<>(testerRandom, new ByteBuffersDirectory(), mode, pairs, outputs);
        FST<Long> fst = tester.doTest();
        String bytes = "-\t-";
        if (fst != null) {
          ByteBuffersDataOutput out = new ByteBuffersDataOutput();
          fst.save(out, out);
          Digest d = new Digest();
          byte[] a = out.toArrayCopy();
          d.add(a);
          bytes = a.length + "\t" + String.format("%016x", d.h);
        }
        w.println(
            seeds[si] + "\t" + mode + "\t" + pairs.size() + "\t" + tester.nodeCount + "\t"
                + tester.arcCount + "\t" + bytes + "\t" + testerRandom.nextLong());
      }
    }
  }

  static void compare(PrintWriter w) {
    w.println("# kind\tinputs\tresult");
    String[] strings = {
      "", "a", "b", "ab", "￿", "𐀀", "𐀀a", "", "퟿", "aa"
    };
    for (String a : strings) {
      for (String b : strings) {
        w.println(
            "codepoint\t"
                + OracleSupport.units(a)
                + " "
                + OracleSupport.units(b)
                + "\t"
                + TestUtil.STRING_CODEPOINT_COMPARATOR.compare(a, b));
      }
    }
    byte[][] refs = {
      {}, {0x61, 0x62}, {(byte) 0xc3, (byte) 0xa9}, {(byte) 0xe2, (byte) 0x82, (byte) 0xac},
      {(byte) 0xf0, (byte) 0x90, (byte) 0x80, (byte) 0x80}, {(byte) 0x80}, {(byte) 0xf8, 0, 0, 0},
      {0x61, (byte) 0xbf}, {(byte) 0xef, (byte) 0xbf, (byte) 0xbf}
    };
    for (byte[] ref : refs) {
      w.println(
          "bytesRefToString\t"
              + hex(ref)
              + "\t"
              + OracleSupport.units(TestUtil.bytesRefToString(new BytesRef(ref))));
    }
    w.println("bytesRefToString\tnull\t" + TestUtil.bytesRefToString(null));
    float[] floats = {
      0f, -0f, 1f, Math.nextUp(1f), Math.nextDown(1f), Float.MIN_VALUE, -Float.MIN_VALUE, Float.NaN,
      Float.POSITIVE_INFINITY, Float.MAX_VALUE, 3.5f
    };
    for (float x : floats) {
      for (float y : floats) {
        for (short ulps : new short[] {0, 1, 2, Short.MAX_VALUE}) {
          w.println(
              "floatUlpEquals\t"
                  + Integer.toHexString(Float.floatToRawIntBits(x))
                  + " "
                  + Integer.toHexString(Float.floatToRawIntBits(y))
                  + " "
                  + ulps
                  + "\t"
                  + TestUtil.floatUlpEquals(x, y, ulps));
        }
      }
    }
    double[] doubles = {
      0d, -0d, 1d, Math.nextUp(1d), Math.nextDown(1d), Double.MIN_VALUE, -Double.MIN_VALUE,
      Double.NaN, Double.NEGATIVE_INFINITY, 3.5d
    };
    for (double x : doubles) {
      for (double y : doubles) {
        for (int ulps : new int[] {0, 1, 2, Integer.MAX_VALUE}) {
          w.println(
              "doubleUlpEquals\t"
                  + Long.toHexString(Double.doubleToRawLongBits(x))
                  + " "
                  + Long.toHexString(Double.doubleToRawLongBits(y))
                  + " "
                  + ulps
                  + "\t"
                  + TestUtil.doubleUlpEquals(x, y, ulps));
        }
      }
    }
  }


  static String err(Throwable t) {
    return OracleSupport.err(t);
  }

  static String describe(MergePolicy mp) {
    String d = "noCFSRatio=" + mp.getNoCFSRatio();
    if (mp instanceof LogMergePolicy lmp) {
      d += " mergeFactor=" + lmp.getMergeFactor();
    } else if (mp instanceof TieredMergePolicy tmp) {
      d += " maxMergeAtOnce=" + tmp.getMaxMergeAtOnce() + " segmentsPerTier=" + tmp.getSegmentsPerTier();
    } else if (mp instanceof FilterMergePolicy fmp) {
      d += " delegate(" + describe(fmp.unwrap()) + ")";
    }
    return d;
  }

  static String describe(MergeScheduler ms) {
    if (ms instanceof ConcurrentMergeScheduler cms) {
      return "maxMergeCount=" + cms.getMaxMergeCount() + " maxThreadCount=" + cms.getMaxThreadCount();
    }
    return ms.getClass().getSimpleName();
  }

  static ScoreDoc sd(int doc, float score) {
    return new ScoreDoc(doc, score);
  }

  static FieldDoc fd(int doc, float score, Object... fields) {
    return new FieldDoc(doc, score, fields);
  }

  static TopDocs td(long hits, boolean equal, ScoreDoc... docs) {
    return new TopDocs(
        new TotalHits(
            hits, equal ? TotalHits.Relation.EQUAL_TO : TotalHits.Relation.GREATER_THAN_OR_EQUAL_TO),
        docs);
  }

  static void consistent(PrintWriter w, String label, TopDocs expected, TopDocs actual) {
    String result;
    try {
      TestUtil.assertConsistent(expected, actual);
      result = "ok";
    } catch (Throwable t) {
      result = err(t);
    }
    w.println("assertConsistent\t" + label + "\t" + result);
  }

  static void members(PrintWriter w) throws Exception {
    w.println("# method\tcase\tresult (value, or class<TAB>message)");
    // reduceOpenFiles and syncConcurrentMerges on a live writer.
    String[] policyNames = {
      "tiered", "tiered(3,4.0)", "tiered(20,30.0)", "logdoc", "logbytes(3)", "filter(tiered)"
    };
    String[] schedulerNames = {"cms", "cms(6,4)", "serial"};
    for (int p = 0; p < policyNames.length; p++) {
      for (int m = 0; m < schedulerNames.length; m++) {
        MergePolicy mp;
        switch (p) {
          case 0 -> mp = new TieredMergePolicy();
          case 1 -> {
            TieredMergePolicy t = new TieredMergePolicy();
            t.setMaxMergeAtOnce(3);
            t.setSegmentsPerTier(4.0);
            mp = t;
          }
          case 2 -> {
            TieredMergePolicy t = new TieredMergePolicy();
            t.setMaxMergeAtOnce(20);
            t.setSegmentsPerTier(30.0);
            mp = t;
          }
          case 3 -> mp = new LogDocMergePolicy();
          case 4 -> {
            LogByteSizeMergePolicy l = new LogByteSizeMergePolicy();
            l.setMergeFactor(3);
            mp = l;
          }
          default -> mp = new FilterMergePolicy(new TieredMergePolicy());
        }
        MergeScheduler ms;
        switch (m) {
          case 0 -> ms = new ConcurrentMergeScheduler();
          case 1 -> {
            ConcurrentMergeScheduler c = new ConcurrentMergeScheduler();
            c.setMaxMergesAndThreads(6, 4);
            ms = c;
          }
          default -> ms = new SerialMergeScheduler();
        }
        IndexWriterConfig c = new IndexWriterConfig();
        c.setMergePolicy(mp);
        c.setMergeScheduler(ms);
        try (ByteBuffersDirectory d = new ByteBuffersDirectory();
            IndexWriter iw = new IndexWriter(d, c)) {
          String before = describe(iw.getConfig().getMergePolicy()) + " " + describe(iw.getConfig().getMergeScheduler());
          TestUtil.reduceOpenFiles(iw);
          TestUtil.syncConcurrentMerges(iw);
          w.println(
              "reduceOpenFiles\t" + policyNames[p] + " " + schedulerNames[m] + "\t" + before + " -> "
                  + describe(iw.getConfig().getMergePolicy()) + " "
                  + describe(iw.getConfig().getMergeScheduler()));
        }
      }
    }
    // getPostingsFormat / getDocValuesFormat.
    Codec[] codecs = {
      new Lucene104Codec(),
      new AssertingCodec(),
      TestUtil.alwaysPostingsFormat(new LuceneFixedGap()),
      TestUtil.alwaysDocValuesFormat(new Lucene90DocValuesFormat()),
      new SimpleTextCodec(),
      new CheapBastardCodec()
    };
    String[] codecNames = {"lucene104", "asserting", "alwaysFixedGap", "alwaysLucene90", "simpletext", "cheapbastard"};
    for (int i = 0; i < codecs.length; i++) {
      for (String field : new String[] {"f", "id"}) {
        w.println("getPostingsFormat\t" + codecNames[i] + " " + field + "\t" + TestUtil.getPostingsFormat(codecs[i], field));
        w.println("getDocValuesFormat\t" + codecNames[i] + " " + field + "\t" + TestUtil.getDocValuesFormat(codecs[i], field));
      }
    }
    w.println("getPostingsFormat\tdefault f\t" + TestUtil.getPostingsFormat("f"));
    w.println("getDocValuesFormat\tdefault f\t" + TestUtil.getDocValuesFormat("f"));
    w.println("fieldSupportsHugeBinaryDocValues\tf\t" + TestUtil.fieldSupportsHugeBinaryDocValues("f"));
    // cloneDocument.
    Document doc = new Document();
    doc.add(new KeywordField("kw", "v1", Field.Store.YES));
    doc.add(new KeywordField("kw2", "v2", Field.Store.NO));
    doc.add(new IntField("int", 7, Field.Store.YES));
    doc.add(new IntField("int2", -3, Field.Store.NO));
    doc.add(new NumericDocValuesField("ndv", 42L));
    doc.add(new BinaryDocValuesField("bdv", new BytesRef("bin")));
    doc.add(new SortedDocValuesField("sdv", new BytesRef("sorted")));
    doc.add(new BinaryPoint("bp", new byte[] {1, 2}, new byte[] {3, 4}));
    doc.add(new IntPoint("ip", 5, 6));
    doc.add(new StringField("s", "str", Field.Store.YES));
    doc.add(new TextField("t", "some text", Field.Store.NO));
    doc.add(new StoredField("st", "stored"));
    Document clone = TestUtil.cloneDocument(doc);
    for (IndexableField f : clone.getFields()) {
      w.println("cloneDocument\t" + f.name() + "\t" + f);
    }
    IndexableField[] bad = {
      new LongField("lf", 5L, Field.Store.NO),
      new SortedSetDocValuesField("ssdv", new BytesRef("x")),
      new StoredField("sti", 5)
    };
    for (IndexableField f : bad) {
      Document one = new Document();
      one.add(f);
      String result;
      try {
        result = TestUtil.cloneDocument(one).getFields().get(0).toString();
      } catch (Throwable t) {
        result = err(t);
      }
      w.println("cloneDocument\t" + f.name() + "\t" + result);
    }
    // addIndexesSlowly: two source indexes of 3 and 2 documents (one deleted).
    try (ByteBuffersDirectory a = new ByteBuffersDirectory();
        ByteBuffersDirectory b = new ByteBuffersDirectory();
        ByteBuffersDirectory target = new ByteBuffersDirectory()) {
      for (ByteBuffersDirectory src : new ByteBuffersDirectory[] {a, b}) {
        try (IndexWriter iw = new IndexWriter(src, new IndexWriterConfig())) {
          int n = src == a ? 3 : 2;
          for (int i = 0; i < n; i++) {
            Document d = new Document();
            d.add(new StringField("id", "" + i, Field.Store.YES));
            iw.addDocument(d);
          }
          if (src == b) {
            iw.deleteDocuments(new org.apache.lucene.index.Term("id", "0"));
          }
        }
      }
      try (DirectoryReader ra = DirectoryReader.open(a);
          DirectoryReader rb = DirectoryReader.open(b);
          IndexWriter iw =
              new IndexWriter(target, new IndexWriterConfig().setMergeScheduler(new SerialMergeScheduler()))) {
        TestUtil.addIndexesSlowly(iw, ra, rb);
        iw.commit();
        try (DirectoryReader r = DirectoryReader.open(target)) {
          w.println(
              "addIndexesSlowly\ta(3)+b(2,-1)\tsegments=" + r.leaves().size() + " maxDoc=" + r.maxDoc()
                  + " numDocs=" + r.numDocs());
        }
      }
    }
    // assertConsistent.
    ScoreDoc[] three = {sd(1, 2f), sd(2, 1f), sd(3, 0.5f)};
    consistent(w, "same", td(3, true, three), td(3, true, sd(1, 2f), sd(2, 1f), sd(3, 0.5f)));
    consistent(w, "zero-vs-two", td(0, true), td(2, true, sd(1, 1f), sd(2, 1f)));
    consistent(w, "two-vs-zero", td(2, false, sd(1, 1f), sd(2, 1f)), td(0, true));
    consistent(w, "eq5-vs-eq4", td(5, true, sd(1, 1f)), td(4, true, sd(1, 1f)));
    consistent(w, "eq5-vs-gte6", td(5, true, sd(1, 1f)), td(6, false, sd(1, 1f)));
    consistent(w, "eq5-vs-gte3", td(5, true, sd(1, 1f)), td(3, false, sd(1, 1f)));
    consistent(w, "gte5-vs-eq4", td(5, false, sd(1, 1f)), td(4, true, sd(1, 1f)));
    consistent(w, "gte5-vs-eq7", td(5, false, sd(1, 1f)), td(7, true, sd(1, 1f)));
    consistent(w, "gte5-vs-gte1", td(5, false, sd(1, 1f)), td(1, false, sd(1, 1f)));
    consistent(w, "size", td(3, true, three), td(3, true, sd(1, 2f), sd(2, 1f)));
    consistent(w, "doc", td(3, true, three), td(3, true, sd(1, 2f), sd(4, 1f), sd(3, 0.5f)));
    consistent(w, "score", td(1, true, sd(1, 1.5f)), td(1, true, sd(1, 0.1f)));
    consistent(w, "score-nan", td(1, true, sd(1, Float.NaN)), td(1, true, sd(1, Float.NaN)));
    consistent(w, "score-zero", td(1, true, sd(1, 0f)), td(1, true, sd(1, -0f)));
    consistent(w, "fields-same", td(1, true, fd(1, 1f, 3, new BytesRef("ab"))), td(1, true, fd(1, 1f, 3, new BytesRef("ab"))));
    consistent(w, "fields-int", td(1, true, fd(1, 1f, 3)), td(1, true, fd(1, 1f, 4)));
    consistent(w, "fields-bytes", td(1, true, fd(1, 1f, 1, new BytesRef("ab"))), td(1, true, fd(1, 1f, 1, new BytesRef("ac"))));
    consistent(w, "fields-float", td(1, true, fd(1, 1f, 1f)), td(1, true, fd(1, 1f, 2f)));
    consistent(w, "fields-long-int", td(1, true, fd(1, 1f, 5L)), td(1, true, fd(1, 1f, 5)));
    consistent(w, "fields-null-value", td(1, true, fd(1, 1f, (Object) null)), td(1, true, fd(1, 1f, 5)));
    consistent(w, "fields-longer", td(1, true, fd(1, 1f, 3)), td(1, true, fd(1, 1f, 3, 1f)));
    consistent(w, "fields-shorter", td(1, true, fd(1, 1f, 3, 1f)), td(1, true, fd(1, 1f, 3)));
    consistent(w, "fields-null", td(1, true, new FieldDoc(1, 1f)), td(1, true, fd(1, 1f, 3)));
    consistent(w, "fields-null-actual", td(1, true, fd(1, 1f, 3)), td(1, true, new FieldDoc(1, 1f)));
    consistent(w, "fields-both-null", td(1, true, new FieldDoc(1, 1f)), td(1, true, new FieldDoc(1, 1f)));
    consistent(w, "fielddoc-vs-scoredoc", td(1, true, fd(1, 1f, 3)), td(1, true, sd(1, 1f)));
    consistent(w, "scoredoc-vs-fielddoc", td(1, true, sd(1, 1f)), td(1, true, fd(1, 1f, 3, 2.5f)));
  }


  static void ram(PrintWriter w, String label, Object o) {
    w.println(label + "\t" + RamUsageTester.ramUsed(o) + "\t" + RamUsageTester.humanSizeOf(o));
  }

  static void ramUsage(PrintWriter w) {
    w.println("# case\tramUsed\thumanSizeOf");
    for (int n : new int[] {0, 1, 7, 8, 9, 100, 1000}) {
      ram(w, "byte[" + n + "]", new byte[n]);
    }
    ram(w, "boolean[9]", new boolean[9]);
    ram(w, "char[5]", new char[5]);
    ram(w, "short[3]", new short[3]);
    ram(w, "int[3]", new int[3]);
    ram(w, "float[2]", new float[2]);
    ram(w, "long[5]", new long[5]);
    ram(w, "double[3]", new double[3]);
    ram(w, "int[100000]", new int[100000]);
    ram(w, "string()", "");
    ram(w, "string(a)", "a");
    ram(w, "string(abcdefgh)", "abcdefgh");
    ram(w, "string(x*100)", "x".repeat(100));
    ram(w, "string(U+10000)", "𐀀");
    ram(w, "bytesref()", new BytesRef());
    ram(w, "bytesref(byte[10])", new BytesRef(new byte[10]));
    ram(w, "bytesref(hello)", new BytesRef("hello"));
    String a = "a";
    ram(w, "object[](a,a,b,null)", new Object[] {a, a, "b", null});
    ram(w, "object[0]", new Object[0]);
    ram(w, "int[][](2,1)", new int[][] {{1, 2}, {3}});
    java.util.ArrayList<String> list = new java.util.ArrayList<>();
    list.add("one");
    list.add("two");
    list.add("three");
    ram(w, "arraylist(one,two,three)", list);
    java.util.HashMap<String, String> map = new java.util.HashMap<>();
    map.put("k1", "v1");
    map.put("key2", "value2");
    ram(w, "hashmap(k1=v1,key2=value2)", map);
    RamUsageTester.Accumulator shallowArrays =
        new RamUsageTester.Accumulator() {
          @Override
          public long accumulateArray(
              Object array, long shallowSize, java.util.List<Object> values,
              java.util.Collection<Object> queue) {
            return shallowSize;
          }
        };
    w.println(
        "shallowArrays:object[](a,a,b,null)\t"
            + RamUsageTester.ramUsed(new Object[] {a, a, "b", null}, shallowArrays)
            + "\t-");
    w.println(
        "shallowArrays:bytesref(hello)\t"
            + RamUsageTester.ramUsed(new BytesRef("hello"), shallowArrays)
            + "\t-");
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_generators.tsv"), "UTF-8")) {
      generators(w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_patterns.tsv"), "UTF-8")) {
      patterns(w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_linefiledocs.tsv"), "UTF-8")) {
      lineFileDocs(w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_automaton.tsv"), "UTF-8")) {
      automata(w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_fsttester.tsv"), "UTF-8")) {
      fstTester(w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_compare.tsv"), "UTF-8")) {
      compare(w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_members.tsv"), "UTF-8")) {
      members(w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_ramusage.tsv"), "UTF-8")) {
      ramUsage(w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testutil_english.tsv"), "UTF-8")) {
      w.println("# method\tvalue\tresult");
      for (int i = -1000; i <= 1000; i++) {
        w.println("int\t" + i + "\t" + English.intToEnglish(i));
      }
      for (int i : new int[] {Integer.MIN_VALUE, Integer.MAX_VALUE}) {
        w.println("int\t" + i + "\t" + English.intToEnglish(i));
      }
      long p = 1;
      for (int k = 0; k <= 18; k++, p *= 10) {
        for (long v : new long[] {p, -p, p - 1}) {
          w.println("long\t" + v + "\t" + English.longToEnglish(v));
        }
      }
      for (long v : new long[] {Long.MIN_VALUE, Long.MAX_VALUE}) {
        w.println("long\t" + v + "\t" + English.longToEnglish(v));
      }
      Random r = new Random(0);
      for (int i = 0; i < 500; i++) {
        long v = r.nextLong();
        long d = 1;
        for (int k = 0; k < i % 19; k++) {
          d *= 10;
        }
        w.println("long\t" + v / d + "\t" + English.longToEnglish(v / d));
      }
    }
  }
}
