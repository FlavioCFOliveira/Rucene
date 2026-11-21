// Oracle of task #331 (rucene-test-framework/tests/nio_oracle.rs): the
// default java.nio.file provider of JDK 25 and the call order of Lucene
// 10.5.0's FSDirectory, NIOFSDirectory, MMapDirectory, lock factories and
// IOUtils over a recording FilterFileSystemProvider. Writes
// <out>/rucene-test-framework/nio_default.tsv and nio_store_trace.tsv; the
// temporary directory is masked as $D.
package org.apache.lucene.tests.mockfile;

import java.io.*;
import java.net.URI;
import java.nio.channels.*;
import java.nio.file.*;
import java.nio.file.attribute.*;
import java.util.*;
import java.util.concurrent.ExecutorService;
import org.apache.lucene.store.*;
import org.apache.lucene.util.IOUtils;

public class NioOracle {
  static String D;
  static final List<String> trace = new ArrayList<>();

  static String m(Object o) {
    return String.valueOf(o).replace(D, "$D").replace("\n", "\\n").replace("\t", "\\t");
  }

  static String err(Throwable t) {
    return t.getClass().getName() + "\t" + (t.getMessage() == null ? "<null>" : m(t.getMessage()));
  }

  interface Op { Object run() throws Throwable; }

  static void row(PrintWriter w, String label, Op op) {
    try {
      Object r = op.run();
      w.println(label + "\tOK\t" + (r == null ? "-" : m(r)));
    } catch (Throwable t) {
      w.println(label + "\t" + err(t));
    }
  }

  static String opts(Collection<?> options) {
    List<String> s = new ArrayList<>();
    for (Object o : options) s.add(o.toString());
    Collections.sort(s);
    return s.toString();
  }

  static void rec(String s) { trace.add(m(s)); }

  /** Records every provider call, and the closes, forces, sizes and locks of the channels. */
  static class RecordingFS extends FilterFileSystemProvider {
    RecordingFS(FileSystem delegate) { super("record://", delegate); }

    @Override public void createDirectory(Path dir, FileAttribute<?>... attrs) throws IOException { rec("createDirectory " + dir); super.createDirectory(dir, attrs); }
    @Override public void delete(Path path) throws IOException { rec("delete " + path); super.delete(path); }
    @Override public boolean deleteIfExists(Path path) throws IOException { rec("deleteIfExists " + path); return super.deleteIfExists(path); }
    @Override public void copy(Path s, Path t, CopyOption... o) throws IOException { rec("copy " + opts(Arrays.asList(o)) + " " + s + " " + t); super.copy(s, t, o); }
    @Override public void move(Path s, Path t, CopyOption... o) throws IOException { rec("move " + opts(Arrays.asList(o)) + " " + s + " " + t); super.move(s, t, o); }
    @Override public void checkAccess(Path path, AccessMode... modes) throws IOException { rec("checkAccess " + opts(Arrays.asList(modes)) + " " + path); super.checkAccess(path, modes); }
    @Override public <A extends BasicFileAttributes> A readAttributes(Path path, Class<A> type, LinkOption... o) throws IOException { rec("readAttributes " + opts(Arrays.asList(o)) + " " + path); return super.readAttributes(path, type, o); }
    @Override public InputStream newInputStream(Path path, OpenOption... o) throws IOException { rec("newInputStream " + opts(Arrays.asList(o)) + " " + path); return super.newInputStream(path, o); }
    @Override public OutputStream newOutputStream(Path path, OpenOption... o) throws IOException {
      rec("newOutputStream " + opts(Arrays.asList(o)) + " " + path);
      return new FilterOutputStream2(super.newOutputStream(path, o)) {
        @Override public void close() throws IOException { rec("close OutputStream " + path); super.close(); }
      };
    }
    @Override public FileChannel newFileChannel(Path path, Set<? extends OpenOption> o, FileAttribute<?>... a) throws IOException {
      rec("newFileChannel " + opts(o) + " " + path);
      return new FilterFileChannel(super.newFileChannel(path, o, a)) {
        @Override public void force(boolean meta) throws IOException { rec("force " + meta + " " + path); super.force(meta); }
        @Override public long size() throws IOException { rec("size " + path); return super.size(); }
        @Override public FileLock tryLock(long p, long s, boolean sh) throws IOException { rec("tryLock " + path); return super.tryLock(p, s, sh); }
        @Override protected void implCloseChannel() throws IOException { rec("close FileChannel " + path); super.implCloseChannel(); }
      };
    }
    @Override public SeekableByteChannel newByteChannel(Path path, Set<? extends OpenOption> o, FileAttribute<?>... a) throws IOException {
      rec("newByteChannel " + opts(o) + " " + path);
      return new FilterSeekableByteChannel(super.newByteChannel(path, o, a)) {
        @Override public void close() throws IOException { rec("close SeekableByteChannel " + path); super.close(); }
      };
    }
    @Override public DirectoryStream<Path> newDirectoryStream(Path dir, DirectoryStream.Filter<? super Path> filter) throws IOException {
      rec("newDirectoryStream " + dir);
      DirectoryStream<Path> s = super.newDirectoryStream(dir, filter);
      return new DirectoryStream<Path>() {
        @Override public Iterator<Path> iterator() { return s.iterator(); }
        @Override public void close() throws IOException { rec("close DirectoryStream " + dir); s.close(); }
      };
    }
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    Path tmp = Files.createTempDirectory("nio-oracle").toRealPath();
    D = tmp.toString();
    try (PrintWriter w = new PrintWriter(new File(out, "nio_default.tsv"), "UTF-8")) {
      w.println("# label\tOK\tresult | label\tclass\tmessage; default provider of JDK 25, $D = temp dir");
      Path d = tmp;
      Path nope = d.resolve("nope");
      row(w, "delete missing", () -> { Files.delete(nope); return null; });
      row(w, "deleteIfExists missing", () -> Files.deleteIfExists(nope));
      row(w, "createDirectory sub", () -> Files.createDirectory(d.resolve("sub")));
      row(w, "write sub/f", () -> Files.write(d.resolve("sub/f"), new byte[] {1, 2, 3}));
      row(w, "delete nonempty", () -> { Files.delete(d.resolve("sub")); return null; });
      row(w, "createDirectory exists", () -> Files.createDirectory(d.resolve("sub")));
      row(w, "createDirectory noparent", () -> Files.createDirectory(d.resolve("x/y")));
      row(w, "createDirectories nested", () -> Files.createDirectories(d.resolve("p/q/r")));
      row(w, "createDirectories under file", () -> Files.createDirectories(d.resolve("sub/f/x")));
      row(w, "createDirectories on file", () -> Files.createDirectories(d.resolve("sub/f")));
      row(w, "createFile exists", () -> Files.createFile(d.resolve("sub/f")));
      row(w, "newOutputStream CREATE_NEW exists", () -> { Files.newOutputStream(d.resolve("sub/f"), StandardOpenOption.WRITE, StandardOpenOption.CREATE_NEW).close(); return null; });
      row(w, "newOutputStream READ", () -> Files.newOutputStream(d.resolve("sub/g"), StandardOpenOption.READ));
      row(w, "newInputStream WRITE", () -> Files.newInputStream(d.resolve("sub/f"), StandardOpenOption.WRITE));
      row(w, "FileChannel READ missing", () -> FileChannel.open(nope, StandardOpenOption.READ));
      row(w, "FileChannel WRITE dir", () -> FileChannel.open(d.resolve("sub"), StandardOpenOption.WRITE));
      row(w, "FileChannel READ APPEND", () -> FileChannel.open(d.resolve("sub/f"), StandardOpenOption.READ, StandardOpenOption.APPEND));
      row(w, "FileChannel APPEND TRUNCATE", () -> FileChannel.open(d.resolve("sub/f"), StandardOpenOption.APPEND, StandardOpenOption.TRUNCATE_EXISTING));
      row(w, "size", () -> Files.size(d.resolve("sub/f")));
      row(w, "size missing", () -> Files.size(nope));
      row(w, "move missing ATOMIC", () -> Files.move(nope, d.resolve("nope2"), StandardCopyOption.ATOMIC_MOVE));
      row(w, "createDirectory sub2", () -> Files.createDirectory(d.resolve("sub2")));
      row(w, "write sub2/g", () -> Files.write(d.resolve("sub2/g"), new byte[] {1}));
      row(w, "move onto nonempty dir ATOMIC", () -> Files.move(d.resolve("sub/f"), d.resolve("sub2"), StandardCopyOption.ATOMIC_MOVE));
      row(w, "move onto existing", () -> Files.move(d.resolve("sub/f"), d.resolve("sub2/g")));
      row(w, "move onto existing REPLACE", () -> Files.move(d.resolve("sub/f"), d.resolve("sub2/g"), StandardCopyOption.REPLACE_EXISTING));
      row(w, "copy back", () -> Files.copy(d.resolve("sub2/g"), d.resolve("sub/f")));
      row(w, "copy onto existing", () -> Files.copy(d.resolve("sub2/g"), d.resolve("sub/f")));
      row(w, "toRealPath missing", () -> nope.toRealPath());
      row(w, "newDirectoryStream missing", () -> Files.newDirectoryStream(nope));
      row(w, "newDirectoryStream file", () -> Files.newDirectoryStream(d.resolve("sub/f")));
      row(w, "readSymbolicLink file", () -> Files.readSymbolicLink(d.resolve("sub/f")));
      row(w, "createLink exists", () -> Files.createLink(d.resolve("sub/f"), d.resolve("sub2/g")));
      row(w, "exists", () -> Files.exists(d.resolve("sub/f")));
      row(w, "isDirectory", () -> Files.isDirectory(d.resolve("sub")));
      row(w, "isRegularFile", () -> Files.isRegularFile(d.resolve("sub/f")));
      row(w, "notExists", () -> Files.notExists(nope));
      row(w, "isHidden", () -> Files.isHidden(d.resolve(".x")));
      row(w, "isSameFile", () -> Files.isSameFile(d.resolve("sub/f"), d.resolve("sub/../sub/f")));
      row(w, "readAttributes basic", () -> Files.readAttributes(d.resolve("sub/f"), "basic:size,isRegularFile,isDirectory"));
      row(w, "readAttributes view", () -> Files.readAttributes(d.resolve("sub/f"), "foo:size"));
      row(w, "getAttribute bogus", () -> Files.getAttribute(d.resolve("sub/f"), "basic:bogus"));
      row(w, "setAttribute bogus", () -> Files.setAttribute(d.resolve("sub/f"), "basic:bogus", 1));
      row(w, "readAllLines", () -> { Files.write(d.resolve("lines"), "a\nb\r\nc".getBytes("UTF-8")); return Files.readAllLines(d.resolve("lines")); });
      row(w, "list sorted", () -> { List<String> l = new ArrayList<>(); try (DirectoryStream<Path> s = Files.newDirectoryStream(d)) { for (Path p : s) l.add(p.getFileName().toString()); } Collections.sort(l); return l; });
      row(w, "glob", () -> { List<String> l = new ArrayList<>(); try (DirectoryStream<Path> s = Files.newDirectoryStream(d, "s*")) { for (Path p : s) l.add(p.getFileName().toString()); } Collections.sort(l); return l; });
      row(w, "IOUtils.rm", () -> { IOUtils.rm(d.resolve("p")); return Files.exists(d.resolve("p")); });
      row(w, "uri", () -> d.resolve("\u00e4 b%").toUri().toString());
      row(w, "uri round trip", () -> Paths.get(new URI(d.resolve("\u00e4 b%").toUri().toString())));
      row(w, "provider getPath authority", () -> FileSystems.getDefault().provider().getPath(new URI("file://host/x")));
      row(w, "provider getPath scheme", () -> FileSystems.getDefault().provider().getPath(new URI("http:///x")));
      row(w, "provider getPath query", () -> FileSystems.getDefault().provider().getPath(new URI("file:///x?q")));
      row(w, "provider getPath fragment", () -> FileSystems.getDefault().provider().getPath(new URI("file:///x#f")));
      row(w, "getPath nul", () -> Paths.get("a\0b"));
      row(w, "pathMatcher syntax", () -> FileSystems.getDefault().getPathMatcher("foo:x"));
      row(w, "pathMatcher none", () -> FileSystems.getDefault().getPathMatcher("x"));
      row(w, "close default", () -> { FileSystems.getDefault().close(); return null; });
      row(w, "views", () -> new TreeSet<>(FileSystems.getDefault().supportedFileAttributeViews()));
      row(w, "IOUtils.fsync missing", () -> { IOUtils.fsync(nope, false); return null; });
      row(w, "IOUtils.fsync dir", () -> { IOUtils.fsync(d, true); return null; });
      row(w, "IOUtils.deleteFilesIfExist", () -> { IOUtils.deleteFilesIfExist(d.resolve("sub2/g"), nope); return Files.exists(d.resolve("sub2/g")); });
      row(w, "FileLock", () -> { try (FileChannel fc = FileChannel.open(d.resolve("lock"), StandardOpenOption.CREATE, StandardOpenOption.WRITE)) { FileLock l = fc.tryLock(); String a = l.toString(); fc.close(); return a + " " + l + " " + l.isValid(); } });
      row(w, "closed channel size", () -> { FileChannel fc = FileChannel.open(d.resolve("lock"), StandardOpenOption.READ); fc.close(); return fc.size(); });
    }
    trace(out, tmp);
    IOUtils.rm(tmp);
  }

  static void trace(File out, Path tmp) throws Exception {
    RecordingFS provider = new RecordingFS(FileSystems.getDefault());
    FileSystem fs = provider.getFileSystem(URI.create("file:///"));
    Path root = fs.getPath(tmp.toString());
    try (PrintWriter w = new PrintWriter(new File(out, "nio_store_trace.tsv"), "UTF-8")) {
      w.println("# step\ttrace of provider calls (FSDirectory over a recording FilterFileSystemProvider), $D = temp dir");
      step(w, "NIOFSDirectory.<init>", () -> { dirs.put("nio", new NIOFSDirectory(root.resolve("idx"))); });
      Directory dir = dirs.get("nio");
      step(w, "createOutput", () -> { IndexOutput o = dir.createOutput("_0.cfs", IOContext.DEFAULT); o.writeBytes(new byte[64], 64); o.close(); });
      step(w, "sync", () -> dir.sync(List.of("_0.cfs")));
      step(w, "createTempOutput", () -> { IndexOutput o = dir.createTempOutput("pending", "tmp", IOContext.DEFAULT); o.writeInt(7); o.close(); names.add(o.getName()); });
      step(w, "rename", () -> dir.rename(names.get(0), "segments_1"));
      step(w, "syncMetaData", () -> dir.syncMetaData());
      step(w, "fileLength", () -> dir.fileLength("segments_1"));
      step(w, "openInput", () -> { IndexInput in = dir.openInput("segments_1", IOContext.DEFAULT); in.readByte(); in.close(); });
      step(w, "listAll", () -> dir.listAll());
      step(w, "obtainLock", () -> { Lock l = dir.obtainLock("write.lock"); l.ensureValid(); l.close(); });
      step(w, "deleteFile", () -> dir.deleteFile("_0.cfs"));
      step(w, "deleteFile missing", () -> dir.deleteFile("_0.cfs"));
      step(w, "close", () -> dir.close());
      step(w, "MMapDirectory", () -> { try (Directory mm = new MMapDirectory(root.resolve("mm"))) { IndexOutput o = mm.createOutput("a", IOContext.DEFAULT); o.writeLong(1); o.close(); IndexInput in = mm.openInput("a", IOContext.DEFAULT); in.readLong(); in.close(); } });
      step(w, "SimpleFSLockFactory", () -> { try (Directory s = FSDirectory.open(root.resolve("s"), SimpleFSLockFactory.INSTANCE)) { Lock l = s.obtainLock("write.lock"); l.ensureValid(); l.close(); } });
      step(w, "IOUtils.rm", () -> IOUtils.rm(root.resolve("idx"), root.resolve("mm"), root.resolve("s")));
    }
  }

  static final Map<String, Directory> dirs = new HashMap<>();
  static final List<String> names = new ArrayList<>();

  interface Step { void run() throws Throwable; }
  interface StepV { Object run() throws Throwable; }

  static void step(PrintWriter w, String name, Step s) {
    trace.clear();
    String outcome = "OK";
    try { s.run(); } catch (Throwable t) { outcome = err(t).replace("\t", " "); }
    w.println(name + "\t" + outcome + "\t" + String.join(" | ", trace));
  }

  static void step(PrintWriter w, String name, StepV s) {
    step(w, name, (Step) () -> s.run());
  }
}
