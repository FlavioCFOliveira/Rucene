/*
 * Writes crates/rucene-test-framework/src/jdk/collator_data.rs: the data
 * java.text.RuleBasedCollator reads on the running JDK (25) to build
 * Collator.getInstance(Locale.ROOT):
 *
 *  - ROOT_RULES: ((RuleBasedCollator) Collator.getInstance(Locale.ROOT))
 *    .getRules(), the rules the collator is built from;
 *  - DECOMPOSITIONS: for every code point but the Hangul syllables
 *    (U+AC00..U+D7A3, which both readers skip), the canonical decomposition
 *    of jdk.internal.icu.text.Normalizer2.getNFDInstance().getDecomposition
 *    when it has one. RBTableBuilder reads it through
 *    NormalizerImpl.canonicalDecomposeWithSingleQuotation (the rules) and
 *    sun.text.ComposedCharIter (NormalizerImpl.getDecompose: the precomposed
 *    characters it adds as expansions);
 *  - COMPAT_DECOMPOSITIONS: for every code point but the Hangul syllables,
 *    Normalizer2.getNFKDInstance().getDecomposition when it differs from
 *    the canonical one (FULL_DECOMPOSITION: the NFKD text the collation
 *    element iterator reads);
 *  - COMBINING_CLASSES: every code point whose
 *    jdk.internal.icu.lang.UCharacter.getCombiningClass is not 0
 *    (canonicalDecomposeWithSingleQuotation's canonical reordering, and the
 *    canonical ordering of the NFD and NFKD texts).
 *
 * As a self-check, the generator recomputes NormalizerImpl.getDecompose's
 * iteration (code points below U+2FA1E, skipping U+30FF..U+F8FF,
 * U+115BC..U+1D15D and U+1D1C1..U+2F7FF, at most 2100 entries) over
 * DECOMPOSITIONS and fails unless it equals what sun.text.ComposedCharIter
 * returns; collator.rs reproduces that iteration.
 *
 * Usage (JDK 25):
 *   java --add-exports java.base/jdk.internal.icu.text=ALL-UNNAMED \
 *        --add-exports java.base/jdk.internal.icu.lang=ALL-UNNAMED \
 *        --add-exports java.base/sun.text=ALL-UNNAMED \
 *        tools/gen/CollatorData.java [OUT.rs] [--check]
 * OUT.rs defaults to crates/rucene-test-framework/src/jdk/collator_data.rs;
 * --check compares instead of writing and exits 1 on a difference.
 */

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.text.Collator;
import java.text.RuleBasedCollator;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

import jdk.internal.icu.lang.UCharacter;
import jdk.internal.icu.text.Normalizer2;
import sun.text.ComposedCharIter;

public class CollatorData {
  /** A Rust string literal of {@code s} (no surrogates). */
  private static String rustString(String s) {
    StringBuilder b = new StringBuilder("\"");
    for (int i = 0; i < s.length(); ) {
      int cp = s.codePointAt(i);
      i += Character.charCount(cp);
      if (cp == '"' || cp == '\\') {
        b.append('\\').append((char) cp);
      } else if (cp >= 0x20 && cp < 0x7f) {
        b.append((char) cp);
      } else {
        b.append("\\u{").append(Integer.toHexString(cp)).append('}');
      }
    }
    return b.append('"').toString();
  }

  private static String units(String s) {
    StringBuilder b = new StringBuilder("&[");
    for (int i = 0; i < s.length(); i++) {
      if (i > 0) b.append(", ");
      b.append(String.format("0x%04X", (int) s.charAt(i)));
    }
    return b.append(']').toString();
  }

  public static void main(String[] args) throws Exception {
    Path out = Path.of("crates/rucene-test-framework/src/jdk/collator_data.rs");
    boolean check = false;
    for (String a : args) {
      if (a.equals("--check")) check = true;
      else out = Path.of(a);
    }
    RuleBasedCollator root = (RuleBasedCollator) Collator.getInstance(Locale.ROOT);
    String rules = root.getRules();
    for (int i = 0; i < rules.length(); i++) {
      if (Character.isSurrogate(rules.charAt(i))) {
        throw new IllegalStateException("the root rules hold a surrogate at " + i);
      }
    }
    Normalizer2 nfd = Normalizer2.getNFDInstance();
    Normalizer2 nfkd = Normalizer2.getNFKDInstance();
    List<Integer> cps = new ArrayList<>();
    List<String> decomps = new ArrayList<>();
    List<Integer> compatCps = new ArrayList<>();
    List<String> compatDecomps = new ArrayList<>();
    List<int[]> ccs = new ArrayList<>();
    for (int cp = 0; cp <= Character.MAX_CODE_POINT; cp++) {
      int cc = UCharacter.getCombiningClass(cp);
      if (cc != 0) ccs.add(new int[] {cp, cc});
      if (cp >= 0xAC00 && cp <= 0xD7A3) continue;
      String d = nfd.getDecomposition(cp);
      if (d != null) {
        cps.add(cp);
        decomps.add(d);
      }
      String k = nfkd.getDecomposition(cp);
      if (k != null && !k.equals(d)) {
        compatCps.add(cp);
        compatDecomps.add(k);
      }
    }
    // Self-check: NormalizerImpl.getDecompose's iteration over the table
    // equals ComposedCharIter.
    List<Integer> iterated = new ArrayList<>();
    for (int i = 0; i < cps.size() && iterated.size() < 2100; i++) {
      int cp = cps.get(i);
      boolean skipped = cp >= 0x2fa1e
          || (cp >= 0x30ff && cp < 0xf900)
          || (cp >= 0x115bc && cp < 0x1d15e)
          || (cp >= 0x1d1c1 && cp < 0x2f800);
      if (!skipped) iterated.add(cp);
    }
    List<Integer> composed = new ArrayList<>();
    ComposedCharIter it = new ComposedCharIter();
    for (int c; (c = it.next()) != ComposedCharIter.DONE; ) {
      composed.add(c);
      if (!it.decomposition().equals(decomps.get(cps.indexOf(c)))) {
        throw new IllegalStateException("decomposition of " + Integer.toHexString(c));
      }
    }
    if (!composed.equals(iterated)) {
      throw new IllegalStateException("ComposedCharIter (" + composed.size()
          + ") differs from the recomputed iteration (" + iterated.size() + ")");
    }

    StringBuilder b = new StringBuilder();
    b.append("// @generated by tools/gen/CollatorData.java on ")
        .append(System.getProperty("java.vm.name")).append(' ')
        .append(Runtime.version().feature())
        .append(". Do not edit.\n\n");
    b.append("//! The data `java.text.RuleBasedCollator` reads to build JDK 25's\n");
    b.append("//! `Collator.getInstance(Locale.ROOT)` (see `tools/gen/CollatorData.java`).\n\n");
    b.append("/// `((RuleBasedCollator) Collator.getInstance(Locale.ROOT)).getRules()`.\n");
    b.append("pub(super) static ROOT_RULES: &str = ").append(rustString(rules)).append(";\n\n");
    b.append("/// `Normalizer2.getNFDInstance().getDecomposition(cp)` of every code point\n");
    b.append("/// that has one, Hangul syllables excepted, ascending, as UTF-16 units.\n");
    b.append("pub(super) static DECOMPOSITIONS: &[(u32, &[u16])] = &[\n");
    for (int i = 0; i < cps.size(); i++) {
      b.append(String.format("    (0x%04X, %s),\n", cps.get(i), units(decomps.get(i))));
    }
    b.append("];\n\n");
    b.append("/// `Normalizer2.getNFKDInstance().getDecomposition(cp)` of every code point\n");
    b.append("/// where it differs from [`DECOMPOSITIONS`], Hangul syllables excepted,\n");
    b.append("/// ascending, as UTF-16 units.\n");
    b.append("pub(super) static COMPAT_DECOMPOSITIONS: &[(u32, &[u16])] = &[\n");
    for (int i = 0; i < compatCps.size(); i++) {
      b.append(String.format("    (0x%04X, %s),\n", compatCps.get(i), units(compatDecomps.get(i))));
    }
    b.append("];\n\n");
    b.append("/// `UCharacter.getCombiningClass(cp)` of every code point where it is not\n");
    b.append("/// 0, ascending.\n");
    b.append("pub(super) static COMBINING_CLASSES: &[(u32, u8)] = &[\n");
    for (int[] e : ccs) {
      b.append(String.format("    (0x%04X, %d),\n", e[0], e[1]));
    }
    b.append("];\n");
    byte[] bytes = b.toString().getBytes(StandardCharsets.UTF_8);
    if (check) {
      byte[] old = Files.exists(out) ? Files.readAllBytes(out) : new byte[0];
      if (!java.util.Arrays.equals(old, bytes)) {
        System.err.println(out + " differs from the JDK's data");
        System.exit(1);
      }
      System.out.println(out + " is current");
    } else {
      Files.write(out, bytes);
      System.out.println("wrote " + out + ": rules " + rules.length() + " chars, "
          + cps.size() + " decompositions, " + compatCps.size() + " compatibility ones, " + ccs.size() + " combining classes, "
          + composed.size() + " composed characters");
    }
  }
}
