// Oracle of task #347 (rucene-test-framework/tests/random_index_oracle.rs):
// RandomCodec's picks per seed, scripted RandomIndexWriter sessions, the
// postings RandomPostingsTester generates and its testTerms/testFull runs, and
// DocHelper's field set. Writes <out>/rucene-test-framework/random_codec.tsv,
// random_iw.tsv, random_postings.tsv, random_postings_terms.tsv,
// random_postings_full.tsv and doc_helper.tsv.
package org.apache.lucene.tests.index;

import com.carrotsearch.randomizedtesting.OracleSupport;
import java.io.File;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.EnumSet;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.Set;
import java.util.SortedMap;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.codecs.DocValuesFormat;
import org.apache.lucene.codecs.FieldsProducer;
import org.apache.lucene.codecs.PostingsFormat;
import org.apache.lucene.codecs.blocktreeords.BlockTreeOrdsPostingsFormat;
import org.apache.lucene.codecs.memory.DirectPostingsFormat;
import org.apache.lucene.codecs.memory.FSTPostingsFormat;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.NumericDocValuesField;
import org.apache.lucene.document.SortedDocValuesField;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.IndexableField;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.LogDocMergePolicy;
import org.apache.lucene.index.PointValues;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.SegmentReader;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.TermQuery;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.tests.util.LuceneTestCase;
import org.apache.lucene.tests.util.TestUtil;
import org.apache.lucene.util.BytesRef;
import org.junit.runner.JUnitCore;
import org.junit.runner.Result;
import org.junit.runner.notification.Failure;

public final class RandomIndexOracle {
  private RandomIndexOracle() {}

  /** The fields whose per-field formats RandomCodec rows record. */
  static final String[] FIELDS = {
    "id", "body", "title", "f0", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "été", "_x"
  };

  static final String[] WORDS = {"alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"};

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    // maybeChangeLiveIndexWriterConfig switches on the class environment's live
    // flush mode, which no runner sets here.
    setLiveFlushMode("EITHER");
    randomCodec(out);
    randomCodecPoints(out);
    randomCodecFiles(out);
    randomIndexWriter(out);
    postings(out);
    postingsTerms(out);
    docHelper(out);
    postingsFull(out);
  }

  @SuppressWarnings({"unchecked", "rawtypes"})
  static void setLiveFlushMode(String mode) throws Exception {
    Class<?> modes = Class.forName("org.apache.lucene.tests.util.LuceneTestCase$LiveIWCFlushMode");
    Field f = LuceneTestCase.class.getDeclaredField("liveIWCFlushMode");
    f.setAccessible(true);
    f.set(null, Enum.valueOf((Class) modes, mode));
  }

  static Object field(Object o, String name) throws Exception {
    Field f = o.getClass().getDeclaredField(name);
    f.setAccessible(true);
    return f.get(o);
  }

  static String hex(byte[] b, int off, int len) {
    StringBuilder sb = new StringBuilder();
    for (int i = off; i < off + len; i++) sb.append(String.format("%02x", b[i] & 0xff));
    return sb.toString();
  }

  // ---------------------------------------------------------------------------
  // RandomCodec
  // ---------------------------------------------------------------------------

  @SuppressWarnings("unchecked")
  static String codecRow(Random r, Set<String> avoid) throws Exception {
    RandomCodec c = avoid == null ? new RandomCodec(r) : new RandomCodec(r, avoid);
    List<String> parts = new ArrayList<>();
    List<String> formats = new ArrayList<>();
    for (PostingsFormat p : (List<PostingsFormat>) field(c, "formats")) formats.add(p.toString());
    parts.add("formats=" + String.join("|", formats));
    List<String> dv = new ArrayList<>();
    for (DocValuesFormat d : (List<DocValuesFormat>) field(c, "dvFormats")) dv.add(d.toString());
    parts.add("dvFormats=" + String.join("|", dv));
    parts.add("formatNames=" + String.join("|", c.formatNames));
    parts.add("dvFormatNames=" + String.join("|", c.dvFormatNames));
    parts.add("perFieldSeed=" + field(c, "perFieldSeed"));
    parts.add("maxPointsInLeafNode=" + field(c, "maxPointsInLeafNode"));
    parts.add("maxMBSortInHeap=" + field(c, "maxMBSortInHeap"));
    parts.add("bkdSplitRandomSeed=" + field(c, "bkdSplitRandomSeed"));
    for (String f : FIELDS) {
      parts.add(
          OracleSupport.units(f)
              + "="
              + c.getPostingsFormatForField(f)
              + "/"
              + c.getDocValuesFormatForField(f));
    }
    parts.add("toString=" + c);
    parts.add("next=" + r.nextLong());
    return String.join("\t", parts);
  }

  static void randomCodec(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "random_codec.tsv"), "UTF-8")) {
      w.println("# seed\tavoid\tRandomCodec(new Random(seed)[, avoid]): its lists, names, parameters, per-field picks, toString, next draw");
      for (long seed : OracleSupport.seeds()) {
        w.println(seed + "\t-\t" + codecRow(new Random(seed), null));
      }
      String[][] avoids = {
        {"Lucene104", "Asserting"}, {"MockRandom", "Direct", "FST50", "Lucene90"}, {"BlockTreeOrds", "LuceneFixedGap", "LuceneVarGapFixedInterval"}
      };
      long[] seeds = OracleSupport.seeds();
      for (int i = 0; i < avoids.length; i++) {
        for (int k = 0; k < 3; k++) {
          long seed = seeds[10 + i * 3 + k];
          Set<String> avoid = new HashSet<>(Arrays.asList(avoids[i]));
          w.println(seed + "\t" + String.join("|", avoids[i]) + "\t" + codecRow(new Random(seed), avoid));
        }
      }
    }
  }

  /**
   * The points RandomCodec writes: 3 segments of 1500 documents with a 2D and a 3D IntPoint drawn
   * from new Random(seed), then forceMerge(1); the point trees of each step.
   */
  static String pointsRow(long seed) throws Exception {
    List<String> parts = new ArrayList<>();
    Random values = new Random(seed);
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    IndexWriterConfig c = new IndexWriterConfig(new StandardAnalyzer());
    RandomCodec codec = new RandomCodec(new Random(seed));
    c.setCodec(codec);
    c.setMergeScheduler(new SerialMergeScheduler());
    c.setMergePolicy(new LogDocMergePolicy());
    org.apache.lucene.index.IndexWriter w = new org.apache.lucene.index.IndexWriter(dir, c);
    for (int seg = 0; seg < 3; seg++) {
      for (int i = 0; i < 1500; i++) {
        Document d = new Document();
        d.add(new IntPoint("p2", values.nextInt(1000), values.nextInt()));
        d.add(new IntPoint("p3", values.nextInt(50), values.nextInt(100000), values.nextInt()));
        w.addDocument(d);
      }
      w.commit();
    }
    parts.add("flushed=" + trees(dir));
    w.forceMerge(1);
    w.commit();
    parts.add("merged=" + trees(dir));
    w.close();
    return String.join("\t", parts);
  }

  static String trees(ByteBuffersDirectory dir) throws Exception {
    List<String> parts = new ArrayList<>();
    try (DirectoryReader reader = DirectoryReader.open(dir)) {
      for (LeafReaderContext ctx : reader.leaves()) {
        for (String field : new String[] {"p2", "p3"}) {
          List<String> lines = new ArrayList<>();
          walk(ctx.reader().getPointValues(field).getPointTree(), 0, lines);
          parts.add(field + ":" + lines.size() + ":" + IndexWrappersOracle.digest(lines));
        }
      }
    }
    return String.join("|", parts);
  }

  static void randomCodecPoints(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "random_codec_points.tsv"), "UTF-8")) {
      w.println("# seed\tRandomIndexOracle.pointsRow(seed): the point trees RandomCodec(new Random(seed)) writes, flushed and merged");
      long[] seeds = OracleSupport.seeds();
      for (int i = 0; i < 6; i++) {
        w.println(seeds[i] + "\t" + pointsRow(seeds[i]));
      }
    }
  }

  /**
   * The files RandomCodec(new Random(seed)) writes for one document of ten string fields and ten
   * numeric doc values fields, then for updates of three of the doc values fields.
   */
  static String filesRow(long seed) throws Exception {
    List<String> parts = new ArrayList<>();
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    IndexWriterConfig c = new IndexWriterConfig(new StandardAnalyzer());
    RandomCodec codec = new RandomCodec(new Random(seed));
    c.setCodec(codec);
    c.setMergeScheduler(new SerialMergeScheduler());
    c.setMergePolicy(new LogDocMergePolicy());
    c.setUseCompoundFile(false);
    org.apache.lucene.index.IndexWriter w = new org.apache.lucene.index.IndexWriter(dir, c);
    Document d = new Document();
    d.add(new StringField("id", "0", org.apache.lucene.document.Field.Store.NO));
    for (int i = 0; i < 10; i++) {
      d.add(new StringField("f" + i, "v" + i, org.apache.lucene.document.Field.Store.NO));
      d.add(new NumericDocValuesField("d" + i, i));
    }
    w.addDocument(d);
    w.commit();
    String[] files = dir.listAll();
    Arrays.sort(files);
    parts.add("files=" + String.join("|", files));
    w.updateNumericDocValue(new Term("id", "0"), "d1", 10L);
    w.updateNumericDocValue(new Term("id", "0"), "d4", 40L);
    w.updateNumericDocValue(new Term("id", "0"), "d7", 70L);
    w.commit();
    files = dir.listAll();
    Arrays.sort(files);
    parts.add("updated=" + String.join("|", files));
    w.close();
    parts.add("codec=" + codec);
    return String.join("\t", parts);
  }

  static void randomCodecFiles(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "random_codec_files.tsv"), "UTF-8")) {
      w.println("# seed\tRandomIndexOracle.filesRow(seed): the files of one document through RandomCodec(new Random(seed)), then after three doc values updates");
      for (long seed : OracleSupport.seeds()) {
        w.println(seed + "\t" + filesRow(seed));
      }
    }
  }

  // ---------------------------------------------------------------------------
  // RandomIndexWriter sessions
  // ---------------------------------------------------------------------------

  static Document doc(int i) {
    Document d = new Document();
    d.add(new StringField("id", Integer.toString(i), org.apache.lucene.document.Field.Store.YES));
    d.add(
        new TextField(
            "body",
            WORDS[i % WORDS.length] + " " + WORDS[(i * 3) % WORDS.length] + " w" + (i % 17),
            org.apache.lucene.document.Field.Store.NO));
    d.add(new NumericDocValuesField("num", i));
    d.add(new SortedDocValuesField("cat", new BytesRef("c" + (i % 5))));
    d.add(new IntPoint("pt", i, (i * 7) % 13));
    return d;
  }

  static void walk(PointValues.PointTree tree, int depth, List<String> lines) throws Exception {
    byte[] min = tree.getMinPackedValue();
    byte[] max = tree.getMaxPackedValue();
    lines.add(depth + " " + hex(min, 0, min.length) + " " + hex(max, 0, max.length) + " " + tree.size());
    if (tree.moveToChild()) {
      do {
        walk(tree, depth + 1, lines);
      } while (tree.moveToSibling());
      tree.moveToParent();
    }
  }

  static String session(long seed) throws Exception {
    List<String> parts = new ArrayList<>();
    Random r = new Random(seed);
    ByteBuffersDirectory dir = new ByteBuffersDirectory();
    IndexWriterConfig c = new IndexWriterConfig(new StandardAnalyzer());
    RandomCodec codec = new RandomCodec(new Random(seed + 1));
    c.setCodec(codec);
    c.setMergeScheduler(new SerialMergeScheduler());
    c.setMergePolicy(new LogDocMergePolicy());
    RandomIndexWriter w = new RandomIndexWriter(r, dir, c);
    for (int i = 0; i < 80; i++) {
      int op = i % 10;
      if (i == 25) {
        w.addDocuments(List.of(doc(1000 + i), doc(2000 + i)));
      } else if (i == 45) {
        w.updateDocuments(new Term("id", "5"), List.of(doc(3000), doc(3001)));
      } else if (op == 7) {
        w.updateDocument(new Term("id", Integer.toString(i - 3)), doc(i));
      } else {
        w.addDocument(doc(i));
      }
      if (op == 9) w.deleteDocuments(new Term("id", Integer.toString(i - 5)));
      if (i == 30) parts.add("commit30=" + (w.commit() > 0));
      if (i == 50) {
        DirectoryReader rd = w.getReader();
        parts.add("reader50=" + rd.numDocs() + "/" + rd.maxDoc());
        rd.close();
      }
      if (i == 55) w.updateNumericDocValue(new Term("id", "10"), "num", 777L);
      if (i == 60) w.deleteDocuments(new TermQuery(new Term("body", "gamma")));
      if (i == 65) w.flush();
      if (i == 70) w.forceMergeDeletes();
    }
    org.apache.lucene.index.IndexWriter.DocStats ds = w.getDocStats();
    parts.add("docStats=" + ds.maxDoc + "/" + ds.numDocs);
    w.close();
    parts.add("codec=" + codec);
    DirectoryReader reader = DirectoryReader.open(dir);
    parts.add("reader=" + reader.numDocs() + "/" + reader.maxDoc() + "/" + reader.leaves().size());
    for (LeafReaderContext ctx : reader.leaves()) {
      SegmentReader sr = (SegmentReader) ctx.reader();
      List<String> lines = new ArrayList<>();
      PointValues pv = sr.getPointValues("pt");
      if (pv != null) walk(pv.getPointTree(), 0, lines);
      parts.add(
          "leaf="
              + sr.getSegmentName()
              + "/"
              + sr.maxDoc()
              + "/"
              + sr.numDocs()
              + "/"
              + sr.getSegmentInfo().info.getCodec().getName()
              + "/points="
              + lines.size()
              + ":"
              + IndexWrappersOracle.digest(lines));
    }
    reader.close();
    String[] files = dir.listAll();
    Arrays.sort(files);
    parts.add("files=" + String.join("|", files));
    parts.add("next=" + r.nextLong());
    return String.join("\t", parts);
  }

  static void randomIndexWriter(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "random_iw.tsv"), "UTF-8")) {
      w.println("# seed\tthe scripted RandomIndexWriter session of RandomIndexOracle.session(seed)");
      long[] seeds = OracleSupport.seeds();
      for (int i = 0; i < 12; i++) {
        long seed = seeds[i];
        String row;
        try {
          row = session(seed);
        } catch (Throwable t) {
          row = "error\t" + OracleSupport.err(t);
        }
        w.println(seed + "\t" + row);
      }
    }
  }

  // ---------------------------------------------------------------------------
  // RandomPostingsTester
  // ---------------------------------------------------------------------------

  /** Every posting of every term, as lines, with the most a field allows. */
  @SuppressWarnings("unchecked")
  static List<String> postingsLines(RandomPostingsTester t) throws Exception {
    List<String> lines = new ArrayList<>();
    Map<String, SortedMap<BytesRef, Object>> fields =
        (Map<String, SortedMap<BytesRef, Object>>) field(t, "fields");
    for (Map.Entry<String, SortedMap<BytesRef, Object>> f : fields.entrySet()) {
      for (Map.Entry<BytesRef, Object> e : f.getValue().entrySet()) {
        Field seedField = e.getValue().getClass().getDeclaredField("seed");
        seedField.setAccessible(true);
        long seed = seedField.getLong(e.getValue());
        String term = e.getKey().utf8ToString();
        RandomPostingsTester.SeedPostings sp =
            RandomPostingsTester.getSeedPostings(
                term, seed, IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS, true);
        lines.add(f.getKey() + ":" + term + " seed=" + seed + " docFreq=" + sp.docFreq);
        int doc;
        while ((doc = sp.nextDoc()) != PostingsEnum.NO_MORE_DOCS) {
          StringBuilder sb = new StringBuilder();
          sb.append(doc).append(' ').append(sp.freq());
          for (int i = 0; i < sp.freq(); i++) {
            int pos = sp.nextPosition();
            BytesRef p = sp.getPayload();
            sb.append(' ')
                .append(pos)
                .append(',')
                .append(sp.startOffset())
                .append(',')
                .append(sp.endOffset())
                .append(',')
                .append(p == null ? "-" : hex(p.bytes, p.offset, p.length));
          }
          lines.add(sb.toString());
        }
      }
    }
    return lines;
  }

  static void postings(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "random_postings.tsv"), "UTF-8")) {
      w.println("# seed\tnew RandomPostingsTester(new Random(seed)): fields, terms per field, maxDoc, allTerms digest, postings count:digest, next draw");
      for (long seed : OracleSupport.seeds()) {
        Random r = new Random(seed);
        RandomPostingsTester t = new RandomPostingsTester(r);
        @SuppressWarnings("unchecked")
        Map<String, SortedMap<BytesRef, Object>> fields =
            (Map<String, SortedMap<BytesRef, Object>>) field(t, "fields");
        List<String> names = new ArrayList<>();
        for (Map.Entry<String, SortedMap<BytesRef, Object>> f : fields.entrySet()) {
          names.add(f.getKey() + ":" + f.getValue().size());
        }
        List<String> all = new ArrayList<>();
        for (RandomPostingsTester.FieldAndTerm ft : t.allTerms) {
          all.add(ft.field + ":" + ft.term.utf8ToString() + ":" + ft.ord);
        }
        List<String> lines = postingsLines(t);
        w.println(
            seed
                + "\tfields="
                + String.join("|", names)
                + "\tmaxDoc="
                + field(t, "maxDoc")
                + "\tallTerms="
                + all.size()
                + ":"
                + IndexWrappersOracle.digest(all)
                + "\tpostings="
                + lines.size()
                + ":"
                + IndexWrappersOracle.digest(lines)
                + "\tnext="
                + r.nextLong());
      }
    }
  }

  static Codec codecOf(String name) {
    switch (name) {
      case "default":
        return TestUtil.getDefaultCodec();
      case "FST50":
        return TestUtil.alwaysPostingsFormat(new FSTPostingsFormat());
      case "BlockTreeOrds":
        return TestUtil.alwaysPostingsFormat(new BlockTreeOrdsPostingsFormat());
      case "Direct":
        return TestUtil.alwaysPostingsFormat(new DirectPostingsFormat());
      default:
        throw new IllegalArgumentException(name);
    }
  }

  static final String[] TERMS_CODECS = {"default", "FST50", "BlockTreeOrds", "Direct"};

  static void postingsTerms(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "random_postings_terms.tsv"), "UTF-8")) {
      w.println("# seed\tcodec\tmaxAllowed\tpayloads\tthreads\tbuildIndex(codec, ByteBuffersDirectory, maxAllowed, payloads, false), testFields, testTerms(options, maxAllowed, maxAllowed, false): outcome and the tester's next draw");
      long[] seeds = OracleSupport.seeds();
      IndexOptions[] maxes = {
        IndexOptions.DOCS, IndexOptions.DOCS_AND_FREQS, IndexOptions.DOCS_AND_FREQS_AND_POSITIONS,
        IndexOptions.DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
      };
      int row = 0;
      for (String codecName : TERMS_CODECS) {
        for (IndexOptions max : maxes) {
          for (boolean payloads : new boolean[] {false, true}) {
            boolean threads = row % 5 == 4;
            long seed = seeds[row % seeds.length];
            row++;
            Random r = new Random(seed);
            String outcome;
            try {
              RandomPostingsTester t = new RandomPostingsTester(r);
              ByteBuffersDirectory dir = new ByteBuffersDirectory();
              FieldsProducer fp = t.buildIndex(codecOf(codecName), dir, max, payloads, false);
              t.testFields(fp);
              EnumSet<RandomPostingsTester.Option> options =
                  EnumSet.allOf(RandomPostingsTester.Option.class);
              if (!threads) options.remove(RandomPostingsTester.Option.THREADS);
              t.testTerms(fp, options, max, max, false);
              fp.close();
              dir.close();
              outcome = "ok\t" + r.nextLong();
            } catch (Throwable e) {
              outcome = "error\t" + OracleSupport.err(e);
            }
            w.println(seed + "\t" + codecName + "\t" + max + "\t" + payloads + "\t" + threads + "\t" + outcome);
          }
        }
      }
    }
  }

  static final List<String> FULL = Collections.synchronizedList(new ArrayList<>());

  static void postingsFull(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "random_postings_full.tsv"), "UTF-8")) {
      w.println("# seed\tTestRandomPostingsFull.testFull under RandomizedRunner (tests.seed=seed): per codec, testFull(codec, createTempDir(), DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS, true), then random().nextLong()");
      for (String seed : new String[] {"DEADBEEF", "1BADC0DE"}) {
        FULL.clear();
        System.setProperty("tests.seed", seed);
        Result res = new JUnitCore().run(TestRandomPostingsFull.class);
        for (Failure f : res.getFailures()) w.println("# failure " + f.toString().replace("\n", "\\n"));
        w.println(seed + "\t" + String.join("\t", FULL));
      }
      System.clearProperty("tests.seed");
    }
  }

  // ---------------------------------------------------------------------------
  // DocHelper
  // ---------------------------------------------------------------------------

  static String describe(IndexableField f) {
    var t = f.fieldType();
    String value =
        f.stringValue() != null
            ? OracleSupport.units(f.stringValue()).length() > 200
                ? "len" + f.stringValue().length()
                : OracleSupport.units(f.stringValue())
            : f.binaryValue() != null ? "bin" + hex(f.binaryValue().bytes, f.binaryValue().offset, f.binaryValue().length) : "-";
    return f.name()
        + ":"
        + f.getClass().getSimpleName()
        + ":stored="
        + t.stored()
        + ":tokenized="
        + t.tokenized()
        + ":"
        + t.indexOptions()
        + ":omitNorms="
        + t.omitNorms()
        + ":tv="
        + t.storeTermVectors()
        + t.storeTermVectorPositions()
        + t.storeTermVectorOffsets()
        + ":"
        + t.docValuesType()
        + ":"
        + value;
  }

  static void docHelper(File out) throws Exception {
    try (PrintWriter w = new PrintWriter(new File(out, "doc_helper.tsv"), "UTF-8")) {
      w.println("# label\tvalue: DocHelper's fields, its maps' key orders, nameValues, setupDoc, createDocument");
      for (IndexableField f : DocHelper.fields) w.println("field\t" + describe(f));
      String[] names = {"all", "indexed", "stored", "unstored", "unindexed", "termvector", "notermvector", "lazy", "noNorms", "noTf"};
      @SuppressWarnings("unchecked")
      Map<String, IndexableField>[] maps =
          new Map[] {
            DocHelper.all, DocHelper.indexed, DocHelper.stored, DocHelper.unstored, DocHelper.unindexed,
            DocHelper.termvector, DocHelper.notermvector, DocHelper.lazy, DocHelper.noNorms, DocHelper.noTf
          };
      for (int i = 0; i < names.length; i++) {
        w.println("map." + names[i] + "\t" + String.join("|", maps[i].keySet()));
      }
      w.println("nameValues\t" + String.join("|", DocHelper.nameValues.keySet()));
      Document d = new Document();
      DocHelper.setupDoc(d);
      w.println("setupDoc\t" + DocHelper.numFields(d));
      Document c = DocHelper.createDocument(7, "idx", 4);
      List<String> fs = new ArrayList<>();
      for (IndexableField f : c.getFields()) fs.add(describe(f));
      w.println("createDocument\t" + String.join("|", fs));
    }
  }
}
