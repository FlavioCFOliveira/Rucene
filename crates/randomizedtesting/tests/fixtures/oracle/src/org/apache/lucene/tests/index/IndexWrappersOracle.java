// Oracle of the index readers and reader wrappers of org.apache.lucene.tests.index (rmp #344):
// AssertingLeafReader / AssertingDirectoryReader, AllDeletedFilterReader, FieldFilterLeafReader,
// MergeReaderWrapper, MergingCodecReader, MergingDirectoryReaderWrapper, MismatchedCodecReader,
// MismatchedDirectoryReader, MismatchedLeafReader, OwnCacheKeyMultiReader, PointsStackTracker and
// PerThreadPKLookup. See README.md in this directory.
package org.apache.lucene.tests.index;

import com.carrotsearch.randomizedtesting.OracleSupport;
import java.io.BufferedReader;
import java.io.File;
import java.io.FileInputStream;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Random;
import java.util.concurrent.Callable;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.codecs.simpletext.SimpleTextReadOracle;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.IntPoint;
import org.apache.lucene.document.LongPoint;
import org.apache.lucene.document.StringField;
import org.apache.lucene.index.CodecReader;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.FieldInfo;
import org.apache.lucene.index.FieldInfos;
import org.apache.lucene.index.IndexReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.LeafReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.NumericDocValues;
import org.apache.lucene.index.PointValues;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.SerialMergeScheduler;
import org.apache.lucene.index.StoredFieldVisitor;
import org.apache.lucene.index.TermVectors;
import org.apache.lucene.index.Terms;
import org.apache.lucene.index.TermsEnum;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.util.Bits;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.LiveDocs;

/**
 * Writes {@code crates/rucene-test-framework/tests/fixtures/iw_wrappers.tsv}: per corpus case
 * ({@code nocfs}, {@code cfs}: the default-codec corpus of {@code mdw_corpus.tsv}, which this
 * oracle reads from the output directory, where {@code MockDirectoryWrapperOracle} writes it
 * first), scenario and step, the observable outcome of each wrapper over the corpus' leaves: its
 * {@code toString()}, document counts, field-presence probes, the cache-helper identity relations,
 * the field numbers its stored fields report, and the digest of {@code
 * SimpleTextReadOracle.dumpLeaf} through it (its {@code "   live"} impacts lines digested apart,
 * with the dump's {@code leaf}, {@code live} and {@code fi} lines in full); the {@code Random} draws a {@code Mismatched*} wrapper leaves next; {@code
 * PerThreadPKLookup} lookups (and after {@code reopen}); {@code OwnCacheKeyMultiReader}'s key and
 * listeners; the class a reopen of each directory wrapper returns; the misuse of the asserting
 * views (with the error, thread ids masked as {@code #N}); and {@code PointsStackTracker} fed by the
 * {@code compare} calls of a {@code PointValues.intersect} over an index of 5000 points.
 */
public final class IndexWrappersOracle {
  private IndexWrappersOracle() {}

  static PrintWriter w;

  static final String[] PROBE_FIELDS = {
    "id", "body", "tv", "dv_num", "dv_bin", "dv_sorted", "dv_snum", "dv_sset", "p_int", "p_long",
    "vf", "vb", "nope"
  };

  static final List<String> FILTER_FIELDS =
      Arrays.asList("body", "dv_bin", "dv_num", "p_int", "s_str", "tv", "vb", "id", "dv_sset");

  static String m(Object o) {
    String s = String.valueOf(o);
    return s.replaceAll("@[0-9a-f]+", "@HASH")
        .replaceAll("Thread\\[#[0-9]+,", "Thread[#N,")
        .replace("\n", "\\n")
        .replace("\t", "\\t");
  }

  static String err(Throwable t) {
    return "err\t" + t.getClass().getName() + "\t" + (t.getMessage() == null ? "<null>" : m(t.getMessage()));
  }

  static void row(String c, String scenario, String step, String value) {
    w.println(c + "\t" + scenario + "\t" + step + "\t" + value);
  }

  interface Step {
    Object run() throws Throwable;
  }

  static void rec(String c, String scenario, String step, Step s) {
    String value;
    try {
      value = "ok\t" + m(s.run());
    } catch (Throwable t) {
      value = err(t);
    }
    row(c, scenario, step, value);
  }

  // -------------------------------------------------------------------------------------------
  // Digest (FNV-1a 64 over each line: its length as 4 LE bytes, then its UTF-16 units as 2 LE
  // bytes each), as OracleSupport.Rec.s does.
  // -------------------------------------------------------------------------------------------

  static String digest(List<String> lines) {
    long h = 0xcbf29ce484222325L;
    for (String v : lines) {
      int n = v.length();
      for (int k = 0; k < 4; k++) {
        h ^= (n >>> (8 * k)) & 0xff;
        h *= 0x100000001b3L;
      }
      for (int k = 0; k < v.length(); k++) {
        char ch = v.charAt(k);
        h ^= ch & 0xff;
        h *= 0x100000001b3L;
        h ^= (ch >>> 8) & 0xff;
        h *= 0x100000001b3L;
      }
    }
    return String.format("%016x", h) + "/" + lines.size();
  }

  // -------------------------------------------------------------------------------------------
  // The corpus
  // -------------------------------------------------------------------------------------------

  static Directory loadCorpus(File tsv, String c) throws Exception {
    Directory dir = new ByteBuffersDirectory();
    try (BufferedReader r =
        new BufferedReader(
            new InputStreamReader(new FileInputStream(tsv), StandardCharsets.UTF_8))) {
      for (String line; (line = r.readLine()) != null; ) {
        if (line.startsWith("#") || line.isEmpty()) {
          continue;
        }
        String[] f = line.split("\t", -1);
        if (!f[0].equals(c)) {
          continue;
        }
        byte[] b = new byte[f[2].length() / 2];
        for (int i = 0; i < b.length; i++) {
          b[i] = (byte) Integer.parseInt(f[2].substring(2 * i, 2 * i + 2), 16);
        }
        try (IndexOutput out = dir.createOutput(f[1], IOContext.DEFAULT)) {
          out.writeBytes(b, b.length);
        }
      }
    }
    return dir;
  }

  // -------------------------------------------------------------------------------------------
  // What is recorded of one wrapped leaf
  // -------------------------------------------------------------------------------------------

  static char present(Step s) {
    try {
      return s.run() == null ? '0' : '1';
    } catch (Throwable t) {
      return 'E';
    }
  }

  static String probe(LeafReader r) {
    StringBuilder sb = new StringBuilder();
    for (String f : PROBE_FIELDS) {
      if (sb.length() > 0) {
        sb.append(',');
      }
      sb.append(f).append('=');
      sb.append(present(() -> r.terms(f)));
      sb.append(present(() -> r.getNumericDocValues(f)));
      sb.append(present(() -> r.getBinaryDocValues(f)));
      sb.append(present(() -> r.getSortedDocValues(f)));
      sb.append(present(() -> r.getSortedNumericDocValues(f)));
      sb.append(present(() -> r.getSortedSetDocValues(f)));
      sb.append(present(() -> r.getNormValues(f)));
      sb.append(present(() -> r.getPointValues(f)));
      sb.append(present(() -> r.getFloatVectorValues(f)));
      sb.append(present(() -> r.getByteVectorValues(f)));
    }
    return sb.toString();
  }

  static String helperRelation(IndexReader.CacheHelper mine, IndexReader.CacheHelper inner, IndexReader.CacheHelper again) {
    if (mine == null) {
      return "null";
    }
    String rel = inner != null && mine.getKey() == inner.getKey() ? "same" : "own";
    return rel + (again != null && again.getKey() == mine.getKey() ? "+stable" : "+unstable");
  }

  static String cache(LeafReader r, LeafReader in) {
    return "core="
        + helperRelation(r.getCoreCacheHelper(), in.getCoreCacheHelper(), r.getCoreCacheHelper())
        + " reader="
        + helperRelation(
            r.getReaderCacheHelper(), in.getReaderCacheHelper(), r.getReaderCacheHelper());
  }

  static String storedNumbers(LeafReader r) throws Exception {
    StringBuilder sb = new StringBuilder();
    for (int doc = 0; doc < Math.min(3, r.maxDoc()); doc++) {
      sb.append(doc).append(':');
      r.storedFields()
          .document(
              doc,
              new StoredFieldVisitor() {
                @Override
                public Status needsField(FieldInfo fieldInfo) {
                  sb.append(' ').append(fieldInfo.name).append('#').append(fieldInfo.number);
                  return Status.YES;
                }
              });
      sb.append(';');
    }
    return sb.toString();
  }

  static String liveDocs(LeafReader r) {
    Bits live = r.getLiveDocs();
    if (live == null) {
      return "null";
    }
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < live.length(); i++) {
      sb.append(live.get(i) ? '1' : '0');
    }
    if (live instanceof LiveDocs ld) {
      sb.append(" deleted=").append(ld.deletedCount());
    } else {
      sb.append(" bits");
    }
    return sb.toString();
  }

  static void leaf(String c, String scenario, int ord, LeafReader r, LeafReader in) {
    String p = "leaf" + ord + ".";
    rec(c, scenario, p + "toString", () -> r.toString());
    rec(
        c,
        scenario,
        p + "docs",
        () ->
            r.maxDoc() + "/" + r.numDocs() + "/" + r.numDeletedDocs() + "/" + r.hasDeletions());
    rec(c, scenario, p + "live", () -> liveDocs(r));
    rec(c, scenario, p + "probe", () -> probe(r));
    rec(c, scenario, p + "cache", () -> cache(r, in));
    rec(c, scenario, p + "stored", () -> storedNumbers(r));
    List<String> lines = new ArrayList<>();
    rec(
        c,
        scenario,
        p + "dump",
        () -> {
          SimpleTextReadOracle.dumpLeaf(ord, r, lines);
          return digest(notLive(lines));
        });
    // the dump's "   live" lines (impacts read through a view), digested apart
    List<String> live = new ArrayList<>(lines);
    live.removeAll(notLive(lines));
    row(c, scenario, p + "dumplive", "ok\t" + digest(live));
    List<String> head = new ArrayList<>();
    for (String l : lines) {
      if (l.startsWith("leaf ") || l.startsWith(" live ") || l.startsWith(" fi ")) {
        head.add(l);
      }
    }
    row(c, scenario, p + "fi", "ok\t" + m(String.join("|", head)));
  }

  static List<String> notLive(List<String> lines) {
    List<String> out = new ArrayList<>();
    for (String l : lines) {
      if (!l.startsWith("   live")) {
        out.add(l);
      }
    }
    return out;
  }

  static void dirReader(String c, String scenario, DirectoryReader d, DirectoryReader in) {
    rec(c, scenario, "toString", () -> d.toString());
    rec(c, scenario, "docs", () -> d.maxDoc() + "/" + d.numDocs() + "/" + d.leaves().size());
    rec(
        c,
        scenario,
        "cache",
        () ->
            helperRelation(
                d.getReaderCacheHelper(), in.getReaderCacheHelper(), d.getReaderCacheHelper()));
    List<LeafReaderContext> mine = d.leaves();
    List<LeafReaderContext> inner = in.leaves();
    for (int i = 0; i < mine.size(); i++) {
      leaf(c, scenario, i, mine.get(i).reader(), inner.get(i).reader());
    }
  }

  // -------------------------------------------------------------------------------------------
  // Scenarios
  // -------------------------------------------------------------------------------------------

  static void wrappers(String c, DirectoryReader reader) throws Exception {
    List<LeafReaderContext> leaves = reader.leaves();
    rec(c, "plain", "docs", () -> reader.maxDoc() + "/" + reader.numDocs() + "/" + leaves.size());
    rec(c, "plain", "toString", () -> reader.toString());
    for (LeafReaderContext ctx : leaves) {
      leaf(c, "plain", ctx.ord, ctx.reader(), ctx.reader());
    }

    dirReader(c, "asserting_dir", new AssertingDirectoryReader(reader), reader);

    for (LeafReaderContext ctx : leaves) {
      LeafReader in = ctx.reader();
      rec(c, "alldeleted", "leaf" + ctx.ord + ".new", () -> {
        AllDeletedFilterReader r = new AllDeletedFilterReader(in);
        leaf(c, "alldeleted", ctx.ord, r, in);
        return "built";
      });
      rec(c, "asserting_alldeleted", "leaf" + ctx.ord + ".new", () -> {
        AssertingLeafReader r = new AssertingLeafReader(new AllDeletedFilterReader(in));
        leaf(c, "asserting_alldeleted", ctx.ord, r, in);
        return "built";
      });
      for (boolean negate : new boolean[] {false, true}) {
        String scenario = negate ? "fieldfilter_neg" : "fieldfilter";
        FieldFilterLeafReader r =
            new FieldFilterLeafReader(in, new java.util.HashSet<>(FILTER_FIELDS), negate);
        leaf(c, scenario, ctx.ord, r, in);
      }
      FieldFilterLeafReader single =
          new FieldFilterLeafReader(in, new java.util.HashSet<>(List.of("body")), false);
      rec(c, "fieldfilter_single", "leaf" + ctx.ord + ".toString", () -> single.toString());
      FieldFilterLeafReader none =
          new FieldFilterLeafReader(in, new java.util.HashSet<>(), false);
      rec(c, "fieldfilter_empty", "leaf" + ctx.ord + ".toString", () -> none.toString());
      rec(
          c,
          "fieldfilter_empty",
          "leaf" + ctx.ord + ".tv",
          () -> {
            StringBuilder sb = new StringBuilder();
            TermVectors tv = none.termVectors();
            for (int d = 0; d < Math.min(4, none.maxDoc()); d++) {
              sb.append(tv.get(d) == null ? 'n' : 'f');
            }
            return sb.toString();
          });
    }

    long[] seeds = OracleSupport.seeds();
    long[] mismatchedSeeds = {seeds[0], seeds[5], seeds[10]};
    for (long seed : mismatchedSeeds) {
      String s = "seed" + seed;
      Random random = new Random(seed);
      DirectoryReader md = new MismatchedDirectoryReader(reader, random);
      dirReader(c, "mismatched_dir_" + s, md, reader);
      rec(c, "mismatched_dir_" + s, "next", () -> random.nextLong());

      Random r2 = new Random(seed);
      for (LeafReaderContext ctx : leaves) {
        leaf(c, "mismatched_leaf_" + s, ctx.ord, new MismatchedLeafReader(ctx.reader(), r2), ctx.reader());
      }
      rec(c, "mismatched_leaf_" + s, "next", () -> r2.nextLong());

      Random r3 = new Random(seed);
      for (LeafReaderContext ctx : leaves) {
        leaf(
            c,
            "mismatched_codec_" + s,
            ctx.ord,
            new MismatchedCodecReader((CodecReader) ctx.reader(), r3),
            ctx.reader());
      }
      rec(c, "mismatched_codec_" + s, "next", () -> r3.nextLong());
    }
    // the shuffle alone, for every seed of the grid
    for (long seed : seeds) {
      Random random = new Random(seed);
      rec(
          c,
          "shuffle",
          "seed" + seed,
          () -> {
            StringBuilder sb = new StringBuilder();
            FieldInfos infos = MismatchedLeafReader.shuffleInfos(leaves.get(0).reader().getFieldInfos(), random);
            for (FieldInfo fi : infos) {
              sb.append(fi.name).append('#').append(fi.number).append(' ');
            }
            return sb.toString() + random.nextLong();
          });
    }

    dirReader(c, "merging_dir", new MergingDirectoryReaderWrapper(reader), reader);
    for (LeafReaderContext ctx : leaves) {
      leaf(c, "merging_codec", ctx.ord, new MergingCodecReader((CodecReader) ctx.reader()), ctx.reader());
      leaf(c, "merge_wrapper", ctx.ord, new MergeReaderWrapper((CodecReader) ctx.reader()), ctx.reader());
    }

    pkLookup(c, reader);
  }

  static void pkLookup(String c, DirectoryReader reader) throws Exception {
    PerThreadPKLookup pk = new PerThreadPKLookup(reader, "id");
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < 340; i++) {
      sb.append(pk.lookup(new BytesRef(String.format("id-%04d", i)))).append(' ');
    }
    String all = sb.toString();
    rec(c, "pklookup", "lookups", () -> all);
    rec(c, "pklookup", "missing", () -> pk.lookup(new BytesRef("nope")));
    PerThreadPKLookup again = pk.reopen(reader);
    StringBuilder sb2 = new StringBuilder();
    for (int i = 0; i < 340; i += 7) {
      sb2.append(again.lookup(new BytesRef(String.format("id-%04d", i)))).append(' ');
    }
    String reopened = sb2.toString();
    rec(c, "pklookup", "reopen", () -> reopened);
    rec(c, "pklookup", "reopen_null", () -> pk.reopen(null));
    // the old lookup still answers after its enums were shared
    rec(c, "pklookup", "old_after_reopen", () -> pk.lookup(new BytesRef("id-0042")));
    // over all-deleted leaves (an OwnCacheKeyMultiReader that does not close them)
    List<IndexReader> subs = new ArrayList<>();
    for (LeafReaderContext ctx : reader.leaves()) {
      subs.add(new AllDeletedFilterReader(ctx.reader()));
    }
    rec(
        c,
        "pklookup",
        "alldeleted",
        () -> {
          MultiReaderHolder h = new MultiReaderHolder(subs);
          PerThreadPKLookup dead = new PerThreadPKLookup(h.reader, "id");
          StringBuilder s3 = new StringBuilder();
          for (int i = 0; i < 340; i += 13) {
            s3.append(dead.lookup(new BytesRef(String.format("id-%04d", i)))).append(' ');
          }
          return s3.toString();
        });
  }

  /** An OwnCacheKeyMultiReader over sub-readers it must not close: each is incRef'ed first. */
  static final class MultiReaderHolder {
    final OwnCacheKeyMultiReader reader;

    MultiReaderHolder(List<IndexReader> subs) throws Exception {
      for (IndexReader r : subs) {
        r.incRef();
      }
      reader = new OwnCacheKeyMultiReader(subs.toArray(new IndexReader[0]));
    }
  }

  static void ownCacheKey(String c, DirectoryReader reader) throws Exception {
    List<IndexReader> subs = new ArrayList<>();
    for (LeafReaderContext ctx : reader.leaves()) {
      subs.add(ctx.reader());
    }
    MultiReaderHolder h = new MultiReaderHolder(subs);
    OwnCacheKeyMultiReader r = h.reader;
    rec(c, "owncachekey", "toString", () -> r.toString());
    rec(c, "owncachekey", "docs", () -> r.maxDoc() + "/" + r.numDocs() + "/" + r.leaves().size());
    rec(
        c,
        "owncachekey",
        "key",
        () -> {
          IndexReader.CacheKey key = r.getReaderCacheHelper().getKey();
          StringBuilder sb = new StringBuilder();
          sb.append(key == r.getReaderCacheHelper().getKey() ? "stable" : "unstable");
          for (IndexReader sub : subs) {
            IndexReader.CacheHelper sh = sub.getReaderCacheHelper();
            sb.append(sh != null && sh.getKey() == key ? " same" : " other");
          }
          return sb.toString();
        });
    AtomicInteger calls = new AtomicInteger();
    rec(
        c,
        "owncachekey",
        "addListener",
        () -> {
          r.getReaderCacheHelper().addClosedListener(k -> calls.incrementAndGet());
          r.getReaderCacheHelper().addClosedListener(k -> calls.incrementAndGet());
          return calls.get();
        });
    rec(c, "owncachekey", "close", () -> {
      r.close();
      return calls.get() + " refCount=" + r.getRefCount() + " subRefCount=" + subs.get(0).getRefCount();
    });
    rec(
        c,
        "owncachekey",
        "addListenerAfterClose",
        () -> {
          r.getReaderCacheHelper().addClosedListener(k -> calls.incrementAndGet());
          return calls.get();
        });
  }

  static Object inThread(String name, Callable<Object> body) throws Throwable {
    AtomicReference<Object> result = new AtomicReference<>();
    AtomicReference<Throwable> error = new AtomicReference<>();
    Thread t =
        new Thread(
            () -> {
              try {
                result.set(body.call());
              } catch (Throwable e) {
                error.set(e);
              }
            },
            name);
    t.start();
    t.join();
    if (error.get() != null) {
      throw error.get();
    }
    return result.get();
  }

  static void misuse(String c, DirectoryReader reader) throws Exception {
    LeafReader in = reader.leaves().get(0).reader();
    LeafReader deleted = null;
    for (LeafReaderContext ctx : reader.leaves()) {
      if (ctx.reader().hasDeletions()) {
        deleted = ctx.reader();
        break;
      }
    }
    final LeafReader withDeletions = deleted;
    AssertingLeafReader r = new AssertingLeafReader(in);
    rec(c, "misuse", "termvectors_thread", () -> {
      TermVectors tv = (TermVectors) inThread("iw-acquire", () -> r.termVectors());
      return inThread("iw-consume", () -> tv.get(0));
    });
    rec(c, "misuse", "storedfields_thread", () -> {
      org.apache.lucene.index.StoredFields sf =
          (org.apache.lucene.index.StoredFields) inThread("iw-acquire", () -> r.storedFields());
      return inThread("iw-consume", () -> { sf.document(0); return "read"; });
    });
    AssertingLeafReader rd = new AssertingLeafReader(withDeletions);
    rec(c, "misuse", "bits_thread", () -> {
      Bits live = (Bits) inThread("iw-acquire", () -> rd.getLiveDocs());
      return inThread("iw-consume", () -> live.get(0));
    });
    rec(c, "misuse", "bits_out_of_range", () -> {
      Bits live = rd.getLiveDocs();
      return live.get(live.length());
    });
    rec(c, "misuse", "bits_kind", () -> {
      Bits live = rd.getLiveDocs();
      return live instanceof LiveDocs ld ? "livedocs deleted=" + ld.deletedCount() : "bits";
    });
    rec(c, "misuse", "nextdoc_after_end", () -> {
      TermsEnum te = r.terms("body").iterator();
      te.next();
      PostingsEnum pe = te.postings(null, PostingsEnum.FREQS);
      while (pe.nextDoc() != DocIdSetIterator.NO_MORE_DOCS) {}
      return pe.nextDoc();
    });
    rec(c, "misuse", "term_unpositioned", () -> {
      TermsEnum te = r.terms("body").iterator();
      while (te.next() != null) {}
      return te.term();
    });
    rec(c, "misuse", "dv_unpositioned", () -> {
      NumericDocValues dv = r.getNumericDocValues("dv_num");
      return dv.longValue();
    });
    rec(c, "misuse", "dv_thread", () -> {
      NumericDocValues dv = (NumericDocValues) inThread("iw-acquire", () -> r.getNumericDocValues("dv_num"));
      return inThread("iw-consume", () -> dv.nextDoc());
    });
    rec(c, "misuse", "mismatched_bogus", () -> {
      FieldInfo fi = in.getFieldInfos().fieldInfo("s_str");
      new MismatchedLeafReader.MismatchedVisitor(
              new StoredFieldVisitor() {
                @Override
                public Status needsField(FieldInfo fieldInfo) {
                  return Status.YES;
                }
              },
              new FieldInfos(new FieldInfo[0]))
          .stringField(fi, "x");
      return "visited";
    });
    rec(c, "misuse", "merging_dir_cast", () -> new MergingDirectoryReaderWrapper(new AssertingDirectoryReader(reader)));
  }

  /** The directory of {@code c} with one more document committed. */
  static void addOne(Directory dir) throws Exception {
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setMergeScheduler(new SerialMergeScheduler());
    try (IndexWriter iw = new IndexWriter(dir, config)) {
      Document d = new Document();
      d.add(new StringField("id", "id-9999", Field.Store.YES));
      iw.addDocument(d);
      iw.commit();
    }
  }

  static void reopen(String c, File corpus) throws Exception {
    String[] names = {"asserting_dir", "mismatched_dir", "merging_dir"};
    for (String name : names) {
      Directory dir = loadCorpus(corpus, c);
      DirectoryReader reader = DirectoryReader.open(dir);
      DirectoryReader wrapped =
          switch (name) {
            case "asserting_dir" -> new AssertingDirectoryReader(reader);
            case "mismatched_dir" -> new MismatchedDirectoryReader(reader, new Random(42));
            default -> new MergingDirectoryReaderWrapper(reader);
          };
      rec(c, "reopen", name + ".unchanged", () -> DirectoryReader.openIfChanged(wrapped));
      addOne(dir);
      rec(c, "reopen", name + ".changed", () -> {
        DirectoryReader r = DirectoryReader.openIfChanged(wrapped);
        String s = r.toString();
        String result = s.substring(0, s.indexOf('(')) + " " + r.maxDoc() + "/" + r.numDocs() + "/" + r.leaves().size();
        r.close();
        return result;
      });
      wrapped.close();
    }
    // MergeReaderWrapper is a reader of its own: closing it closes the wrapped leaf
    Directory dir = loadCorpus(corpus, c);
    DirectoryReader reader = DirectoryReader.open(dir);
    MergeReaderWrapper mrw = new MergeReaderWrapper((CodecReader) reader.leaves().get(0).reader());
    rec(c, "mergewrapper_close", "close", () -> {
      mrw.close();
      return mrw.getRefCount() + "/" + reader.leaves().get(0).reader().getRefCount();
    });
    rec(c, "mergewrapper_close", "terms", () -> mrw.terms("body"));
    rec(c, "mergewrapper_close", "storedFields", () -> mrw.storedFields());
    // the directory reader then decRefs a leaf already closed
    rec(c, "mergewrapper_close", "reader_close", () -> {
      reader.close();
      return "closed";
    });
    lifecycle(c, corpus);
  }

  /**
   * A wrapper is a reader of its own: a FilterLeafReader registers as a parent of the reader it
   * wraps (closing that reader makes the wrapper fail ensureOpen) and closes it when it closes; a
   * FilterCodecReader calls its doClose.
   */
  static void lifecycle(String c, File corpus) throws Exception {
    String[] names = {"asserting", "alldeleted", "fieldfilter", "mismatched", "merging_codec", "mismatched_codec"};
    for (String name : names) {
      for (String what : new String[] {"inner_closed", "wrapper_closed"}) {
        DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
        LeafReader in = reader.leaves().get(0).reader();
        LeafReader w =
            switch (name) {
              case "asserting" -> new AssertingLeafReader(in);
              case "alldeleted" -> new AllDeletedFilterReader(in);
              case "fieldfilter" -> new FieldFilterLeafReader(in, new java.util.HashSet<>(FILTER_FIELDS), false);
              case "mismatched" -> new MismatchedLeafReader(in, new Random(7));
              case "merging_codec" -> new MergingCodecReader((CodecReader) in);
              default -> new MismatchedCodecReader((CodecReader) in, new Random(7));
            };
        String s = "lifecycle_" + name;
        if (what.equals("inner_closed")) {
          rec(c, s, what + ".close", () -> {
            reader.close();
            return w.getRefCount() + "/" + in.getRefCount();
          });
        } else {
          rec(c, s, what + ".close", () -> {
            w.close();
            return w.getRefCount() + "/" + in.getRefCount();
          });
        }
        rec(c, s, what + ".terms", () -> w.terms("body") == null ? "null" : "terms");
        rec(c, s, what + ".numeric", () -> w.getNumericDocValues("dv_num") == null ? "null" : "dv");
        rec(c, s, what + ".liveDocs", () -> w.getLiveDocs() == null ? "null" : "bits");
        rec(c, s, what + ".fieldInfos", () -> w.getFieldInfos().size());
        rec(c, s, what + ".leaves", () -> w.leaves().size());
        if (what.equals("wrapper_closed")) {
          rec(c, s, what + ".reader_close", () -> {
            reader.close();
            return "closed";
          });
        }
      }
    }
    // FieldFilterLeafReader does not filter getNumericDocValues, which AssertingLeafReader
    // checks against the (filtered) field infos
    DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
    LeafReader in = reader.leaves().get(0).reader();
    AssertingLeafReader a =
        new AssertingLeafReader(new FieldFilterLeafReader(in, new java.util.HashSet<>(List.of("body")), false));
    rec(c, "asserting_fieldfilter", "numeric", () -> a.getNumericDocValues("dv_num"));
    rec(c, "asserting_fieldfilter", "binary", () -> a.getBinaryDocValues("dv_bin"));
    rec(c, "asserting_fieldfilter", "norms", () -> a.getNormValues("tv"));
    rec(c, "asserting_fieldfilter", "points", () -> a.getPointValues("p_int") == null ? "null" : "points");
    reader.close();
    coreLifecycle(c, corpus);
  }

  /** What a reader answers once closed: each member reports its ensureOpen() outcome. */
  static void probeClosed(String c, String s, String what, LeafReader r) {
    rec(c, s, what + ".terms", () -> r.terms("body") == null ? "null" : "terms");
    rec(c, s, what + ".numeric", () -> r.getNumericDocValues("dv_num") == null ? "null" : "dv");
    rec(c, s, what + ".norms", () -> r.getNormValues("body") == null ? "null" : "norms");
    rec(c, s, what + ".points", () -> r.getPointValues("p_int") == null ? "null" : "points");
    rec(c, s, what + ".pointsMissing", () -> r.getPointValues("nope") == null ? "null" : "points");
    rec(c, s, what + ".floatVectors", () -> r.getFloatVectorValues("vf") == null ? "null" : "vectors");
    rec(c, s, what + ".storedFields", () -> { r.storedFields(); return "storedFields"; });
    rec(c, s, what + ".termVectors", () -> { r.termVectors(); return "termVectors"; });
    rec(c, s, what + ".liveDocs", () -> r.getLiveDocs() == null ? "null" : "bits");
    rec(c, s, what + ".fieldInfos", () -> r.getFieldInfos().size());
    rec(c, s, what + ".metaData", () -> r.getMetaData().createdVersionMajor());
    rec(c, s, what + ".checkIntegrity", () -> { r.checkIntegrity(); return "checked"; });
    rec(c, s, what + ".numDocs", () -> r.numDocs() + "/" + r.maxDoc());
    rec(c, s, what + ".leaves", () -> r.leaves().size());
  }

  /** A FilterLeafReader of core: the class is abstract, its subclasses supply the cache helpers. */
  static LeafReader coreFilter(LeafReader in) {
    return new org.apache.lucene.index.FilterLeafReader(in) {
      @Override
      public CacheHelper getCoreCacheHelper() {
        return in.getCoreCacheHelper();
      }

      @Override
      public CacheHelper getReaderCacheHelper() {
        return in.getReaderCacheHelper();
      }
    };
  }

  /** A FilterCodecReader of core, as {@link #coreFilter}. */
  static CodecReader coreCodecFilter(CodecReader in) {
    return new org.apache.lucene.index.FilterCodecReader(in) {
      @Override
      public CacheHelper getCoreCacheHelper() {
        return in.getCoreCacheHelper();
      }

      @Override
      public CacheHelper getReaderCacheHelper() {
        return in.getReaderCacheHelper();
      }
    };
  }

  /**
   * The reader state of the core readers (rmp #344 round 2): every IndexReader has its own
   * refCount and closed state; a composite (BaseCompositeReader), a FilterLeafReader and a
   * ParallelLeafReader register as parents of their sub-readers; SegmentReader and CodecReader
   * call ensureOpen() where Java's members do.
   */
  static void coreLifecycle(String c, File corpus) throws Exception {
    // SegmentReader: the bounds of CodecReader.storedFields(), then every member once closed
    {
      DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
      CodecReader seg = (CodecReader) reader.leaves().get(0).reader();
      IndexReader.CacheHelper core = seg.getCoreCacheHelper();
      IndexReader.CacheHelper mine = seg.getReaderCacheHelper();
      String s = "core_segment";
      rec(c, s, "doc.maxDoc", () -> { seg.storedFields().document(seg.maxDoc()); return "doc"; });
      rec(c, s, "doc.minus1", () -> { seg.storedFields().document(-1); return "doc"; });
      rec(c, s, "prefetch.maxDoc", () -> { seg.storedFields().prefetch(seg.maxDoc()); return "prefetch"; });
      rec(c, s, "close", () -> {
        reader.close();
        return seg.getRefCount() + "/" + reader.getRefCount();
      });
      probeClosed(c, s, "closed", seg);
      rec(c, s, "closed.coreHelper.add", () -> { core.addClosedListener(k -> {}); return "added"; });
      rec(c, s, "closed.readerHelper.add", () -> { mine.addClosedListener(k -> {}); return "added"; });
      // registerParentReader calls ensureOpen() on the closed child
      rec(c, s, "closed.multiReader", () -> new org.apache.lucene.index.MultiReader(seg).maxDoc());
      rec(c, s, "closed.filterLeaf", () -> coreFilter(seg).maxDoc());
      rec(c, s, "closed.parallelLeaf", () -> new org.apache.lucene.index.ParallelLeafReader(seg).maxDoc());
    }
    // StandardDirectoryReader is a parent of its segment readers
    {
      DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
      LeafReader seg = reader.leaves().get(0).reader();
      String s = "core_directory";
      rec(c, s, "child_close", () -> {
        seg.close();
        return seg.getRefCount() + "/" + reader.getRefCount();
      });
      rec(c, s, "leaves", () -> reader.leaves().size());
      rec(c, s, "docFreq", () -> reader.docFreq(new org.apache.lucene.index.Term("body", "the")));
      rec(c, s, "storedFields", () -> { reader.storedFields(); return "storedFields"; });
      rec(c, s, "termVectors", () -> { reader.termVectors(); return "termVectors"; });
      rec(c, s, "numDocs", () -> reader.numDocs() + "/" + reader.maxDoc());
      rec(c, s, "close", () -> { reader.close(); return "closed"; });
    }
    // MultiReader and ParallelCompositeReader are parents of their sub-readers, transitively
    for (String name : new String[] {"multi", "parallel_composite"}) {
      DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
      IndexReader composite =
          name.equals("multi")
              ? new org.apache.lucene.index.MultiReader(reader)
              : new org.apache.lucene.index.ParallelCompositeReader(reader);
      String s = "core_" + name;
      rec(c, s, "child_close", () -> {
        reader.close();
        return composite.getRefCount() + "/" + reader.getRefCount();
      });
      rec(c, s, "leaves", () -> composite.leaves().size());
      rec(c, s, "docFreq", () -> composite.docFreq(new org.apache.lucene.index.Term("body", "the")));
      rec(c, s, "storedFields", () -> { composite.storedFields(); return "storedFields"; });
      rec(c, s, "close", () -> { composite.close(); return composite.getRefCount() + "/" + reader.getRefCount(); });
    }
    // FilterLeafReader, ParallelLeafReader (parents of the wrapped reader, close it when closed),
    // FilterCodecReader (calls its doClose) and SlowCodecReaderWrapper (does nothing on close)
    String[] names = {"filter_leaf", "parallel_leaf", "filter_codec", "slow_codec"};
    for (String name : names) {
      for (String what : new String[] {"inner_closed", "wrapper_closed"}) {
        DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
        LeafReader in = reader.leaves().get(0).reader();
        LeafReader w =
            switch (name) {
              case "filter_leaf" -> coreFilter(in);
              case "parallel_leaf" -> new org.apache.lucene.index.ParallelLeafReader(in);
              case "filter_codec" -> coreCodecFilter((CodecReader) in);
              default -> org.apache.lucene.index.SlowCodecReaderWrapper.wrap(coreFilter(in));
            };
        String s = "core_" + name;
        rec(c, s, what + ".close", () -> {
          if (what.equals("inner_closed")) {
            reader.close();
          } else {
            w.close();
          }
          return w.getRefCount() + "/" + in.getRefCount();
        });
        probeClosed(c, s, what, w);
        if (what.equals("wrapper_closed")) {
          rec(c, s, what + ".reader_close", () -> { reader.close(); return "closed"; });
        }
      }
    }
    // FilterDirectoryReader (ExitableDirectoryReader): a parent of its wrapped leaves, closes the
    // wrapped reader when closed
    for (String what : new String[] {"inner_closed", "wrapper_closed"}) {
      DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
      DirectoryReader w = org.apache.lucene.index.ExitableDirectoryReader.wrap(reader, () -> false);
      String s = "core_filter_directory";
      rec(c, s, what + ".close", () -> {
        if (what.equals("inner_closed")) {
          reader.close();
        } else {
          w.close();
        }
        return w.getRefCount() + "/" + reader.getRefCount();
      });
      rec(c, s, what + ".leaves", () -> w.leaves().size());
      rec(c, s, what + ".docFreq", () -> w.docFreq(new org.apache.lucene.index.Term("body", "the")));
      rec(c, s, what + ".storedFields", () -> { w.storedFields(); return "storedFields"; });
      rec(c, s, what + ".numDocs", () -> w.numDocs() + "/" + w.maxDoc());
      rec(c, s, what + ".version", () -> w.getVersion() > 0);
    }
    coreRound3(c, corpus);
  }

  /**
   * The last core items of rmp #344 (round 3): BaseCompositeReader.readerIndex bounds the
   * document id by maxDoc in a DirectoryReader's stored fields and term vectors; a
   * ParallelCompositeReader over more than IndexWriter.getActualMaxDocs() documents is refused
   * as BaseCompositeReader's constructor refuses a MultiReader; ParallelLeafReader closes or
   * dereferences its sub-readers with IndexReader.decRef, which notifies their reader-closed
   * listeners.
   */
  static void coreRound3(String c, File corpus) throws Exception {
    {
      DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
      String s = "core_directory_bounds";
      int max = reader.maxDoc();
      StoredFieldVisitor visitor = new org.apache.lucene.document.DocumentStoredFieldVisitor();
      rec(c, s, "storedFields.last", () -> { reader.storedFields().document(max - 1, visitor); return "doc"; });
      rec(c, s, "storedFields.maxDoc", () -> { reader.storedFields().document(max, visitor); return "doc"; });
      rec(c, s, "storedFields.minus1", () -> { reader.storedFields().document(-1, visitor); return "doc"; });
      rec(c, s, "storedFields.prefetch.maxDoc", () -> { reader.storedFields().prefetch(max); return "prefetch"; });
      rec(c, s, "termVectors.last", () -> reader.termVectors().get(max - 1) == null ? "null" : "fields");
      rec(c, s, "termVectors.maxDoc", () -> { reader.termVectors().get(max); return "fields"; });
      rec(c, s, "termVectors.minus1", () -> { reader.termVectors().get(-1); return "fields"; });
      rec(c, s, "termVectors.prefetch.maxDoc", () -> { reader.termVectors().prefetch(max); return "prefetch"; });
      reader.close();
    }
    {
      DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
      String s = "core_max_docs";
      // IndexWriter.setMaxDocs is package-private (tests lower the limit through it)
      java.lang.reflect.Method setMaxDocs =
          IndexWriter.class.getDeclaredMethod("setMaxDocs", int.class);
      setMaxDocs.setAccessible(true);
      setMaxDocs.invoke(null, reader.maxDoc() - 1);
      try {
        rec(c, s, "parallel_composite", () -> new org.apache.lucene.index.ParallelCompositeReader(false, reader).maxDoc());
        rec(c, s, "multi", () -> new org.apache.lucene.index.MultiReader(new IndexReader[] {reader}, false).maxDoc());
      } finally {
        setMaxDocs.invoke(null, IndexWriter.MAX_DOCS);
      }
      rec(c, s, "refCount", () -> reader.getRefCount());
      rec(c, s, "close", () -> { reader.close(); return reader.getRefCount(); });
    }
    for (boolean closeSubs : new boolean[] {true, false}) {
      DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
      LeafReader seg = reader.leaves().get(0).reader();
      AtomicInteger readerClosed = new AtomicInteger();
      AtomicInteger coreClosed = new AtomicInteger();
      seg.getReaderCacheHelper().addClosedListener(k -> readerClosed.incrementAndGet());
      seg.getCoreCacheHelper().addClosedListener(k -> coreClosed.incrementAndGet());
      LeafReader w = new org.apache.lucene.index.ParallelLeafReader(closeSubs, seg);
      String s = "core_parallel_leaf_listeners_" + (closeSubs ? "close" : "decref");
      Callable<Object> state =
          () -> readerClosed.get() + "/" + coreClosed.get() + "/" + seg.getRefCount();
      rec(c, s, "wrapper_close", () -> { w.close(); return state.call(); });
      rec(c, s, "reader_close", () -> { reader.close(); return state.call(); });
    }
    // the wrapper's decRef is the sub-reader's last: it closes the sub-reader
    {
      DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
      LeafReader seg = reader.leaves().get(0).reader();
      AtomicInteger readerClosed = new AtomicInteger();
      AtomicInteger coreClosed = new AtomicInteger();
      seg.getReaderCacheHelper().addClosedListener(k -> readerClosed.incrementAndGet());
      seg.getCoreCacheHelper().addClosedListener(k -> coreClosed.incrementAndGet());
      LeafReader w = new org.apache.lucene.index.ParallelLeafReader(false, seg);
      String s = "core_parallel_leaf_listeners_decref_last";
      Callable<Object> state =
          () -> readerClosed.get() + "/" + coreClosed.get() + "/" + seg.getRefCount();
      rec(c, s, "reader_close", () -> { reader.close(); return state.call(); });
      rec(c, s, "wrapper_close", () -> { w.close(); return state.call(); });
    }
  }

  // -------------------------------------------------------------------------------------------
  // PointsStackTracker
  // -------------------------------------------------------------------------------------------

  static Directory pointsIndex() throws Exception {
    Directory dir = new ByteBuffersDirectory();
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setMergeScheduler(new SerialMergeScheduler());
    try (IndexWriter iw = new IndexWriter(dir, config)) {
      for (int i = 0; i < 5000; i++) {
        Document d = new Document();
        d.add(new IntPoint("p", (i * 7919) % 10007, (i * 104729) % 10009));
        d.add(new LongPoint("q", (i * 2654435761L) % 1000003L - 500000L));
        iw.addDocument(d);
      }
      iw.forceMerge(1);
      iw.commit();
    }
    return dir;
  }

  /**
   * PerThreadPKLookup over two segments holding the same ids: it looks in the larger segment
   * first, so it answers that segment's document.
   */
  static void pkDuplicates() throws Exception {
    Directory dir = new ByteBuffersDirectory();
    IndexWriterConfig config = new IndexWriterConfig(new StandardAnalyzer());
    config.setMergeScheduler(new SerialMergeScheduler());
    try (IndexWriter iw = new IndexWriter(dir, config)) {
      for (String[] batch : new String[][] {{"a", "b"}, {"a", "c", "d", "b"}}) {
        for (String id : batch) {
          Document d = new Document();
          d.add(new StringField("id", id, Field.Store.YES));
          iw.addDocument(d);
        }
        iw.commit();
      }
    }
    try (DirectoryReader reader = DirectoryReader.open(dir)) {
      PerThreadPKLookup pk = new PerThreadPKLookup(reader, "id");
      StringBuilder sb = new StringBuilder(reader.leaves().size() + ":");
      for (String id : new String[] {"a", "b", "c", "d", "e"}) {
        sb.append(' ').append(pk.lookup(new BytesRef(id)));
      }
      row("pkdup", "pklookup", "lookups", "ok\t" + sb);
    }
  }

  static String hex(byte[] b) {
    StringBuilder sb = new StringBuilder();
    for (byte x : b) sb.append(String.format("%02x", x & 0xff));
    return sb.toString();
  }

  static void tracker(LeafReader leaf, String field, int numDims, int bytesPerDim) throws Exception {
    PointsStackTracker tracker = new PointsStackTracker(numDims, bytesPerDim);
    List<String> sizes = new ArrayList<>();
    leaf.getPointValues(field)
        .intersect(
            new PointValues.IntersectVisitor() {
              @Override
              public void visit(int docID) {}

              @Override
              public void visit(int docID, byte[] packedValue) {}

              @Override
              public PointValues.Relation compare(byte[] min, byte[] max) {
                tracker.onCompare(min, max);
                sizes.add(Integer.toString(tracker.stack.size()));
                return PointValues.Relation.CELL_CROSSES_QUERY;
              }
            });
    row("points", "tracker", field + ".compares", "ok\t" + digest(sizes) + " " + String.join(",", sizes));
    StringBuilder sb = new StringBuilder();
    for (PointsStackTracker.Cell cell : tracker.stack) {
      sb.append(hex(cell.minPackedValue)).append('-').append(hex(cell.maxPackedValue)).append(' ');
    }
    row("points", "tracker", field + ".stack", "ok\t" + sb);
    // contains, on the last two cells
    int n = tracker.stack.size();
    if (n >= 2) {
      row(
          "points",
          "tracker",
          field + ".contains",
          "ok\t"
              + tracker.stack.get(n - 2).contains(tracker.stack.get(n - 1))
              + " "
              + tracker.stack.get(n - 1).contains(tracker.stack.get(n - 2)));
    }
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    File corpus = new File(out, "mdw_corpus.tsv");
    try (PrintWriter pw = new PrintWriter(new File(out, "iw_wrappers.tsv"), "UTF-8")) {
      w = pw;
      w.println("# case\tscenario\tstep\toutcome (ok\\tvalue | err\\tclass\\tmessage)");
      for (String c : new String[] {"nocfs", "cfs"}) {
        DirectoryReader reader = DirectoryReader.open(loadCorpus(corpus, c));
        wrappers(c, reader);
        misuse(c, reader);
        ownCacheKey(c, reader);
        reader.close();
        reopen(c, corpus);
      }
      try (Directory dir = pointsIndex();
          DirectoryReader reader = DirectoryReader.open(dir)) {
        LeafReader leaf = reader.leaves().get(0).reader();
        tracker(leaf, "p", 2, 4);
        tracker(leaf, "q", 1, 8);
        // a segment without doc values: getDocValuesSkipper dereferences the missing producer
        MergeReaderWrapper mrw = new MergeReaderWrapper((CodecReader) leaf);
        rec("points", "mergewrapper", "skipper", () -> mrw.getDocValuesSkipper("p"));
        rec("points", "mergewrapper", "numeric", () -> mrw.getNumericDocValues("p"));
        rec("points", "mergewrapper", "norms", () -> mrw.getNormValues("p"));
      }
      pkDuplicates();
    }
  }
}
