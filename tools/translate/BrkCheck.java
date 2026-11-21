import java.text.BreakIterator;
import java.util.*;
import java.util.function.IntPredicate;

/**
 * Checks the reading of JDK 25's forward break rules that crates/rucene-analysis-common/src/jdk/
 * break_iterator_rules.rs implements: Java models of the root word table, the sentence table and the
 * Thai word table (its rule-based ranges, before the dictionary splits them), with the JDK's
 * handleNext, against BreakIterator.getWordInstance(Locale.ROOT), getSentenceInstance(Locale.ROOT)
 * and getWordInstance(th), on pseudo-random strings over the rules' classes. Prints each difference
 * and the count; 0 expected. Not a test of the Rust code: it validates the derivation the Rust
 * follows. Run with JDK 25: java tools/translate/BrkCheck.java [seed] [maxLength]
 */
public class BrkCheck {
  static int ty(int c) { return Character.getType(c); }
  static boolean isL(int c) { int t = ty(c); return t >= 1 && t <= 5; }
  static boolean isN(int c) { int t = ty(c); return t >= 9 && t <= 11; }
  static boolean in(int c, int... cps) { for (int x : cps) if (x == c) return true; return false; }
  static boolean range(int c, int a, int b) { return c >= a && c <= b; }

  // ---------------- generic rule engine ----------------
  static abstract class Re {}
  static final class Cl extends Re { final int idx; Cl(int i) { idx = i; } } // -1 = any
  static final class Seq extends Re { final Re[] p; Seq(Re... p) { this.p = p; } }
  static final class Alt extends Re { final Re[] p; Alt(Re... p) { this.p = p; } }
  static final class Opt extends Re { final Re r; Opt(Re r) { this.r = r; } }
  static final class Star extends Re { final Re r; Star(Re r) { this.r = r; } }
  static final class Slash extends Re {}

  static final class Nfa {
    List<List<Integer>> eps = new ArrayList<>();
    List<int[]> edges = new ArrayList<>(); // per state: {class, target} or null
    List<Integer> flags = new ArrayList<>(); // 1 END, 2 LA
    int add() { eps.add(new ArrayList<>()); edges.add(null); flags.add(0); return eps.size() - 1; }
  }
  static final class Engine {
    final List<IntPredicate> classes;
    final IntPredicate ignoreSet;
    final Nfa nfa = new Nfa();
    final int start;
    final Map<BitSet, Integer> ids = new HashMap<>();
    final List<BitSet> sets = new ArrayList<>();
    final List<Map<Long, Integer>> trans = new ArrayList<>();
    final List<Integer> dflags = new ArrayList<>();
    final IntPredicate dictionary; // may be null

    Engine(List<IntPredicate> classes, IntPredicate ignoreSet, IntPredicate dictionary, Re... rules) {
      this.classes = classes; this.ignoreSet = ignoreSet; this.dictionary = dictionary;
      start = nfa.add();
      for (Re r : rules) {
        int s = nfa.add();
        nfa.eps.get(start).add(s);
        boolean[] slash = {false};
        int e = build(r, s, slash);
        nfa.flags.set(e, nfa.flags.get(e) | 1 | (slash[0] ? 2 : 0));
      }
      sets.add(new BitSet()); trans.add(new HashMap<>()); dflags.add(0); // STOP = 0
      BitSet st = new BitSet(); st.set(start); id(closure(st)); // START = 1
    }
    int build(Re r, int s, boolean[] slash) {
      if (r instanceof Cl c) { int t = nfa.add(); nfa.edges.set(s, new int[] {c.idx, t}); return t; }
      if (r instanceof Seq q) { int cur = s; for (Re x : q.p) {
          if (x instanceof Slash) { slash[0] = true; int t = nfa.add(); nfa.eps.get(cur).add(t); nfa.flags.set(t, 2); cur = t; continue; }
          int a = nfa.add(); nfa.eps.get(cur).add(a); cur = build(x, a, slash); } return cur; }
      if (r instanceof Alt a) { int end = nfa.add(); for (Re x : a.p) { int b = nfa.add(); nfa.eps.get(s).add(b); int e = build(x, b, slash); nfa.eps.get(e).add(end); } return end; }
      if (r instanceof Opt o) { int b = nfa.add(); nfa.eps.get(s).add(b); int e = build(o.r, b, slash); int end = nfa.add(); nfa.eps.get(e).add(end); nfa.eps.get(s).add(end); return end; }
      Star st = (Star) r; int loop = nfa.add(); nfa.eps.get(s).add(loop); int b = nfa.add(); nfa.eps.get(loop).add(b); int e = build(st.r, b, slash); nfa.eps.get(e).add(loop); return loop;
    }
    BitSet closure(BitSet s) {
      BitSet r = (BitSet) s.clone(); Deque<Integer> q = new ArrayDeque<>(); s.stream().forEach(q::add);
      while (!q.isEmpty()) { int x = q.poll(); for (int y : nfa.eps.get(x)) if (!r.get(y)) { r.set(y); q.add(y); } }
      return r;
    }
    int id(BitSet s) {
      if (s.isEmpty()) return 0;
      Integer i = ids.get(s); if (i != null) return i;
      int n = sets.size(); ids.put(s, n); sets.add(s); trans.add(new HashMap<>());
      int f = 0; for (int x = s.nextSetBit(0); x >= 0; x = s.nextSetBit(x + 1)) f |= nfa.flags.get(x);
      dflags.add(f); return n;
    }
    long mask(int c) { long m = 0; for (int i = 0; i < classes.size(); i++) if (classes.get(i).test(c)) m |= 1L << i; return m; }
    boolean ignore(int c) { return ignoreSet.test(c) && mask(c) == 0; }
    int next(int state, int c) {
      long m = mask(c);
      Integer t = trans.get(state).get(m); if (t != null) return t;
      BitSet out = new BitSet(); BitSet s = sets.get(state);
      for (int x = s.nextSetBit(0); x >= 0; x = s.nextSetBit(x + 1)) {
        int[] e = nfa.edges.get(x); if (e == null) continue;
        if (e[0] == -1 || (m & (1L << e[0])) != 0) out.set(e[1]);
      }
      int r = id(closure(out)); trans.get(state).put(m, r); return r;
    }
  }

  // JDK iterator helpers over a String
  static int cpAt(String s, int i) { char c = s.charAt(i); if (Character.isHighSurrogate(c) && i + 1 < s.length() && Character.isLowSurrogate(s.charAt(i + 1))) return Character.toCodePoint(c, s.charAt(i + 1)); return c; }
  static int count(String s, int i) { return cpAt(s, i) >= 0x10000 ? 2 : 1; }

  interface Dfa { int start(); int next(int state, int c); boolean ignore(int c); boolean end(int s); boolean la(int s); }

  static int dictCount;
  static IntPredicate dictPred;
  static int handleNext(Dfa d, String s, int i) {
    int len = s.length();
    if (i == len) return -1;
    int result = Math.min(i + count(s, i), len);
    int la = 0; int state = d.start();
    int pos = i; int c = cpAt(s, pos);
    while (c != -1 && state != 0) {
      if (!d.ignore(c)) { if (dictPred != null && dictPred.test(c)) dictCount++; state = d.next(state, c); }
      if (d.la(state)) { if (d.end(state)) result = la; else la = Math.min(pos + count(s, pos), len); }
      else if (d.end(state)) result = Math.min(pos + count(s, pos), len);
      if (pos == len || pos + count(s, pos) >= len) c = -1; else { pos += count(s, pos); c = cpAt(s, pos); }
    }
    if (c == -1 && la == len) result = la;
    return result;
  }
  static Dfa of(Engine e) { return new Dfa() {
    public int start() { return 1; } public int next(int st, int c) { return e.next(st, c); }
    public boolean ignore(int c) { return e.ignore(c); } public boolean end(int st) { return (e.dflags.get(st) & 1) != 0; }
    public boolean la(int st) { return (e.dflags.get(st) & 2) != 0; } }; }

  // ---------------- word rules (base) ----------------
  static Engine baseWord() {
    IntPredicate kanji = c -> c == 0x3005 || range(c, 0x4e00, 0x9fa5) || range(c, 0xf900, 0xfa2d);
    IntPredicate kata = c -> range(c, 0x30a1, 0x30fa) || c == 0x30fd || c == 0x30fe;
    IntPredicate hira = c -> range(c, 0x3041, 0x3094) || c == 0x309d || c == 0x309e;
    IntPredicate cjkd = c -> range(c, 0x3099, 0x309c) || c == 0x30fb || c == 0x30fc;
    IntPredicate enclosing = c -> ty(c) == 6 || ty(c) == 7;
    List<IntPredicate> cl = List.of(
      enclosing,                                                                    // 0
      c -> c == 0x964 || c == 0x965,                                               // 1 danda
      c -> (isL(c) || ty(c) == 8) && !(kanji.test(c) || kata.test(c) || hira.test(c) || cjkd.test(c)), // 2 letter-base
      BrkCheck::isN,                                                                // 3 digit-base
      c -> ty(c) == 20 || ty(c) == 23 || in(c, 0xad, 0x2027, '"', '\'', '.'),     // 4 mid-word
      c -> in(c, '"', '\'', ',', 0x66b, '.'),                                       // 5 mid-num
      c -> (ty(c) == 26 || c == '#' || c == '.') && c != 0xa2,                       // 6 pre-num
      c -> in(c, '%', '&', 0xa2, 0x66a, 0x2030, 0x2031),                           // 7 post-num
      c -> in(c, '\n', 0xc, 0x2028, 0x2029),                                         // 8 ls
      c -> ty(c) == 12 || c == '\t',                                                // 9 ws-base
      c -> c == '\r',                                                               // 10
      c -> kata.test(c) || cjkd.test(c),                                            // 11
      c -> hira.test(c) || cjkd.test(c),                                            // 12
      kanji,                                                                        // 13
      c -> !enclosing.test(c) && !(ty(c) == 15 || ty(c) == 16 || ty(c) == 13 || ty(c) == 14)); // 14 base
    Re enc = new Cl(0);
    Re let = new Seq(new Cl(2), new Star(enc));
    Re dgt = new Seq(new Cl(3), new Star(enc));
    Re ws = new Seq(new Cl(9), new Star(enc));
    Re word = new Seq(new Seq(let, new Star(let), new Star(new Seq(new Cl(4), let, new Star(let)))), new Opt(new Cl(1)));
    Re number = new Seq(dgt, new Star(dgt), new Star(new Seq(new Cl(5), dgt, new Star(dgt))));
    Re tail = new Opt(new Seq(number, new Opt(new Cl(7))));
    return new Engine(cl, c -> ty(c) == 16, null,
      new Cl(-1),
      new Seq(new Opt(word), new Star(new Seq(number, word)), tail),
      new Seq(new Cl(6), new Star(new Seq(number, word)), tail),
      new Seq(new Star(ws), new Opt(new Cl(10)), new Opt(new Cl(8))),
      new Star(new Cl(11)), new Star(new Cl(12)), new Star(new Cl(13)),
      new Seq(new Cl(14), enc, new Star(enc)));
  }

  static final IntPredicate THAI_DICT = c -> range(c, 0xe01, 0xe2e) || range(c, 0xe30, 0xe3a) || range(c, 0xe40, 0xe44) || range(c, 0xe47, 0xe4e);
  static Engine thaiWord() {
    IntPredicate kanji = c -> c == 0x3005 || range(c, 0x4e00, 0x9fa5) || range(c, 0xf900, 0xfa2d);
    IntPredicate kata = c -> range(c, 0x30a1, 0x30fa);
    IntPredicate hira = c -> range(c, 0x3041, 0x3094);
    IntPredicate cjkd = c -> range(c, 0x3099, 0x309c);
    IntPredicate ign = c -> (ty(c) == 6 || ty(c) == 7 || ty(c) == 16) && !THAI_DICT.test(c);
    List<IntPredicate> cl = List.of(
      THAI_DICT,                                                                   // 0
      c -> c == 0xe2f,                                                              // 1 paiyannoi
      c -> c == 0xe46,                                                              // 2 maiyamok
      c -> c == 0x964 || c == 0x965,                                               // 3 danda
      c -> (isL(c) || ty(c) == 8) && !(kanji.test(c) || kata.test(c) || hira.test(c) || cjkd.test(c) || THAI_DICT.test(c)), // 4 let
      BrkCheck::isN,                                                                // 5 dgt
      c -> ty(c) == 20 || in(c, 0xad, 0x2027, '"', '\'', '.'),                      // 6 mid-word
      c -> in(c, '"', '\'', ',', 0x66b, '.'),                                       // 7 mid-num
      c -> (ty(c) == 26 || c == '#' || c == '.') && c != 0xa2,                       // 8 pre-num
      c -> in(c, '%', '&', 0xa2, 0x66a, 0x2030, 0x2031),                           // 9 post-num
      c -> in(c, '\n', 0xc, 0x2028, 0x2029),                                         // 10 ls
      c -> ty(c) == 12 || c == '\t',                                                // 11 ws
      c -> c == '\r',                                                               // 12
      c -> c == 0xe25,                                                              // 13 lo ling
      c -> c != 0xe25 && !ign.test(c),                                              // 14 [^[ล<ignore>]]
      c -> c != 0xe2f && !ign.test(c),                                              // 15 [^[<paiyannoi><ignore>]]
      c -> kata.test(c) || cjkd.test(c),                                            // 16
      c -> hira.test(c) || cjkd.test(c),                                            // 17
      kanji);                                                                       // 18
    Re let = new Cl(4), dgt = new Cl(5), dict = new Cl(0), pai = new Cl(1);
    Re word = new Seq(new Seq(let, new Star(let), new Star(new Seq(new Cl(6), let, new Star(let)))), new Opt(new Cl(3)));
    Re number = new Seq(dgt, new Star(dgt), new Star(new Seq(new Cl(7), dgt, new Star(dgt))));
    Re tail = new Opt(new Seq(number, new Opt(new Cl(9))));
    return new Engine(cl, ign, THAI_DICT,
      new Cl(-1),
      new Seq(new Opt(word), new Star(new Seq(number, word)), tail),
      new Seq(new Cl(8), new Star(new Seq(number, word)), tail),
      new Seq(dict, new Star(dict), new Opt(new Seq(new Opt(pai), new Cl(2)))),
      new Seq(dict, new Star(dict), pai, new Slash(), new Alt(new Cl(14), new Seq(new Cl(13), new Cl(15)))),
      new Seq(pai, new Cl(13), pai),
      new Seq(new Star(new Cl(11)), new Opt(new Cl(12)), new Opt(new Cl(10))),
      new Star(new Cl(16)), new Star(new Cl(17)), new Star(new Cl(18)));
  }

  // ---------------- sentence rules: hand DFA ----------------
  static final int S_PS = 0, S_DANDA = 1, S_PERIOD = 2, S_TERM = 3, S_SPACE = 4, S_QUOTE = 5, S_START = 6, S_END = 7, S_LC = 8, S_NOTLC = 9, S_DIGIT = 10, S_SENT = 11, S_OTHER = 12;
  static int scat(int c) {
    int t = ty(c);
    if (c == 0x2029) return S_PS;
    if (c == 0x964 || c == 0x965) return S_DANDA;
    if (c == '.' || c == 0xff0e) return S_PERIOD;
    if (in(c, '!', '?', 0x3002, 0xff01, 0xff1f)) return S_TERM;
    if (in(c, '\t', '\r', '\f', '\n', 0x2028) || t == 12) return S_SPACE;
    if (c == '"' || c == '\'') return S_QUOTE;
    if (t == 21 || t == 29) return S_START;
    if (t == 22 || t == 30) return S_END;
    if (t == 2) return S_LC;
    if (isL(c)) return S_NOTLC;
    if (isN(c)) return S_DIGIT;
    if (c == ',') return S_OTHER;
    return S_SENT;
  }
  static final int X = 1, Y = 2, D = 3, D2 = 4, T = 5, U = 6, V = 7, W = 8, A = 9, M = 10, C = 11, SP = 12, NE = 13, C2 = 14, G = 15;
  static int xNext(int k) { return k == S_PS ? Y : k == S_DANDA ? D : k == S_PERIOD ? A : k == S_TERM ? T : X; }
  static boolean sps(int k) { return k == S_START || k == S_SENT || k == S_DANDA || k == S_QUOTE; }
  static int sNext(int s, int k) {
    switch (s) {
      case X: return xNext(k);
      case Y: case W: case NE: return 0;
      case D: case D2: return k == S_SPACE ? D2 : 0;
      case T: case U: return (k == S_TERM || k == S_PERIOD || k == S_END || k == S_QUOTE) ? U : k == S_SPACE ? V : k == S_PS ? W : 0;
      case V: return k == S_SPACE ? V : k == S_PS ? W : 0;
      case A: return (k == S_PERIOD || k == S_END) ? A : k == S_QUOTE ? M : k == S_SPACE ? C : sps(k) ? SP : xNext(k);
      case M: return (k == S_PERIOD || k == S_END) ? A : k == S_QUOTE ? M : k == S_SPACE ? C : sps(k) ? G : (k == S_LC || k == S_NOTLC) ? NE : xNext(k);
      case C: return k == S_SPACE ? C2 : k == S_NOTLC ? NE : sps(k) ? SP : xNext(k);
      case C2: return k == S_SPACE ? C2 : (k == S_NOTLC || k == S_LC) ? NE : sps(k) ? G : xNext(k);
      case SP: return sps(k) ? SP : (k == S_LC || k == S_NOTLC) ? NE : xNext(k);
      case G: return sps(k) ? SP : (k == S_LC || k == S_NOTLC) ? NE : 0;
    }
    throw new IllegalStateException();
  }
  static Dfa sentence() { return new Dfa() {
    public int start() { return X; } public int next(int s, int c) { return sNext(s, scat(c)); }
    public boolean ignore(int c) { int t = ty(c); return t == 6 || t == 7 || t == 16; }
    public boolean end(int s) { return s == X || s == Y || s == D || s == D2 || s == T || s == U || s == V || s == W || s == NE; }
    public boolean la(int s) { return s == A || s == M || s == C || s == C2 || s == NE; } }; }

  // ---------------- comparison ----------------
  static final String ALPHA =
      "aAzZéßΩωжЖאبकः́⃝​­\u0001 \t\n\r\f\u000b  　 "
    + ".. .  ..\"\"'!? 0959٣²Ⅻ.,'\"!?-_()[]«»‘’“”#$%&¢€٪‰‱٫‧।॥。！？．:;/@+=©"
    + "一々アァヽー・あゝ゛゙漢ㄱ가𝐀😀𐀀";
  static int[] alphaCps() { return ALPHA.codePoints().toArray(); }
  static final String THAI = "กขฯลๆะั็๑ไทยภาษา";

  static List<Integer> jdk(BreakIterator bi, String s) {
    bi.setText(s); List<Integer> r = new ArrayList<>(); r.add(bi.first());
    for (int b = bi.next(); b != BreakIterator.DONE; b = bi.next()) r.add(b); return r;
  }
  static List<Integer> mine(Dfa d, String s, List<int[]> ranges) {
    List<Integer> r = new ArrayList<>(); r.add(0); int i = 0;
    while (true) { dictCount = 0; int st = i; int b = handleNext(d, s, i); if (b == -1) break;
      if (ranges != null && dictCount > 1 && b - st > 1) ranges.add(new int[] {st, b});
      r.add(b); i = b; }
    return r;
  }
  static String show(String s) { StringBuilder b = new StringBuilder(); for (char c : s.toCharArray()) b.append(c < 0x20 || c > 0x7e ? String.format("\\u%04x", (int) c) : String.valueOf(c)); return b.toString(); }

  public static void main(String[] args) {
    Random r = new Random(args.length > 0 ? Long.parseLong(args[0]) : 1);
    int[] cps = alphaCps();
    int[] thai = (ALPHA + THAI + THAI + THAI).codePoints().toArray();
    Dfa base = of(baseWord()), th = of(thaiWord()), sent = sentence();
    BreakIterator jw = BreakIterator.getWordInstance(Locale.ROOT);
    BreakIterator js = BreakIterator.getSentenceInstance(Locale.ROOT);
    BreakIterator jt = BreakIterator.getWordInstance(new Locale.Builder().setLanguageTag("th").build());
    int bad = 0;
    for (int n = 0; n < 200000 && bad < 15; n++) {
      int len = 1 + r.nextInt(args.length > 1 ? Integer.parseInt(args[1]) : 12);
      StringBuilder sb = new StringBuilder(); for (int i = 0; i < len; i++) sb.appendCodePoint(cps[r.nextInt(cps.length)]);
      String s = sb.toString();
      dictPred = null;
      List<Integer> a = jdk(jw, s), b = mine(base, s, null);
      if (!a.equals(b)) { bad++; System.out.println("WORD " + show(s) + " jdk=" + a + " mine=" + b); }
      a = jdk(js, s); b = mine(sent, s, null);
      if (!a.equals(b)) { bad++; System.out.println("SENT " + show(s) + " jdk=" + a + " mine=" + b); }
      sb = new StringBuilder(); for (int i = 0; i < len; i++) sb.appendCodePoint(thai[r.nextInt(thai.length)]);
      s = sb.toString();
      dictPred = THAI_DICT; List<int[]> ranges = new ArrayList<>();
      a = jdk(jt, s); b = mine(th, s, ranges);
      // every rule boundary must be a JDK boundary; extra JDK boundaries only inside dictionary ranges
      boolean ok = a.containsAll(b);
      for (int x : a) if (!b.contains(x)) { boolean inside = false; for (int[] g : ranges) if (x > g[0] && x < g[1]) inside = true; if (!inside) ok = false; }
      if (!ok) { bad++; System.out.println("THAI " + show(s) + " jdk=" + a + " mine=" + b); }
    }
    System.out.println("mismatches=" + bad);
  }
}
