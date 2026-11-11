// Oracle of task #339 (rucene-test-framework/tests/mockfile_oracle.rs): the
// fault models of Lucene 10.5.0's org.apache.lucene.tests.mockfile filesystems
// driven through fixed operation sequences. Writes
// <out>/rucene-test-framework/mockfile.tsv; the temporary directory is masked
// as $D and identity hash codes as @HASH.
package org.apache.lucene.tests.mockfile;

import java.io.*;
import java.net.URI;
import java.nio.ByteBuffer;
import java.nio.channels.*;
import java.nio.file.*;
import java.util.*;
import org.apache.lucene.util.IOUtils;
import org.apache.lucene.util.InfoStream;

public class MockfileOracle {
  static String D;
  static String REL;
  static PrintWriter w;

  static String m(Object o) {
    String s = String.valueOf(o);
    if (REL != null) s = s.replace(REL, "$R");
    s = s.replace(D, "$D");
    return s.replaceAll("@[0-9a-f]+", "@HASH").replace("\n", "\\n").replace("\t", "\\t");
  }

  static String err(Throwable t) {
    String c = t.getCause() == null ? "" : " <- " + t.getCause().getClass().getName() + ": " + m(t.getCause().getMessage());
    return t.getClass().getName() + "\t" + (t.getMessage() == null ? "<null>" : m(t.getMessage())) + c;
  }

  interface Op { Object run() throws Throwable; }

  static void row(String label, Op op) {
    try {
      Object r = op.run();
      w.println(label + "\tOK\t" + (r == null ? "-" : m(r)));
    } catch (Throwable t) {
      w.println(label + "\t" + err(t));
    }
  }

  static Path fresh(String name) throws IOException {
    Path p = Paths.get(D, name);
    Files.createDirectory(p);
    return p;
  }

  static List<String> names(DirectoryStream<Path> s) throws IOException {
    List<String> l = new ArrayList<>();
    for (Path p : s) l.add(p.getFileName().toString());
    s.close();
    return l;
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    Path tmp = Files.createTempDirectory("mockfile-oracle").toRealPath();
    D = tmp.toString();
    w = new PrintWriter(new File(out, "mockfile.tsv"), "UTF-8");
    w.println("# label\tOK\tresult | label\tclass\tmessage[ <- cause]; $D = temp dir");
    FileSystem def = FileSystems.getDefault();

    // LeakFS: every handle kind, leaked and closed.
    for (String kind : new String[] {"InputStream", "OutputStream", "FileChannel", "SeekableByteChannel", "DirectoryStream", "AsynchronousFileChannel"}) {
      Path base = fresh("leak-" + kind);
      Files.write(base.resolve("f"), new byte[] {1});
      FileSystem fs = new LeakFS(def).getFileSystem(URI.create("file:///"));
      Path f = new FilterPath(base.resolve("f"), (FilterFileSystem) fs);
      Path dir = new FilterPath(base, (FilterFileSystem) fs);
      Closeable c;
      switch (kind) {
        case "InputStream": c = Files.newInputStream(f); break;
        case "OutputStream": c = Files.newOutputStream(f); break;
        case "FileChannel": c = FileChannel.open(f, StandardOpenOption.READ); break;
        case "SeekableByteChannel": c = Files.newByteChannel(f); break;
        case "DirectoryStream": c = Files.newDirectoryStream(dir); break;
        default: c = AsynchronousFileChannel.open(f, StandardOpenOption.READ); break;
      }
      row("LeakFS " + kind + " toString", () -> c.toString());
      row("LeakFS " + kind + " leaked close", () -> { fs.close(); return null; });
      c.close();
      c.close();
      row("LeakFS " + kind + " closed close", () -> { fs.close(); return null; });
    }
    row("LeakFS provider toString", () -> new LeakFS(def).toString());
    row("LeakFS scheme", () -> new LeakFS(def).getScheme());

    // HandleLimitFS: the limit, the count after a refused open.
    {
      Path base = fresh("limit");
      Files.write(base.resolve("f"), new byte[] {1});
      HandleLimitFS provider = new HandleLimitFS(def, 2);
      FileSystem fs = provider.getFileSystem(URI.create("file:///"));
      Path f = new FilterPath(base.resolve("f"), (FilterFileSystem) fs);
      InputStream a = Files.newInputStream(f);
      InputStream b = Files.newInputStream(f);
      row("HandleLimitFS count 2", () -> provider.count.get());
      row("HandleLimitFS third", () -> Files.newInputStream(f));
      row("HandleLimitFS count after refusal", () -> provider.count.get());
      a.close();
      row("HandleLimitFS count after close", () -> provider.count.get());
      row("HandleLimitFS reopen", () -> { Files.newInputStream(f).close(); return provider.count.get(); });
      b.close();
      row("HandleLimitFS count end", () -> provider.count.get());
      row("HandleLimitFS MAX_OPEN_FILES", () -> HandleLimitFS.MaxOpenHandles.MAX_OPEN_FILES);
    }

    // WindowsFS: deletes and moves of open files, reserved names.
    {
      Path base = fresh("windows");
      FileSystem fs = new WindowsFS(def).getFileSystem(URI.create("file:///"));
      Path dir = fs.getPath(base.toString());
      Path f = dir.resolve("f");
      Files.write(f, new byte[] {1});
      OutputStream o = Files.newOutputStream(f, StandardOpenOption.APPEND);
      row("WindowsFS delete open", () -> { Files.delete(f); return null; });
      row("WindowsFS deleteIfExists open", () -> Files.deleteIfExists(f));
      row("WindowsFS move open source", () -> Files.move(f, dir.resolve("g")));
      Files.write(dir.resolve("h"), new byte[] {2});
      row("WindowsFS move onto open target", () -> Files.move(dir.resolve("h"), f, StandardCopyOption.REPLACE_EXISTING));
      row("WindowsFS close after transfer", () -> { o.close(); return null; });
      row("WindowsFS delete closed", () -> { Files.delete(f); return Files.exists(f); });
      row("WindowsFS delete missing", () -> { Files.delete(f); return null; });
      row("WindowsFS deleteIfExists missing", () -> Files.deleteIfExists(f));
      for (String name : new String[] {"CON", "aux", "LPT1", "a:b", "a<b", "q?", "x*", "ok-name", "COM10"}) {
        row("WindowsFS resolve " + name, () -> dir.resolve(name).getFileName());
      }
      row("WindowsFS resolve root", () -> dir.resolve(fs.getPath("/")));
      Path g = dir.resolve("g2");
      Files.write(g, new byte[] {1});
      FileChannel ch = FileChannel.open(g, StandardOpenOption.READ);
      row("WindowsFS delete open channel", () -> { Files.delete(g); return null; });
      ch.close();
      row("WindowsFS delete after channel", () -> { Files.delete(g); return null; });
      row("WindowsFS mismatch", () -> dir.resolve(Paths.get("x")));
      FileSystem leak = new LeakFS(def).getFileSystem(URI.create("file:///"));
      row("WindowsFS mismatch filter", () -> dir.resolve(leak.getPath("x")));
    }

    // VirusCheckingFS: fixed salts.
    for (long salt : new long[] {0L, 1L, 42L, -7L, 0x5DEECE66DL}) {
      Path base = fresh("virus" + salt);
      VirusCheckingFS provider = new VirusCheckingFS(def, salt);
      FileSystem fs = provider.getFileSystem(URI.create("file:///"));
      Path dir = new FilterPath(base, (FilterFileSystem) fs);
      StringBuilder sb = new StringBuilder();
      for (int i = 0; i < 20; i++) {
        Path p = dir.resolve(i == 7 ? "write.lock" : ("f" + i));
        Files.write(p, new byte[] {1});
        try { Files.delete(p); sb.append(i).append(":ok "); } catch (AccessDeniedException e) { sb.append(i).append(":denied "); }
      }
      w.println("VirusCheckingFS salt=" + salt + "\tOK\t" + sb.toString().trim());
      row("VirusCheckingFS salt=" + salt + " refusal message", () -> {
        for (int i = 0; i < 40; i++) { Path p = dir.resolve("g" + i); Files.write(p, new byte[] {1}); try { Files.delete(p); } catch (AccessDeniedException e) { return e; } }
        return "none";
      });
      row("VirusCheckingFS salt=" + salt + " missing", () -> { Files.delete(dir.resolve("missing")); return null; });
      provider.disable();
      row("VirusCheckingFS salt=" + salt + " disabled", () -> { int n = 0; for (int i = 0; i < 20; i++) { Path p = dir.resolve("h" + i); Files.write(p, new byte[] {1}); Files.delete(p); n++; } return n + " " + provider.isEnabled(); });
    }

    // ShuffleFS: fixed seeds.
    for (long seed : new long[] {0L, 1L, -1L, 42L, Long.MIN_VALUE, 0xDEADBEEFL}) {
      Path base = fresh("shuffle" + seed);
      for (String n : new String[] {"b", "a", "_0.cfs", "segments_1", "write.lock", "c", "Z", "\u00e9", "10", "2"}) Files.write(base.resolve(n), new byte[] {1});
      FileSystem fs = new ShuffleFS(def, seed).getFileSystem(URI.create("file:///"));
      Path dir = new FilterPath(base, (FilterFileSystem) fs);
      row("ShuffleFS seed=" + seed, () -> names(Files.newDirectoryStream(dir)));
      row("ShuffleFS seed=" + seed + " glob", () -> names(Files.newDirectoryStream(dir, "[a-c]*")));
    }

    // ExtrasFS.
    for (boolean[] mode : new boolean[][] {{true, false}, {true, true}, {false, true}}) {
      Path base = fresh("extras" + mode[0] + mode[1]);
      FileSystem fs = new ExtrasFS(def, mode[0], mode[1]).getFileSystem(URI.create("file:///"));
      Path dir = new FilterPath(base, (FilterFileSystem) fs);
      row("ExtrasFS active=" + mode[0] + " dir=" + mode[1], () -> { Path s = dir.resolve("sub"); Files.createDirectory(s); List<String> l = names(Files.newDirectoryStream(s)); return l + " " + (l.isEmpty() ? "" : Files.isDirectory(s.resolve(l.get(0)))); });
      row("ExtrasFS active=" + mode[0] + " dir=" + mode[1] + " createDirectories", () -> { Path s = dir.resolve("p/q"); Files.createDirectories(s); List<String> l = names(Files.newDirectoryStream(dir.resolve("p"))); Collections.sort(l); return l; });
    }
    row("ExtrasFS isExtra", () -> ExtrasFS.isExtra("extra0") + " " + ExtrasFS.isExtra("extra1"));

    // DisableFsyncFS.
    {
      Path base = fresh("fsync");
      FileSystem fs = new DisableFsyncFS(def).getFileSystem(URI.create("file:///"));
      Path f = new FilterPath(base.resolve("f"), (FilterFileSystem) fs);
      row("DisableFsyncFS force", () -> { try (FileChannel c = FileChannel.open(f, StandardOpenOption.CREATE, StandardOpenOption.WRITE)) { c.write(ByteBuffer.wrap(new byte[] {1, 2})); c.force(true); return c.size(); } });
      row("DisableFsyncFS force closed", () -> { FileChannel c = FileChannel.open(f, StandardOpenOption.WRITE); c.close(); c.force(true); return "no-op"; });
      row("DisableFsyncFS IOUtils.fsync", () -> { IOUtils.fsync(f, false); return null; });
    }

    // VerboseFS.
    {
      Path base = fresh("verbose");
      List<String> log = new ArrayList<>();
      InfoStream is = new InfoStream() {
        @Override public void message(String component, String message) { log.add(component + ": " + message); }
        @Override public boolean isEnabled(String component) { return true; }
        @Override public void close() {}
      };
      FileSystem fs = new VerboseFS(def, is).getFileSystem(URI.create("file:///"));
      Path dir = new FilterPath(base, (FilterFileSystem) fs);
      REL = Paths.get(".").toAbsolutePath().normalize().relativize(base).toString();
      Op[] ops = new Op[] {
        () -> Files.createDirectory(dir.resolve("d")),
        () -> Files.createDirectory(dir.resolve("d")),
        () -> { Files.newOutputStream(dir.resolve("a")).close(); return null; },
        () -> { Files.newOutputStream(dir.resolve("b"), StandardOpenOption.CREATE_NEW, StandardOpenOption.WRITE).close(); return null; },
        () -> { FileChannel.open(dir.resolve("a"), StandardOpenOption.WRITE).close(); return null; },
        () -> { FileChannel.open(dir.resolve("a"), StandardOpenOption.READ).close(); return null; },
        () -> FileChannel.open(dir.resolve("nope"), StandardOpenOption.READ),
        // An EnumSet, whose order is the declaration order (a HashSet of options
        // iterates in an order that changes between runs).
        () -> { Files.newByteChannel(dir.resolve("c"), EnumSet.of(StandardOpenOption.CREATE, StandardOpenOption.APPEND)).close(); return null; },
        () -> Files.copy(dir.resolve("a"), dir.resolve("e")),
        () -> Files.move(dir.resolve("e"), dir.resolve("g"), StandardCopyOption.ATOMIC_MOVE),
        () -> Files.createLink(dir.resolve("l"), dir.resolve("a")),
        () -> Files.createSymbolicLink(dir.resolve("s"), dir.resolve("a")),
        () -> { Files.delete(dir.resolve("g")); return null; },
        () -> { Files.delete(dir.resolve("g")); return null; },
        () -> Files.deleteIfExists(dir.resolve("g")),
        () -> Files.deleteIfExists(dir.resolve("a")),
        () -> Files.setAttribute(dir.resolve("b"), "basic:bogus", 1),
      };
      for (int i = 0; i < ops.length; i++) {
        log.clear();
        Op op = ops[i];
        String label = "VerboseFS op" + i;
        try { op.run(); w.println(label + "\tOK\t" + m(String.join(" | ", log))); } catch (Throwable t) { w.println(label + "\t" + err(t) + "\t" + m(String.join(" | ", log))); }
      }
      REL = null;
    }

    // FilterPath and FilterFileSystem.
    {
      Path base = fresh("filter");
      LeakFS provider = new LeakFS(def);
      FileSystem fs = provider.getFileSystem(URI.create("file:///"));
      Path dir = new FilterPath(base, (FilterFileSystem) fs);
      row("FilterPath toString", () -> dir.resolve("x").toString());
      row("FilterPath equals", () -> dir.resolve("x").equals(fs.getPath(D, "x")) + " " + (dir.resolve("x").hashCode() == base.resolve("x").hashCode()));
      row("FilterPath unwrap", () -> FilterPath.unwrap(dir.resolve("x")).getFileSystem() == def);
      row("FilterPath uri", () -> { Path p = dir.resolve("\u00e4\u4e2d"); return provider.getPath(p.toUri()).equals(p); });
      row("FilterFileSystem separator", () -> fs.getSeparator());
      row("FilterFileSystem roots", () -> fs.getRootDirectories().iterator().next());
      row("FilterFileSystem views", () -> new TreeSet<>(fs.supportedFileAttributeViews()));
      row("FilterFileSystem isOpen", () -> fs.isOpen() + " " + fs.isReadOnly());
      row("FilterFileSystem matcher", () -> { PathMatcher pm = fs.getPathMatcher("glob:*.txt"); return pm.matches(fs.getPath("a.txt")) + " " + pm.matches(Paths.get("a.txt")); });
      // The store's type and name depend on the filesystem the temporary
      // directory lives on; they are masked as $TYPE/$NAME when they are the
      // underlying store's, which is what FilterFileStore forwards.
      row("FilterFileStore", () -> { FileStore s = Files.getFileStore(dir); FileStore d = Files.getFileStore(base); return (s.type().equals(d.type()) ? "$TYPE" : s.type()) + " " + (s.name().equals(d.name()) ? "$NAME" : s.name()); });
      row("readSymbolicLink unwrapped", () -> { Files.createSymbolicLink(dir.resolve("s"), dir.resolve("t")); Path p = Files.readSymbolicLink(dir.resolve("s")); return p.getFileSystem() == def; });
      row("newFileSystem", () -> provider.newFileSystem(URI.create("file:///"), Map.of()) == fs);
    }

    w.close();
    IOUtils.rm(tmp);
  }
}
