package org.apache.lucene.tests.store;

import com.carrotsearch.randomizedtesting.OracleSupport;
import java.io.File;
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Random;
import java.util.TreeSet;
import java.util.concurrent.atomic.AtomicInteger;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.codecs.Codec;
import org.apache.lucene.codecs.simpletext.SimpleTextOracle;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.StringField;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.CheckIndex;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.index.NoMergePolicy;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.ChecksumIndexInput;
import org.apache.lucene.store.DataAccessHint;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.FileTypeHint;
import org.apache.lucene.store.IOContext;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.tests.util.LuceneTestCase;
import org.apache.lucene.tests.util.TestUtil;

/**
 * The oracle of {@code org.apache.lucene.tests.store} (rmp #343), for {@code
 * crates/rucene-test-framework/tests/mock_directory_wrapper_oracle.rs}. Writes into {@code
 * <out>/rucene-test-framework/}:
 *
 * <ul>
 *   <li>{@code mdw_index.tsv}: the files (name, hex bytes) of a small index written with
 *       Lucene 10.5.0's default codec (5 documents, an indexed stored {@code StringField} and a
 *       {@code TextField}, no compound files, no doc values), which both sides load into a {@code
 *       ByteBuffersDirectory}.
 *   <li>{@code mdw_corpus.tsv}: the files (case, name, hex bytes) of the corpus of {@code
 *       SimpleTextOracle.writeCorpus} written with Lucene 10.5.0's default codec, without ({@code
 *       nocfs}) and with ({@code cfs}) compound files: every kind of data (postings with
 *       positions, offsets and payloads, stored fields, term vectors, norms, every doc-values
 *       type with updates, points, float and byte vectors, deletions), several segments.
 *   <li>{@code mdw_steps.tsv}: per scenario, seed and step, the call and its outcome ({@code
 *       ok<TAB>value}, or {@code err<TAB>class<TAB>message}, with {@code <TAB>cause=class: message}
 *       for a {@code RuntimeException} with a cause), identity hashes masked as {@code @HASH}.
 *       Scenarios: random {@code IOException}s on write ({@code write_io}) and on open ({@code
 *       open_io}); open-file tracking and the close-time checks ({@code tracking}); {@code
 *       Failure}s counting the failure points of each call and firing on a stack frame ({@code
 *       failure}); disk full ({@code disk_full}); {@code crash()} of synthetic unsynced files
 *       ({@code crash}); {@code crash()}, {@code checkIndexOnClose}, {@code corruptUnknownFiles}
 *       and the unreferenced-files check over the index ({@code index}); {@code
 *       SerialIOCountingDirectory} counts on a fixed read pattern ({@code serial_io}); {@code
 *       CorruptingIndexOutput} ({@code corrupting}); the slow open/closers ({@code slow}); {@code
 *       RawDirectoryWrapper} ({@code raw}); throttling ({@code throttling}); the corpus opened and
 *       closed through a {@code DirectoryReader}, read by {@code CheckIndex} at {@code
 *       MIN_LEVEL_FOR_SLOW_CHECKS} through the wrapper, then the wrapper closed ({@code
 *       index_read_nocfs}, {@code index_read_cfs}), which fails if a reader left an input
 *       open. Each scenario that
 *       draws ends with the wrapper's next {@code randomState.nextLong()}, which pins the number
 *       of draws.
 * </ul>
 */
public final class MockDirectoryWrapperOracle {
  private MockDirectoryWrapperOracle() {}

  static PrintWriter w;
  static String scenario;
  static long seed;
  static int step;

  static String m(Object o) {
    String s = String.valueOf(o);
    return s.replaceAll("@[0-9a-f]+", "@HASH").replace("\n", "\\n").replace("\t", "\\t");
  }

  static String err(Throwable t) {
    String s =
        t.getClass().getName() + "\t" + (t.getMessage() == null ? "<null>" : m(t.getMessage()));
    if (t.getClass() == RuntimeException.class && t.getCause() != null) {
      Throwable c = t.getCause();
      s += "\tcause=" + c.getClass().getName() + ": " + m(c.getMessage());
    }
    return s;
  }

  interface Op {
    Object run() throws Throwable;
  }

  /** Records one step; returns the value, or null when it threw. */
  static Object rec(String call, Op op) {
    String outcome;
    Object v = null;
    try {
      v = op.run();
      outcome = "ok\t" + (v instanceof IndexOutput || v instanceof IndexInput ? "" : m(v));
    } catch (Throwable t) {
      outcome = "err\t" + err(t);
      v = null;
    }
    w.println(scenario + "\t" + seed + "\t" + (step++) + "\t" + call + "\t" + outcome);
    return v;
  }

  static void begin(String s, long sd) {
    scenario = s;
    seed = sd;
    step = 0;
  }

  static byte[] bytes(int n, int k) {
    byte[] b = new byte[n];
    for (int i = 0; i < n; i++) {
      b[i] = (byte) (i * 31 + k * 7);
    }
    return b;
  }

  static long fnv(byte[] b) {
    long h = 0xcbf29ce484222325L;
    for (byte x : b) {
      h ^= (x & 0xff);
      h *= 0x100000001b3L;
    }
    return h;
  }

  static byte[] read(Directory dir, String name) throws Exception {
    try (IndexInput in = dir.openInput(name, IOContext.READONCE)) {
      byte[] b = new byte[(int) in.length()];
      in.readBytes(b, 0, b.length);
      return b;
    }
  }

  /** {@code name:length:fnv} of every file of {@code dir}, sorted. */
  static String digest(Directory dir) throws Exception {
    List<String> parts = new ArrayList<>();
    for (String name : dir.listAll()) {
      byte[] b = read(dir, name);
      parts.add(name + ":" + b.length + ":" + String.format("%016x", fnv(b)));
    }
    return String.join(",", parts);
  }

  static String hex(byte[] b) {
    StringBuilder sb = new StringBuilder();
    for (byte x : b) sb.append(String.format("%02x", x & 0xff));
    return sb.toString();
  }

  static MockDirectoryWrapper mdw(long seed) {
    return new MockDirectoryWrapper(new Random(seed), new ByteBuffersDirectory());
  }

  static void nextLong(MockDirectoryWrapper dir) {
    rec("nextLong", () -> dir.randomState.nextLong());
  }

  // ---------------------------------------------------------------------------

  static void writeIo(long sd) throws Exception {
    begin("write_io", sd);
    MockDirectoryWrapper dir = mdw(sd);
    dir.setCheckIndexOnClose(false);
    dir.setRandomIOExceptionRate(0.4);
    for (int i = 0; i < 10; i++) {
      final int k = i;
      String name = "w" + i;
      IndexOutput out = (IndexOutput) rec("createOutput " + name, () -> dir.createOutput(name, IOContext.DEFAULT));
      if (out == null) continue;
      rec("writeInt", () -> { out.writeInt(k); return null; });
      rec("writeBytes 300", () -> { out.writeBytes(bytes(300, k), 0, 300); return null; });
      rec("writeString", () -> { out.writeString("abc" + k); return null; });
      rec("writeVLong", () -> { out.writeVLong(123456789L * k); return null; });
      rec("getFilePointer", out::getFilePointer);
      rec("close", () -> { out.close(); return null; });
    }
    rec("files", () -> digest(dir.getDelegate()));
    nextLong(dir);
    rec("close dir", () -> { dir.close(); return null; });
  }

  static void openIo(long sd) throws Exception {
    begin("open_io", sd);
    MockDirectoryWrapper dir = mdw(sd);
    dir.setCheckIndexOnClose(false);
    for (int i = 0; i < 5; i++) {
      try (IndexOutput out = dir.getDelegate().createOutput("r" + i, IOContext.DEFAULT)) {
        out.writeBytes(bytes(10 + i, i), 0, 10 + i);
      }
    }
    dir.setRandomIOExceptionRateOnOpen(0.5);
    for (int i = 0; i < 16; i++) {
      if (i == 10) {
        dir.setAllowRandomFileNotFoundException(false);
      }
      String name = i % 3 == 0 ? "missing" + i : "r" + (i % 5);
      IndexInput in = (IndexInput) rec("openInput " + name, () -> dir.openInput(name, IOContext.DEFAULT));
      if (in != null) {
        rec("readByte", in::readByte);
        rec("length", in::length);
        rec("close", () -> { in.close(); return null; });
      }
      String c = "c" + i;
      IndexOutput out = (IndexOutput) rec("createOutput " + c, () -> dir.createOutput(c, IOContext.DEFAULT));
      if (out != null) rec("close", () -> { out.close(); return null; });
      IndexOutput tmp = (IndexOutput) rec("createTempOutput", () -> dir.createTempOutput("p", "s", IOContext.DEFAULT));
      if (tmp != null) {
        rec("getName", tmp::getName);
        rec("close", () -> { tmp.close(); return null; });
      }
    }
    nextLong(dir);
    rec("close dir", () -> { dir.close(); return null; });
  }

  static void tracking() throws Exception {
    // An unclosed output.
    begin("tracking", 1);
    MockDirectoryWrapper d1 = mdw(1);
    IndexOutput o1 = d1.createOutput("a", IOContext.DEFAULT);
    o1.writeBytes(bytes(10, 1), 0, 10);
    rec("getFileHandleCount", d1::getFileHandleCount);
    rec("openInput a (open for write)", () -> d1.openInput("a", IOContext.DEFAULT));
    rec("createOutput a (again)", () -> d1.createOutput("a", IOContext.DEFAULT));
    rec("toString", d1::toString);
    rec("isOpen", d1::isOpen);
    rec("close dir", () -> { d1.close(); return null; });
    rec("isOpen", d1::isOpen);
    nextLong(d1);

    // An unclosed input.
    begin("tracking", 2);
    MockDirectoryWrapper d2 = mdw(2);
    IndexOutput o2 = d2.createOutput("b", IOContext.DEFAULT);
    o2.writeBytes(bytes(10, 2), 0, 10);
    o2.close();
    IndexInput i2 = d2.openInput("b", IOContext.DEFAULT);
    rec("getFileHandleCount", d2::getFileHandleCount);
    rec("close dir", () -> { d2.close(); return null; });
    nextLong(d2);

    // Several open files: the map in the message (the cause, in identity order in Java, is not
    // recorded: the first handle's trace there is not reproducible).
    begin("tracking", 3);
    MockDirectoryWrapper d3 = mdw(3);
    List<Object> keep = new ArrayList<>();
    for (String n : new String[] {"zz", "a1", "_0.cfs", "b"}) {
      IndexOutput o = d3.createOutput(n, IOContext.DEFAULT);
      o.writeByte((byte) 1);
      o.close();
      keep.add(d3.openInput(n, IOContext.DEFAULT));
    }
    keep.add(d3.openInput("b", IOContext.DEFAULT));
    try {
      d3.close();
      w.println("tracking\t3\t0\tclose dir\tok\tnull");
    } catch (RuntimeException e) {
      w.println("tracking\t3\t0\tclose dir\terr\t" + e.getClass().getName() + "\t" + m(e.getMessage()));
    }

    // Deleting, renaming and overwriting open files; abusing closed streams.
    begin("tracking", 4);
    MockDirectoryWrapper d = mdw(4);
    d.setCheckIndexOnClose(false);
    try (IndexOutput o = d.getDelegate().createOutput("x", IOContext.DEFAULT)) {
      o.writeBytes(bytes(20, 4), 0, 20);
    }
    try (IndexOutput o = d.getDelegate().createOutput("segments_1", IOContext.DEFAULT)) {
      o.writeBytes(bytes(20, 5), 0, 20);
    }
    d.setAssertNoDeleteOpenFile(true);
    IndexInput x = (IndexInput) rec("openInput x", () -> d.openInput("x", IOContext.DEFAULT));
    rec("deleteFile x (open)", () -> { d.deleteFile("x"); return null; });
    rec("rename x y (source open)", () -> { d.rename("x", "y"); return null; });
    rec("rename z x (dest open)", () -> { d.rename("z", "x"); return null; });
    rec("createOutput x (open)", () -> d.createOutput("x", IOContext.DEFAULT));
    rec("getOpenDeletedFiles", () -> new TreeSet<>(d.getOpenDeletedFiles()));
    rec("close x", () -> { x.close(); return null; });
    d.setAssertNoDeleteOpenFile(false);
    IndexInput x2 = (IndexInput) rec("openInput x", () -> d.openInput("x", IOContext.DEFAULT));
    IndexInput c1 = (IndexInput) rec("clone", () -> x2.clone());
    IndexInput s1 = (IndexInput) rec("slice", () -> x2.slice("sl", 2, 5));
    rec("slice readByte", s1::readByte);
    rec("slice toString", s1::toString);
    rec("getInputCloneCount", d::getInputCloneCount);
    rec("toString", x2::toString);
    rec("deleteFile x (open, allowed)", () -> { d.deleteFile("x"); return null; });
    rec("getOpenDeletedFiles", () -> new TreeSet<>(d.getOpenDeletedFiles()));
    rec("readByte", x2::readByte);
    rec("close x", () -> { x2.close(); return null; });
    rec("readByte (closed)", x2::readByte);
    rec("clone (closed)", () -> x2.clone());
    rec("clone readByte (parent closed)", c1::readByte);
    rec("clone close", () -> { c1.close(); return null; });
    rec("close x again", () -> { x2.close(); return null; });
    rec("openInput nope", () -> d.openInput("nope", IOContext.DEFAULT));
    rec("openInput segments_1 READONCE", () -> { d.openInput("segments_1", IOContext.READONCE).close(); return null; });
    for (int i = 0; i < 4; i++) {
      rec("openInput segments_1 DEFAULT", () -> d.openInput("segments_1", IOContext.DEFAULT));
    }
    IndexOutput q = (IndexOutput) rec("createOutput q", () -> d.createOutput("q", IOContext.DEFAULT));
    rec("write q", () -> { q.writeBytes(bytes(5, 6), 0, 5); return null; });
    rec("close q", () -> { q.close(); return null; });
    rec("writeByte q (closed)", () -> { q.writeByte((byte) 1); return null; });
    rec("close q again", () -> { q.close(); return null; });
    rec("getFileHandleCount", d::getFileHandleCount);
    // crash() with an output and an input open.
    IndexOutput r = (IndexOutput) rec("createOutput r", () -> d.createOutput("r", IOContext.DEFAULT));
    rec("write r", () -> { r.writeBytes(bytes(40, 7), 0, 40); return null; });
    IndexInput qi = (IndexInput) rec("openInput q", () -> d.openInput("q", IOContext.DEFAULT));
    rec("getFileHandleCount", d::getFileHandleCount);
    rec("crash", () -> { d.crash(); return null; });
    rec("getFileHandleCount", d::getFileHandleCount);
    rec("files", () -> digest(d.getDelegate()));
    rec("writeByte r (crashed)", () -> { r.writeByte((byte) 1); return null; });
    rec("close r", () -> { r.close(); return null; });
    rec("readByte q (crashed)", qi::readByte);
    rec("close q", () -> { qi.close(); return null; });
    rec("createOutput after crash", () -> d.createOutput("t", IOContext.DEFAULT));
    rec("createTempOutput after crash", () -> d.createTempOutput("t", "u", IOContext.DEFAULT));
    rec("deleteFile after crash", () -> { d.deleteFile("q"); return null; });
    rec("rename after crash", () -> { d.rename("q", "q2"); return null; });
    rec("sync after crash", () -> { d.sync(List.of("q")); return null; });
    rec("syncMetaData after crash", () -> { d.syncMetaData(); return null; });
    rec("openInput after crash", () -> { d.openInput("q", IOContext.READONCE).close(); return null; });
    rec("clearCrash", () -> { d.clearCrash(); return null; });
    rec("createOutput after clearCrash", () -> { d.createOutput("t", IOContext.DEFAULT).close(); return null; });
    rec("listAll", () -> Arrays.toString(d.listAll()));
    rec("fileLength t", () -> d.fileLength("t"));
    rec("sizeInBytes", d::sizeInBytes);
    nextLong(d);
    rec("close dir", () -> { d.close(); return null; });
  }

  static final class CountingFailure extends MockDirectoryWrapper.Failure {
    final AtomicInteger count = new AtomicInteger();

    @Override
    public void eval(MockDirectoryWrapper dir) {
      count.incrementAndGet();
    }
  }

  static void failure() throws Exception {
    begin("failure", 7);
    MockDirectoryWrapper d = mdw(7);
    d.setCheckIndexOnClose(false);
    CountingFailure counting = new CountingFailure();
    d.failOn(counting);
    // The number of failure points each call passes.
    String[] names = new String[1];
    Op[] ops = {
      () -> { IndexOutput o = d.createOutput("f", IOContext.DEFAULT); o.writeInt(1); o.writeBytes(bytes(100, 1), 0, 100); o.copyBytes(new org.apache.lucene.store.ByteArrayDataInput(bytes(30, 2)), 30); o.close(); return null; },
      () -> { IndexInput i = d.openInput("f", IOContext.DEFAULT); i.readByte(); IndexInput c = i.clone(); c.close(); i.close(); return null; },
      () -> { d.sync(List.of("f")); return null; },
      () -> { d.syncMetaData(); return null; },
      () -> { d.rename("f", "g"); return null; },
      () -> { d.listAll(); d.fileLength("g"); d.sizeInBytes(); return null; },
      () -> { IndexOutput o = d.createTempOutput("p", "s", IOContext.DEFAULT); names[0] = o.getName(); o.close(); return null; },
      () -> { d.deleteFile(names[0]); return null; },
      () -> { d.setFailOnCreateOutput(false); d.createOutput("h", IOContext.DEFAULT).close(); return null; },
      () -> { d.setFailOnOpenInput(false); d.openInput("h", IOContext.DEFAULT).close(); return null; },
    };
    for (int k = 0; k < ops.length; k++) {
      int before = counting.count.get();
      Op op = ops[k];
      rec("op" + k, () -> { op.run(); return counting.count.get() - before; });
    }
    // A failure that fires on a stack frame: deleteFile.
    MockDirectoryWrapper.Failure onDelete =
        new MockDirectoryWrapper.Failure() {
          @Override
          public void eval(MockDirectoryWrapper dir) throws java.io.IOException {
            if (doFail && LuceneTestCase.callStackContainsAnyOf("deleteFile")) {
              throw new MockDirectoryWrapper.FakeIOException();
            }
          }
        };
    d.failOn(onDelete);
    rec("deleteFile g (no doFail)", () -> { d.getDelegate().copyFrom(d.getDelegate(), "g", "g2", IOContext.DEFAULT); d.deleteFile("g"); return null; });
    onDelete.setDoFail();
    rec("deleteFile g2 (doFail)", () -> { d.deleteFile("g2"); return null; });
    rec("createOutput k (doFail)", () -> { IndexOutput o = d.createOutput("k", IOContext.DEFAULT); o.writeByte((byte) 3); o.close(); return null; });
    rec("rename k k2 (doFail)", () -> { d.rename("k", "k2"); return null; });
    onDelete.clearDoFail();
    rec("deleteFile g2 (cleared)", () -> { d.deleteFile("g2"); return null; });
    rec("listAll", () -> Arrays.toString(d.listAll()));
    nextLong(d);
    rec("close dir", () -> { d.close(); return null; });
  }

  static void diskFull() throws Exception {
    begin("disk_full", 11);
    MockDirectoryWrapper d = mdw(11);
    d.setCheckIndexOnClose(false);
    d.setMaxSizeInBytes(100);
    IndexOutput x = d.createOutput("x", IOContext.DEFAULT);
    rec("x writeBytes 60", () -> { x.writeBytes(bytes(60, 1), 0, 60); return null; });
    rec("x writeBytes 60", () -> { x.writeBytes(bytes(60, 2), 0, 60); return null; });
    rec("x close", () -> { x.close(); return null; });
    rec("sizeInBytes", d::sizeInBytes);
    IndexOutput y = d.createOutput("y", IOContext.DEFAULT);
    rec("y writeBytes 60", () -> { y.writeBytes(bytes(60, 3), 0, 60); return null; });
    rec("y close", () -> { y.close(); return null; });
    rec("getMaxUsedSizeInBytes", d::getMaxUsedSizeInBytes);
    d.setMaxSizeInBytes(150);
    IndexOutput z = d.createOutput("z", IOContext.DEFAULT);
    rec("z writeBytes 60", () -> { z.writeBytes(bytes(60, 4), 0, 60); return null; });
    rec("z close", () -> { z.close(); return null; });
    rec("sizeInBytes", d::sizeInBytes);
    rec("getMaxUsedSizeInBytes", d::getMaxUsedSizeInBytes);
    d.setMaxSizeInBytes(160);
    IndexOutput v = d.createOutput("v", IOContext.DEFAULT);
    rec("v copyBytes 50", () -> {
      try (IndexInput in = d.getDelegate().openInput("x", IOContext.READONCE)) {
        v.copyBytes(in, 50);
      }
      return null;
    });
    rec("v close", () -> { v.close(); return null; });
    rec("toString", d::toString);
    rec("getMaxSizeInBytes", d::getMaxSizeInBytes);
    d.setMaxSizeInBytes(0);
    d.setTrackDiskUsage(true);
    rec("resetMaxUsedSizeInBytes", () -> { d.resetMaxUsedSizeInBytes(); return d.getMaxUsedSizeInBytes(); });
    IndexOutput u = d.createOutput("u", IOContext.DEFAULT);
    rec("u writeBytes 70", () -> { u.writeBytes(bytes(70, 5), 0, 70); return null; });
    rec("u close", () -> { u.close(); return null; });
    rec("getMaxUsedSizeInBytes", d::getMaxUsedSizeInBytes);
    rec("files", () -> digest(d.getDelegate()));
    nextLong(d);
    rec("close dir", () -> { d.close(); return null; });
  }

  static void crash(long sd, boolean alwaysCorrupt) throws Exception {
    begin(alwaysCorrupt ? "crash_always" : "crash", sd);
    MockDirectoryWrapper d = mdw(sd);
    d.setCheckIndexOnClose(false);
    d.alwaysCorrupt = alwaysCorrupt;
    for (int i = 0; i < 8; i++) {
      final int k = i;
      String name = "_" + i + ".dat";
      rec("write " + name, () -> {
        try (IndexOutput o = d.createOutput(name, IOContext.DEFAULT)) {
          int n = k == 7 ? 0 : 1 + (k * 397) % 1500;
          o.writeBytes(bytes(n, k), 0, n);
        }
        return null;
      });
    }
    rec("sync", () -> { d.sync(List.of("_0.dat", "_3.dat")); return null; });
    rec("crash", () -> { d.crash(); return null; });
    rec("files", () -> digest(d.getDelegate()));
    nextLong(d);
    rec("close dir", () -> { d.close(); return null; });
  }

  static Directory writeIndex() throws Exception {
    Directory dir = new ByteBuffersDirectory();
    IndexWriterConfig iwc = new IndexWriterConfig(new StandardAnalyzer());
    iwc.setUseCompoundFile(false);
    iwc.setMergePolicy(NoMergePolicy.INSTANCE);
    try (IndexWriter iw = new IndexWriter(dir, iwc)) {
      for (int i = 0; i < 5; i++) {
        Document doc = new Document();
        doc.add(new StringField("id", "d" + i, Field.Store.YES));
        doc.add(new TextField("body", "hello world number " + i, Field.Store.NO));
        iw.addDocument(doc);
      }
      iw.commit();
    }
    return dir;
  }

  static MockDirectoryWrapper copyIndex(Directory index, long sd, String[] synced) throws Exception {
    MockDirectoryWrapper d = mdw(sd);
    for (String name : index.listAll()) {
      d.copyFrom(index, name, name, IOContext.DEFAULT);
    }
    d.sync(Arrays.asList(synced));
    return d;
  }

  static String[] synced(Directory index, String... exts) throws Exception {
    List<String> out = new ArrayList<>();
    for (String name : index.listAll()) {
      boolean keep = name.startsWith("segments") || name.endsWith(".si");
      for (String e : exts) keep |= name.endsWith(e);
      if (keep) out.add(name);
    }
    return out.toArray(String[]::new);
  }

  static void index(Directory index, long sd) throws Exception {
    String[] none = synced(index);
    String[] most = synced(index, ".fdt", ".fdx", ".fdm", ".fnm", ".nvd", ".nvm", ".doc", ".tim", ".tip", ".tmd");
    String[] all = index.listAll();

    begin("index_crash", sd);
    MockDirectoryWrapper d = copyIndex(index, sd, none);
    d.setCheckIndexOnClose(false);
    rec("crash", () -> { d.crash(); return null; });
    rec("files", () -> digest(d.getDelegate()));
    nextLong(d);

    begin("index_close", sd);
    MockDirectoryWrapper d2 = copyIndex(index, sd, none);
    rec("close dir", () -> { d2.close(); return null; });
    nextLong(d2);

    begin("index_close_most", sd);
    MockDirectoryWrapper d3 = copyIndex(index, sd, most);
    rec("close dir", () -> { d3.close(); return null; });
    nextLong(d3);

    begin("index_close_clean", sd);
    MockDirectoryWrapper d4 = copyIndex(index, sd, all);
    d4.setCrossCheckTermVectorsOnClose(false);
    rec("getLevelForCheckOnClose", d4::getLevelForCheckOnClose);
    rec("close dir", () -> { d4.close(); return null; });
    nextLong(d4);

    begin("index_unknown", sd);
    MockDirectoryWrapper d5 = copyIndex(index, sd, all);
    d5.setCheckIndexOnClose(false);
    for (String n : new String[] {"_9.fake", "pending_segments_7", "write.lock", "other.txt", "_a_Lucene90_0.dvd"}) {
      try (IndexOutput o = d5.getDelegate().createOutput(n, IOContext.DEFAULT)) {
        o.writeBytes(bytes(33, n.length()), 0, 33);
      }
    }
    rec("corruptUnknownFiles", () -> { d5.corruptUnknownFiles(); return null; });
    rec("files", () -> digest(d5.getDelegate()));
    nextLong(d5);
  }

  static void unreferenced(Directory index) throws Exception {
    begin("index_unref", 0);
    MockDirectoryWrapper d = copyIndex(index, 0, index.listAll());
    d.setCheckIndexOnClose(false);
    d.setAssertNoUnrefencedFilesOnClose(true);
    try (IndexOutput o = d.getDelegate().createOutput("_77.fdt", IOContext.DEFAULT)) {
      o.writeBytes(bytes(12, 1), 0, 12);
    }
    rec("close dir", () -> { d.close(); return null; });

    begin("index_unref", 1);
    MockDirectoryWrapper d2 = copyIndex(index, 1, index.listAll());
    d2.setCheckIndexOnClose(false);
    d2.setAssertNoUnrefencedFilesOnClose(true);
    rec("close dir", () -> { d2.close(); return null; });
  }

  static void serialIo() throws Exception {
    begin("serial_io", 0);
    SerialIOCountingDirectory dir = new SerialIOCountingDirectory(new ByteBuffersDirectory());
    try (IndexOutput o = dir.createOutput("s", IOContext.DEFAULT)) {
      o.writeBytes(bytes(256 * 1024, 9), 0, 256 * 1024);
    }
    IndexInput in = dir.openInput("s", IOContext.DEFAULT);
    rec("toString", in::toString);
    rec("readByte@0", () -> { in.readByte(); return dir.count(); });
    rec("readByte@8192", () -> { in.seek(8192); in.readByte(); return dir.count(); });
    rec("readBytes@40960x100", () -> { in.seek(40960); in.readBytes(new byte[100], 0, 100); return dir.count(); });
    rec("readByte@12288 (backwards)", () -> { in.seek(12288); in.readByte(); return dir.count(); });
    rec("readByte@16384 (read-ahead)", () -> { in.seek(16384); in.readByte(); return dir.count(); });
    rec("readInt@36860 (page boundary)", () -> { in.seek(36860); in.readInt(); return dir.count(); });
    rec("prefetch@20*4096x8192", () -> { in.prefetch(20 * 4096, 8192); return dir.count(); });
    rec("prefetch@40*4096x100", () -> { in.prefetch(40 * 4096, 100); return dir.count(); });
    rec("readByte@20*4096", () -> { in.seek(20 * 4096); in.readByte(); return dir.count(); });
    rec("readByte@40*4096", () -> { in.seek(40 * 4096); in.readByte(); return dir.count(); });
    rec("prefetch@2*4096x10 (window)", () -> { in.prefetch(2 * 4096, 10); return dir.count(); });
    rec("prefetch@50*4096x10", () -> { in.prefetch(50 * 4096, 10); return dir.count(); });
    rec("readVLong@50*4096", () -> { in.seek(50 * 4096); in.readVLong(); return dir.count(); });
    rec("skipBytes 9000", () -> { in.skipBytes(9000); in.readByte(); return dir.count(); });
    IndexInput sl = in.slice("sl", 30 * 4096, 5 * 4096);
    rec("slice readByte@0", () -> { sl.readByte(); return dir.count(); });
    rec("slice readBytes@4096x10", () -> { sl.seek(4096); sl.readBytes(new byte[10], 0, 10); return dir.count(); });
    rec("slice getFilePointer", sl::getFilePointer);
    rec("slice length", sl::length);
    rec("slice of slice", () -> in.slice("bad", 60 * 4096, 10 * 4096));
    IndexInput cl = in.clone();
    rec("clone readByte", () -> { cl.seek(0); cl.readByte(); return dir.count(); });
    IndexInput rnd = dir.openInput("s", IOContext.DEFAULT.withHints(DataAccessHint.RANDOM));
    rec("random readByte@0", () -> { rnd.readByte(); return dir.count(); });
    rec("random readByte@4096", () -> { rnd.seek(4096); rnd.readByte(); return dir.count(); });
    rec("random readByte@100 (same page)", () -> { rnd.seek(100); rnd.readByte(); return dir.count(); });
    rec("random prefetch@4096*3x10", () -> { rnd.prefetch(3 * 4096, 10); return dir.count(); });
    rec("random readByte@3*4096", () -> { rnd.seek(3 * 4096); rnd.readByte(); return dir.count(); });
    IndexInput idx = dir.openInput("s", IOContext.DEFAULT.withHints(FileTypeHint.INDEX));
    rec("index open", dir::count);
    rec("index reads", () -> { idx.seek(50000); idx.readByte(); idx.seek(10); idx.readByte(); return dir.count(); });
    ChecksumIndexInput ck = dir.openChecksumInput("s");
    rec("openChecksumInput", dir::count);
    rec("checksum reads", () -> { ck.readBytes(new byte[20000], 0, 20000); ck.readByte(); return dir.count(); });
    in.close();
    rnd.close();
    idx.close();
    ck.close();
    rec("close dir", () -> { dir.close(); return dir.count(); });
  }

  static void corrupting() throws Exception {
    begin("corrupting", 0);
    Directory dir = new ByteBuffersDirectory();
    CorruptingIndexOutput out = new CorruptingIndexOutput(dir, 5, dir.createOutput("c", IOContext.DEFAULT));
    rec("write 20", () -> { out.writeBytes(bytes(20, 3), 0, 20); return null; });
    rec("getFilePointer", out::getFilePointer);
    rec("getChecksum", out::getChecksum);
    rec("getName", out::getName);
    rec("toString", out::toString);
    rec("close", () -> { out.close(); return null; });
    rec("close again", () -> { out.close(); return null; });
    rec("bytes c", () -> hex(read(dir, "c")));
    rec("listAll", () -> Arrays.toString(dir.listAll()));
    CorruptingIndexOutput out2 = new CorruptingIndexOutput(dir, 10, dir.createOutput("d", IOContext.DEFAULT));
    rec("write 10", () -> { out2.writeBytes(bytes(10, 4), 0, 10); return null; });
    rec("close (byte past end)", () -> { out2.close(); return null; });
    rec("listAll", () -> Arrays.toString(dir.listAll()));

    begin("corrupting", 5);
    MockDirectoryWrapper d = mdw(5);
    d.setCheckIndexOnClose(false);
    CorruptingIndexOutput out3 = new CorruptingIndexOutput(d, 3, d.createOutput("m", IOContext.DEFAULT));
    rec("write 8", () -> { out3.writeBytes(bytes(8, 5), 0, 8); return null; });
    rec("close", () -> { out3.close(); return null; });
    rec("bytes m", () -> hex(read(d.getDelegate(), "m")));
    rec("getFileHandleCount", d::getFileHandleCount);
    rec("listAll", () -> Arrays.toString(d.listAll()));
    nextLong(d);
    rec("close dir", () -> { d.close(); return null; });
  }

  static void slow(long sd) throws Exception {
    begin("slow", sd);
    MockDirectoryWrapper d = mdw(sd);
    d.setCheckIndexOnClose(false);
    d.setUseSlowOpenClosers(true);
    try (IndexOutput o = d.getDelegate().createOutput("s", IOContext.DEFAULT)) {
      o.writeBytes(bytes(10, 1), 0, 10);
    }
    List<String> hits = new ArrayList<>();
    for (int i = 0; i < 400; i++) {
      IndexInput in = d.openInput("s", IOContext.READONCE);
      if (in.getClass() != MockIndexInputWrapper.class) {
        IndexInput c = in.clone();
        hits.add(i + ":" + in.getClass().getSimpleName() + ":" + c.getClass().getSimpleName());
      }
      in.close();
    }
    rec("slow wrappers", () -> String.join(",", hits));
    nextLong(d);
    rec("close dir", () -> { d.close(); return null; });
  }

  static void raw() throws Exception {
    begin("raw", 0);
    Directory src = new ByteBuffersDirectory();
    try (IndexOutput o = src.createOutput("a", IOContext.DEFAULT)) {
      o.writeBytes(bytes(7, 1), 0, 7);
    }
    RawDirectoryWrapper raw = new RawDirectoryWrapper(new ByteBuffersDirectory());
    rec("toString", raw::toString);
    rec("copyFrom", () -> { raw.copyFrom(src, "a", "b", IOContext.DEFAULT); return Arrays.toString(raw.listAll()); });
    rec("openChecksumInput", () -> {
      try (ChecksumIndexInput in = raw.openChecksumInput("b")) {
        in.readBytes(new byte[7], 0, 7);
        return in.getChecksum();
      }
    });
    rec("isOpen", raw::isOpen);
    rec("close", () -> { raw.close(); return raw.isOpen(); });
  }

  static void throttling(long sd) throws Exception {
    begin("throttling", sd);
    MockDirectoryWrapper d = mdw(sd);
    d.setCheckIndexOnClose(false);
    d.setThrottling(MockDirectoryWrapper.Throttling.ALWAYS);
    IndexOutput t = (IndexOutput) rec("createOutput t", () -> d.createOutput("t", IOContext.DEFAULT));
    rec("toString", t::toString);
    rec("getName", t::getName);
    rec("write", () -> { t.writeBytes(bytes(10, 1), 0, 10); return null; });
    rec("close", () -> { t.close(); return null; });
    d.setThrottling(MockDirectoryWrapper.Throttling.SOMETIMES);
    for (int i = 0; i < 5; i++) {
      String n = "s" + i;
      rec("createOutput " + n, () -> { d.createOutput(n, IOContext.DEFAULT).close(); return null; });
    }
    rec("files", () -> digest(d.getDelegate()));
    nextLong(d);
    rec("close dir", () -> { d.close(); return null; });
  }

  static Directory writeCorpus(boolean compound) throws Exception {
    Directory dir = new ByteBuffersDirectory();
    SimpleTextOracle.writeCorpus(dir, compound, Codec.getDefault());
    return dir;
  }

  /**
   * The corpus read in full through the wrapper: a {@code DirectoryReader} opened and closed,
   * {@code CheckIndex} at {@code MIN_LEVEL_FOR_SLOW_CHECKS}, the wrapper closed (refused while an
   * input is open). The wrapper draws on every {@code openInput} (the slow-opener coin), so the
   * final {@code nextLong} also pins how many inputs were opened.
   */
  static void indexRead(Directory corpus, String label, long sd) throws Exception {
    begin("index_read_" + label, sd);
    MockDirectoryWrapper d = copyIndex(corpus, sd, corpus.listAll());
    DirectoryReader[] reader = new DirectoryReader[1];
    rec(
        "open reader",
        () -> {
          reader[0] = DirectoryReader.open(d);
          return reader[0].maxDoc() + "/" + reader[0].numDocs() + "/" + reader[0].leaves().size();
        });
    rec("close reader", () -> { reader[0].close(); return null; });
    rec(
        "checkIndex",
        () -> {
          CheckIndex.Status status =
              TestUtil.checkIndex(d, CheckIndex.Level.MIN_LEVEL_FOR_SLOW_CHECKS, true, false, null);
          return status.clean + "/" + status.numSegments;
        });
    rec("close dir", () -> { d.close(); return null; });
    nextLong(d);
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    long[] seeds = OracleSupport.seeds();
    Directory index = writeIndex();
    try (PrintWriter iw = new PrintWriter(new File(out, "mdw_index.tsv"), "UTF-8")) {
      iw.println("# name\thex bytes: the index MockDirectoryWrapperOracle's index scenarios copy");
      for (String name : index.listAll()) {
        iw.println(name + "\t" + hex(read(index, name)));
      }
    }
    Directory nocfs = writeCorpus(false);
    Directory cfs = writeCorpus(true);
    try (PrintWriter cw = new PrintWriter(new File(out, "mdw_corpus.tsv"), "UTF-8")) {
      cw.println("# case\tname\thex bytes: the corpus MockDirectoryWrapperOracle's index_read scenarios copy");
      for (String name : nocfs.listAll()) {
        cw.println("nocfs\t" + name + "\t" + hex(read(nocfs, name)));
      }
      for (String name : cfs.listAll()) {
        cw.println("cfs\t" + name + "\t" + hex(read(cfs, name)));
      }
    }
    try (PrintWriter pw = new PrintWriter(new File(out, "mdw_steps.tsv"), "UTF-8")) {
      w = pw;
      w.println("# scenario\tseed\tstep\tcall\toutcome (ok\\tvalue | err\\tclass\\tmessage[\\tcause=...])");
      for (int i = 0; i < 8; i++) writeIo(seeds[i]);
      for (int i = 0; i < 8; i++) openIo(seeds[i]);
      tracking();
      failure();
      diskFull();
      for (long s : seeds) crash(s, false);
      for (int i = 0; i < 4; i++) crash(seeds[i], true);
      for (int i = 0; i < 10; i++) index(index, seeds[i]);
      unreferenced(index);
      serialIo();
      corrupting();
      for (int i = 0; i < 4; i++) slow(seeds[i]);
      raw();
      for (int i = 0; i < 3; i++) throttling(seeds[i]);
      for (int i = 0; i < 4; i++) indexRead(nocfs, "nocfs", seeds[i]);
      for (int i = 0; i < 4; i++) indexRead(cfs, "cfs", seeds[i]);
    }
  }
}
