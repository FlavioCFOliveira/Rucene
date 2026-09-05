/*
 * Measures, from lucene-core-10.5.0, the observable behaviour of
 * FSTCompiler.FixedLengthArcsBuffer, which the Rucene portability test
 * `tests/portability/fst_arcs_buffer.rs` asserts.
 *
 * Lives in org.apache.lucene.util.fst because FixedLengthArcsBuffer is a
 * package-private nested class of FSTCompiler (`FSTCompiler.java:1164`) whose
 * methods are package-private too, so it can only be driven from inside the
 * package.
 *
 * Prints key=value lines on stdout; byte arrays are printed as lowercase hex.
 */
package org.apache.lucene.util.fst;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.store.ByteBuffersDataOutput;
import org.apache.lucene.util.ArrayUtil;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.IntsRefBuilder;

public final class RuceneFixedLengthArcsBufferFixture {

  private static final int[] CAPACITIES = {1, 10, 11, 12, 13, 20, 100, 1000, 65536};

  /** The three values a node header's two vints take, spanning every vint width. */
  private static final int[] VINTS = {0, 1, 127, 128, 16383, 16384, 2097151, 2097152, 268435455};

  private static String hex(byte[] bytes, int len) {
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < len; i++) {
      sb.append(String.format(java.util.Locale.ROOT, "%02x", bytes[i]));
    }
    return sb.toString();
  }

  public static void main(String[] args) throws IOException {
    // ---- the field initialiser, FSTCompiler.java:1168 -----------------------
    FSTCompiler.FixedLengthArcsBuffer fresh = new FSTCompiler.FixedLengthArcsBuffer();
    System.out.println("initial.length=" + fresh.getBytes().length);
    System.out.println("initial.position=" + fresh.getPosition());
    System.out.println("initial.bytes=" + hex(fresh.getBytes(), fresh.getBytes().length));

    // ---- ensureCapacity, FSTCompiler.java:1172-1178 -------------------------
    for (int capacity : CAPACITIES) {
      FSTCompiler.FixedLengthArcsBuffer buffer = new FSTCompiler.FixedLengthArcsBuffer();
      buffer.ensureCapacity(capacity);
      System.out.println("ensure_capacity." + capacity + ".length=" + buffer.getBytes().length);
      System.out.println(
          "oversize." + capacity + "=" + ArrayUtil.oversize(capacity, Byte.BYTES));
    }

    // ensureCapacity that grows replaces the array, so the bytes already
    // written are gone, and resets the output over the new array, so the
    // position goes back to zero.
    FSTCompiler.FixedLengthArcsBuffer grown = new FSTCompiler.FixedLengthArcsBuffer();
    grown.writeByte((byte) 0xAB).writeByte((byte) 0xCD).writeByte((byte) 0xEF);
    System.out.println("grow.position_before=" + grown.getPosition());
    System.out.println("grow.bytes_before=" + hex(grown.getBytes(), 3));
    grown.ensureCapacity(100);
    System.out.println("grow.position_after=" + grown.getPosition());
    System.out.println("grow.length_after=" + grown.getBytes().length);
    System.out.println("grow.bytes_after=" + hex(grown.getBytes(), 3));

    // ensureCapacity that does not grow leaves both alone.
    FSTCompiler.FixedLengthArcsBuffer kept = new FSTCompiler.FixedLengthArcsBuffer();
    kept.writeByte((byte) 0xAB).writeByte((byte) 0xCD).writeByte((byte) 0xEF);
    kept.ensureCapacity(5);
    System.out.println("nogrow.position_after=" + kept.getPosition());
    System.out.println("nogrow.length_after=" + kept.getBytes().length);
    System.out.println("nogrow.bytes_after=" + hex(kept.getBytes(), 3));

    // ---- resetPosition, FSTCompiler.java:1180-1183 --------------------------
    FSTCompiler.FixedLengthArcsBuffer reset = new FSTCompiler.FixedLengthArcsBuffer();
    reset.writeByte((byte) 0x11).writeByte((byte) 0x22);
    reset.resetPosition();
    reset.writeByte((byte) 0x33);
    System.out.println("reset.position=" + reset.getPosition());
    System.out.println("reset.bytes=" + hex(reset.getBytes(), 2));

    // ---- the node header the compiler actually builds -----------------------
    //
    // flags byte, then two vints: numArcs and maxBytesPerArc for a binary-search
    // node, labelRange and maxBytesPerArcWithoutLabel for a direct-addressing
    // one. The eleven-byte initial array is sized for exactly this.
    FSTCompiler.FixedLengthArcsBuffer header = new FSTCompiler.FixedLengthArcsBuffer();
    for (int first : VINTS) {
      for (int second : VINTS) {
        header.resetPosition();
        header.writeByte((byte) 0x20);
        header.writeVInt(first);
        header.writeVInt(second);
        System.out.println(
            "header."
                + first
                + "."
                + second
                + "="
                + header.getPosition()
                + ":"
                + hex(header.getBytes(), header.getPosition()));
      }
    }

    // The widest header the eleven-byte array holds: one flags byte plus two
    // five-byte vints. One more byte would be past the end.
    FSTCompiler.FixedLengthArcsBuffer widest = new FSTCompiler.FixedLengthArcsBuffer();
    widest.resetPosition();
    widest.writeByte((byte) 0x20);
    widest.writeVInt(-1);
    widest.writeVInt(-1);
    System.out.println("widest.position=" + widest.getPosition());
    System.out.println("widest.bytes=" + hex(widest.getBytes(), widest.getPosition()));

    // ---- whole FSTs, so the header bytes are seen in their final place ----
    dumpFst("continuous", continuousTerms());
    dumpFst("sparse", sparseTerms());
    dumpFst("gapped", gappedTerms());

    System.out.println("read_ok=true");
  }

  /** One term and the output it maps to. */
  private record Term(byte[] bytes, long output) {}

  /**
   * Every prefix leads to the same three-way suffix, so the whole FST folds into
   * a handful of shared nodes and the root becomes a wide, contiguous node --
   * the CONTINUOUS header.
   */
  private static List<Term> continuousTerms() {
    List<Term> terms = new ArrayList<>();
    for (int a = 'a'; a <= 'z'; a++) {
      for (int b = 'a'; b <= 'z'; b++) {
        for (int c : new int[] {'0', '5', '9'}) {
          terms.add(new Term(new byte[] {(byte) a, (byte) b, (byte) c}, a * 1000L + b * 10L + c));
        }
      }
    }
    return terms;
  }

  /**
   * First labels spread over almost the whole byte range: direct addressing
   * would need too many presence bytes, so the node falls back to BINARY_SEARCH.
   */
  private static List<Term> sparseTerms() {
    List<Term> terms = new ArrayList<>();
    int[] firsts = {1, 50, 100, 150, 200, 250};
    for (int i = 0; i < firsts.length; i++) {
      for (int j = 0; j < 8; j++) {
        terms.add(
            new Term(
                new byte[] {(byte) firsts[i], (byte) 'x', (byte) j}, (i + 1) * 97L + j));
      }
    }
    return terms;
  }

  /** Nearly contiguous first labels with a single gap: DIRECT_ADDRESSING. */
  private static List<Term> gappedTerms() {
    List<Term> terms = new ArrayList<>();
    int[] firsts = {10, 11, 12, 13, 14, 16, 17, 18, 19, 20, 21};
    for (int i = 0; i < firsts.length; i++) {
      for (int j = 0; j < 8; j++) {
        terms.add(
            new Term(
                new byte[] {(byte) firsts[i], (byte) 'y', (byte) j}, (i + 3) * 31L + j));
      }
    }
    return terms;
  }

  /**
   * Builds the FST, prints the node-kind counters the compiler ended with, and
   * prints the bytes {@code FST.save} writes.
   *
   * <p>The terms are generated in ascending unsigned byte order and are not
   * sorted here, so that neither side has to agree on a comparator: Java's
   * {@code byte} is signed and Rust's {@code u8} is not, and a first label of
   * 250 would sort differently under the two.
   */
  private static void dumpFst(String name, List<Term> terms) throws IOException {
    PositiveIntOutputs outputs = PositiveIntOutputs.getSingleton();
    FSTCompiler<Long> compiler =
        new FSTCompiler.Builder<>(FST.INPUT_TYPE.BYTE1, outputs).build();
    IntsRefBuilder scratch = new IntsRefBuilder();
    for (Term term : terms) {
      compiler.add(Util.toIntsRef(new BytesRef(term.bytes()), scratch), term.output());
    }
    FST.FSTMetadata<Long> metadata = compiler.compile();
    // The public getters, not the raw fields: getNodeCount() adds one for the
    // implicit final node (`FSTCompiler.java:370-372`).
    System.out.println("fst." + name + ".node_count=" + compiler.getNodeCount());
    System.out.println("fst." + name + ".arc_count=" + compiler.getArcCount());
    System.out.println("fst." + name + ".binary_search=" + compiler.binarySearchNodeCount);
    System.out.println("fst." + name + ".direct_addressing=" + compiler.directAddressingNodeCount);
    System.out.println("fst." + name + ".continuous=" + compiler.continuousNodeCount);

    FST<Long> fst = FST.fromFSTReader(metadata, compiler.getFSTReader());
    ByteBuffersDataOutput meta = new ByteBuffersDataOutput();
    ByteBuffersDataOutput data = new ByteBuffersDataOutput();
    fst.save(meta, data);
    byte[] metaBytes = meta.toArrayCopy();
    byte[] dataBytes = data.toArrayCopy();
    System.out.println("fst." + name + ".meta=" + hex(metaBytes, metaBytes.length));
    System.out.println("fst." + name + ".data=" + hex(dataBytes, dataBytes.length));
  }
}
