/*
 * Measures, from lucene-core-10.5.0, every number and string that the Rucene
 * portability test `tests/portability/store_analysis_contracts.rs` asserts.
 *
 * Lives in org.apache.lucene.store so that ByteBuffersDataInput.blockSize(),
 * which is private, can be reached reflectively from the same runtime package,
 * and so that a Tokenizer subclass can call the protected final correctOffset.
 *
 * Prints key=value lines on stdout. Values that may contain whitespace, a BOM
 * or a '=' are base64-encoded, which is what keeps a trimmed line honest.
 */
package org.apache.lucene.store;

import java.io.BufferedOutputStream;
import java.io.ByteArrayInputStream;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.Reader;
import java.lang.reflect.Method;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Collections;
import java.util.List;
import org.apache.lucene.analysis.CharArraySet;
import org.apache.lucene.analysis.CharFilter;
import org.apache.lucene.analysis.Tokenizer;
import org.apache.lucene.analysis.WordlistLoader;
import org.apache.lucene.util.RamUsageEstimator;

public final class RuceneStoreAnalysisFixture {

  private static final String BOM = "\uFEFF";

  private static String b64(String s) {
    return Base64.getEncoder().encodeToString(s.getBytes(StandardCharsets.UTF_8));
  }

  /** A CharFilter that maps every output offset to twice its value. */
  private static final class DoublingCharFilter extends CharFilter {
    DoublingCharFilter(Reader in) {
      super(in);
    }

    @Override
    protected int correct(int currentOff) {
      return currentOff * 2;
    }

    @Override
    public int read(char[] cbuf, int off, int len) throws IOException {
      return input.read(cbuf, off, len);
    }

    @Override
    public void close() throws IOException {
      input.close();
    }
  }

  /** A Tokenizer that produces nothing; it exists to expose correctOffset. */
  private static final class ProbeTokenizer extends Tokenizer {
    @Override
    public boolean incrementToken() {
      return false;
    }

    int probe(int currentOff) {
      return correctOffset(currentOff);
    }
  }

  public static void main(String[] args) throws Exception {
    Path dir = Paths.get(args.length > 0 ? args[0] : ".");

    System.out.println("num_bytes_object_ref=" + RamUsageEstimator.NUM_BYTES_OBJECT_REF);

    // ---------------------------------------------------------------- 1) ByteBuffersDataInput
    // Each row is a block layout: the capacity of every ByteBuffer handed to
    // the constructor. All blocks but the last must share one power-of-two
    // remaining, which ensureAssumptions enforces.
    // Each row lists the blocks as capacity/remaining pairs. They differ on the
    // last block of the final rows: ramBytesUsed() charges capacity while
    // length() counts remaining, and a grid where the two always coincide
    // cannot tell the two terms apart. Only the last block may be short —
    // ensureAssumptions requires every earlier one to share one power-of-two
    // remaining.
    int[][] layouts = {
      {1, 1},
      {8, 8},
      {1024, 1024},
      {4096, 4096},
      {65536, 65536},
      {1024, 1024, 1024, 1024},
      {1024, 1024, 7, 7},
      {1024, 1024, 1024, 1024, 1024, 1024},
      {1024, 1024, 1024, 1024, 1, 1},
      {8, 8, 8, 8},
      {8, 8, 8, 8, 8, 8},
      {8, 8, 3, 3},
      {4096, 4096, 4096, 4096, 4096, 4096, 4096, 4096},
      {65536, 65536, 65536, 65536, 1, 1},
      // capacity > remaining on the last block:
      {1024, 1024, 1024, 300},
      {8, 8, 4096, 1},
      {65536, 65536, 65536, 0},
      {4096, 300},
    };
    Method blockSize = ByteBuffersDataInput.class.getDeclaredMethod("blockSize");
    blockSize.setAccessible(true);
    for (int[] layout : layouts) {
      List<ByteBuffer> buffers = new ArrayList<>();
      StringBuilder key = new StringBuilder();
      for (int b = 0; b < layout.length; b += 2) {
        int cap = layout[b];
        int remaining = layout[b + 1];
        ByteBuffer bb = ByteBuffer.allocate(cap);
        for (int i = 0; i < remaining; i++) {
          bb.put((byte) i);
        }
        bb.flip();
        buffers.add(bb);
        if (key.length() > 0) {
          key.append('.');
        }
        key.append(cap).append('@').append(remaining);
      }
      ByteBuffersDataInput in = new ByteBuffersDataInput(buffers);
      System.out.println("bbdi_ram=" + key + ":" + in.ramBytesUsed());
      System.out.println("bbdi_blocksize=" + key + ":" + blockSize.invoke(in));
      System.out.println("bbdi_tostring=" + key + ":" + b64(in.toString()));
    }

    // ---------------------------------------------------------------- 2) ByteBuffersDataOutput
    for (int written : new int[] {0, 1, 100, 1024, 1025, 8192}) {
      ByteBuffersDataOutput out = ByteBuffersDataOutput.newResettableInstance();
      for (int i = 0; i < written; i++) {
        out.writeByte((byte) i);
      }
      System.out.println("bbdo_ram=" + written + ":" + out.ramBytesUsed());
    }

    // ---------------------------------------------------------------- 3) WordlistLoader.getLines
    // A corpus that exercises every rule and their order: a BOM ahead of a run
    // of comments and blanks, a '#' that only becomes leading after a trim, a
    // line whose only content is trimmable, a no-break space that Java does NOT
    // trim, and a lone carriage return as a line terminator.
    String corpus =
        BOM
            + "# leading comment after the BOM\n"
            + "\n"
            + "   \n"
            + "first\n"
            + "  spaced  \r\n"
            + "   # not a comment: '#' is not first before the trim\n"
            + "#dropped\n"
            + "\n"
            + "x\n"
            + "\u00A0nbsp\u00A0\n"
            + "\u0001ctl\u0001\n"
            + "tab\there\n"
            + "\rlone-cr-terminated\r"
            + "trailing-no-newline";
    List<String> lines =
        WordlistLoader.getLines(
            new ByteArrayInputStream(corpus.getBytes(StandardCharsets.UTF_8)),
            StandardCharsets.UTF_8);
    System.out.println("corpus=" + b64(corpus));
    System.out.println("getlines_count=" + lines.size());
    for (int i = 0; i < lines.size(); i++) {
      System.out.println("getlines=" + i + ":" + b64(lines.get(i)));
    }

    // A BOM that survives a run of skipped lines: the strip is guarded by
    // lines.isEmpty(), not by "is this the first line".
    String bomLate = BOM + "#c\n" + BOM + "kept\n";
    List<String> bomLines =
        WordlistLoader.getLines(
            new ByteArrayInputStream(bomLate.getBytes(StandardCharsets.UTF_8)),
            StandardCharsets.UTF_8);
    System.out.println("bomlate=" + b64(bomLate));
    System.out.println("bomlate_count=" + bomLines.size());
    for (int i = 0; i < bomLines.size(); i++) {
      System.out.println("bomlate_line=" + i + ":" + b64(bomLines.get(i)));
    }

    // ---------------------------------------------------------------- 4) String.trim()
    String[] trimProbes = {
      "  x  ", "x", "\t\nx\r ", "\u00A0x\u00A0", "\u0001x\u0001",
      "\u001Fx\u001F", "\u0020x\u0020", "\u00A0", "\u0001", " ", ""
    };
    for (String probe : trimProbes) {
      System.out.println("trim=" + b64(probe) + ":" + b64(probe.trim()));
    }

    // ---------------------------------------------------------------- 5) Snowball word set
    String snowball =
        "a b | comment\n c d\n  e\tf  \n|whole line\ng\n" + "h\u00A0i\n" + "j\u000Bk\n";
    CharArraySet snow =
        WordlistLoader.getSnowballWordSet(
            new ByteArrayInputStream(snowball.getBytes(StandardCharsets.UTF_8)),
            StandardCharsets.UTF_8);
    List<String> snowWords = new ArrayList<>();
    for (Object o : snow) {
      snowWords.add(new String((char[]) o));
    }
    Collections.sort(snowWords);
    System.out.println("snowball=" + b64(snowball));
    System.out.println("snowball_count=" + snowWords.size());
    for (int i = 0; i < snowWords.size(); i++) {
      System.out.println("snowball_word=" + i + ":" + b64(snowWords.get(i)));
    }

    // ---------------------------------------------------------------- 6) Tokenizer.correctOffset
    ProbeTokenizer plain = new ProbeTokenizer();
    // Before setReader/reset the input is the ILLEGAL_STATE_READER sentinel.
    System.out.println("correctoffset_unset=" + plain.probe(7));
    ProbeTokenizer filtered = new ProbeTokenizer();
    filtered.setReader(new DoublingCharFilter(new java.io.StringReader("hello")));
    filtered.reset();
    System.out.println("correctoffset_filter=" + filtered.probe(7));
    ProbeTokenizer nested = new ProbeTokenizer();
    nested.setReader(
        new DoublingCharFilter(new DoublingCharFilter(new java.io.StringReader("hello"))));
    nested.reset();
    System.out.println("correctoffset_nested=" + nested.probe(7));
    ProbeTokenizer bare = new ProbeTokenizer();
    bare.setReader(new java.io.StringReader("hello"));
    bare.reset();
    System.out.println("correctoffset_plain=" + bare.probe(7));

    // ---------------------------------------------------------------- 7) file bytes: the closes
    // (a) Read back the file Rucene's OutputStreamDataOutput wrote and closed.
    Path rustFile = dir.resolve("rust_out.bin");
    if (Files.exists(rustFile)) {
      InputStreamDataInput in = new InputStreamDataInput(new FileInputStream(rustFile.toFile()));
      StringBuilder sb = new StringBuilder();
      sb.append(in.readByte()).append(',');
      sb.append(in.readShort()).append(',');
      sb.append(in.readInt()).append(',');
      sb.append(in.readLong()).append(',');
      sb.append(in.readVInt()).append(',');
      sb.append(in.readVLong()).append(',');
      sb.append(in.readString());
      in.close();
      System.out.println("rust_file_len=" + Files.size(rustFile));
      System.out.println("rust_file_values=" + b64(sb.toString()));
    }

    // (b) Write a file with Java's OutputStreamDataOutput and close it.
    Path javaFile = dir.resolve("java_out.bin");
    OutputStreamDataOutput out =
        new OutputStreamDataOutput(
            new BufferedOutputStream(new FileOutputStream(javaFile.toFile())));
    out.writeByte((byte) 0x42);
    out.writeShort((short) 0x1234);
    out.writeInt(0xDEADBEEF);
    out.writeLong(0x1234567890ABCDEFL);
    out.writeVInt(16384);
    out.writeVLong(1_000_000_000_000L);
    out.writeString("stream test");
    out.close();
    System.out.println("java_file_len=" + Files.size(javaFile));

    System.out.println("read_ok=true");
  }
}
