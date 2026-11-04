package org.apache.lucene.tests.codecs;

import java.io.File;
import java.io.IOException;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Random;
import java.util.concurrent.atomic.AtomicReference;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.codecs.StoredFieldsWriter;
import org.apache.lucene.codecs.TermVectorsWriter;
import org.apache.lucene.codecs.lucene104.Lucene104Codec;
import org.apache.lucene.codecs.simpletext.SimpleTextOracle;
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

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    Path work = Files.createTempDirectory("testcodecs-oracle");
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
