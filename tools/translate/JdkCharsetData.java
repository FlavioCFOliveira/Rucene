// Generates, from the running JDK's public java.nio.charset API (run it with
// JDK 25; the reflection that reads a decoder's shift state needs the opens):
//
//   java -Xmx4g --add-opens java.base/sun.nio.cs=ALL-UNNAMED \
//     --add-opens jdk.charsets/sun.nio.cs.ext=ALL-UNNAMED \
//     tools/translate/JdkCharsetData.java \
//     crates/rucene-analysis-common/src/jdk/charset_data.rs \
//     crates/rucene-analysis-common/src/jdk/charset_multibyte.bin
//
// * charset_data.rs: every charset Charset.availableCharsets() holds, with its
//   canonical name, its aliases and how it decodes; for every single-byte
//   charset the decoding of each of the 256 bytes: the UTF-16 unit, or whether
//   the byte is malformed or unmappable input. A charset is single-byte when
//   every byte decodes on its own to one unit or to an error of length 1, and a
//   stateless table decodes a fixed pseudo-random byte sequence exactly as the
//   charset's own decoder does in one pass.
// * charset_multibyte.bin: the data of every other charset but UTF-8, each
//   item zlib-compressed on its own:
//   - a byte-sequence machine for every table-driven multi-byte charset (the
//     DBCS, EUC, Shift_JIS, Big5, HKSCS, EBCDIC-stateful and ISO-2022
//     charsets). It is found by probing the charset's decoder byte by byte:
//     from each state, a trie whose every path is the shortest byte sequence
//     the decoder decides on (a character, a malformed or unmappable input of
//     some length, or a shift of state that outputs nothing), with the state
//     after it and whether the decision reports OVERFLOW first when the output
//     is full. A state is the value of the decoder's mutable primitive fields,
//     read by reflection after each decision. The machine is then checked
//     against the decoder on pseudo-random streams fed in random chunks into
//     random small output buffers, call for call (result, input position,
//     output);
//   - GB18030: the two-byte table and the four-byte table of the Basic
//     Multilingual Plane, checked against the decoder on every two-byte and
//     four-byte sequence and on pseudo-random streams;
//   - ISCII91: the unit of each byte, checked on pseudo-random streams.
//   The UTF-16, UTF-32 and CESU-8 decoders, and x-JISAutoDetect (which
//   delegates to the ISO-2022-JP, EUC-JP and Shift_JIS machines), need no
//   data.
//
// No JDK code is reproduced: every value comes from calling the public API.
// The generator fails on any inconsistency it checks. Deterministic: the
// output depends only on the JDK's charset data (it runs about two minutes).
import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.CharBuffer;
import java.nio.charset.Charset;
import java.nio.charset.CharsetDecoder;
import java.nio.charset.CoderResult;
import java.nio.charset.CodingErrorAction;
import java.nio.file.Files;
import java.nio.file.Path;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.security.MessageDigest;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Random;
import java.util.TreeSet;
import java.util.zip.Deflater;

public class JdkCharsetData {
  static final int UNMAPPABLE = 0x10000;
  static final int MALFORMED = 0x10001;

  /** The charsets decoded by an algorithm of the emulation, and its Rust form. */
  static final Map<String, String> ALGORITHMIC = new LinkedHashMap<>();

  /** The decoder class the algorithm of each algorithmic charset reproduces. */
  static final Map<String, String> ALGORITHMIC_CLASS = new LinkedHashMap<>();

  static {
    algorithmic("UTF-16", "Utf16 { expected: ByteOrder::None, default: ByteOrder::Big }",
        "sun.nio.cs.UTF_16$Decoder");
    algorithmic("UTF-16BE", "Utf16 { expected: ByteOrder::Big, default: ByteOrder::Big }",
        "sun.nio.cs.UTF_16BE$Decoder");
    algorithmic("UTF-16LE", "Utf16 { expected: ByteOrder::Little, default: ByteOrder::Big }",
        "sun.nio.cs.UTF_16LE$Decoder");
    algorithmic("x-UTF-16LE-BOM", "Utf16 { expected: ByteOrder::None, default: ByteOrder::Little }",
        "sun.nio.cs.UTF_16LE_BOM$Decoder");
    algorithmic("UTF-32", "Utf32(ByteOrder::None)", "sun.nio.cs.UTF_32Coder$Decoder");
    algorithmic("UTF-32BE", "Utf32(ByteOrder::Big)", "sun.nio.cs.UTF_32Coder$Decoder");
    algorithmic("UTF-32LE", "Utf32(ByteOrder::Little)", "sun.nio.cs.UTF_32Coder$Decoder");
    algorithmic("X-UTF-32BE-BOM", "Utf32(ByteOrder::Big)", "sun.nio.cs.UTF_32Coder$Decoder");
    algorithmic("X-UTF-32LE-BOM", "Utf32(ByteOrder::Little)", "sun.nio.cs.UTF_32Coder$Decoder");
    algorithmic("CESU-8", "Cesu8", "sun.nio.cs.CESU_8$Decoder");
    algorithmic("GB18030", "Gb18030", "sun.nio.cs.GB18030$Decoder");
    algorithmic("x-ISCII91", "Iscii91", "sun.nio.cs.ext.ISCII91$Decoder");
    algorithmic("x-JISAutoDetect", "JisAutoDetect", "sun.nio.cs.ext.JISAutoDetect$Decoder");
  }

  static void algorithmic(String name, String rust, String decoderClass) {
    ALGORITHMIC.put(name, rust);
    ALGORITHMIC_CLASS.put(name, decoderClass);
  }

  public static void main(String[] args) throws Exception {
    check(args.length == 2, "usage: JdkCharsetData <charset_data.rs> <charset_multibyte.bin>");
    StringBuilder out = new StringBuilder();
    out.append(String.join("\n",
        "/*",
        " * Licensed to the Apache Software Foundation (ASF) under one or more",
        " * contributor license agreements.  See the NOTICE file distributed with",
        " * this work for additional information regarding copyright ownership.",
        " * The ASF licenses this file to You under the Apache License, Version 2.0",
        " * (the \"License\"); you may not use this file except in compliance with",
        " * the License.  You may obtain a copy of the License at",
        " *",
        " *     http://www.apache.org/licenses/LICENSE-2.0",
        " *",
        " * Unless required by applicable law or agreed to in writing, software",
        " * distributed under the License is distributed on an \"AS IS\" BASIS,",
        " * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.",
        " * See the License for the specific language governing permissions and",
        " * limitations under the License.",
        " */",
        "",
        "//! The charsets of the JDK that runs Lucene 10.5.0, as",
        "//! [`charset`](super::charset) reads them: the canonical name and the",
        "//! aliases of every charset `Charset.availableCharsets()` lists, how each",
        "//! decodes, and the decoding table of every single-byte charset (the data",
        "//! of the multi-byte charsets is `charset_multibyte.bin`).",
        "//!",
        "//! Generated by `tools/translate/JdkCharsetData.java` from OpenJDK "
            + Runtime.version() + ", which calls the",
        "//! public `java.nio.charset` API for every charset and byte; no JDK code is",
        "//! reproduced. Do not edit by hand.",
        "",
        "use super::charset::{CharsetEntry, DecoderData};",
        "use super::charset_multibyte::ByteOrder;",
        "",
        ""));

    Map<String, Integer> tableIds = new LinkedHashMap<>();
    List<int[]> tables = new ArrayList<>();
    StringBuilder entries = new StringBuilder();
    entries.append("/// Every charset of `Charset.availableCharsets()`, in its order.\n");
    entries.append("pub(super) static CHARSETS: &[CharsetEntry] = &[\n");
    int singleByte = 0;
    List<Charset> machines = new ArrayList<>();
    List<String> algorithmicSeen = new ArrayList<>();
    for (Charset cs : Charset.availableCharsets().values()) {
      for (String alias : withName(cs)) {
        check(Charset.forName(alias).equals(cs), "forName(" + alias + ")");
        check(Charset.forName(alias.toUpperCase(Locale.ROOT)).equals(cs), "upper " + alias);
        check(Charset.forName(alias.toLowerCase(Locale.ROOT)).equals(cs), "lower " + alias);
      }
      String decoder;
      if (cs.name().equals("UTF-8")) {
        decoder = "DecoderData::Utf8";
      } else if (ALGORITHMIC.containsKey(cs.name())) {
        check(cs.newDecoder().getClass().getName().equals(ALGORITHMIC_CLASS.get(cs.name())),
            "decoder class of " + cs.name());
        decoder = "DecoderData::" + ALGORITHMIC.get(cs.name());
        algorithmicSeen.add(cs.name());
      } else {
        int[] table = singleByteTable(cs);
        if (table == null) {
          decoder = "DecoderData::Machine(" + machines.size() + ")";
          machines.add(cs);
        } else {
          singleByte++;
          String key = Arrays.toString(table);
          Integer id = tableIds.get(key);
          if (id == null) {
            id = tables.size();
            tableIds.put(key, id);
            tables.add(table);
          }
          decoder = "DecoderData::SingleByte(&TABLE_" + id + ")";
        }
      }
      entries.append("    CharsetEntry {\n");
      entries.append("        name: \"").append(cs.name()).append("\",\n");
      entries.append("        aliases: &[");
      boolean first = true;
      for (String alias : new TreeSet<>(cs.aliases())) {
        if (!first) entries.append(", ");
        entries.append('"').append(alias).append('"');
        first = false;
      }
      entries.append("],\n");
      entries.append("        decoder: ").append(decoder).append(",\n");
      entries.append("    },\n");
    }
    entries.append("];\n");
    check(singleByte > 0, "no single-byte charset");
    check(algorithmicSeen.size() == ALGORITHMIC.size(), "algorithmic charsets " + algorithmicSeen);
    out.append(entries);
    out.append("\n/// The item of `charset_multibyte.bin` that holds the machine of each\n");
    out.append("/// `DecoderData::Machine`, by index.\n");
    out.append("pub(super) static MACHINE_ITEMS: &[&str] = &[\n");
    for (Charset cs : machines) {
      out.append("    \"M:").append(cs.name()).append("\",\n");
    }
    out.append("];\n");
    for (int i = 0; i < tables.size(); i++) {
      out.append("\n/// A single-byte decoding table: the UTF-16 unit of each byte, or\n");
      out.append("/// `0x10000` (unmappable) or `0x10001` (malformed).\n");
      out.append("#[rustfmt::skip]\n");
      out.append("static TABLE_").append(i).append(": [u32; 256] = [\n");
      int[] table = tables.get(i);
      for (int row = 0; row < 256; row += 8) {
        out.append("   ");
        for (int b = row; b < row + 8; b++) {
          out.append(String.format(" 0x%05X,", table[b]));
        }
        out.append('\n');
      }
      out.append("];\n");
    }

    // The multi-byte data.
    Map<String, byte[]> items = new LinkedHashMap<>();
    for (Charset cs : machines) {
      long t0 = System.nanoTime();
      Machine m = new Explorer(cs).run();
      verifyMachine(cs, m);
      byte[] raw = m.serialize();
      items.put("M:" + cs.name(), raw);
      System.err.printf("%-20s states=%d nodes=%d extras=%d raw=%d (%.1fs)%n", cs.name(),
          m.roots.length, m.nodes.size(), m.extras.size(), raw.length,
          (System.nanoTime() - t0) / 1e9);
    }
    items.put("GB18030", gb18030());
    items.put("ISCII91", iscii91());
    writeBin(Path.of(args[1]), items);
    Files.writeString(Path.of(args[0]), out.toString());
  }

  static List<String> withName(Charset cs) {
    List<String> names = new ArrayList<>();
    names.add(cs.name());
    names.addAll(new TreeSet<>(cs.aliases()));
    return names;
  }

  static void check(boolean ok, String what) {
    if (!ok) throw new IllegalStateException("check failed: " + what);
  }

  static CharsetDecoder reporting(Charset cs) {
    return cs.newDecoder()
        .onMalformedInput(CodingErrorAction.REPORT)
        .onUnmappableCharacter(CodingErrorAction.REPORT);
  }

  /** The table of a single-byte charset, or null if the charset is not one. */
  static int[] singleByteTable(Charset cs) {
    CharsetDecoder probe;
    try {
      probe = cs.newDecoder();
    } catch (UnsupportedOperationException e) {
      return null;
    }
    if (probe.maxCharsPerByte() != 1.0f) return null;
    if (!probe.replacement().equals("�")) return null;
    int[] table = new int[256];
    for (int b = 0; b < 256; b++) {
      CharsetDecoder d = reporting(cs);
      CharBuffer out = CharBuffer.allocate(8);
      CoderResult cr = d.decode(ByteBuffer.wrap(new byte[] {(byte) b}), out, true);
      if (cr.isUnmappable()) {
        if (cr.length() != 1) return null;
        table[b] = UNMAPPABLE;
      } else if (cr.isMalformed()) {
        if (cr.length() != 1) return null;
        table[b] = MALFORMED;
      } else {
        if (!cr.isUnderflow()) return null;
        if (!d.flush(out).isUnderflow()) return null;
        out.flip();
        if (out.remaining() != 1) return null;
        table[b] = out.get();
      }
    }
    // a stateless table must decode a long sequence as the decoder does
    byte[] bytes = new byte[256 + 8192];
    for (int b = 0; b < 256; b++) bytes[b] = (byte) b;
    long seed = 42;
    for (int i = 256; i < bytes.length; i++) {
      seed = seed * 6364136223846793005L + 1442695040888963407L;
      bytes[i] = (byte) (seed >>> 56);
    }
    CharsetDecoder d =
        cs.newDecoder()
            .onMalformedInput(CodingErrorAction.REPLACE)
            .onUnmappableCharacter(CodingErrorAction.REPLACE);
    String decoded;
    try {
      decoded = d.decode(ByteBuffer.wrap(bytes)).toString();
    } catch (Exception e) {
      return null;
    }
    StringBuilder expected = new StringBuilder();
    for (byte b : bytes) {
      int v = table[b & 0xff];
      expected.append(v >= UNMAPPABLE ? '�' : (char) v);
    }
    if (!decoded.equals(expected.toString())) return null;
    return table;
  }

  // ---------------------------------------------------------------- machines

  /** What the decoder decides at the end of a byte sequence. */
  static final class Leaf {
    static final int EMIT = 0, MALFORMED = 1, UNMAPPABLE = 2, SHIFT = 3;
    int kind;
    int consumed; // EMIT, SHIFT: the bytes the decision consumes
    char[] units = new char[0]; // EMIT
    int length; // MALFORMED, UNMAPPABLE
    boolean overflowFirst; // the decoder reports OVERFLOW first when the output is full
    int next = -1; // the state after the decision
  }

  /** A trie node: a child node or a leaf per byte. */
  static final class Node {
    final Object[] e = new Object[256];
    boolean overflowFirst; // input ending inside this node: OVERFLOW first when the output is full
  }

  /** The outcome of decoding a byte sequence from a state, the output holding `cap` units. */
  static final class Outcome {
    CoderResult cr;
    int consumed;
    char[] out;
  }

  /**
   * A step of the way to a state: `bytes` decoded whole, malformed input and unmappable
   * characters replaced (skipped); or, when `error` is not 0, decoded until the decoder reports
   * malformed or unmappable input of length `error` at their start (the bytes after the error
   * are the lookahead the decoder needed, not decoded).
   */
  record Step(byte[] bytes, int error) {}

  /** A decoder of `cs` in the state `recipe` leaves it in, set to REPORT. */
  static CharsetDecoder reach(Charset cs, List<Step> recipe) {
    CharsetDecoder d = cs.newDecoder();
    for (Step step : recipe) {
      CodingErrorAction a = step.error() == 0 ? CodingErrorAction.REPLACE : CodingErrorAction.REPORT;
      d.onMalformedInput(a).onUnmappableCharacter(a);
      ByteBuffer p = ByteBuffer.wrap(step.bytes());
      CharBuffer sink = CharBuffer.allocate(step.bytes().length * 2 + 16);
      CoderResult r = d.decode(p, sink, false);
      if (step.error() == 0) {
        check(r.isUnderflow() && !p.hasRemaining(), cs + ": step " + Arrays.toString(step.bytes()));
      } else {
        check(r.isError() && r.length() == step.error() && p.position() == 0,
            cs + ": error step " + Arrays.toString(step.bytes()));
      }
    }
    return d.onMalformedInput(CodingErrorAction.REPORT)
        .onUnmappableCharacter(CodingErrorAction.REPORT);
  }

  static List<Step> then(List<Step> recipe, byte[] bytes, int error) {
    List<Step> r = new ArrayList<>(recipe);
    r.add(new Step(bytes, error));
    return r;
  }

  static Outcome probe(Charset cs, List<Step> recipe, byte[] seq, int cap) {
    CharsetDecoder d = reach(cs, recipe);
    ByteBuffer in = ByteBuffer.wrap(seq);
    CharBuffer out = CharBuffer.allocate(cap);
    Outcome o = new Outcome();
    o.cr = d.decode(in, out, false);
    o.consumed = in.position();
    out.flip();
    o.out = new char[out.remaining()];
    out.get(o.out);
    return o;
  }

  static byte[] append(byte[] a, int b) {
    byte[] r = Arrays.copyOf(a, a.length + 1);
    r[a.length] = (byte) b;
    return r;
  }

  static byte[] concat(byte[] a, byte[] b, int bLen) {
    byte[] r = Arrays.copyOf(a, a.length + bLen);
    System.arraycopy(b, 0, r, a.length, bLen);
    return r;
  }

  static final int MAX_DEPTH = 8;
  static final int MAX_STATES = 1024;

  /**
   * The mutable state of a decoder: its non-static, non-final primitive fields (the shift state
   * of the stateful decoders; arrays, strings and objects are scratch buffers or lazily cached
   * tables). Read by reflection, which needs --add-opens for the JDK's charset packages.
   */
  static final class StateReader {
    final List<Field> fields = new ArrayList<>();

    StateReader(Class<?> c) {
      for (; c != CharsetDecoder.class; c = c.getSuperclass()) {
        for (Field f : c.getDeclaredFields()) {
          int m = f.getModifiers();
          if (Modifier.isStatic(m) || Modifier.isFinal(m)) continue;
          if (!f.getType().isPrimitive()) continue;
          f.setAccessible(true);
          fields.add(f);
        }
      }
    }

    String key(CharsetDecoder d) {
      StringBuilder sb = new StringBuilder();
      for (Field f : fields) {
        try {
          sb.append(f.getName()).append('=').append(f.get(d)).append(';');
        } catch (IllegalAccessException e) {
          throw new IllegalStateException(e);
        }
      }
      return sb.toString();
    }
  }

  /** Explores the states of a charset's decoder. */
  static final class Explorer {
    final Charset cs;
    final StateReader reader;
    final List<Node> roots = new ArrayList<>();
    final List<List<Step>> recipes = new ArrayList<>();
    final Map<String, Integer> byKey = new HashMap<>();
    final ArrayDeque<Integer> pending = new ArrayDeque<>();

    Explorer(Charset cs) {
      this.cs = cs;
      this.reader = new StateReader(cs.newDecoder().getClass());
    }

    Machine run() {
      state(List.of());
      while (!pending.isEmpty()) {
        int id = pending.poll();
        roots.set(id, node(id, recipes.get(id), new byte[0]));
      }
      return new Machine(this);
    }

    /** The id of the state `recipe` reaches, a new state being queued for exploration. */
    int state(List<Step> recipe) {
      String key = reader.key(reach(cs, recipe));
      Integer id = byKey.get(key);
      if (id != null) return id;
      id = roots.size();
      check(id < MAX_STATES, cs + ": too many states");
      byKey.put(key, id);
      roots.add(null);
      recipes.add(recipe);
      pending.add(id);
      return id;
    }

    Node node(int state, List<Step> prefix, byte[] seq) {
      check(seq.length < MAX_DEPTH, cs + ": sequence too long " + Arrays.toString(seq));
      Node n = new Node();
      if (seq.length > 0) {
        Outcome o = probe(cs, prefix, seq, 0);
        check(o.consumed == 0 && o.out.length == 0, cs + ": branch at cap 0");
        if (o.cr.isOverflow()) n.overflowFirst = true;
        else check(o.cr.isUnderflow(), cs + ": branch at cap 0 " + o.cr);
      }
      for (int b = 0; b < 256; b++) {
        byte[] s = append(seq, b);
        Outcome o = probe(cs, prefix, s, 1);
        if (o.cr.isUnderflow() && o.consumed == 0 && o.out.length == 0) {
          n.e[b] = node(state, prefix, s);
          continue;
        }
        Leaf leaf = new Leaf();
        if (o.cr.isOverflow() && o.consumed == 0 && o.out.length == 0) {
          o = probe(cs, prefix, s, 2);
          check(o.out.length == 2 && o.consumed > 0, cs + ": two units at " + Arrays.toString(s));
        }
        List<Step> after;
        if (o.out.length > 0) {
          leaf.kind = Leaf.EMIT;
          leaf.consumed = o.consumed;
          leaf.units = o.out;
          leaf.overflowFirst = true;
          after = then(prefix, Arrays.copyOf(s, o.consumed), 0);
        } else if (o.consumed > 0) {
          leaf.kind = Leaf.SHIFT;
          leaf.consumed = o.consumed;
          after = then(prefix, Arrays.copyOf(s, o.consumed), 0);
        } else {
          check(o.cr.isError(), cs + ": unexpected " + o.cr + " at " + Arrays.toString(s));
          leaf.kind = o.cr.isMalformed() ? Leaf.MALFORMED : Leaf.UNMAPPABLE;
          leaf.length = o.cr.length();
          check(leaf.length <= s.length, cs + ": error longer than its input");
          after = then(prefix, s, leaf.length);
        }
        if (leaf.kind != Leaf.EMIT) {
          Outcome z = probe(cs, prefix, s, 0);
          if (z.cr.isOverflow()) {
            check(z.consumed == 0, cs + ": overflow after consuming at " + Arrays.toString(s));
            leaf.overflowFirst = true;
          } else {
            check(z.consumed == o.consumed && z.cr.toString().equals(o.cr.toString()),
                cs + ": cap 0 differs at " + Arrays.toString(s));
          }
        }
        leaf.next = reader.fields.isEmpty() ? state : state(after);
        n.e[b] = leaf;
      }
      return n;
    }
  }

  // Entry encoding (u32): tag in bits 31-30.
  //   0: child node index
  //   1: emit one unit (bits 0-15), consuming bits 16-19 bytes, OVERFLOW first
  //   2: error, length in bits 0-7, unmappable bit 8, OVERFLOW-first bit 9
  //   3: extra leaf index
  // The plain tags 1 and 2 keep the state; an extra leaf names the next state.
  // Extra leaf: [consumed, units (0-2), unit 1, unit 2, next state, OVERFLOW first,
  //   error (0 none, 1 malformed, 2 unmappable), error length].
  static final int TAG_NODE = 0, TAG_EMIT1 = 1, TAG_ERROR = 2, TAG_EXTRA = 3;

  /** A machine as the emulation reads it. */
  static final class Machine {
    int[] roots;
    final List<int[]> nodes = new ArrayList<>(); // [flags, lo, hi, default, entries lo..hi]
    final List<int[]> extras = new ArrayList<>(); // see the extra leaf above

    Machine(Explorer x) {
      Map<String, Integer> nodeIds = new HashMap<>();
      Map<String, Integer> extraIds = new HashMap<>();
      roots = new int[x.roots.size()];
      for (int s = 0; s < roots.length; s++) {
        roots[s] = encode(s, x.roots.get(s), nodeIds, extraIds);
      }
    }

    int encode(int state, Node n, Map<String, Integer> nodeIds, Map<String, Integer> extraIds) {
      int[] es = new int[256];
      for (int b = 0; b < 256; b++) {
        Object o = n.e[b];
        if (o instanceof Node c) {
          es[b] = (TAG_NODE << 30) | encode(state, c, nodeIds, extraIds);
        } else {
          es[b] = encodeLeaf(state, (Leaf) o, extraIds);
        }
      }
      Map<Integer, Integer> counts = new HashMap<>();
      for (int e : es) counts.merge(e, 1, Integer::sum);
      int def = es[0];
      for (Map.Entry<Integer, Integer> c : counts.entrySet()) {
        if (c.getValue() > counts.get(def)
            || (c.getValue().equals(counts.get(def)) && c.getKey() < def)) {
          def = c.getKey();
        }
      }
      int lo = 0;
      while (lo < 256 && es[lo] == def) lo++;
      int hi = 255;
      while (hi >= lo && es[hi] == def) hi--;
      int[] rec;
      if (lo > hi) {
        rec = new int[] {n.overflowFirst ? 1 : 0, 1, 0, def};
      } else {
        rec = new int[4 + hi - lo + 1];
        rec[0] = n.overflowFirst ? 1 : 0;
        rec[1] = lo;
        rec[2] = hi;
        rec[3] = def;
        System.arraycopy(es, lo, rec, 4, hi - lo + 1);
      }
      String key = Arrays.toString(rec);
      Integer id = nodeIds.get(key);
      if (id == null) {
        id = nodes.size();
        nodeIds.put(key, id);
        nodes.add(rec);
      }
      return id;
    }

    int encodeLeaf(int state, Leaf l, Map<String, Integer> extraIds) {
      boolean same = l.next == state;
      int of0 = l.overflowFirst ? 1 : 0;
      switch (l.kind) {
        case Leaf.MALFORMED:
        case Leaf.UNMAPPABLE:
          check(l.length > 0 && l.length < 256, "error length");
          int unm = l.kind == Leaf.UNMAPPABLE ? 1 : 0;
          if (same) return (TAG_ERROR << 30) | l.length | (unm << 8) | (of0 << 9);
          return (TAG_EXTRA << 30)
              | extra(new int[] {0, 0, 0, 0, l.next, of0, 1 + unm, l.length}, extraIds);
        case Leaf.EMIT:
          check(l.consumed < 16, "consumed");
          if (l.units.length == 1 && same) {
            return (TAG_EMIT1 << 30) | l.units[0] | (l.consumed << 16);
          }
          return (TAG_EXTRA << 30) | extra(new int[] {l.consumed, l.units.length, l.units[0],
              l.units.length > 1 ? l.units[1] : 0, l.next, 1, 0, 0}, extraIds);
        default:
          return (TAG_EXTRA << 30)
              | extra(new int[] {l.consumed, 0, 0, 0, l.next, of0, 0, 0}, extraIds);
      }
    }

    int extra(int[] rec, Map<String, Integer> extraIds) {
      String key = Arrays.toString(rec);
      Integer id = extraIds.get(key);
      if (id == null) {
        id = extras.size();
        extraIds.put(key, id);
        extras.add(rec);
      }
      return id;
    }

    int entry(int node, int b) {
      int[] rec = nodes.get(node);
      return (b < rec[1] || b > rec[2]) ? rec[3] : rec[4 + b - rec[1]];
    }

    /**
     * The machine's decodeLoop: decodes src[sp..sl) into dst[dp..dl), as the emulation does;
     * returns the result, sets pos[0] (sp), pos[1] (dp) and state[0].
     */
    CoderResult decodeLoop(byte[] src, int[] pos, int sl, char[] dst, int dl, int[] state) {
      int sp = pos[0], dp = pos[1];
      try {
        while (true) {
          if (sp >= sl) return CoderResult.UNDERFLOW;
          int node = roots[state[0]];
          int p = sp;
          while (true) {
            if (p >= sl) {
              if (dp >= dl && (nodes.get(node)[0] & 1) != 0) return CoderResult.OVERFLOW;
              return CoderResult.UNDERFLOW;
            }
            int e = entry(node, src[p++] & 0xff);
            int tag = e >>> 30;
            if (tag == TAG_NODE) {
              node = e & 0x3FFFFFFF;
              continue;
            }
            if (tag == TAG_EMIT1) {
              if (dp >= dl) return CoderResult.OVERFLOW;
              dst[dp++] = (char) (e & 0xFFFF);
              sp += (e >>> 16) & 0xF;
              break;
            }
            if (tag == TAG_ERROR) {
              if (dp >= dl && (e & (1 << 9)) != 0) return CoderResult.OVERFLOW;
              int length = e & 0xFF;
              return (e & (1 << 8)) != 0
                  ? CoderResult.unmappableForLength(length)
                  : CoderResult.malformedForLength(length);
            }
            int[] x = extras.get(e & 0x3FFFFFFF);
            if (x[6] != 0) {
              if (dp >= dl && x[5] != 0) return CoderResult.OVERFLOW;
              state[0] = x[4];
              return x[6] == 2
                  ? CoderResult.unmappableForLength(x[7])
                  : CoderResult.malformedForLength(x[7]);
            }
            if (x[1] > 0) {
              if (dl - dp < x[1]) return CoderResult.OVERFLOW;
              dst[dp++] = (char) x[2];
              if (x[1] > 1) dst[dp++] = (char) x[3];
            } else if (dp >= dl && x[5] != 0) {
              return CoderResult.OVERFLOW;
            }
            state[0] = x[4];
            sp += x[0];
            break;
          }
        }
      } finally {
        pos[0] = sp;
        pos[1] = dp;
      }
    }

    byte[] serialize() throws IOException {
      ByteArrayOutputStream bos = new ByteArrayOutputStream();
      DataOutputStream out = new DataOutputStream(bos);
      out.writeInt(roots.length);
      for (int r : roots) out.writeInt(r);
      out.writeInt(nodes.size());
      // node headers, then the entries of every node as byte planes of the
      // differences between consecutive entries (zlib compresses runs of
      // consecutive units to runs of equal bytes)
      int total = 0;
      for (int[] rec : nodes) {
        out.writeByte(rec[0]);
        out.writeByte(rec[1]);
        out.writeByte(rec[2]);
        out.writeInt(rec[3]);
        total += rec.length - 4;
      }
      int[] all = new int[total];
      int k = 0;
      for (int[] rec : nodes) {
        for (int i = 4; i < rec.length; i++) all[k++] = rec[i];
      }
      writePlanes(out, all);
      out.writeInt(extras.size());
      for (int[] x : extras) {
        out.writeByte(x[0]);
        out.writeByte(x[1]);
        out.writeShort(x[2]);
        out.writeShort(x[3]);
        out.writeShort(x[4]);
        out.writeByte(x[5]);
        out.writeByte(x[6]);
        out.writeByte(x[7]);
      }
      out.flush();
      return bos.toByteArray();
    }
  }

  /** Writes `values` as four byte planes (low byte first) of their successive differences. */
  static void writePlanes(DataOutputStream out, int[] values) throws IOException {
    int[] deltas = new int[values.length];
    int prev = 0;
    for (int i = 0; i < values.length; i++) {
      deltas[i] = values[i] - prev;
      prev = values[i];
    }
    for (int shift = 0; shift < 32; shift += 8) {
      for (int d : deltas) out.writeByte(d >>> shift);
    }
  }

  // ------------------------------------------------------- the verification

  /** A decoder under test, called as CharsetDecoder.decode(in, out, endOfInput) is. */
  interface Dec {
    CoderResult decode(ByteBuffer in, CharBuffer out, boolean endOfInput);
  }

  static Dec jdk(Charset cs) {
    CharsetDecoder d = reporting(cs);
    return d::decode;
  }

  /** The machine behind CharsetDecoder.decode's REPORT logic. */
  static Dec mirror(Machine m) {
    int[] state = {0};
    return (in, out, endOfInput) -> {
      byte[] src = in.array(); // the harness wraps its array at offset 0
      char[] dst = new char[out.capacity()];
      int[] pos = {in.position(), out.position()};
      CoderResult cr = m.decodeLoop(src, pos, in.limit(), dst, out.limit(), state);
      int dp0 = out.position();
      out.put(dst, dp0, pos[1] - dp0);
      in.position(pos[0]);
      if (cr.isUnderflow() && endOfInput && in.hasRemaining()) {
        cr = CoderResult.malformedForLength(in.remaining());
      }
      return cr;
    };
  }

  /** The log of decoding `data` in random chunks into random small buffers, skipping errors. */
  static List<String> harness(Dec dec, byte[] data, long seed) {
    Random r = new Random(seed);
    List<String> log = new ArrayList<>();
    int avail = 0, pos = 0;
    boolean eoi = false;
    for (int calls = 0; ; calls++) {
      check(calls < data.length * 8 + 1000, "harness does not end");
      if (!eoi) {
        avail = Math.min(data.length, avail + r.nextInt(9));
        eoi = avail == data.length && r.nextInt(3) == 0;
      }
      ByteBuffer in = ByteBuffer.wrap(data, 0, avail);
      in.position(pos);
      CharBuffer out = CharBuffer.allocate(r.nextInt(4));
      CoderResult cr = dec.decode(in, out, eoi);
      out.flip();
      log.add(cr + "@" + in.position() + ":" + out);
      pos = in.position();
      if (cr.isError()) pos += cr.length();
      if (eoi && cr.isUnderflow() && pos == avail) break;
    }
    return log;
  }

  static void compare(String what, List<String> a, List<String> b) {
    int n = Math.min(a.size(), b.size());
    for (int i = 0; i < n; i++) {
      if (!a.get(i).equals(b.get(i))) {
        throw new IllegalStateException(what + ": call " + i + " jdk=" + a.get(i)
            + " model=" + b.get(i) + " previous=" + (i > 0 ? a.get(i - 1) : "-"));
      }
    }
    check(a.size() == b.size(), what + ": call count " + a.size() + " vs " + b.size());
  }

  /** Streams that walk the machine's tries, with random bytes between. */
  static byte[] stream(Machine m, long seed, int length) {
    Random r = new Random(seed);
    ByteArrayOutputStream out = new ByteArrayOutputStream();
    int state = 0;
    while (out.size() < length) {
      if (r.nextInt(10) == 0) {
        out.write(r.nextInt(256));
        continue;
      }
      int node = m.roots[state];
      while (true) {
        // prefer children, then characters, then shifts, then errors
        List<Integer> kids = new ArrayList<>(), emits = new ArrayList<>(),
            shifts = new ArrayList<>(), errors = new ArrayList<>();
        for (int b = 0; b < 256; b++) {
          int e = m.entry(node, b);
          int tag = e >>> 30;
          if (tag == TAG_NODE) kids.add(b);
          else if (tag == TAG_EMIT1) emits.add(b);
          else if (tag == TAG_ERROR) errors.add(b);
          else if (m.extras.get(e & 0x3FFFFFFF)[6] != 0) errors.add(b);
          else if (m.extras.get(e & 0x3FFFFFFF)[1] > 0) emits.add(b);
          else shifts.add(b);
        }
        int pick = r.nextInt(20);
        List<Integer> from;
        if (pick < 9 && !kids.isEmpty()) from = kids;
        else if (pick < 16 && !emits.isEmpty()) from = emits;
        else if (pick < 19 && !shifts.isEmpty()) from = shifts;
        else if (!errors.isEmpty()) from = errors;
        else if (!emits.isEmpty()) from = emits;
        else if (!kids.isEmpty()) from = kids;
        else from = shifts;
        int b = from.get(r.nextInt(from.size()));
        out.write(b);
        int e = m.entry(node, b);
        int tag = e >>> 30;
        if (tag == TAG_NODE) {
          node = e & 0x3FFFFFFF;
          continue;
        }
        if (tag == TAG_EXTRA) {
          state = m.extras.get(e & 0x3FFFFFFF)[4];
        }
        break;
      }
    }
    return out.toByteArray();
  }

  static void verifyMachine(Charset cs, Machine m) {
    for (int i = 0; i < 6; i++) {
      byte[] data = stream(m, 1000 + i, 20000);
      long hs = 77 + i;
      compare(cs.name() + " stream " + i, harness(jdk(cs), data, hs), harness(mirror(m), data, hs));
    }
    Random r = new Random(5);
    for (int i = 0; i < 4; i++) {
      byte[] data = new byte[20000];
      r.nextBytes(data);
      compare(cs.name() + " random " + i, harness(jdk(cs), data, i), harness(mirror(m), data, i));
    }
  }

  // ---------------------------------------------------------------- GB18030

  /** The decoding of one sequence by a fresh decoder: a unit string, or "M<n>"/"U<n>". */
  static String decodeOne(Charset cs, byte[] seq) {
    CharsetDecoder d = reporting(cs);
    CharBuffer out = CharBuffer.allocate(8);
    ByteBuffer in = ByteBuffer.wrap(seq);
    CoderResult cr = d.decode(in, out, true);
    if (cr.isError()) return (cr.isMalformed() ? "M" : "U") + cr.length() + "@" + in.position();
    check(cr.isUnderflow() && !in.hasRemaining(), "decodeOne");
    check(d.flush(out).isUnderflow(), "flush");
    out.flip();
    return out.toString();
  }

  static byte[] gb18030() throws IOException {
    Charset cs = Charset.forName("GB18030");
    // two bytes: b1 0x81-0xFE, b2 0x40-0xFE (0x7F is malformed input)
    char[] dbl = new char[126 * 191];
    for (int b1 = 0x81; b1 <= 0xFE; b1++) {
      for (int b2 = 0x40; b2 <= 0xFE; b2++) {
        String s = decodeOne(cs, new byte[] {(byte) b1, (byte) b2});
        int i = (b1 - 0x81) * 191 + (b2 - 0x40);
        if (b2 == 0x7F) {
          check(s.equals("M2@0"), "GB18030 0x7F");
          dbl[i] = 0;
        } else {
          check(s.length() == 1, "GB18030 double " + b1 + " " + b2 + " " + s);
          dbl[i] = s.charAt(0);
        }
      }
    }
    // four bytes in the Basic Multilingual Plane: linear offsets 0 to 0x99FB
    char[] four = new char[0x99FC];
    for (int off = 0; off < four.length; off++) {
      String s = decodeOne(cs, gbFour(off));
      check(s.length() == 1, "GB18030 four " + off + " " + s);
      four[off] = s.charAt(0);
    }
    Gb18030Mirror mirror = new Gb18030Mirror(dbl, four);
    // every sequence of one, two and four bytes, decoded alone
    for (int b1 = 0; b1 < 256; b1++) {
      checkGb(cs, mirror, new byte[] {(byte) b1});
      for (int b2 = 0; b2 < 256; b2++) {
        checkGb(cs, mirror, new byte[] {(byte) b1, (byte) b2});
      }
    }
    for (int b1 = 0x81; b1 <= 0xFE; b1++) {
      for (int b2 = 0x30; b2 <= 0x39; b2++) {
        for (int b3 = 0; b3 < 256; b3++) {
          checkGb(cs, mirror, new byte[] {(byte) b1, (byte) b2, (byte) b3});
          if (b3 < 0x81 || b3 > 0xFE) continue;
          // every valid fourth byte; every fourth byte for one prefix in 37
          boolean all = (b1 + b2 + b3) % 37 == 0;
          for (int b4 = 0; b4 < 256; b4++) {
            if (all || (b4 >= 0x2F && b4 <= 0x3A) || b4 == 0 || b4 == 0x80 || b4 == 0xFF) {
              checkGb(cs, mirror, new byte[] {(byte) b1, (byte) b2, (byte) b3, (byte) b4});
            }
          }
        }
      }
    }
    Random r = new Random(18030);
    for (int i = 0; i < 8; i++) {
      byte[] data = gbStream(r, 20000);
      compare("GB18030 stream " + i, harness(jdk(cs), data, i), harness(mirror.dec(), data, i));
    }
    ByteArrayOutputStream bos = new ByteArrayOutputStream();
    DataOutputStream out = new DataOutputStream(bos);
    int[] values = new int[dbl.length + four.length];
    for (int i = 0; i < dbl.length; i++) values[i] = dbl[i];
    for (int i = 0; i < four.length; i++) values[dbl.length + i] = four[i];
    out.writeInt(dbl.length);
    out.writeInt(four.length);
    writePlanes16(out, values);
    out.flush();
    return bos.toByteArray();
  }

  static byte[] gbFour(int off) {
    int b4 = off % 10 + 0x30;
    off /= 10;
    int b3 = off % 126 + 0x81;
    off /= 126;
    int b2 = off % 10 + 0x30;
    off /= 10;
    int b1 = off + 0x81;
    return new byte[] {(byte) b1, (byte) b2, (byte) b3, (byte) b4};
  }

  static byte[] gbStream(Random r, int length) {
    ByteArrayOutputStream out = new ByteArrayOutputStream();
    while (out.size() < length) {
      int k = r.nextInt(10);
      if (k < 2) {
        out.write(r.nextInt(256));
      } else if (k < 4) {
        out.write(r.nextInt(128));
      } else if (k < 7) {
        out.write(0x81 + r.nextInt(126));
        out.write(0x40 + r.nextInt(191));
      } else {
        out.write(0x81 + r.nextInt(126));
        out.write(0x30 + r.nextInt(10));
        out.write(r.nextInt(8) == 0 ? r.nextInt(256) : 0x81 + r.nextInt(126));
        out.write(r.nextInt(8) == 0 ? r.nextInt(256) : 0x30 + r.nextInt(10));
      }
    }
    return out.toByteArray();
  }

  static void checkGb(Charset cs, Gb18030Mirror m, byte[] seq) {
    for (int cap = 0; cap <= 2; cap++) {
      CharsetDecoder d = reporting(cs);
      ByteBuffer in = ByteBuffer.wrap(seq);
      CharBuffer out = CharBuffer.allocate(cap);
      CoderResult cr = d.decode(in, out, false);
      out.flip();
      String a = cr + "@" + in.position() + ":" + out;
      ByteBuffer in2 = ByteBuffer.wrap(seq);
      CharBuffer out2 = CharBuffer.allocate(cap);
      CoderResult cr2 = m.dec().decode(in2, out2, false);
      out2.flip();
      String b = cr2 + "@" + in2.position() + ":" + out2;
      check(a.equals(b), "GB18030 " + Arrays.toString(seq) + " cap " + cap + ": " + a + " vs " + b);
    }
  }

  /** The GB18030 decoder of the emulation, over the generated tables. */
  static final class Gb18030Mirror {
    final char[] dbl, four;

    Gb18030Mirror(char[] dbl, char[] four) {
      this.dbl = dbl;
      this.four = four;
    }

    Dec dec() {
      return (in, out, eoi) -> {
        CoderResult cr = loop(in, out);
        if (cr.isUnderflow() && eoi && in.hasRemaining()) {
          cr = CoderResult.malformedForLength(in.remaining());
        }
        return cr;
      };
    }

    CoderResult loop(ByteBuffer in, CharBuffer out) {
      while (in.hasRemaining()) {
        int sp = in.position(), sl = in.limit();
        int b1 = in.get(sp) & 0xff;
        int size = 1;
        char c1, c2 = 0;
        int n = 1;
        if (b1 < 0x80) {
          c1 = (char) b1;
        } else if (b1 < 0x81 || b1 > 0xfe) {
          return CoderResult.malformedForLength(1);
        } else {
          if (sl - sp < 2) return CoderResult.UNDERFLOW;
          int b2 = in.get(sp + 1) & 0xff;
          size = 2;
          if (b2 < 0x30) return CoderResult.malformedForLength(1);
          if (b2 <= 0x39) {
            if (sl - sp < 4) return CoderResult.UNDERFLOW;
            int b3 = in.get(sp + 2) & 0xff;
            if (b3 < 0x81 || b3 > 0xfe) return CoderResult.malformedForLength(3);
            int b4 = in.get(sp + 3) & 0xff;
            size = 4;
            if (b4 < 0x30 || b4 > 0x39) return CoderResult.malformedForLength(4);
            if (!out.hasRemaining()) return CoderResult.OVERFLOW;
            int off = (((b1 - 0x81) * 10 + (b2 - 0x30)) * 126 + b3 - 0x81) * 10 + b4 - 0x30;
            if (off < four.length) {
              c1 = four[off];
            } else if (off >= 0x2E248 && off < 0x12E248) {
              int cp = off - 0x1e248;
              if (out.remaining() < 2) return CoderResult.OVERFLOW;
              c1 = (char) ((cp - 0x10000) / 0x400 + 0xD800);
              c2 = (char) ((cp - 0x10000) % 0x400 + 0xDC00);
              n = 2;
            } else {
              return CoderResult.malformedForLength(4);
            }
          } else if (b2 == 0x7f || b2 == 0xff || b2 < 0x40) {
            return CoderResult.malformedForLength(2);
          } else {
            c1 = dbl[(b1 - 0x81) * 191 + (b2 - 0x40)];
          }
        }
        if (!out.hasRemaining()) return CoderResult.OVERFLOW;
        out.put(c1);
        if (n == 2) out.put(c2);
        in.position(sp + size);
      }
      return CoderResult.UNDERFLOW;
    }
  }

  /** Writes `values` (16-bit) as two byte planes of their successive differences. */
  static void writePlanes16(DataOutputStream out, int[] values) throws IOException {
    int prev = 0;
    int[] deltas = new int[values.length];
    for (int i = 0; i < values.length; i++) {
      deltas[i] = values[i] - prev;
      prev = values[i];
    }
    for (int d : deltas) out.writeByte(d);
    for (int d : deltas) out.writeByte(d >>> 8);
  }

  // ---------------------------------------------------------------- ISCII91

  static final char NUKTA = '़', HALANT = '्', INVALID = '￿';

  static byte[] iscii91() throws IOException {
    Charset cs = Charset.forName("x-ISCII91");
    char[] table = new char[256];
    for (int b = 0; b < 256; b++) {
      String s = decodeOne(cs, new byte[] {(byte) b});
      if (s.startsWith("U1@")) {
        table[b] = INVALID;
      } else {
        check(s.length() == 1, "ISCII91 byte " + b + " " + s);
        table[b] = s.charAt(0);
      }
    }
    Random r = new Random(91);
    for (int i = 0; i < 8; i++) {
      byte[] data = new byte[20000];
      for (int j = 0; j < data.length; j++) {
        data[j] = (byte) (r.nextInt(3) == 0 ? r.nextInt(256) : 0xA0 + r.nextInt(0x5B));
      }
      compare("ISCII91 stream " + i, harness(jdk(cs), data, i), harness(isciiMirror(table), data, i));
    }
    ByteArrayOutputStream bos = new ByteArrayOutputStream();
    DataOutputStream out = new DataOutputStream(bos);
    for (char c : table) out.writeShort(c);
    out.flush();
    return bos.toByteArray();
  }

  /** The ISCII91 decoder of the emulation, over the generated table. */
  static Dec isciiMirror(char[] table) {
    char[] context = {INVALID};
    boolean[] flushing = {false};
    return (in, out, eoi) -> {
      CoderResult cr = isciiLoop(table, context, flushing, in, out);
      if (cr.isUnderflow() && eoi && in.hasRemaining()) {
        cr = CoderResult.malformedForLength(in.remaining());
      }
      return cr;
    };
  }

  static CoderResult isciiLoop(char[] table, char[] ctx, boolean[] fl, ByteBuffer in, CharBuffer out) {
    while (in.hasRemaining()) {
      char cur = table[in.get(in.position()) & 0xff];
      if (ctx[0] == '�') {
        if (!out.hasRemaining()) return CoderResult.OVERFLOW;
        out.put('�');
        ctx[0] = INVALID;
        fl[0] = false;
        in.position(in.position() + 1);
        continue;
      }
      switch (cur) {
        case 'ँ', 'इ', 'ई', 'ऋ', 'ि', 'ी', 'ृ', '।' -> {
          if (fl[0]) {
            if (!out.hasRemaining()) return CoderResult.OVERFLOW;
            out.put(ctx[0]);
            ctx[0] = cur;
            in.position(in.position() + 1);
            continue;
          }
          ctx[0] = cur;
          fl[0] = true;
          in.position(in.position() + 1);
          continue;
        }
        case NUKTA -> {
          if (!out.hasRemaining()) return CoderResult.OVERFLOW;
          char mapped = switch (ctx[0]) {
            case 'ँ' -> 'ॐ';
            case 'इ' -> 'ऌ';
            case 'ई' -> 'ॡ';
            case 'ऋ' -> 'ॠ';
            case 'ि' -> 'ॢ';
            case 'ी' -> 'ॣ';
            case 'ृ' -> 'ॄ';
            case '।' -> 'ऽ';
            default -> 0;
          };
          if (mapped != 0) {
            out.put(mapped);
          } else {
            if (fl[0]) {
              out.put(ctx[0]);
              ctx[0] = cur;
              in.position(in.position() + 1);
              continue;
            }
            out.put(ctx[0] == HALANT ? '‍' : NUKTA);
          }
        }
        case HALANT -> {
          if (!out.hasRemaining()) return CoderResult.OVERFLOW;
          if (fl[0]) {
            out.put(ctx[0]);
            ctx[0] = cur;
            in.position(in.position() + 1);
            continue;
          }
          out.put(ctx[0] == HALANT ? '‌' : HALANT);
        }
        case INVALID -> {
          if (fl[0]) {
            if (!out.hasRemaining()) return CoderResult.OVERFLOW;
            out.put(ctx[0]);
            ctx[0] = cur;
            in.position(in.position() + 1);
            continue;
          }
          return CoderResult.unmappableForLength(1);
        }
        default -> {
          if (!out.hasRemaining()) return CoderResult.OVERFLOW;
          if (fl[0]) {
            out.put(ctx[0]);
            ctx[0] = cur;
            in.position(in.position() + 1);
            continue;
          }
          out.put(cur);
        }
      }
      ctx[0] = cur;
      fl[0] = false;
      in.position(in.position() + 1);
    }
    return CoderResult.UNDERFLOW;
  }

  // -------------------------------------------------------------- the file

  /**
   * The container: "RJCS", a u32 item count, per item its name (u16 length, UTF-8), the u32
   * offset and u32 length of its zlib stream and the u32 length of the inflated data, then the
   * streams; items with the same data share one stream.
   */
  static void writeBin(Path path, Map<String, byte[]> items) throws Exception {
    List<byte[]> streams = new ArrayList<>();
    Map<String, Integer> sameAs = new HashMap<>();
    List<Integer> streamOf = new ArrayList<>();
    for (byte[] raw : items.values()) {
      String key = Arrays.toString(MessageDigest.getInstance("SHA-256").digest(raw));
      Integer earlier = sameAs.get(key);
      if (earlier != null) {
        streamOf.add(earlier);
        continue;
      }
      sameAs.put(key, streams.size());
      streamOf.add(streams.size());
      Deflater d = new Deflater(Deflater.BEST_COMPRESSION);
      d.setInput(raw);
      d.finish();
      ByteArrayOutputStream z = new ByteArrayOutputStream();
      byte[] buf = new byte[65536];
      while (!d.finished()) z.write(buf, 0, d.deflate(buf));
      d.end();
      streams.add(z.toByteArray());
    }
    ByteArrayOutputStream head = new ByteArrayOutputStream();
    DataOutputStream h = new DataOutputStream(head);
    h.writeBytes("RJCS");
    h.writeInt(items.size());
    int headerSize = 8;
    for (String name : items.keySet()) {
      headerSize += 2 + name.getBytes(java.nio.charset.StandardCharsets.UTF_8).length + 12;
    }
    int[] offsets = new int[streams.size()];
    int offset = headerSize;
    for (int k = 0; k < streams.size(); k++) {
      offsets[k] = offset;
      offset += streams.get(k).length;
    }
    int i = 0;
    long rawTotal = 0;
    for (Map.Entry<String, byte[]> item : items.entrySet()) {
      byte[] name = item.getKey().getBytes(java.nio.charset.StandardCharsets.UTF_8);
      int k = streamOf.get(i);
      h.writeShort(name.length);
      h.write(name);
      h.writeInt(offsets[k]);
      h.writeInt(streams.get(k).length);
      h.writeInt(item.getValue().length);
      rawTotal += item.getValue().length;
      i++;
    }
    h.flush();
    check(head.size() == headerSize, "header size");
    ByteArrayOutputStream all = new ByteArrayOutputStream();
    all.write(head.toByteArray());
    for (byte[] s : streams) all.write(s);
    Files.write(path, all.toByteArray());
    System.err.printf(
        "charset_multibyte.bin: %d items (%d distinct), %d bytes inflated, %d bytes compressed%n",
        items.size(), streams.size(), rawTotal, all.size());
  }
}
