package org.apache.lucene.tests.util;

import com.carrotsearch.randomizedtesting.OracleSupport;
import java.io.File;
import java.io.PrintWriter;
import java.text.CollationKey;
import java.text.Collator;
import java.text.ParseException;
import java.text.RuleBasedCollator;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Random;
import org.junit.internal.AssumptionViolatedException;

/**
 * java.text.RuleBasedCollator on JDK 25, for crates/rucene-test-framework/src/jdk/collator.rs and
 * LuceneTestCase.collate (rmp #349). Writes rucene-test-framework/collator.tsv:
 *
 * <ul>
 *   <li>{@code root-rules}: the units of Collator.getInstance(Locale.ROOT).getRules(), its
 *       hashCode, strength and decomposition.
 *   <li>{@code str i units}: the grid strings.
 *   <li>{@code rules name units}: the collators, built with new RuleBasedCollator(rules) (the
 *       rule sets of RuleBasedCollator's javadoc, and sets for each rule feature: contraction,
 *       expansion, French secondary '@', Thai/Lao swapping '!', rules whose canonical
 *       decomposition reorders combining marks or yields a syntax character the builder quotes);
 *       {@code root} is Collator.getInstance(ROOT).
 *   <li>{@code error name units class message offset}: rules that do not parse.
 *   <li>{@code cmp name strength decomposition i signs}: Integer.signum(compare(grid[i],
 *       grid[j])) for every j, as '<', '=', '>', at every strength and decomposition.
 *   <li>{@code keys name strength decomposition digest}: the FNV-1a digest of every grid
 *       string's collation key (length, then UTF-16 units little-endian), and {@code key i
 *       units} the root collator's keys in full.
 *   <li>{@code random k digest signs keycmp collate keydigest}: 4000 pairs drawn by
 *       TestUtil.randomUnicodeString (even k) and TestUtil.randomRealisticUnicodeString (odd k)
 *       from one new Random(20261106): the digest of the pair, the root collator's compare signs
 *       at every strength and decomposition, the sign of the default keys' compareTo,
 *       LuceneTestCase.collate(root, a, b) (or {@code assume} with the message), and the digest
 *       of the default keys.
 * </ul>
 */
public class CollatorOracle {
  static final String[] GRID = {
    "", "a", "A", "b", "B", "ab", "aB", "Ab", "AB", "abc", "ba", "aa", "aaa",
    "\u00e0", "a\u0300", "\u00e0b", "a\u0300b", "\u00c0", "\u00e1", "a\u0301", "\u00e2", "\u00e4",
    "\u00e5", "a\u030a", "\u212b", "\u00c5", "e", "\u00e9", "e\u0301", "\u00e8", "\u00ea", "\u00eb",
    "c\u00f4te", "co\u0302te\u0301", "cote", "cot\u00e9", "c\u00f4t\u00e9", "\u00e6", "ae", "AE",
    "\u00c6", "\u0153", "oe", "\u00df", "ss", "SS", "\u00fe", "th", "TH", "\u00f0", "d", "ch",
    "cH", "Ch", "CH", "c", "h", "cz", "-", "a-b", "ab-", "co-op", "coop", "co op", " ", "a b",
    "\t", "\u0000", "\u200b", "a\u200bb", "\u00ad", "a\u00adb", "1", "10", "2", "!", "@", "#",
    "\u2126", "\u03a9", "\u03c9", "\u03b1\u0300\u0345", "\u1fb3\u0340", "\u1e09", "\u0f73",
    "\u1e09\u0f73", "\ufb01", "fi", "\uff11", "\u4e2d", "\u6587", "\u4e2d\u6587", "\uac00",
    "\u1100\u1161", "\u0e40\u0e01", "\u0e01\u0e40", "\u0e01", "\u0e41\u0e02", "\u0ec0\u0e81",
    "\ud834\udd1e", "\ud83d\ude00", "\ud840\udc00", "a\ud83d\ude00", "\ud83d\ude00a", "\uffff",
    "\ufffd", "z", "Z", "zz", "\u017f", "\u1e9b", "hc", "Hc", "hca", "cot\u00e8",
    "cote\u0300", "c\u00f4t\u00e8", ";", "\u037e", "=\u0338", "\u2260", "`", "\u1fef"
  };

  static final String NORWEGIAN =
      "< a, A < b, B < c, C < d, D < e, E < f, F < g, G < h, H < i, I"
          + "< j, J < k, K < l, L < m, M < n, N < o, O < p, P < q, Q < r, R"
          + "< s, S < t, T < u, U < v, V < w, W < x, X < y, Y < z, Z"
          + "< \u00e6, \u00c6"
          + "< \u00f8, \u00d8"
          + "< \u00e5 = a\u030a,"
          + "  \u00c5 = A\u030a;"
          + "  aa, AA";

  static final String OLD_RULES =
      "=\u0301;\u0300;\u0302;\u0308"
          + ";\u0327;\u0303;\u0304;\u0305"
          + ";\u0306;\u0307;\u0309;\u030a"
          + ";\u030b;\u030c;\u030d;\u030e"
          + ";\u030f;\u0310;\u0311;\u0312"
          + "< a , A ; ae, AE ; \u00e6 , \u00c6"
          + "< b , B < c, C < e, E & C < d, D";

  static Map<String, String> ruleSets() {
    Map<String, String> m = new LinkedHashMap<>();
    m.put("simple", "< a< b< c< d");
    m.put("norwegian", NORWEGIAN);
    m.put("accents", OLD_RULES + "& \u0300 ; \u0308 ; \u0302");
    m.put("contraction", "< a, A < b, B < c, C < ch, cH, Ch, CH < d, D < h, H & b < cz");
    m.put("expansion", "< a, A < e, E < \u00e6/e, \u00c6/E < o < \u0153/e < z");
    m.put("french", "=\u0301;\u0300;\u0302 < a, A < c, C < e, E < o, O < t, T@");
    m.put("thai", "< a < \u0e01 < \u0e02 < \u0e40 < \u0e41! < \u0e81 < \u0ec0");
    m.put("reorder", "< a < \u1fb3\u0340 < \u1e09\u0f73 < \u00e0\u0f73 < b");
    m.put("ignorables", ", '-' ; ' ' < a < b & b < c");
    m.put("quoting", "< a < \u037e < b < \u2260 < c < \u1fef < d");
    return m;
  }

  static final String[] BAD_RULES = {
    "", "a < b", "< a < -", "< a < b'", "< a < a", "& x < y", "< a /", "< a < b & ' ' < c",
    "< a < \ud834\udd5eb < c < d"
  };

  static long fnv(long h, String s) {
    h = fnvInt(h, s.length());
    for (int k = 0; k < s.length(); k++) {
      char c = s.charAt(k);
      h ^= (c & 0xff);
      h *= 0x100000001b3L;
      h ^= ((c >>> 8) & 0xff);
      h *= 0x100000001b3L;
    }
    return h;
  }

  static long fnvInt(long h, int v) {
    for (int k = 0; k < 4; k++) {
      h ^= ((v >>> (8 * k)) & 0xff);
      h *= 0x100000001b3L;
    }
    return h;
  }

  static String keyString(CollationKey k) {
    byte[] b = k.toByteArray();
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < b.length; i += 2) {
      sb.append((char) (((b[i] & 0xff) << 8) | (b[i + 1] & 0xff)));
    }
    return sb.toString();
  }

  static char sign(int v) {
    return v < 0 ? '<' : v > 0 ? '>' : '=';
  }

  static String signs(Collator base, String a, String b) {
    StringBuilder sb = new StringBuilder();
    for (int d = 0; d <= 2; d++) {
      for (int s = 0; s <= 3; s++) {
        Collator c = (Collator) base.clone();
        c.setDecomposition(d);
        c.setStrength(s);
        sb.append(sign(c.compare(a, b)));
      }
    }
    return sb.toString();
  }

  public static void main(String[] args) throws Exception {
    File out = new File(args[0], "rucene-test-framework");
    out.mkdirs();
    try (PrintWriter w = new PrintWriter(new File(out, "collator.tsv"), "UTF-8")) {
      w.println("# java.text.RuleBasedCollator on JDK 25 (CollatorOracle)");
      RuleBasedCollator root = (RuleBasedCollator) Collator.getInstance(Locale.ROOT);
      w.println("root-rules\t" + OracleSupport.units(root.getRules()) + "\t" + root.hashCode()
          + "\t" + root.getStrength() + "\t" + root.getDecomposition());
      for (int i = 0; i < GRID.length; i++) {
        w.println("str\t" + i + "\t" + OracleSupport.units(GRID[i]));
      }
      Map<String, Collator> collators = new LinkedHashMap<>();
      collators.put("root", root);
      for (Map.Entry<String, String> e : ruleSets().entrySet()) {
        w.println("rules\t" + e.getKey() + "\t" + OracleSupport.units(e.getValue()));
        collators.put(e.getKey(), new RuleBasedCollator(e.getValue()));
      }
      for (int i = 0; i < BAD_RULES.length; i++) {
        String r = BAD_RULES[i];
        String result;
        try {
          new RuleBasedCollator(r);
          result = "ok\t-\t-";
        } catch (ParseException e) {
          result = OracleSupport.err(e) + "\t" + e.getErrorOffset();
        } catch (RuntimeException e) {
          result = OracleSupport.err(e) + "\t-";
        }
        w.println("error\t" + i + "\t" + OracleSupport.units(r) + "\t" + result);
      }
      for (Map.Entry<String, Collator> e : collators.entrySet()) {
        for (int d = 0; d <= 2; d++) {
          for (int s = 0; s <= 3; s++) {
            Collator c = (Collator) e.getValue().clone();
            c.setDecomposition(d);
            c.setStrength(s);
            long h = 0xcbf29ce484222325L;
            for (int i = 0; i < GRID.length; i++) {
              StringBuilder sb = new StringBuilder();
              for (int j = 0; j < GRID.length; j++) sb.append(sign(c.compare(GRID[i], GRID[j])));
              w.println("cmp\t" + e.getKey() + "\t" + s + "\t" + d + "\t" + i + "\t" + sb);
              h = fnv(h, keyString(c.getCollationKey(GRID[i])));
            }
            w.println("keys\t" + e.getKey() + "\t" + s + "\t" + d + "\t" + String.format("%016x", h));
          }
        }
      }
      for (int i = 0; i < GRID.length; i++) {
        w.println("key\t" + i + "\t" + OracleSupport.units(keyString(root.getCollationKey(GRID[i]))));
      }
      Random r = new Random(20261106L);
      for (int k = 0; k < 4000; k++) {
        String a, b;
        if (k % 2 == 0) {
          a = TestUtil.randomUnicodeString(r);
          b = TestUtil.randomUnicodeString(r);
        } else {
          a = TestUtil.randomRealisticUnicodeString(r);
          b = TestUtil.randomRealisticUnicodeString(r);
        }
        long pair = fnv(fnv(0xcbf29ce484222325L, a), b);
        CollationKey ka = root.getCollationKey(a);
        CollationKey kb = root.getCollationKey(b);
        String collate;
        try {
          collate = Integer.toString(Integer.signum(LuceneTestCase.collate(root, a, b)));
        } catch (AssumptionViolatedException e) {
          collate = "assume:" + e.getMessage();
        }
        long keys = fnv(fnv(0xcbf29ce484222325L, keyString(ka)), keyString(kb));
        w.println("random\t" + k + "\t" + String.format("%016x", pair) + "\t" + signs(root, a, b)
            + "\t" + sign(ka.compareTo(kb)) + "\t" + collate + "\t" + String.format("%016x", keys));
      }
    }
  }
}
