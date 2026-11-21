/*
 * Measures, from lucene-core-10.5.0, when BytesRefFSTEnum (through FSTEnum)
 * opens FST.BytesReaders, and what BytesRefFSTEnum.current() returns, which the
 * Rucene portability test `tests/portability/fst_enum_reader.rs` asserts.
 *
 * The FST is read through a counting FSTReader, so every
 * FSTReader.getReverseBytesReader() call -- every FST.getBytesReader() -- is
 * seen. Each operation prints how many readers it opened.
 *
 * Prints key=value lines on stdout; byte strings are printed as lowercase hex
 * ("-" for the empty one), a missing result as "null".
 */
package org.apache.lucene.util.fst;

import java.io.IOException;
import org.apache.lucene.store.DataOutput;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.IntsRefBuilder;

public final class RuceneFSTEnumReaderFixture {

  /**
   * The terms, in order. The root and the node after "a" have enough arcs to be written as arrays,
   * the deeper nodes as lists, so a seek crosses array and list nodes.
   */
  static final String[] TERMS = {
    "", "a", "aa", "ab", "abc", "abd", "ac", "ae", "ag", "ai", "b", "ba", "c", "d", "e", "f", "g",
    "zÿ", "ÿÿ"
  };

  /** The seek targets, as Latin-1 strings. */
  static final String[] TARGETS = {
    "", "a", "aa", "abz", "ab", "abcd", "ad", "aj", "b", "bb", "c", "h", "z", "ÿ", "ÿÿÿ"
  };

  static final class CountingReader implements FSTReader {
    final FSTReader in;
    int opened;

    CountingReader(FSTReader in) {
      this.in = in;
    }

    @Override
    public FST.BytesReader getReverseBytesReader() {
      opened++;
      return in.getReverseBytesReader();
    }

    @Override
    public void writeTo(DataOutput out) throws IOException {
      in.writeTo(out);
    }

    @Override
    public long ramBytesUsed() {
      return in.ramBytesUsed();
    }
  }

  static BytesRef bytes(String s) {
    return new BytesRef(s.getBytes(java.nio.charset.StandardCharsets.ISO_8859_1));
  }

  static String hex(BytesRef b) {
    if (b.length == 0) {
      return "-";
    }
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < b.length; i++) {
      sb.append(String.format(java.util.Locale.ROOT, "%02x", b.bytes[b.offset + i] & 0xff));
    }
    return sb.toString();
  }

  static String result(BytesRefFSTEnum.InputOutput<Long> io) {
    return io == null ? "null" : hex(io.input) + ":" + io.output;
  }

  static String current(BytesRefFSTEnum<Long> e) {
    BytesRefFSTEnum.InputOutput<Long> io = e.current();
    return hex(io.input) + ":" + io.output;
  }

  public static void main(String[] args) throws IOException {
    FSTCompiler<Long> compiler =
        new FSTCompiler.Builder<>(FST.INPUT_TYPE.BYTE1, PositiveIntOutputs.getSingleton()).build();
    IntsRefBuilder scratch = new IntsRefBuilder();
    for (int i = 0; i < TERMS.length; i++) {
      compiler.add(Util.toIntsRef(bytes(TERMS[i]), scratch), (long) i * 3);
    }
    FST.FSTMetadata<Long> metadata = compiler.compile();
    CountingReader counting = new CountingReader(compiler.getFSTReader());
    FST<Long> fst = FST.fromFSTReader(metadata, counting);

    int before = counting.opened;
    BytesRefFSTEnum<Long> e = new BytesRefFSTEnum<>(fst);
    System.out.println("ctor=" + (counting.opened - before));
    System.out.println("current.fresh=" + current(e));

    for (int i = 0; ; i++) {
      before = counting.opened;
      BytesRefFSTEnum.InputOutput<Long> io = e.next();
      System.out.println("next." + i + "=" + (counting.opened - before) + ":" + result(io));
      if (i == 0) {
        System.out.println("current.first=" + current(e));
      }
      if (io == null) {
        break;
      }
    }
    System.out.println("current.exhausted=" + current(e));

    for (String t : TARGETS) {
      BytesRef target = bytes(t);
      before = counting.opened;
      String ceil = result(e.seekCeil(target));
      System.out.println("ceil." + hex(target) + "=" + (counting.opened - before) + ":" + ceil);
      before = counting.opened;
      String floor = result(e.seekFloor(target));
      System.out.println("floor." + hex(target) + "=" + (counting.opened - before) + ":" + floor);
      before = counting.opened;
      String exact = result(e.seekExact(target));
      System.out.println("exact." + hex(target) + "=" + (counting.opened - before) + ":" + exact);
      System.out.println("current." + hex(target) + "=" + current(e));
    }
    System.out.println("read_ok=true");
  }
}
