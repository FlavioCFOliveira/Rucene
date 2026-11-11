package org.apache.lucene.tests.codecs;

import java.io.File;
import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import org.apache.lucene.util.BytesRef;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Random;
import java.util.concurrent.atomic.AtomicReference;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.codecs.StoredFieldsWriter;
import org.apache.lucene.codecs.TermVectorsWriter;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.DocValuesFormat;
import org.apache.lucene.codecs.KnnVectorsFormat;
import org.apache.lucene.codecs.PostingsFormat;
import org.apache.lucene.codecs.simpletext.PostingsFormatOracle;
import org.apache.lucene.codecs.simpletext.SimpleTextOracle;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.FieldType;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.IndexOptions;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.Term;
import org.apache.lucene.index.TieredMergePolicy;
import org.apache.lucene.tests.codecs.mockrandom.MockRandomPostingsFormat;
import org.apache.lucene.tests.codecs.ramonly.RAMOnlyPostingsFormat;
import org.apache.lucene.tests.codecs.uniformsplit.UniformSplitRot13PostingsFormat;
import org.apache.lucene.index.BinaryDocValues;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.SegmentInfo;
import org.apache.lucene.index.SortedDocValues;
import org.apache.lucene.index.SortedSetDocValues;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.NIOFSDirectory;
import org.apache.lucene.tests.codecs.asserting.AssertingCodec;
import org.apache.lucene.tests.codecs.cheapbastard.CheapBastardCodec;
import org.apache.lucene.tests.codecs.compressing.CompressingCodec;
import org.apache.lucene.tests.codecs.compressing.DeflateWithPresetCompressingCodec;
import org.apache.lucene.tests.codecs.compressing.FastCompressingCodec;
import org.apache.lucene.tests.codecs.compressing.FastDecompressionCompressingCodec;
import org.apache.lucene.tests.codecs.compressing.HighCompressionCompressingCodec;
import org.apache.lucene.tests.codecs.compressing.LZ4WithPresetCompressingCodec;
import org.apache.lucene.tests.codecs.compressing.dummy.DummyCompressingCodec;
import org.apache.lucene.tests.codecs.cranky.CrankyCodec;
import org.apache.lucene.tests.codecs.vector.ConfigurableMCodec;
import org.apache.lucene.util.StringHelper;
import org.apache.lucene.util.Version;

/**
 * The oracle of the test codecs of {@code org.apache.lucene.tests.codecs} (rmp #340 and the
 * {@code cranky} package of #341), for {@code
 * crates/rucene-test-framework/tests/test_codecs_oracle.rs}. Writes into {@code
 * <out>/rucene-test-framework/}:
 *
 * <ul>
 *   <li>{@code testcodecs_files.tsv}: every file of the {@code nocfs} and {@code cfs} corpus of
 *       {@link SimpleTextOracle#writeCorpus} written with each codec (case {@code <codec>/<case>}).
 *   <li>{@code testcodecs_misuse.tsv}: the error each misuse of an {@code Asserting} object raises
 *       (class, message), on the {@code nocfs} index written with {@link AssertingCodec}.
 *   <li>{@code testcodecs_cranky_draws.tsv}: the bound of every {@code random.nextInt(bound)} a
 *       {@link CrankyCodec} over {@link Lucene104Codec} draws while it writes the {@code nocfs}
 *       corpus, with a random that never draws 0 (no failure).
 *   <li>{@code testcodecs_cranky_seeds.tsv}: per seed, the {@code nocfs} corpus written with a
 *       {@link CrankyCodec} over {@code new Random(seed)}: the draws made up to the first failure
 *       and the root cause it raised (class, message).
 *   <li>The postings formats of the {@code bloom}, {@code uniformsplit} and {@code mockrandom}
 *       packages (rmp #341), through {@link PostingsFormatOracle#run}: {@code tc_bloom_*} ({@code
 *       TestBloomFilteredLucenePostings}), {@code tc_rot13_*} ({@code UniformSplitRot13}), {@code
 *       tc_strot13_*} ({@code STUniformSplitRot13}) and {@code tc_mockrandom_s<seed>_*} ({@code new
 *       MockRandomPostingsFormat(new Random(seed))}, a new one per corpus, seeds 1 to 3). The
 *       {@code blockterms} formats are {@code lucene-codecs}' fixtures {@code fixedgap_*}, {@code
 *       vargap_fixed_*} and {@code vargap_docfreq_*}, written with the same classes.
 *   <li>{@code testcodecs_ramonly_*.tsv}: {@code RAMOnlyPostingsFormat}: {@code _corpus.tsv}, the
 *       root cause of writing each corpus with a new instance; {@code _files.tsv}, {@code
 *       _reads.tsv} and {@code _enum.tsv}, the {@code ramonly} index of {@link #writeRamOnly},
 *       written and read in this JVM with the instance found by name.
 *   <li>{@code testcodecs_rot13_flags.tsv}: the encoding flags of {@code
 *       UniformSplitRot13PostingsFormat} after each Rot13 format writes the {@code nocfs} corpus,
 *       and after it is opened.
 *   <li>{@code testcodecs_spi.tsv}: every entry of the four {@code META-INF/services} files of
 *       {@code lucene-test-framework}: the provider's name, {@code toString()}, and the class
 *       {@code forName(name)} returns.
 * </ul>
 */
public final class TestCodecsOracle {
  private TestCodecsOracle() {}

  static final String[] CODECS = {
    "Asserting",
    "CheapBastard",
    "FastCompressing",
    "FastDecompressionCompressing",
    "HighCompressionCompressing",
    "DummyCompressing",
    "DeflateWithPresetCompressing",
    "LZ4WithPresetCompressing",
    "ConfigurableM",
    "FastCompressingSuffix",
  };

  static Codec codec(String label) {
    switch (label) {
      case "Asserting":
        return new AssertingCodec();
      case "CheapBastard":
        return new CheapBastardCodec();
      case "FastCompressing":
        return new FastCompressingCodec();
      case "FastDecompressionCompressing":
        return new FastDecompressionCompressingCodec();
      case "HighCompressionCompressing":
        return new HighCompressionCompressingCodec();
      case "DummyCompressing":
        return new DummyCompressingCodec();
      case "DeflateWithPresetCompressing":
        return new DeflateWithPresetCompressingCodec();
      case "LZ4WithPresetCompressing":
        return new LZ4WithPresetCompressingCodec();
      case "ConfigurableM":
        return new ConfigurableMCodec();
      case "FastCompressingSuffix":
        return new FastCompressingCodec(1 << 14, 128, true, 10);
      default:
        throw new AssertionError(label);
    }
  }

  /** {@code class<TAB>message}, {@code null} for a null message, tabs and line breaks escaped. */
  static String err(Throwable t) {
    String m = t.getMessage();
    if (m == null) {
      m = "null";
    }
    return t.getClass().getName() + "\t" + m.replace("\t", "\\t").replace("\n", "\\n");
  }

  interface Misuse {
    void run(LeafReader leaf) throws Exception;
  }

  static void misuse(PrintWriter w, String label, LeafReader leaf, Misuse m) {
    try {
      m.run(leaf);
      w.println(label + "\tnone\t-");
    } catch (Throwable t) {
      w.println(label + "\t" + err(t));
    }
  }

  static TermsEnum positioned(LeafReader leaf, String field) throws IOException {
    TermsEnum te = leaf.terms(field).iterator();
    te.next();
    return te;
  }

  static SegmentInfo segmentInfo(Directory dir, Codec codec, String name) {
    return new SegmentInfo(
        dir,
        Version.LATEST,
        null,
        name,
        1,
        false,
        false,
        codec,
        Collections.emptyMap(),
        StringHelper.randomId(),
        new HashMap<>(),
        null);
  }

  static void misuses(Path nocfs, Path scratch, PrintWriter w) throws Exception {
    try (Directory dir = new NIOFSDirectory(nocfs);
        DirectoryReader reader = DirectoryReader.open(dir)) {
      LeafReader leaf = reader.leaves().get(0).reader();
      misuse(w, "te.docFreq unpositioned", leaf, l -> l.terms("body").iterator().docFreq());
      misuse(w, "te.term unpositioned", leaf, l -> l.terms("body").iterator().term());
      misuse(w, "te.ord unpositioned", leaf, l -> l.terms("body").iterator().ord());
      misuse(
          w,
          "te.postings unpositioned",
          leaf,
          l -> l.terms("body").iterator().postings(null, PostingsEnum.FREQS));
      misuse(
          w,
          "te.totalTermFreq unpositioned",
          leaf,
          l -> l.terms("body").iterator().totalTermFreq());
      misuse(
          w,
          "te.next after end",
          leaf,
          l -> {
            TermsEnum te = l.terms("id").iterator();
            while (te.next() != null) {}
            te.next();
          });
      misuse(
          w,
          "te.impacts without freqs",
          leaf,
          l -> positioned(l, "body").impacts(PostingsEnum.NONE));
      misuse(
          w,
          "pe.freq before nextDoc",
          leaf,
          l -> positioned(l, "body").postings(null, PostingsEnum.FREQS).freq());
      misuse(
          w,
          "pe.advance backwards",
          leaf,
          l -> {
            PostingsEnum pe = positioned(l, "body").postings(null, PostingsEnum.FREQS);
            pe.nextDoc();
            pe.advance(pe.docID());
          });
      misuse(
          w,
          "pe.nextDoc after end",
          leaf,
          l -> {
            PostingsEnum pe = positioned(l, "body").postings(null, PostingsEnum.FREQS);
            while (pe.nextDoc() != DocIdSetIterator.NO_MORE_DOCS) {}
            pe.nextDoc();
          });
      misuse(
          w,
          "pe.advance after end",
          leaf,
          l -> {
            PostingsEnum pe = positioned(l, "body").postings(null, PostingsEnum.FREQS);
            while (pe.nextDoc() != DocIdSetIterator.NO_MORE_DOCS) {}
            pe.advance(1);
          });
      misuse(
          w,
          "pe.nextPosition beyond freq",
          leaf,
          l -> {
            PostingsEnum pe = positioned(l, "body").postings(null, PostingsEnum.POSITIONS);
            pe.nextDoc();
            int freq = pe.freq();
            for (int i = 0; i < freq; i++) {
              pe.nextPosition();
            }
            pe.nextPosition();
          });
      misuse(
          w,
          "pe.nextPosition before nextDoc",
          leaf,
          l -> positioned(l, "body").postings(null, PostingsEnum.POSITIONS).nextPosition());
      misuse(
          w,
          "pe.getPayload before nextPosition",
          leaf,
          l -> {
            PostingsEnum pe = positioned(l, "body").postings(null, PostingsEnum.ALL);
            pe.nextDoc();
            pe.getPayload();
          });
      misuse(
          w,
          "pe.startOffset before nextPosition",
          leaf,
          l -> {
            PostingsEnum pe = positioned(l, "body").postings(null, PostingsEnum.OFFSETS);
            pe.nextDoc();
            pe.startOffset();
          });
      misuse(
          w,
          "pe.freq after end",
          leaf,
          l -> {
            PostingsEnum pe = positioned(l, "body").postings(null, PostingsEnum.FREQS);
            while (pe.nextDoc() != DocIdSetIterator.NO_MORE_DOCS) {}
            pe.freq();
          });
      misuse(
          w,
          "pe.freq after reuse",
          leaf,
          l -> {
            TermsEnum te = positioned(l, "body");
            PostingsEnum pe = te.postings(null, PostingsEnum.FREQS);
            while (pe.nextDoc() != DocIdSetIterator.NO_MORE_DOCS) {}
            te.next();
            PostingsEnum reused = te.postings(pe, PostingsEnum.FREQS);
            if (reused != pe) {
              throw new IllegalStateException("not reused");
            }
            reused.freq();
          });
      misuse(
          w,
          "terms consumed in another thread",
          leaf,
          l -> {
            Terms terms = l.terms("body");
            AtomicReference<Throwable> thrown = new AtomicReference<>();
            Thread t =
                new Thread(
                    () -> {
                      try {
                        terms.iterator();
                      } catch (Throwable e) {
                        thrown.set(e);
                      }
                    });
            t.start();
            t.join();
            if (thrown.get() != null) {
              Throwable e = thrown.get();
              String m = e.getMessage();
              throw new AssertionError(m.substring(0, m.indexOf(" But was acquired in")));
            }
          });
      misuse(
          w, "dv.numeric longValue unpositioned", leaf, l -> l.getNumericDocValues("dv_num").longValue());
      misuse(
          w,
          "dv.numeric advance backwards",
          leaf,
          l -> {
            NumericDocValues dv = l.getNumericDocValues("dv_num");
            dv.nextDoc();
            dv.advance(dv.docID());
          });
      misuse(
          w,
          "dv.numeric advanceExact beyond maxDoc",
          leaf,
          l -> l.getNumericDocValues("dv_num").advanceExact(l.maxDoc()));
      misuse(
          w,
          "dv.binary binaryValue unpositioned",
          leaf,
          l -> {
            BinaryDocValues dv = l.getBinaryDocValues("dv_bin");
            dv.binaryValue();
          });
      misuse(
          w,
          "dv.sorted lookupOrd out of range",
          leaf,
          l -> {
            SortedDocValues dv = l.getSortedDocValues("dv_sorted");
            dv.lookupOrd(dv.getValueCount());
          });
      misuse(
          w, "dv.sorted ordValue unpositioned", leaf, l -> l.getSortedDocValues("dv_sorted").ordValue());
      misuse(
          w,
          "dv.sortedset nextOrd unpositioned",
          leaf,
          l -> {
            SortedSetDocValues dv = l.getSortedSetDocValues("dv_sset");
            dv.nextOrd();
          });
      misuse(
          w,
          "dv.sortednumeric nextValue unpositioned",
          leaf,
          l -> l.getSortedNumericDocValues("dv_snum").nextValue());
      misuse(
          w,
          "livedocs get beyond length",
          leaf,
          l -> {
            for (LeafReaderContext ctx : reader.leaves()) {
              LeafReader r = ctx.reader();
              if (r.getLiveDocs() != null) {
                r.getLiveDocs().get(r.maxDoc());
                return;
              }
            }
            throw new IllegalStateException("no deletions");
          });
      misuse(
          w,
          "livedocs as LiveDocs get beyond length",
          leaf,
          l -> {
            for (LeafReaderContext ctx : reader.leaves()) {
              LeafReader r = ctx.reader();
              org.apache.lucene.util.LiveDocs live = r.getLiveDocsWithDeletedIterator();
              if (live != null) {
                if (live.deletedCount() != r.maxDoc() - r.numDocs()) {
                  throw new IllegalStateException("deletedCount " + live.deletedCount());
                }
                live.get(r.maxDoc());
                return;
              }
            }
            throw new IllegalStateException("no LiveDocs");
          });
    }
    Codec codec = new AssertingCodec();
    try (Directory dir = new NIOFSDirectory(scratch)) {
      misuse(
          w,
          "sfw.finishDocument before startDocument",
          null,
          l -> {
            SegmentInfo si = segmentInfo(dir, codec, "_m1");
            StoredFieldsWriter sfw = codec.storedFieldsFormat().fieldsWriter(dir, si, IOContext.DEFAULT);
            try {
              sfw.finishDocument();
            } finally {
              closeQuietly(sfw);
            }
          });
      misuse(
          w,
          "sfw.startDocument twice",
          null,
          l -> {
            SegmentInfo si = segmentInfo(dir, codec, "_m2");
            StoredFieldsWriter sfw = codec.storedFieldsFormat().fieldsWriter(dir, si, IOContext.DEFAULT);
            try {
              sfw.startDocument();
              sfw.startDocument();
            } finally {
              closeQuietly(sfw);
            }
          });
      misuse(
          w,
          "sfw.finish with a wrong count",
          null,
          l -> {
            SegmentInfo si = segmentInfo(dir, codec, "_m3");
            StoredFieldsWriter sfw = codec.storedFieldsFormat().fieldsWriter(dir, si, IOContext.DEFAULT);
            try {
              sfw.finish(1);
            } finally {
              closeQuietly(sfw);
            }
          });
      misuse(
          w,
          "tvw.finishDocument before startDocument",
          null,
          l -> {
            SegmentInfo si = segmentInfo(dir, codec, "_m4");
            TermVectorsWriter tvw = codec.termVectorsFormat().vectorsWriter(dir, si, IOContext.DEFAULT);
            try {
              tvw.finishDocument();
            } finally {
              closeQuietly(tvw);
            }
          });
      misuse(
          w,
          "tvw.finishDocument with fields left",
          null,
          l -> {
            SegmentInfo si = segmentInfo(dir, codec, "_m5");
            TermVectorsWriter tvw = codec.termVectorsFormat().vectorsWriter(dir, si, IOContext.DEFAULT);
            try {
              tvw.startDocument(1);
              tvw.finishDocument();
            } finally {
              closeQuietly(tvw);
            }
          });
      misuse(
          w,
          "tvw.finish with a wrong count",
          null,
          l -> {
            SegmentInfo si = segmentInfo(dir, codec, "_m6");
            TermVectorsWriter tvw = codec.termVectorsFormat().vectorsWriter(dir, si, IOContext.DEFAULT);
            try {
              tvw.finish(3);
            } finally {
              closeQuietly(tvw);
            }
          });
    }
  }

  static void closeQuietly(java.io.Closeable c) {
    try {
      c.close();
    } catch (Throwable ignored) {
      // the misuse, not the close, is recorded
    }
  }

  /** A random that records the bound of every {@code nextInt(bound)}. */
  static class Recording extends Random {
    final List<Integer> bounds = new ArrayList<>();
    final List<String> labels = new ArrayList<>();
    final boolean neverZero;

    Recording(long seed, boolean neverZero) {
      super(seed);
      this.neverZero = neverZero;
    }

    @Override
    public int nextInt(int bound) {
      int v = super.nextInt(bound);
      if (bounds != null) {
        bounds.add(bound);
        if (neverZero) {
          labels.add(label());
        }
      }
      return neverZero && v == 0 ? 1 : v;
    }
  }

  /** The cranky method drawing: the innermost {@code Cranky*} frame, by simple name. */
  static String label() {
    for (StackTraceElement e : Thread.currentThread().getStackTrace()) {
      if (e.getClassName().contains(".cranky.Cranky")) {
        String c = e.getClassName();
        return c.substring(c.lastIndexOf('.') + 1) + "." + e.getMethodName();
      }
    }
    return "?";
  }

  static Throwable rootCause(Throwable t) {
    while (t.getCause() != null && t.getCause() != t) {
      t = t.getCause();
    }
    return t;
  }

  /** The entries of {@code lucene-test-framework}'s {@code META-INF/services} files. */
  static final String[][] SERVICES = {
    {"Codec", "org.apache.lucene.tests.codecs.asserting.AssertingCodec"},
    {"Codec", "org.apache.lucene.tests.codecs.cheapbastard.CheapBastardCodec"},
    {"Codec", "org.apache.lucene.tests.codecs.compressing.DeflateWithPresetCompressingCodec"},
    {"Codec", "org.apache.lucene.tests.codecs.compressing.FastCompressingCodec"},
    {"Codec", "org.apache.lucene.tests.codecs.compressing.FastDecompressionCompressingCodec"},
    {"Codec", "org.apache.lucene.tests.codecs.compressing.HighCompressionCompressingCodec"},
    {"Codec", "org.apache.lucene.tests.codecs.compressing.LZ4WithPresetCompressingCodec"},
    {"Codec", "org.apache.lucene.tests.codecs.compressing.dummy.DummyCompressingCodec"},
    {"Codec", "org.apache.lucene.tests.codecs.vector.ConfigurableMCodec"},
    {"DocValuesFormat", "org.apache.lucene.tests.codecs.asserting.AssertingDocValuesFormat"},
    {"KnnVectorsFormat", "org.apache.lucene.tests.codecs.asserting.AssertingKnnVectorsFormat"},
    {"PostingsFormat", "org.apache.lucene.tests.codecs.mockrandom.MockRandomPostingsFormat"},
    {"PostingsFormat", "org.apache.lucene.tests.codecs.ramonly.RAMOnlyPostingsFormat"},
    {"PostingsFormat", "org.apache.lucene.tests.codecs.blockterms.LuceneFixedGap"},
    {"PostingsFormat", "org.apache.lucene.tests.codecs.blockterms.LuceneVarGapFixedInterval"},
    {"PostingsFormat", "org.apache.lucene.tests.codecs.blockterms.LuceneVarGapDocFreqInterval"},
    {"PostingsFormat", "org.apache.lucene.tests.codecs.bloom.TestBloomFilteredLucenePostings"},
    {"PostingsFormat", "org.apache.lucene.tests.codecs.asserting.AssertingPostingsFormat"},
    {"PostingsFormat", "org.apache.lucene.tests.codecs.uniformsplit.UniformSplitRot13PostingsFormat"},
    {
      "PostingsFormat",
      "org.apache.lucene.tests.codecs.uniformsplit.sharedterms.STUniformSplitRot13PostingsFormat"
    },
  };

  /** {@code service<TAB>class<TAB>getName<TAB>toString<TAB>forName(getName).getClass}. */
  static void spi(PrintWriter w) throws Exception {
    for (String[] e : SERVICES) {
      Object o = Class.forName(e[1]).getDeclaredConstructor().newInstance();
      String name;
      Object found;
      switch (e[0]) {
        case "Codec" -> {
          name = ((Codec) o).getName();
          found = Codec.forName(name);
        }
        case "DocValuesFormat" -> {
          name = ((DocValuesFormat) o).getName();
          found = DocValuesFormat.forName(name);
        }
        case "KnnVectorsFormat" -> {
          name = ((KnnVectorsFormat) o).getName();
          found = KnnVectorsFormat.forName(name);
        }
        default -> {
          name = ((PostingsFormat) o).getName();
          found = PostingsFormat.forName(name);
        }
      }
      w.println(
          e[0] + "\t" + e[1] + "\t" + name + "\t" + o + "\t" + found.getClass().getName());
    }
  }

  /**
   * The {@code ramonly} index: two single-token fields with positions ({@code rp}, {@code rq}),
   * the only postings option {@code RAMOnlyPostingsFormat} writes under {@code -ea} (a field
   * without positions fails its {@code finishDoc} assertion, one with offsets is refused), over
   * terms whose Java order differs from their UTF-8 order (each in at most 8 documents: {@code
   * RAMDocsEnum.advance} on a fresh enum throws, see {@code _corpus.tsv}'s {@code advance}), in
   * two committed batches with
   * deletions (by {@code e_id}, which keeps the default format). With {@code freqs} the documents
   * also hold {@code rf}, a field with freqs and no positions; with {@code merge} the index is
   * then merged with {@code forceMerge(1)}, which fails: the flush stores the positions of {@code
   * rp} and {@code rq} as {@code -1} (it asks for no positions, {@code enumFlags = 0}, when the
   * field has no payloads), which the merge refuses.
   */
  static void writeRamOnly(Directory dir, Codec codec, boolean freqs, boolean merge)
      throws IOException {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setCodec(codec);
    config.setOpenMode(IndexWriterConfig.OpenMode.CREATE);
    config.setMergeScheduler(new SerialMergeScheduler());
    TieredMergePolicy policy = new TieredMergePolicy();
    policy.setNoCFSRatio(0.0);
    config.setMergePolicy(policy);
    config.setUseCompoundFile(false);
    FieldType freqsType = PostingsFormatOracle.sharedType(IndexOptions.DOCS_AND_FREQS);
    FieldType positions = PostingsFormatOracle.sharedType(IndexOptions.DOCS_AND_FREQS_AND_POSITIONS);
    String[] words = {"a", "b\u00e9", "\ud835\udc00x", "\ue000y", "z", "\ud83d\ude00"};
    try (IndexWriter w = new IndexWriter(dir, config)) {
      for (int batch = 0; batch < 2; batch++) {
        for (int i = batch * 60; i < batch * 60 + 60; i++) {
          Document d = new Document();
          d.add(
              new StringField(
                  "e_id", String.format(java.util.Locale.ROOT, "%04d", i), Field.Store.NO));
          if (freqs) {
            d.add(new Field("rf", words[i % 6], freqsType));
          }
          d.add(new Field("rq", words[i % 6] + (i % 13), positions));
          d.add(new Field("rq", words[(i / 2) % 6] + (i % 13), positions));
          d.add(new Field("rp", words[i % 5] + (i % 17), positions));
          d.add(new Field("rp", words[i % 3] + (i % 17), positions));
          w.addDocument(d);
        }
        w.commit();
        if (batch == 0) {
          for (int i = 0; i < 60; i += 7) {
            w.deleteDocuments(new Term("e_id", String.format(java.util.Locale.ROOT, "%04d", i)));
          }
        }
      }
      if (merge) {
        w.forceMerge(1);
        w.commit();
      }
    }
  }

  interface Probe {
    Object get() throws Exception;
  }

  /** The probe's value, {@code null} as {@code null}, a {@code BytesRef} in hex, or {@code !} and the simple name of what it throws. */
  static String probe(Probe p) {
    try {
      Object o = p.get();
      if (o instanceof BytesRef b) {
        return "x" + hex(b);
      }
      return String.valueOf(o);
    } catch (Throwable t) {
      return "!" + t.getClass().getSimpleName();
    }
  }

  static String hex(BytesRef b) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < b.length; i++) {
      sb.append(String.format("%02x", b.bytes[b.offset + i] & 0xff));
    }
    return sb.toString();
  }

  /**
   * What Lucene reads from the {@code ramonly} index: per leaf and field of {@code
   * RAMOnlyPostingsFormat} (every field but {@code e_id}), its statistics, every term (by {@code
   * next()}) with its stats and postings ({@code PostingsEnum.ALL}: doc, freq, positions,
   * payloads), and {@code seekCeil} probes of one enum (the empty term, each term, each term
   * followed by a 0 byte, {@code ffff}): status, {@code term()}, {@code docFreq()} and {@code
   * next()}; then {@code seekExact} of the same targets on a fresh enum. The generic dump does not
   * apply: {@code RAMTermsEnum.docFreq()} throws after a {@code NOT_FOUND} seek.
   */
  static List<String> ramOnlyTrace(Path path) throws IOException {
    List<String> out = new ArrayList<>();
    try (Directory dir = new NIOFSDirectory(path);
        DirectoryReader reader = DirectoryReader.open(dir)) {
      for (LeafReaderContext ctx : reader.leaves()) {
        out.add("leaf " + ctx.ord + " maxDoc=" + ctx.reader().maxDoc());
        for (org.apache.lucene.index.FieldInfo fi : ctx.reader().getFieldInfos()) {
          Terms terms = ctx.reader().terms(fi.name);
          if (terms == null || fi.name.equals("e_id")) {
            continue;
          }
          out.add(
              " field " + fi.name + " size=" + terms.size() + " sttf="
                  + terms.getSumTotalTermFreq() + " sdf=" + terms.getSumDocFreq() + " dc="
                  + terms.getDocCount() + " " + terms.hasFreqs() + "/" + terms.hasPositions()
                  + "/" + terms.hasOffsets() + "/" + terms.hasPayloads());
          List<BytesRef> all = new ArrayList<>();
          TermsEnum te = terms.iterator();
          for (BytesRef t = te.next(); t != null; t = te.next()) {
            all.add(BytesRef.deepCopyOf(t));
            out.add("  t " + hex(t) + " df=" + te.docFreq() + " ttf=" + te.totalTermFreq());
            PostingsEnum pe = te.postings(null, PostingsEnum.ALL);
            for (int d = pe.nextDoc(); d != DocIdSetIterator.NO_MORE_DOCS; d = pe.nextDoc()) {
              StringBuilder sb = new StringBuilder("   d=" + d + " f=" + pe.freq() + " cost=" + pe.cost());
              for (int i = 0; i < pe.freq(); i++) {
                sb.append(' ').append(pe.nextPosition()).append('/').append(pe.startOffset());
                sb.append('/').append(pe.endOffset()).append('/');
                sb.append(probe(pe::getPayload));
              }
              out.add(sb.toString());
            }
            out.add("   end docID=" + probe(pe::docID));
          }
          List<BytesRef> targets = new ArrayList<>();
          targets.add(new BytesRef(""));
          for (BytesRef t : all) {
            targets.add(t);
            BytesRef past = new BytesRef(new byte[t.length + 1]);
            System.arraycopy(t.bytes, t.offset, past.bytes, 0, t.length);
            targets.add(past);
          }
          targets.add(new BytesRef(new byte[] {(byte) 0xff, (byte) 0xff}));
          TermsEnum seek = terms.iterator();
          StringBuilder sb = new StringBuilder("  seek");
          for (BytesRef target : targets) {
            sb.append(' ').append(hex(target)).append(':').append(probe(() -> seek.seekCeil(target)));
            sb.append(':').append(probe(seek::term)).append(':').append(probe(seek::docFreq));
            sb.append(':').append(probe(seek::totalTermFreq));
            sb.append(':').append(probe(() -> seek.postings(null, PostingsEnum.NONE).nextDoc()));
            sb.append(':').append(probe(seek::next));
          }
          out.add(sb.toString());
          TermsEnum exact = terms.iterator();
          sb = new StringBuilder("  exact");
          for (BytesRef target : targets) {
            sb.append(' ').append(hex(target)).append(':').append(probe(() -> exact.seekExact(target)));
          }
          out.add(sb.toString());
          TermsEnum fresh = terms.iterator();
          out.add(
              "  fresh term=" + probe(fresh::term) + " ord=" + probe(fresh::ord) + " seekOrd="
                  + probe(() -> { fresh.seekExact(0L); return null; }));
        }
      }
    }
    return out;
  }

  static String flags() {
    return UniformSplitRot13PostingsFormat.encoderCalled
        + "\t"
        + UniformSplitRot13PostingsFormat.blocksEncoded
        + "\t"
        + UniformSplitRot13PostingsFormat.fieldsMetadataEncoded
        + "\t"
        + UniformSplitRot13PostingsFormat.dictionaryEncoded
        + "\t"
        + UniformSplitRot13PostingsFormat.decoderCalled;
  }

  /** {@code Lucene104Codec} with every field but {@code e_id} on {@code pf}. */
  static Codec perFieldCodec(PostingsFormat pf) {
    return new Lucene104Codec() {
      @Override
      public PostingsFormat getPostingsFormatForField(String field) {
        return field.equals("e_id") ? super.getPostingsFormatForField(field) : pf;
      }
    };
  }

  /** The postings formats of rmp #341 and the service files. */
  static void postingsFormats(File out, Path work) throws Exception {
    // First: the RAMOnly instance found by name numbers its slots from 0 in this JVM.
    Path ramonly = work.resolve("ramonly");
    Files.createDirectories(ramonly);
    try (Directory d = new NIOFSDirectory(ramonly)) {
      writeRamOnly(d, perFieldCodec(PostingsFormat.forName("RAMOnly")), false, false);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testcodecs_ramonly_files.tsv"), "UTF-8")) {
      w.println("# case\tfile\tlength\tfnv1a64\tcontent (\\xHH, \\\\)");
      SimpleTextOracle.files("ramonly", ramonly, w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testcodecs_ramonly_reads.tsv"), "UTF-8")) {
      w.println("# case\tline of TestCodecsOracle.ramOnlyTrace");
      for (String line : ramOnlyTrace(ramonly)) {
        w.println("ramonly\t" + line);
      }
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testcodecs_ramonly_enum.tsv"), "UTF-8")) {
      w.println("# case\tline of PostingsFormatOracle.enumTrace");
      for (String line : PostingsFormatOracle.enumTrace(ramonly)) {
        w.println("ramonly\t" + line);
      }
    }
    try (PrintWriter w =
        new PrintWriter(new File(out, "testcodecs_ramonly_corpus.tsv"), "UTF-8")) {
      w.println("# case\tclass\tmessage (root cause of writing the corpus with a new RAMOnly)");
      for (String c : new String[] {"nocfs", "cfs", "edge", "freqs", "merge"}) {
        Path p = work.resolve("ramonly-" + c);
        Files.createDirectories(p);
        Codec codec = perFieldCodec(new RAMOnlyPostingsFormat());
        try (Directory d = new NIOFSDirectory(p)) {
          if (c.equals("edge")) {
            SimpleTextOracle.writeEdge(d, codec);
          } else if (c.equals("freqs") || c.equals("merge")) {
            writeRamOnly(d, codec, c.equals("freqs"), c.equals("merge"));
          } else {
            SimpleTextOracle.writeCorpus(d, c.equals("cfs"), codec);
          }
          w.println(c + "\tnone\t-");
        } catch (Throwable t) {
          w.println(c + "\t" + err(rootCause(t)));
        }
      }
      // advance on a fresh enum: DocIdSetIterator.slowAdvance asserts docID() < target.
      try (Directory d = new NIOFSDirectory(ramonly);
          DirectoryReader r = DirectoryReader.open(d)) {
        TermsEnum te = r.leaves().get(0).reader().terms("rp").iterator();
        te.next();
        te.postings(null, PostingsEnum.FREQS).advance(1);
        w.println("advance\tnone\t-");
      } catch (Throwable t) {
        w.println("advance\t" + err(rootCause(t)));
      }
    }
    PostingsFormatOracle.run(
        out,
        "tc_bloom",
        "postings",
        "TestBloomFilteredLucenePostings",
        () -> PostingsFormat.forName("TestBloomFilteredLucenePostings"),
        List.of("blm"));
    PostingsFormatOracle.run(
        out,
        "tc_rot13",
        "postings",
        "UniformSplitRot13",
        () -> PostingsFormat.forName("UniformSplitRot13"),
        List.of("ustb", "ustd"));
    PostingsFormatOracle.run(
        out,
        "tc_strot13",
        "postings",
        "STUniformSplitRot13",
        () -> PostingsFormat.forName("STUniformSplitRot13"),
        List.of("stustb", "stustd"));
    for (long seed = 1; seed <= 3; seed++) {
      long s = seed;
      PostingsFormatOracle.run(
          out,
          "tc_mockrandom_s" + seed,
          "postings",
          "MockRandom",
          () -> new MockRandomPostingsFormat(new Random(s)),
          List.of("sd"));
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testcodecs_rot13_flags.tsv"), "UTF-8")) {
      w.println(
          "# format\tstep\tencoderCalled\tblocksEncoded\tfieldsMetadataEncoded"
              + "\tdictionaryEncoded\tdecoderCalled");
      for (String name : new String[] {"UniformSplitRot13", "STUniformSplitRot13"}) {
        UniformSplitRot13PostingsFormat.resetEncodingFlags();
        Path p = work.resolve("flags-" + name);
        Files.createDirectories(p);
        try (Directory d = new NIOFSDirectory(p)) {
          SimpleTextOracle.writeCorpus(d, false, perFieldCodec(PostingsFormat.forName(name)));
        }
        w.println(name + "\twrite\t" + flags());
        try (Directory d = new NIOFSDirectory(p);
            DirectoryReader r = DirectoryReader.open(d)) {
          w.println(name + "\tread\t" + flags());
        }
      }
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testcodecs_spi.tsv"), "UTF-8")) {
      w.println("# service\tclass\tgetName\ttoString\tforName(getName).getClass");
      spi(w);
    }
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    Path work = Files.createTempDirectory("testcodecs-oracle");
    postingsFormats(out, work);
    try (PrintWriter w = new PrintWriter(new File(out, "testcodecs_files.tsv"), "UTF-8")) {
      w.println("# case (codec/case)\tfile\tlength\tfnv1a64\tcontent (\\xHH, \\\\)");
      for (String label : CODECS) {
        for (String c : new String[] {"nocfs", "cfs"}) {
          Path p = work.resolve(label).resolve(c);
          Files.createDirectories(p);
          try (Directory d = new NIOFSDirectory(p)) {
            SimpleTextOracle.writeCorpus(d, c.equals("cfs"), codec(label));
          }
          SimpleTextOracle.files(label + "/" + c, p, w);
        }
      }
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testcodecs_misuse.tsv"), "UTF-8")) {
      w.println("# misuse\tclass\tmessage (null for none)");
      Path scratch = work.resolve("scratch");
      Files.createDirectories(scratch);
      misuses(work.resolve("Asserting").resolve("nocfs"), scratch, w);
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testcodecs_cranky_draws.tsv"), "UTF-8")) {
      w.println(
          "# draw\tbound\tcranky method (CrankyCodec(Lucene104Codec) writing nocfs, no draw is 0)");
      Recording random = new Recording(0, true);
      Path p = work.resolve("cranky-draws");
      Files.createDirectories(p);
      try (Directory d = new NIOFSDirectory(p)) {
        SimpleTextOracle.writeCorpus(d, false, new CrankyCodec(new Lucene104Codec(), random));
      }
      for (int i = 0; i < random.bounds.size(); i++) {
        w.println(i + "\t" + random.bounds.get(i) + "\t" + random.labels.get(i));
      }
    }
    try (PrintWriter w = new PrintWriter(new File(out, "testcodecs_cranky_seeds.tsv"), "UTF-8")) {
      w.println("# seed\tdraws\tclass\tmessage (root cause; none when the corpus was written)");
      for (long seed = 1; seed <= 40; seed++) {
        Recording random = new Recording(seed, false);
        Path p = work.resolve("cranky-" + seed);
        Files.createDirectories(p);
        String outcome;
        try (Directory d = new NIOFSDirectory(p)) {
          SimpleTextOracle.writeCorpus(d, false, new CrankyCodec(new Lucene104Codec(), random));
          outcome = "none\t-";
        } catch (Throwable t) {
          outcome = err(rootCause(t));
        }
        w.println(seed + "\t" + firstFailure(random, seed) + "\t" + outcome);
      }
    }
    try (PrintWriter w =
        new PrintWriter(new File(out, "testcodecs_compressing_random.tsv"), "UTF-8")) {
      w.println("# seed\tfactory\tcodec toString, or class<TAB>message");
      for (long seed = 1; seed <= 30; seed++) {
        compressing(w, seed, "randomInstance", r -> CompressingCodec.randomInstance(r));
        compressing(w, seed, "reasonableInstance", r -> CompressingCodec.reasonableInstance(r));
        compressing(
            w, seed, "randomInstance(suffix)", r -> CompressingCodec.randomInstance(r, true));
      }
    }
    org.apache.lucene.util.IOUtils.rm(work);
  }

  interface Factory {
    Codec make(Random r);
  }

  static void compressing(PrintWriter w, long seed, String factory, Factory f) {
    try {
      w.println(seed + "\t" + factory + "\t" + f.make(new Random(seed)));
    } catch (Throwable t) {
      w.println(seed + "\t" + factory + "\t" + err(t));
    }
  }

  /** The index of the first draw of 0 (the first failure), or the number of draws. */
  static int firstFailure(Recording random, long seed) {
    int draws = random.bounds.size();
    Random replay = new Random(seed);
    for (int i = 0; i < draws; i++) {
      if (replay.nextInt(random.bounds.get(i)) == 0) {
        return i;
      }
    }
    return draws;
  }
}
