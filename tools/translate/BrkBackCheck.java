import java.lang.reflect.*;
import java.text.BreakIterator;
import java.util.*;
import java.util.function.IntUnaryOperator;

/**
 * Checks the reading of JDK 25's RuleBasedBreakIterator / DictionaryBasedBreakIterator movements
 * (first, last, next, previous, following, preceding, isBoundary, current) that
 * crates/rucene-analysis-common/src/jdk/break_iterator.rs and break_iterator_rules.rs implement:
 * explicit forward tables (BrkCheck's models), backward tables (the pairs of categories the forward
 * table never reads across, plus the sentence rules' backward-only rules) and the JDK's
 * handlePrevious, against the JDK's word (root, Thai on its own thai_dict) and sentence iterators,
 * with random call sequences on pseudo-random strings. Prints each difference and the count; 0
 * expected. Not a test of the Rust code. Run with JDK 25:
 *   java --add-opens java.base/sun.text=ALL-UNNAMED tools/translate/BrkBackCheck.java [seed] [count]
 */
public class BrkBackCheck {
  /** An explicit table set. */
  static final class Tables {
    IntUnaryOperator cat; // code point -> category, -1 ignore
    int ncat;
    int[][] fwd; boolean[] end, la;
    int[][] back;
    boolean[] dictCat; // may be null
  }

  /** Builds explicit forward tables from a BrkCheck engine (all categories, all reachable states). */
  static Tables fromEngine(BrkCheck.Engine e) {
    Map<Long, Integer> catOf = new LinkedHashMap<>();
    int[] cpCat = new int[0x110000];
    for (int c = 0; c < 0x110000; c++) {
      if (e.ignore(c)) { cpCat[c] = -1; continue; }
      long m = e.mask(c);
      Integer id = catOf.get(m); if (id == null) { id = catOf.size(); catOf.put(m, id); }
      cpCat[c] = id;
    }
    // representative code point per category
    int[] rep = new int[catOf.size()];
    Arrays.fill(rep, -1);
    for (int c = 0; c < 0x110000; c++) if (cpCat[c] >= 0 && rep[cpCat[c]] < 0) rep[cpCat[c]] = c;
    Tables t = new Tables(); t.ncat = catOf.size(); t.cat = c -> cpCat[Math.max(0, Math.min(c, 0x10FFFF))];
    List<int[]> rows = new ArrayList<>(); rows.add(new int[t.ncat]);
    Map<Integer, Integer> ids = new HashMap<>(); ids.put(0, 0); ids.put(1, 1);
    List<Integer> order = new ArrayList<>(List.of(0, 1));
    for (int i = 1; i < order.size(); i++) {
      int st = order.get(i); int[] row = new int[t.ncat];
      for (int k = 0; k < t.ncat; k++) {
        int n = e.next(st, rep[k]);
        Integer id = ids.get(n); if (id == null) { id = order.size(); ids.put(n, id); order.add(n); }
        row[k] = id;
      }
      rows.add(row);
    }
    t.fwd = rows.toArray(new int[0][]);
    t.end = new boolean[order.size()]; t.la = new boolean[order.size()];
    for (int i = 1; i < order.size(); i++) { int f = e.dflags.get(order.get(i)); t.end[i] = (f & 1) != 0; t.la[i] = (f & 2) != 0; }
    if (e.dictionary != null) { t.dictCat = new boolean[t.ncat]; for (int k = 0; k < t.ncat; k++) t.dictCat[k] = e.dictionary.test(rep[k]); }
    t.back = pairTable(t, null);
    return t;
  }

  /**
   * The automatic backward table: row 1 (start) reads a category k into row 2+k; row 2+x on
   * category y continues (to row 2+y, or to `ruleStart[y]` when the rules' start row has a cell
   * for y) iff some forward row r has r[y] = n != 0 with n[x] != 0, else stops.
   */
  static int[][] pairTable(Tables t, int[] ruleStart) {
    int n = t.ncat;
    int[][] back = new int[2 + n][n];
    for (int k = 0; k < n; k++) back[1][k] = 2 + k;
    for (int col = 0; col < n; col++)
      for (int[] row : t.fwd) { int nx = row[col]; if (nx == 0) continue;
        for (int c2 = 0; c2 < n; c2++) if (t.fwd[nx][c2] != 0) back[2 + c2][col] = 2 + col; }
    if (ruleStart != null)
      for (int r = 1; r < back.length; r++) for (int k = 0; k < n; k++) if (ruleStart[k] != 0 && back[r][k] != 0) back[r][k] = ruleStart[k];
    return back;
  }

  /** The sentence tables: forward from BrkCheck's hand DFA, backward = the "!" rules + pairs. */
  static Tables sentence() {
    Tables t = new Tables(); t.ncat = 13;
    t.cat = c -> { int ty = Character.getType(c); if (ty == 6 || ty == 7 || ty == 16) return -1; return BrkCheck.scat(c); };
    int ns = 16; t.fwd = new int[ns][13]; t.end = new boolean[ns]; t.la = new boolean[ns];
    BrkCheck.Dfa d = BrkCheck.sentence();
    for (int s = 1; s < ns; s++) for (int k = 0; k < 13; k++) t.fwd[s][k] = BrkCheck.sNext(s, k);
    for (int s = 1; s < ns; s++) { t.end[s] = d.end(s); t.la[s] = d.la(s); }
    // backward rule rows, appended after the pair rows (2 + 13 = 15):
    final int T = 15, W = 16, Y = 17, S1 = 18, S3 = 19, S4 = 20, DONE = 21, ROWS = 22;
    int[] ruleStart = new int[13];
    ruleStart[BrkCheck.S_SENT] = T; ruleStart[BrkCheck.S_DANDA] = T; ruleStart[BrkCheck.S_LC] = S1; ruleStart[BrkCheck.S_DIGIT] = S1;
    int[][] pairs = pairTable(t, ruleStart);
    int[][] back = new int[ROWS][13];
    for (int r = 0; r < pairs.length; r++) back[r] = pairs[r];
    // row 1: the rules' start, its empty cells from the automatic start row
    for (int k = 0; k < 13; k++) back[1][k] = ruleStart[k] != 0 ? ruleStart[k] : 2 + k;
    int Q = BrkCheck.S_QUOTE, SPc = BrkCheck.S_START, SPACE = BrkCheck.S_SPACE, END = BrkCheck.S_END, PER = BrkCheck.S_PERIOD, TERM = BrkCheck.S_TERM;
    for (int r : new int[] {T, W, Y, S1, S3, S4}) for (int k = 0; k < 13; k++) back[r][k] = back[1][k];
    back[T][SPc] = T; back[T][Q] = T; back[T][SPACE] = W; back[T][END] = Y; back[T][PER] = DONE; back[T][TERM] = DONE;
    back[W][SPACE] = W; back[W][END] = Y; back[W][Q] = Y; back[W][PER] = DONE; back[W][TERM] = DONE;
    back[Y][END] = Y; back[Y][Q] = Y; back[Y][PER] = DONE; back[Y][TERM] = DONE;
    back[S1][SPc] = S1; back[S1][Q] = S1; back[S1][SPACE] = S3; back[S1][END] = S4; back[S1][TERM] = DONE;
    back[S3][SPACE] = S3; back[S3][END] = S4; back[S3][Q] = S4; back[S3][TERM] = DONE;
    back[S4][END] = S4; back[S4][Q] = S4; back[S4][TERM] = DONE;
    t.back = back;
    return t;
  }

  // ---------------- the model iterator ----------------
  interface Dict { int next(int state, int c); int nextCol0(int state); }

  static final class Model {
    final Tables t; final Dict dict; String s = ""; int pos; int cachedLastKnownBreak = -1;
    int dictionaryCharCount; int[] cached; int positionInCache;
    Model(Tables t, Dict dict) { this.t = t; this.dict = dict; }
    // StringCharacterIterator over s
    char curU() { return pos < s.length() ? s.charAt(pos) : '￿'; }
    char nextU() { if (pos < s.length() - 1) { pos++; return s.charAt(pos); } pos = s.length(); return '￿'; }
    char prevU() { if (pos > 0) { pos--; return s.charAt(pos); } return '￿'; }
    int count() { char c1 = curU(); if (Character.isHighSurrogate(c1) && pos < s.length()) { char c2 = nextU(); prevU(); if (Character.isLowSurrogate(c2)) return 2; } return 1; }
    int getCurrent() { char c1 = curU(); if (Character.isHighSurrogate(c1) && pos < s.length()) { char c2 = nextU(); prevU(); if (Character.isLowSurrogate(c2)) return Character.toCodePoint(c1, c2); } return c1; }
    int getNext() { int len = s.length(); if (pos == len) return 0xffff; int i = pos + count(); if (i >= len) return 0xffff; pos = i; return getCurrent(); }
    int getPrevious() { char c2 = prevU(); if (Character.isLowSurrogate(c2) && pos > 0) { char c1 = prevU(); if (Character.isHighSurrogate(c1)) return Character.toCodePoint(c1, c2); else nextU(); } return c2; }
    int getNextIndex() { return Math.min(pos + count(), s.length()); }
    int lookupCategory(int c) { int r = t.cat.applyAsInt(c); if (dict != null && r != -1 && t.dictCat[r]) dictionaryCharCount++; return r; }

    void setText(String text) { s = text; pos = 0; cachedLastKnownBreak = -1; cached = null; dictionaryCharCount = 0; positionInCache = 0; }
    int current() { return pos; }
    int first() { cached = null; dictionaryCharCount = 0; positionInCache = 0; pos = 0; return 0; }
    int last() { cached = null; dictionaryCharCount = 0; positionInCache = 0; pos = s.length(); return pos; }

    int ruleHandleNext() {
      int len = s.length();
      if (pos == len) return -1;
      int result = getNextIndex(); int la = 0; int state = 1; int c = getCurrent();
      while (c != 0xffff && state != 0) {
        int cat = lookupCategory(c);
        if (cat != -1) state = t.fwd[state][cat];
        if (t.la[state]) { if (t.end[state]) result = la; else la = getNextIndex(); }
        else if (t.end[state]) result = getNextIndex();
        c = getNext();
      }
      if (c == 0xffff && la == len) result = la;
      pos = result; return result;
    }
    int handlePrevious() {
      int state = 1, category = 0, lastCategory = 0; int c = getCurrent();
      while (c != 0xffff && state != 0) {
        lastCategory = category; category = lookupCategory(c);
        if (category != -1) state = t.back[state][category];
        c = getPrevious();
      }
      if (c != 0xffff) { if (lastCategory != -1) { getNext(); getNext(); } else getNext(); }
      return pos;
    }
    int handleNext() {
      if (dict == null) return ruleHandleNext();
      if (cached == null || positionInCache == cached.length - 1) {
        int startPos = pos; dictionaryCharCount = 0;
        int result = ruleHandleNext();
        if (dictionaryCharCount > 1 && result - startPos > 1) divide(startPos, result);
        else { cached = null; return result; }
      }
      if (cached != null) { ++positionInCache; pos = cached[positionInCache]; return pos; }
      return -9999;
    }
    void divide(int startPos, int endPos) {
      pos = startPos; int c = getCurrent(); int category = lookupCategory(c);
      while (category == -1 || !t.dictCat[category]) { c = getNext(); category = lookupCategory(c); }
      Deque<Integer> current = new ArrayDeque<>(); Deque<Integer> possible = new ArrayDeque<>(); List<Integer> wrong = new ArrayList<>();
      int state = 0; int farthest = pos; Deque<Integer> best = null; c = getCurrent();
      while (true) {
        if (dict.nextCol0(state) == -1) possible.push(pos);
        state = dict.next(state, c);
        if (state == -1) { current.push(pos); break; }
        else if (state == 0 || pos >= endPos) {
          if (pos > farthest) { farthest = pos; best = new ArrayDeque<>(current); }
          while (!possible.isEmpty() && wrong.contains(possible.peek())) possible.pop();
          if (possible.isEmpty()) {
            if (best != null) { current = best; if (farthest < endPos) pos = farthest + 1; else break; }
            else { if ((current.isEmpty() || current.peek() != pos) && pos != startPos) current.push(pos); getNext(); current.push(pos); }
          } else {
            int temp = possible.pop();
            while (!current.isEmpty() && temp < current.peek()) wrong.add(current.pop());
            current.push(temp); pos = current.peek();
          }
          c = getCurrent(); if (pos >= endPos) break;
        } else c = getNext();
      }
      if (!current.isEmpty()) current.pop();
      current.push(endPos);
      cached = new int[current.size() + 1]; cached[0] = startPos;
      Iterator<Integer> it = current.descendingIterator(); int i = 1; while (it.hasNext()) cached[i++] = it.next();
      positionInCache = 0;
    }
    int rulePrevious() {
      if (pos == 0) return -1;
      int start = pos; int lastResult = cachedLastKnownBreak;
      if (lastResult >= start || lastResult <= -1) { getPrevious(); lastResult = handlePrevious(); }
      else pos = lastResult;
      int result = lastResult;
      while (result != -1 && result < start) { lastResult = result; result = handleNext(); }
      pos = lastResult; cachedLastKnownBreak = lastResult; return lastResult;
    }
    int previous() {
      if (dict == null) return rulePrevious();
      if (cached != null && positionInCache > 0) { --positionInCache; pos = cached[positionInCache]; return pos; }
      cached = null; int result = rulePrevious();
      if (cached != null) positionInCache = cached.length - 2;
      return result;
    }
    int ruleFollowing(int offset) {
      pos = offset;
      if (offset == 0) { cachedLastKnownBreak = handleNext(); return cachedLastKnownBreak; }
      int result = cachedLastKnownBreak;
      if (result >= offset || result <= -1) result = handlePrevious(); else pos = result;
      while (result != -1 && result <= offset) result = handleNext();
      cachedLastKnownBreak = result; return result;
    }
    int following(int offset) {
      if (dict == null) return ruleFollowing(offset);
      if (cached == null || offset < cached[0] || offset >= cached[cached.length - 1]) { cached = null; return ruleFollowing(offset); }
      positionInCache = 0; while (positionInCache < cached.length && offset >= cached[positionInCache]) ++positionInCache;
      pos = cached[positionInCache]; return pos;
    }
    int preceding(int offset) {
      if (dict != null) {
        if (cached == null || offset <= cached[0] || offset > cached[cached.length - 1]) cached = null;
        else { positionInCache = 0; while (positionInCache < cached.length && offset > cached[positionInCache]) ++positionInCache; --positionInCache; pos = cached[positionInCache]; return pos; }
      }
      pos = offset; return previous();
    }
    boolean isBoundary(int offset) { if (offset == 0) return true; return following(offset - 1) == offset; }
  }

  static String run(BreakIterator bi, Model m, String s, Random r) {
    bi.setText(s); m.setText(s);
    StringBuilder log = new StringBuilder();
    for (int k = 0; k < 12; k++) {
      int op = r.nextInt(8), off = r.nextInt(s.length() + 1);
      String a, b;
      switch (op) {
        case 0 -> { a = "first " + bi.first(); b = "first " + m.first(); }
        case 1 -> { a = "last " + bi.last(); b = "last " + m.last(); }
        case 2, 3 -> { a = "next " + bi.next(); b = "next " + m.handleNext(); }
        case 4 -> { a = "previous " + bi.previous(); b = "previous " + m.previous(); }
        case 5 -> { a = "following(" + off + ") " + bi.following(off); b = "following(" + off + ") " + m.following(off); }
        case 6 -> { a = "preceding(" + off + ") " + bi.preceding(off); b = "preceding(" + off + ") " + m.preceding(off); }
        default -> { a = "isBoundary(" + off + ") " + bi.isBoundary(off); b = "isBoundary(" + off + ") " + m.isBoundary(off); }
      }
      a += " @" + bi.current(); b += " @" + m.current();
      log.append(a).append("; ");
      if (!a.equals(b)) return log + " JDK=" + a + " MODEL=" + b;
    }
    return null;
  }

  public static void main(String[] args) throws Exception {
    long seed = args.length > 0 ? Long.parseLong(args[0]) : 3;
    int count = args.length > 1 ? Integer.parseInt(args[1]) : 100000;
    Random r = new Random(seed);
    BreakIterator jt = BreakIterator.getWordInstance(new Locale.Builder().setLanguageTag("th").build());
    Field fd = jt.getClass().getDeclaredField("dictionary"); fd.setAccessible(true);
    Object bd = fd.get(jt);
    Method mNext = bd.getClass().getMethod("getNextStateFromCharacter", int.class, int.class); mNext.setAccessible(true);
    Method mState = bd.getClass().getMethod("getNextState", int.class, int.class); mState.setAccessible(true);
    Dict dict = new Dict() {
      public int next(int st, int c) { try { return (short) mNext.invoke(bd, st, c); } catch (Exception x) { throw new RuntimeException(x); } }
      public int nextCol0(int st) { try { return (short) mState.invoke(bd, st, 0); } catch (Exception x) { throw new RuntimeException(x); } } };
    Tables word = fromEngine(BrkCheck.baseWord()), thai = fromEngine(BrkCheck.thaiWord()), sent = sentence();
    BreakIterator jw = BreakIterator.getWordInstance(Locale.ROOT), js = BreakIterator.getSentenceInstance(Locale.ROOT);
    int[] cps = (BrkCheck.ALPHA).codePoints().toArray();
    int[] thaiCps = (BrkCheck.ALPHA + BrkCheck.THAI + BrkCheck.THAI + BrkCheck.THAI + "ภาษาไทยการทดสอบ").codePoints().toArray();
    int bad = 0;
    for (int n = 0; n < count && bad < 12; n++) {
      int len = r.nextInt(16);
      StringBuilder sb = new StringBuilder(); for (int i = 0; i < len; i++) sb.appendCodePoint(cps[r.nextInt(cps.length)]);
      String s = sb.toString(); long sub = r.nextLong();
      String e = run(jw, new Model(word, null), s, new Random(sub));
      if (e != null) { bad++; System.out.println("WORD " + BrkCheck.show(s) + " :: " + e); }
      e = run(js, new Model(sent, null), s, new Random(sub));
      if (e != null) { bad++; System.out.println("SENT " + BrkCheck.show(s) + " :: " + e); }
      sb = new StringBuilder(); for (int i = 0; i < len; i++) sb.appendCodePoint(thaiCps[r.nextInt(thaiCps.length)]);
      s = sb.toString();
      e = run(jt, new Model(thai, dict), s, new Random(sub));
      if (e != null) { bad++; System.out.println("THAI " + BrkCheck.show(s) + " :: " + e); }
    }
    System.out.println("mismatches=" + bad);
  }
}
