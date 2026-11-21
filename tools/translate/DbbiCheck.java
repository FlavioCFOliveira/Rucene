import java.lang.reflect.*;
import java.text.BreakIterator;
import java.util.*;

/**
 * Checks the reading of JDK 25's DictionaryBasedBreakIterator (handleNext, divideUpDictionaryRange)
 * that crates/rucene-analysis-common/src/jdk/break_iterator.rs implements: the Java model, run on the
 * JDK's own thai_dict (read by reflection), against BreakIterator.getWordInstance(th) on
 * pseudo-random Thai strings. Prints each difference and the count; 0 expected. Not a test of the
 * Rust code. Run with JDK 25:
 *   java --add-opens java.base/sun.text=ALL-UNNAMED tools/translate/DbbiCheck.java [seed]
 */
public class DbbiCheck {
  interface Dict { int next(int state, int c); int nextCol0(int state); }
  // text cursor emulating CharacterIterator over a String
  static String s; static int idx;
  static int cur() { if (idx >= s.length()) return 0xFFFF; return BrkCheck.cpAt(s, idx); }
  static int next() { int len = s.length(); if (idx == len || idx + BrkCheck.count(s, idx) >= len) return 0xFFFF; idx += BrkCheck.count(s, idx); return cur(); }
  static boolean isDict(int c) { return c != 0xFFFF && BrkCheck.THAI_DICT.test(c); }
  static boolean ignore(BrkCheck.Engine e, int c) { return c != 0xFFFF && e.ignore(c); }

  static List<Integer> divide(Dict dict, BrkCheck.Engine e, int startPos, int endPos) {
    idx = startPos; int c = cur();
    while (ignore(e, c) || !isDict(c)) c = next();
    List<Deque<Integer>> current = new ArrayList<>(List.of(new ArrayDeque<>()));
    Deque<Integer> possible = new ArrayDeque<>();
    List<Integer> wrong = new ArrayList<>();
    int state = 0; int farthest = idx; Deque<Integer> best = null;
    c = cur();
    while (true) {
      if (dict.nextCol0(state) == -1) possible.push(idx);
      state = dict.next(state, c);
      if (state == -1) { current.get(0).push(idx); break; }
      else if (state == 0 || idx >= endPos) {
        if (idx > farthest) { farthest = idx; best = cloneStack(current.get(0)); }
        while (!possible.isEmpty() && wrong.contains(possible.peek())) possible.pop();
        if (possible.isEmpty()) {
          if (best != null) { current.set(0, best); if (farthest < endPos) idx = farthest + 1; else break; }
          else { if ((current.get(0).isEmpty() || current.get(0).peek() != idx) && idx != startPos) current.get(0).push(idx); next(); current.get(0).push(idx); }
        } else {
          int temp = possible.pop();
          while (!current.get(0).isEmpty() && temp < current.get(0).peek()) wrong.add(current.get(0).pop());
          current.get(0).push(temp); idx = current.get(0).peek();
        }
        c = cur();
        if (idx >= endPos) break;
      } else c = next();
    }
    if (!current.get(0).isEmpty()) current.get(0).pop();
    current.get(0).push(endPos);
    List<Integer> r = new ArrayList<>(); r.add(startPos);
    Iterator<Integer> it = current.get(0).descendingIterator(); while (it.hasNext()) r.add(it.next());
    return r;
  }
  static Deque<Integer> cloneStack(Deque<Integer> d) { return new ArrayDeque<>(d); }

  public static void main(String[] a) throws Exception {
    BreakIterator jt = BreakIterator.getWordInstance(new Locale.Builder().setLanguageTag("th").build());
    Field fd = jt.getClass().getDeclaredField("dictionary"); fd.setAccessible(true);
    Object bd = fd.get(jt);
    Method mNext = bd.getClass().getMethod("getNextStateFromCharacter", int.class, int.class); mNext.setAccessible(true);
    Method mState = bd.getClass().getMethod("getNextState", int.class, int.class); mState.setAccessible(true);
    Dict dict = new Dict() {
      public int next(int st, int c) { try { return (short) mNext.invoke(bd, st, c); } catch (Exception x) { throw new RuntimeException(x); } }
      public int nextCol0(int st) { try { return (short) mState.invoke(bd, st, 0); } catch (Exception x) { throw new RuntimeException(x); } } };
    BrkCheck.Engine e = BrkCheck.thaiWord(); BrkCheck.Dfa d = BrkCheck.of(e);
    BrkCheck.dictPred = BrkCheck.THAI_DICT;
    String[] words = {"ภาษา", "ไทย", "การ", "ทดสอบ", "ประโยค", "ว่า", "ของ", "คน", "กิน", "ข้าว", "แล้ว", "ฯ", "ๆ", "ล", " ", "a", "1", "๑", ".", "ั", "ก", "ข", "ฯลฯ", "​", "́"};
    Random r = new Random(a.length > 0 ? Long.parseLong(a[0]) : 7);
    int bad = 0, n;
    for (n = 0; n < 100000 && bad < 10; n++) {
      StringBuilder sb = new StringBuilder(); int k = 1 + r.nextInt(8);
      for (int i = 0; i < k; i++) sb.append(r.nextInt(4) == 0 ? String.valueOf((char) (0xe01 + r.nextInt(0x4d))) : words[r.nextInt(words.length)]);
      s = sb.toString();
      jt.setText(s); List<Integer> j = new ArrayList<>(); j.add(jt.first()); for (int b = jt.next(); b != BreakIterator.DONE; b = jt.next()) j.add(b);
      List<Integer> m = new ArrayList<>(); m.add(0); int i = 0;
      while (true) {
        BrkCheck.dictCount = 0; int st = i; int b = BrkCheck.handleNext(d, s, i); if (b == -1) break;
        if (BrkCheck.dictCount > 1 && b - st > 1) { List<Integer> cache = divide(dict, e, st, b); m.addAll(cache.subList(1, cache.size())); }
        else m.add(b);
        i = m.get(m.size() - 1);
      }
      if (!j.equals(m)) { bad++; System.out.println(BrkCheck.show(s) + " jdk=" + j + " mine=" + m); }
    }
    System.out.println("checked=" + n + " mismatches=" + bad);
  }
}
