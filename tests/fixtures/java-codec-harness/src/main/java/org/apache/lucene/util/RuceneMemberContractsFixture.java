/*
 * Reference values for the Rucene portability test `portability_member_contracts`.
 *
 * This class lives in `org.apache.lucene.util` on purpose: `ByteBlockPool.setBytesRef`
 * is package-private in Apache Lucene Core 10.5.0, so the only way to drive the real
 * method -- rather than a transcription of it -- is from inside its own package. The
 * other package-private members it measures (`BufferedChecksum.updateShort/Int/Long/Longs`
 * and `Lucene90StoredFieldsFormat.impl`) are reached reflectively. Nothing here is part
 * of the Rucene crate; it is test-fixture code that prints reference values on stdout
 * for the Rust side to assert against, one `kind|...` row per line.
 */
package org.apache.lucene.util;

import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.zip.CRC32;
import org.apache.lucene.analysis.AbstractAnalysisFactory;
import org.apache.lucene.analysis.CharArrayMap;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.codecs.lucene90.Lucene90StoredFieldsFormat;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.internal.hppc.IntArrayList;
import org.apache.lucene.internal.hppc.IntCursor;
import org.apache.lucene.internal.hppc.IntHashSet;
import org.apache.lucene.internal.hppc.LongArrayList;
import org.apache.lucene.internal.hppc.LongCursor;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.DocValuesRewriteMethod;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.MultiTermQuery;
import org.apache.lucene.search.PhraseQuery;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.ScoreMode;
import org.apache.lucene.search.Scorer;
import org.apache.lucene.search.Weight;
import org.apache.lucene.search.similarities.AfterEffectB;
import org.apache.lucene.search.similarities.AfterEffectL;
import org.apache.lucene.search.similarities.AxiomaticF1EXP;
import org.apache.lucene.search.similarities.AxiomaticF1LOG;
import org.apache.lucene.search.similarities.AxiomaticF2EXP;
import org.apache.lucene.search.similarities.AxiomaticF2LOG;
import org.apache.lucene.search.similarities.AxiomaticF3EXP;
import org.apache.lucene.search.similarities.AxiomaticF3LOG;
import org.apache.lucene.search.similarities.BasicModelG;
import org.apache.lucene.search.similarities.BasicModelIF;
import org.apache.lucene.search.similarities.BasicModelIn;
import org.apache.lucene.search.similarities.BasicModelIne;
import org.apache.lucene.search.similarities.DFISimilarity;
import org.apache.lucene.search.similarities.DFRSimilarity;
import org.apache.lucene.search.similarities.DistributionLL;
import org.apache.lucene.search.similarities.DistributionSPL;
import org.apache.lucene.search.similarities.IBSimilarity;
import org.apache.lucene.search.similarities.IndependenceChiSquared;
import org.apache.lucene.search.similarities.IndependenceSaturated;
import org.apache.lucene.search.similarities.IndependenceStandardized;
import org.apache.lucene.search.similarities.LambdaDF;
import org.apache.lucene.search.similarities.LambdaTTF;
import org.apache.lucene.search.similarities.Normalization;
import org.apache.lucene.search.similarities.NormalizationH1;
import org.apache.lucene.search.similarities.NormalizationH2;
import org.apache.lucene.search.similarities.NormalizationH3;
import org.apache.lucene.search.similarities.NormalizationZ;
import org.apache.lucene.store.BufferedChecksum;
import org.apache.lucene.store.BufferedChecksumIndexInput;
import org.apache.lucene.store.ByteBuffersDataOutput;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.ByteBuffersIndexInput;

public final class RuceneMemberContractsFixture {

  private RuceneMemberContractsFixture() {}

  private static void p(String line) {
    System.out.println(line);
  }

  // ---------------------------------------------------------------------------
  // toString() of every concrete implementor of the abstract similarity parts
  // ---------------------------------------------------------------------------

  static void similarities() {
    Object[][] rows = {
      {"AfterEffectB", new AfterEffectB()},
      {"AfterEffectL", new AfterEffectL()},
      {"BasicModelG", new BasicModelG()},
      {"BasicModelIF", new BasicModelIF()},
      {"BasicModelIn", new BasicModelIn()},
      {"BasicModelIne", new BasicModelIne()},
      {"DistributionLL", new DistributionLL()},
      {"DistributionSPL", new DistributionSPL()},
      {"IndependenceChiSquared", new IndependenceChiSquared()},
      {"IndependenceSaturated", new IndependenceSaturated()},
      {"IndependenceStandardized", new IndependenceStandardized()},
      {"LambdaDF", new LambdaDF()},
      {"LambdaTTF", new LambdaTTF()},
      {"NoNormalization", new Normalization.NoNormalization()},
      {"NormalizationH1()", new NormalizationH1()},
      {"NormalizationH1(2.5)", new NormalizationH1(2.5f)},
      {"NormalizationH2()", new NormalizationH2()},
      {"NormalizationH3()", new NormalizationH3()},
      {"NormalizationH3(0.1)", new NormalizationH3(0.1f)},
      {"NormalizationH3(1e-5)", new NormalizationH3(1e-5f)},
      {"NormalizationH3(1e10)", new NormalizationH3(1.0e10f)},
      {"NormalizationZ()", new NormalizationZ()},
      {"NormalizationZ(0.25)", new NormalizationZ(0.25f)},
      {"NormalizationZ(1e-5)", new NormalizationZ(1e-5f)},
      {"AxiomaticF1EXP", new AxiomaticF1EXP()},
      {"AxiomaticF1LOG", new AxiomaticF1LOG()},
      {"AxiomaticF2EXP", new AxiomaticF2EXP()},
      {"AxiomaticF2LOG", new AxiomaticF2LOG()},
      {"AxiomaticF3EXP(0.25,1)", new AxiomaticF3EXP(0.25f, 1)},
      {"AxiomaticF3LOG(0.25,1)", new AxiomaticF3LOG(0.25f, 1)},
      {
        "DFR(G,B,H2)",
        new DFRSimilarity(new BasicModelG(), new AfterEffectB(), new NormalizationH2())
      },
      {
        "IB(LL,DF,H3)",
        new IBSimilarity(new DistributionLL(), new LambdaDF(), new NormalizationH3())
      },
      {"DFI(ChiSquared)", new DFISimilarity(new IndependenceChiSquared())},
    };
    for (Object[] row : rows) {
      p("sim|" + row[0] + "|" + row[1]);
    }
  }

  // ---------------------------------------------------------------------------
  // hppc: IntArrayList / LongArrayList clone, iterator, toString; IntHashSet
  // clone, equals, iterator
  // ---------------------------------------------------------------------------

  static void hppc() {
    p("hppc|IntArrayList|empty|" + new IntArrayList());
    IntArrayList ints = IntArrayList.from(1, -2, Integer.MAX_VALUE, Integer.MIN_VALUE, 0);
    p("hppc|IntArrayList|values|" + ints);
    IntArrayList intsCopy = ints.clone();
    ints.add(9);
    intsCopy.set(0, 7);
    p("hppc|IntArrayList|after_clone_original|" + ints);
    p("hppc|IntArrayList|after_clone_copy|" + intsCopy);
    StringBuilder it = new StringBuilder();
    for (IntCursor c : ints) {
      it.append(c.index).append(':').append(c.value).append(',');
    }
    p("hppc|IntArrayList|iterator|" + it);

    p("hppc|LongArrayList|empty|" + new LongArrayList());
    LongArrayList longs = LongArrayList.from(1L, -2L, Long.MAX_VALUE, Long.MIN_VALUE, 0L);
    p("hppc|LongArrayList|values|" + longs);
    LongArrayList longsCopy = longs.clone();
    longs.add(9L);
    longsCopy.set(0, 7L);
    p("hppc|LongArrayList|after_clone_original|" + longs);
    p("hppc|LongArrayList|after_clone_copy|" + longsCopy);
    StringBuilder lit = new StringBuilder();
    for (LongCursor c : longs) {
      lit.append(c.index).append(':').append(c.value).append(',');
    }
    p("hppc|LongArrayList|iterator|" + lit);

    IntHashSet a = IntHashSet.from(1, 2, 3, 0);
    IntHashSet b = IntHashSet.from(0, 3, 2, 1);
    p("hppc|IntHashSet|equals_reordered|" + a.equals(b) + "|" + b.equals(a));
    IntHashSet c = a.clone();
    p("hppc|IntHashSet|equals_clone|" + a.equals(c));
    c.add(4);
    p("hppc|IntHashSet|clone_then_add|" + a.equals(c) + "|" + a.size() + "|" + c.size());
    p(
        "hppc|IntHashSet|empty_key|"
            + IntHashSet.from(0).equals(IntHashSet.from(0))
            + "|"
            + IntHashSet.from(0).equals(new IntHashSet()));
    int[] seen = new int[c.size()];
    int n = 0;
    for (IntCursor cursor : c) {
      seen[n++] = cursor.value;
    }
    Arrays.sort(seen);
    p("hppc|IntHashSet|iterator_sorted|" + Arrays.toString(seen));
  }

  // ---------------------------------------------------------------------------
  // BufferedChecksum.updateShort/Int/Long/Longs and BufferedChecksumIndexInput
  // ---------------------------------------------------------------------------

  static final String[] CHECKSUM_SCENARIOS = {
    "b=1;b=2;b=3;s=-2;i=-559038737;l=81985529216486895",
    "b=9;L=0:5:1,-1,9223372036854775807,-9223372036854775808,42",
    "L=1:3:0,7,8,9,10",
    "i=1;i=2;i=3;L=0:20:"
        + "0,72340172838076672,144680345676153344,217020518514230016,289360691352306688,"
        + "361700864190383360,434041037028460032,506381209866536704,578721382704613376,"
        + "651061555542690048,723401728380766720,795741901218843392,868082074056920064,"
        + "940422246894996736,1012762419733073408,1085102592571150080,1157442765409226752,"
        + "1229782938247303424,1302123111085380096,1374463283923456768",
    "s=1;s=2;s=3;s=4;s=5;s=6;i=7;l=8",
    "b=1;L=0:0:5",
    "L=0:0:5",
  };

  static void checksums() throws Exception {
    Method updateShort = BufferedChecksum.class.getDeclaredMethod("updateShort", short.class);
    Method updateInt = BufferedChecksum.class.getDeclaredMethod("updateInt", int.class);
    Method updateLong = BufferedChecksum.class.getDeclaredMethod("updateLong", long.class);
    Method updateLongs =
        BufferedChecksum.class.getDeclaredMethod("updateLongs", long[].class, int.class, int.class);
    for (Method m : new Method[] {updateShort, updateInt, updateLong, updateLongs}) {
      m.setAccessible(true);
    }
    for (int size : new int[] {8, 12, 16, 1024}) {
      for (String scenario : CHECKSUM_SCENARIOS) {
        BufferedChecksum checksum = new BufferedChecksum(new CRC32(), size);
        for (String op : scenario.split(";")) {
          String[] kv = op.split("=", 2);
          switch (kv[0]) {
            case "b" -> checksum.update(Integer.parseInt(kv[1]));
            case "s" -> updateShort.invoke(checksum, Short.parseShort(kv[1]));
            case "i" -> updateInt.invoke(checksum, Integer.parseInt(kv[1]));
            case "l" -> updateLong.invoke(checksum, Long.parseLong(kv[1]));
            case "L" -> {
              String[] parts = kv[1].split(":", 3);
              String[] text = parts[2].split(",");
              long[] values = new long[text.length];
              for (int i = 0; i < text.length; i++) {
                values[i] = Long.parseLong(text[i]);
              }
              updateLongs.invoke(
                  checksum, values, Integer.parseInt(parts[0]), Integer.parseInt(parts[1]));
            }
            default -> throw new IllegalArgumentException(op);
          }
        }
        p("cks|" + size + "|" + scenario + "|" + checksum.getValue());
      }
    }

    // The read path: BufferedChecksumIndexInput.readShort/readInt/readLong/readLongs
    // checksum what the wrapped input reads, through the update* methods above.
    ByteBuffersDataOutput out = new ByteBuffersDataOutput();
    out.writeByte((byte) 0x7f);
    out.writeShort((short) -2);
    out.writeInt(-559038737);
    out.writeLong(81985529216486895L);
    for (long v : new long[] {Long.MIN_VALUE, -1L, 0L, 1L, Long.MAX_VALUE}) {
      out.writeLong(v);
    }
    out.writeShort((short) 0x1234);
    byte[] bytes = out.toArrayCopy();
    CRC32 raw = new CRC32();
    raw.update(bytes, 0, bytes.length);
    try (BufferedChecksumIndexInput in =
        new BufferedChecksumIndexInput(new ByteBuffersIndexInput(out.toDataInput(), "fixture"))) {
      byte b = in.readByte();
      short s = in.readShort();
      int i = in.readInt();
      long l = in.readLong();
      long[] dst = new long[7];
      in.readLongs(dst, 1, 5);
      short tail = in.readShort();
      p(
          "bcii|"
              + b
              + "|"
              + s
              + "|"
              + i
              + "|"
              + l
              + "|"
              + Arrays.toString(dst)
              + "|"
              + tail
              + "|"
              + in.getChecksum()
              + "|"
              + raw.getValue());
    }
  }

  /**
   * BufferedChecksumIndexInput.readShort/readInt/readLong/readLongs read the value through the
   * wrapped input and only then checksum it (BufferedChecksumIndexInput.java:50-74), so a read that
   * runs out of input part-way through a value leaves the checksum as it was. Each row: one byte
   * read and checksummed, then a value of `kind` with too few bytes left.
   */
  static void checksumAfterEof() throws Exception {
    String[] kinds = {"short", "int", "long", "longs"};
    int[] partial = {1, 3, 7, 13};
    for (int k = 0; k < kinds.length; k++) {
      ByteBuffersDataOutput out = new ByteBuffersDataOutput();
      out.writeByte((byte) 0x2a);
      for (int b = 1; b <= partial[k]; b++) {
        out.writeByte((byte) b);
      }
      try (BufferedChecksumIndexInput in =
          new BufferedChecksumIndexInput(new ByteBuffersIndexInput(out.toDataInput(), "eof"))) {
        in.readByte();
        String outcome;
        try {
          switch (kinds[k]) {
            case "short" -> in.readShort();
            case "int" -> in.readInt();
            case "long" -> in.readLong();
            default -> in.readLongs(new long[2], 0, 2);
          }
          outcome = "read";
        } catch (java.io.EOFException e) {
          outcome = "eof";
        }
        p("bcii_eof|" + kinds[k] + "|" + partial[k] + "|" + outcome + "|" + in.getChecksum());
      }
    }
  }

  // ---------------------------------------------------------------------------
  // Lucene90StoredFieldsFormat.impl(Mode)
  // ---------------------------------------------------------------------------

  static void storedFieldsImpl() throws Exception {
    Method impl =
        Lucene90StoredFieldsFormat.class.getDeclaredMethod(
            "impl", Lucene90StoredFieldsFormat.Mode.class);
    impl.setAccessible(true);
    for (Lucene90StoredFieldsFormat.Mode mode : Lucene90StoredFieldsFormat.Mode.values()) {
      p(
          "sfimpl|"
              + mode.name()
              + "|"
              + impl.invoke(new Lucene90StoredFieldsFormat(mode), mode));
    }
  }

  // ---------------------------------------------------------------------------
  // DocValuesRewriteMethod.equals / hashCode
  // ---------------------------------------------------------------------------

  static void docValuesRewrite() {
    p("dvrw|hash|" + new DocValuesRewriteMethod().hashCode());
    p("dvrw|equals_new|" + new DocValuesRewriteMethod().equals(new DocValuesRewriteMethod()));
    p(
        "dvrw|equals_singleton|"
            + MultiTermQuery.DOC_VALUES_REWRITE.equals(new DocValuesRewriteMethod()));
    p(
        "dvrw|equals_constant_score|"
            + new DocValuesRewriteMethod().equals(MultiTermQuery.CONSTANT_SCORE_REWRITE));
  }

  // ---------------------------------------------------------------------------
  // ByteBlockPool.setBytesRef (package-private)
  // ---------------------------------------------------------------------------

  static void byteBlockPool() {
    ByteBlockPool pool = new ByteBlockPool(new ByteBlockPool.DirectAllocator());
    pool.nextBuffer();
    int total = 2 * ByteBlockPool.BYTE_BLOCK_SIZE + 100;
    byte[] data = new byte[total];
    for (int i = 0; i < total; i++) {
      data[i] = (byte) (i % 251);
    }
    pool.append(data, 0, total);
    int size = ByteBlockPool.BYTE_BLOCK_SIZE;
    long[][] cases = {
      {0, 0},
      {0, 10},
      {size - 10, 10},
      {size - 9, 10},
      {size - 1, 1},
      {size - 1, 2},
      {size, 5},
      {2L * size - 6, 20},
      {100, size},
      {0, size},
      {1, 2L * size},
    };
    for (long[] c : cases) {
      BytesRefBuilder builder = new BytesRefBuilder();
      BytesRef result = new BytesRef();
      pool.setBytesRef(builder, result, c[0], (int) c[1]);
      String where;
      if (result.bytes == pool.getBuffer((int) (c[0] >> ByteBlockPool.BYTE_BLOCK_SHIFT))) {
        where = "block";
      } else if (result.bytes == builder.bytes()) {
        where = "builder";
      } else {
        where = "other";
      }
      CRC32 crc = new CRC32();
      crc.update(result.bytes, result.offset, result.length);
      p(
          "bbp|"
              + c[0]
              + "|"
              + c[1]
              + "|"
              + where
              + "|"
              + result.offset
              + "|"
              + result.length
              + "|"
              + builder.length()
              + "|"
              + crc.getValue());
    }
  }

  // ---------------------------------------------------------------------------
  // PhraseMatcher.impactsApproximation(), through PhraseScorer
  // ---------------------------------------------------------------------------

  /**
   * An exact phrase scored in TOP_SCORES mode iterates its impacts-aware approximation
   * (`ExactPhraseMatcher.impactsApproximation()`), and `PhraseScorer` pushes the minimum
   * competitive score into it and reads its max-score cache.
   */
  static void phrase() throws Exception {
    try (ByteBuffersDirectory dir = new ByteBuffersDirectory()) {
      try (IndexWriter writer = new IndexWriter(dir, new IndexWriterConfig(new StandardAnalyzer()))) {
        for (String text : new String[] {"x y", "a b", "a b a b", "b a"}) {
          Document doc = new Document();
          doc.add(new TextField("body", text, Field.Store.NO));
          writer.addDocument(doc);
        }
      }
      try (DirectoryReader reader = DirectoryReader.open(dir)) {
        IndexSearcher searcher = new IndexSearcher(reader);
        Query query = searcher.rewrite(new PhraseQuery("body", "a", "b"));
        Weight weight = searcher.createWeight(query, ScoreMode.TOP_SCORES, 1f);
        LeafReaderContext leaf = reader.leaves().get(0);
        Scorer first = weight.scorer(leaf);
        p("phrase|approx_first|" + first.twoPhaseIterator().approximation().nextDoc());
        Scorer pruned = weight.scorer(leaf);
        pruned.setMinCompetitiveScore(Float.MAX_VALUE);
        p("phrase|approx_after_min_score|" + pruned.twoPhaseIterator().approximation().nextDoc());
        Scorer bounds = weight.scorer(leaf);
        p("phrase|advance_shallow_0|" + bounds.advanceShallow(0));
        p(
            "phrase|max_score_bits|"
                + Float.floatToIntBits(bounds.getMaxScore(DocIdSetIterator.NO_MORE_DOCS)));
      }
    }
  }

  // ---------------------------------------------------------------------------
  // AbstractAnalysisFactory.getPattern
  // ---------------------------------------------------------------------------

  /** The smallest concrete factory, so that the protected final getPattern can be called. */
  static final class ProbeFactory extends AbstractAnalysisFactory {
    ProbeFactory() {
      super(new HashMap<>());
    }

    Pattern pattern(Map<String, String> args, String name) {
      return getPattern(args, name);
    }
  }

  /**
   * The dialect corpus: one or more discriminating samples for every construct of the
   * java.util.regex.Pattern summary, every inline flag, and the syntax the two dialects spell
   * differently. Pattern and sample are UTF-8 hex, so that line terminators and `|` survive both
   * the Java source and the row format. The Rust test declares which rows must differ. Rows
   * after the first 166 come from the audit: iteration over empty matches, nested quantifiers,
   * and syntax only one of the two dialects accepts.
   */
  static final String[][] PATTERN_CORPUS = {
    {"c3a9", "65c3a9c389"},
    {"5c5c", "615c62"},
    {"5c30313031", "4107"},
    {"5c3037", "0761"},
    {"5c30333737", "c3bf61"},
    {"5c30343030", "2030"},
    {"5c783431", "614162"},
    {"5c787b31463630307d", "61f09f9880"},
    {"5c7530303431", "614162"},
    {"5c75443833445c7544453030", "61f09f9880"},
    {"5c4e7b4c4154494e20534d414c4c204c4554544552204520574954482041435554457d", "65c3a9"},
    {"5c74", "610962"},
    {"5c6e", "610a62"},
    {"5c72", "610d62"},
    {"5c66", "610c62"},
    {"5c61", "610762"},
    {"5c65", "611b62"},
    {"5c6341", "780179"},
    {"5c633f", "787f79"},
    {"5b6162635d2b", "61626364c3a9"},
    {"5b5e6162635d2b", "61626364c3a9"},
    {"5b612d7a412d5a5d2b", "615ac3a9"},
    {"5b612d645b6d2d705d5d2b", "61626d6e787a"},
    {"5b612d7a26265b6465665d5d2b", "61626364656667"},
    {"5b612d7a26265b5e62635d5d2b", "61626364c3a9"},
    {"5b612d7a26265b5e6d2d705d5d2b", "6162636d6e6f717a"},
    {"5b5e615b625d5d2b", "61626362"},
    {"5b5b3a616c7068613a5d5d2b", "61623a3132"},
    {"5b612d2d625d", "612d62"},
    {"5b7e7e615d2b", "7e617e"},
    {"5b5d615d2b", "5d61"},
    {"5b5e5d615d2b", "5d6162"},
    {"5b5c642d7a5d2b", "312d7a"},
    {"2e2b", "610d62c28563e280a8640a65e280a966"},
    {"5c642b", "3132d9a3efbc94"},
    {"5c442b", "3132d9a3efbc9461"},
    {"5c682b", "612009c2a0e2808362e3808063e2808b64"},
    {"5c482b", "612009c2a062"},
    {"5c732b", "6120090b0c0dc2a0e28083c28562"},
    {"5c532b", "61c2a062"},
    {"5c762b", "610a0b0c0dc285e280a8e280a962"},
    {"5c562b", "61e280a8620b63"},
    {"5c772b", "636166c3a95f31d9a3"},
    {"5c572b", "636166c3a95f31d9a321"},
    {"5c707b4c6f7765727d2b", "6162c3a97a"},
    {"5c707b55707065727d2b", "4142c3895a"},
    {"5c707b41534349497d2b", "617fc3a9"},
    {"5c707b416c7068617d2b", "615ac3a9"},
    {"5c707b44696769747d2b", "3139d9a3"},
    {"5c707b416c6e756d7d2b", "6139c3a9d9a3"},
    {"5c707b50756e63747d2b", "212f3a405b607b7ec2abc2a1"},
    {"5c707b47726170687d2b", "6121c3a92062"},
    {"5c707b5072696e747d2b", "6120c3a9"},
    {"5c707b426c616e6b7d2b", "2009c2a061"},
    {"5c707b436e74726c7d2b", "017fc28561"},
    {"5c707b5844696769747d2b", "66463967efbc90"},
    {"5c707b53706163657d2b", "20090bc2a061"},
    {"5c507b4c6f7765727d2b", "616243c3a9"},
    {"5c707b6a6176614c6f776572436173657d2b", "61c2aac3a941"},
    {"5c707b6a6176615570706572436173657d2b", "41c38961"},
    {"5c707b6a617661576869746573706163657d2b", "20c2a0e280831c61"},
    {"5c707b6a6176614d6972726f7265647d2b", "28293c3ec2ab61"},
    {"5c707b49734c6174696e7d2b", "6162ceb1c3a9"},
    {"5c707b496e477265656b7d2b", "ceb1ceb261"},
    {"5c707b4c757d2b", "41c38961"},
    {"5c707b4973416c70686162657469637d2b", "61c3a931"},
    {"5c707b53637d2b", "24e282ac61"},
    {"5c507b496e477265656b7d2b", "ceb161"},
    {"5b5c707b4c7d26265b5e5c707b4c757d5d5d2b", "6142c3a9"},
    {"5c704c2b", "61c3a931"},
    {"5c707b4c7d2b", "61c3a931"},
    {"5c707b49734c7d2b", "61c3a931"},
    {"5c707b67633d4c757d2b", "41c38961"},
    {"5c707b73633d4c6174696e7d2b", "61ceb1"},
    {"5c707b626c6b3d477265656b7d2b", "ceb161"},
    {"5c707b4c6174696e7d2b", "61ceb1"},
    {"5c707b477265656b7d2b", "61ceb1"},
    {"5e61", "61620a61"},
    {"283f6d295e61", "610a620d610d0a61c28561e280a861"},
    {"283f6d295e", ""},
    {"283f6d295e", "610a"},
    {"6124", "610a"},
    {"6124", "610d0a"},
    {"6124", "610d"},
    {"6124", "61c285"},
    {"24", "610d0a"},
    {"283f6d296124", "610d610d0a61e280a862"},
    {"283f6d2924", "610d0a62"},
    {"5c625c772b5c62", "636166c3a9206e61c3af7665"},
    {"5c62", "61cc816220c3a9"},
    {"5c42", "616220c3a9"},
    {"5c627b677d", "65cc8178"},
    {"5c41", "6162"},
    {"5c476162", "6162786162"},
    {"625c5a", "61620a"},
    {"625c5a", "61620d0a"},
    {"625c7a", "61620a"},
    {"5c52", "610d0a620a630b64c28565"},
    {"5c525c6e", "0d0a"},
    {"5c58", "65cc8178"},
    {"617b322c337d", "6161616161"},
    {"617b322c337d3f", "6161616161"},
    {"617b322c337d2b61", "61616161"},
    {"617b327d", "616161"},
    {"617b322c7d", "61616161"},
    {"613f2b61", "61"},
    {"612a3f62", "616162"},
    {"617b2c337d", "61616161"},
    {"617b3130303030307d", "61"},
    {"7b327d", "61"},
    {"617b", "617b"},
    {"617b787d", "617b787d"},
    {"617c6162", "6162"},
    {"2861292862295c325c31", "61626261"},
    {"2861295c31", "6161"},
    {"5c31286129", "6161"},
    {"2861295c32", "6161"},
    {"2861295c3131", "616131"},
    {"283f3c6e3e61295c6b3c6e3e", "6161"},
    {"283f3c615f623e7829", "78"},
    {"283f3c6e3e6129283f3c6e3e6229", "6162"},
    {"5c2e", "612e62"},
    {"5c51612e625c45", "612e6220617862"},
    {"5c515b5c45", "615b"},
    {"5b5c51612d635c455d2b", "612d632062"},
    {"5c3c", "613c62"},
    {"5c3e", "613e62"},
    {"5cc3a9", "c3a9"},
    {"5c79", "79"},
    {"5c5f", "5f"},
    {"5c51315c45", "31"},
    {"283f3c6e616d653e6129", "61"},
    {"283f3a6129", "61"},
    {"283f692961", "41"},
    {"283f6929283f2d692961", "41"},
    {"283f693a612941", "6141"},
    {"61283f3d6229", "6162"},
    {"61283f216229", "6163"},
    {"283f3c3d612962", "6162"},
    {"283f3c21612962", "6362"},
    {"283f3e617c61622963", "616263206163"},
    {"283f3c3d612b2962", "616162"},
    {"283f503c6e3e6129", "61"},
    {"283f6929c3a9", "c389"},
    {"283f697529c3a9", "c389"},
    {"283f69296b", "e284aa"},
    {"283f6975296b", "e284aa"},
    {"283f692973", "c5bf"},
    {"283f69295b612d7a5d2b", "e284aa5a"},
    {"283f69295c707b4c6f7765727d2b", "615a"},
    {"283f69295c707b4c757d2b", "615a"},
    {"283f64292e", "0d"},
    {"283f64296124", "610d"},
    {"283f73292e", "0a"},
    {"283f78292061206220232063", "6162"},
    {"283f78295b6120625d2b", "616220"},
    {"283f782961235b0a62", "6162"},
    {"283f7829615c2062", "612062"},
    {"283f55295c772b", "636166c3a9"},
    {"283f5529612b", "616161"},
    {"283f752961", "61"},
    {"283f52296124", "610d0a"},
    {"283f632961", "61"},
    {"286162", "78"},
    {"5b612d", "78"},
    {"2a61", "78"},
    {"2861293f283f283129627c6329", "616263"},
    {"615c4b62", "6162"},
    {"61283f23636f6d6d656e742962", "6162"},
    {"5c757b65397d", "c3a9"},
    {"5c553030303030304539", "c3a9"},
    {"283f69292861295c31", "6141"},
    {"787b327d7b337d", "787878787878"},
    {"283f3d61292a62", "6162"},
    {"5c707b49735570706572636173657d2b", "4142c38963"},
    {"5c707b497357686974655f53706163657d2b", "61c2a062"},
    {"5c707b4973456d6f6a697d", "61"},
    {"612a2a", "6161"},
    {"5b612d5c645d", "61"},
    {"5c787b3131303030307d", "61"},
    {"612a", "62616161"},
    {"283f6d2924", "610a62"},
    {"617c", "6261"},
    {"5b6126265d", "61"},
    {"283f3c3d617c62632964", "626364"},
    {"283f6929c39f", "5353"},
    {"283f69295bc3a02dc3bf5d", "c380"},
    {"5c707b4c6f776572636173657d2b", "6162c2aa63"},
    {"5c707b416c6e756d7d2b", "6131c3a9"},
    {"283f55295c707b416c7068617d2b", "61c3a9"},
    {"5c707b497344696769747d2b", "31d9a3"},
    {"5c707b616c6c7d2b", "6162"},
    {"283f3c61313e78295c6b3c61313e", "7878"},
    {"5b5c772d5d", "2d"},
    {"5c777b322c7d3f", "616263"},
    {"283f3a617c62292a2b62", "616162"},
    {"5b5e5c6e5d2b", "610d62"},
    {"283f7329612e62", "610a62"},
    {"5c4261", "6261"},
    {"283f69295c787b323132617d", "6b"},
    {"5c732a", "612062"},
    {"2c3f", "612c62"},
    {"782a", "787861"},
    {"5c642a", "3132616233"},
    {"283f3d62297c61", "6162"},
    {"782a", "61f09f988062"},
    {"", "f09f9880"},
  };

  static String unhex(String hex) {
    byte[] bytes = new byte[hex.length() / 2];
    for (int i = 0; i < bytes.length; i++) {
      bytes[i] = (byte) Integer.parseInt(hex.substring(2 * i, 2 * i + 2), 16);
    }
    return new String(bytes, StandardCharsets.UTF_8);
  }

  static String hex(String text) {
    StringBuilder out = new StringBuilder();
    for (byte b : text.getBytes(StandardCharsets.UTF_8)) {
      out.append(String.format("%02x", b));
    }
    return out.toString();
  }

  static void patterns() {
    ProbeFactory factory = new ProbeFactory();
    for (String[] row : PATTERN_CORPUS) {
      String pattern = unhex(row[0]);
      String sample = unhex(row[1]);
      Map<String, String> args = new HashMap<>();
      args.put("pattern", pattern);
      String outcome;
      try {
        Matcher matcher = factory.pattern(args, "pattern").matcher(sample);
        List<String> found = new ArrayList<>();
        while (matcher.find()) {
          found.add(sample.codePointCount(0, matcher.start()) + ":" + hex(matcher.group()));
        }
        outcome = "OK|" + String.join(",", found);
      } catch (IllegalArgumentException e) {
        outcome = "ERR|";
      }
      p("pat|row|" + row[0] + "|" + row[1] + "|" + outcome);
    }
    for (String bad : new String[] {"(ab", "[a-", "*a"}) {
      Map<String, String> args = new HashMap<>();
      args.put("pattern", bad);
      try {
        factory.pattern(args, "pattern");
        p("pat|reject|" + hex(bad) + "|compiled|" + args.size());
      } catch (IllegalArgumentException e) {
        p("pat|reject|" + hex(bad) + "|" + e.getMessage() + "|" + args.size());
      }
    }
    try {
      factory.pattern(new HashMap<>(), "pattern");
      p("pat|missing|compiled");
    } catch (IllegalArgumentException e) {
      p("pat|missing|" + e.getMessage());
    }
  }

  // ---------------------------------------------------------------------------
  // CharArrayMap.createEntrySet: setValue through the entry set
  // ---------------------------------------------------------------------------

  static String entryWith(CharArrayMap<Integer> map, String key, Integer newValue) {
    for (Map.Entry<Object, Integer> e : map.entrySet()) {
      if (new String((char[]) e.getKey()).equals(key)) {
        try {
          return "old=" + e.setValue(newValue) + ",entry=" + e;
        } catch (UnsupportedOperationException uoe) {
          return "UnsupportedOperationException";
        }
      }
    }
    return "absent";
  }

  static void charArrayMapEntries() {
    CharArrayMap<Integer> map = new CharArrayMap<>(4, false);
    map.put("a", 1);
    map.put("b", 2);
    p("cam|size|" + map.entrySet().size());
    p("cam|set_a|" + entryWith(map, "a", 10));
    p("cam|get_a|" + map.get("a"));
    p("cam|contains_a10|" + map.entrySet().contains(Map.entry("a", 10)));
    p("cam|contains_a1|" + map.entrySet().contains(Map.entry("a", 1)));
    CharArrayMap<Integer> readOnly = CharArrayMap.unmodifiableMap(map);
    p("cam|set_ro|" + entryWith(readOnly, "b", 20));
    p("cam|get_b|" + readOnly.get("b"));
    try {
      readOnly.entrySet().clear();
      p("cam|clear_ro|cleared");
    } catch (UnsupportedOperationException e) {
      p("cam|clear_ro|UnsupportedOperationException");
    }
    map.entrySet().clear();
    p("cam|cleared_size|" + map.size());
  }

  public static void main(String[] args) throws Exception {
    similarities();
    hppc();
    checksums();
    checksumAfterEof();
    storedFieldsImpl();
    docValuesRewrite();
    byteBlockPool();
    phrase();
    patterns();
    charArrayMapEntries();
    p("fixture_ok=true");
  }
}
