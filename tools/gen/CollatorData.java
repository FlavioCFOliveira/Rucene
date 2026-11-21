/*
 * Writes crates/rucene-analysis-common/src/jdk/collator_data.rs: the data
 * java.text.RuleBasedCollator reads on the running JDK (25) to build
 * Collator.getInstance(Locale.ROOT), and the locale tailorings
 * Collator.getInstance(Locale) adds to it:
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
 *    canonical ordering of the NFD and NFKD texts);
 *  - TAILORING_RULES and TAILORINGS: for every language tag of the JRE
 *    adapter's "CollationData" bundles (the set LocaleData weeds candidate
 *    locales with), every key ("Rule", "Rule.<co type>") of the bundle
 *    LocaleData.getBundle("sun.text.resources.CollationData", locale)
 *    returns for it, with the string LocaleResources.getCollationData reads
 *    (its parent chain included). CollatorProviderImpl builds a collator from
 *    CollationRules.DEFAULTRULES + that string.
 *
 * As a second self-check, the generator verifies the resolution collator.rs
 * implements: for every available locale, and a list of edge locales, each
 * with "co", "ks" and "kk" Unicode extensions, the rules of
 * Collator.getInstance(locale) are ROOT_RULES + the tailoring of the first
 * of ResourceBundle.Control's candidate locales whose language tag has one
 * (the key "Rule", or "Rule.<co>" for a "co" type other than "standard"; ""
 * when that tag has no such key), and its strength and decomposition are
 * TERTIARY and NO_DECOMPOSITION, overridden by "ks" and "kk". It fails on
 * any difference.
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
 *        --add-exports java.base/sun.util.locale.provider=ALL-UNNAMED \
 *        --add-exports java.base/sun.util.resources=ALL-UNNAMED \
 *        tools/gen/CollatorData.java [OUT.rs] [--check]
 * OUT.rs defaults to crates/rucene-analysis-common/src/jdk/collator_data.rs;
 * --check compares instead of writing and exits 1 on a difference.
 */

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.text.Collator;
import java.text.RuleBasedCollator;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.ResourceBundle;
import java.util.TreeMap;
import java.util.TreeSet;

import jdk.internal.icu.lang.UCharacter;
import jdk.internal.icu.text.Normalizer2;
import sun.text.ComposedCharIter;
import sun.util.locale.provider.JRELocaleProviderAdapter;
import sun.util.locale.provider.LocaleProviderAdapter;
import sun.util.resources.LocaleData;

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

  /**
   * A Rust string literal of {@code s} that keeps letters and digits as UTF-8
   * and escapes every other non-ASCII character (no surrogates).
   */
  private static String rustText(String s) {
    StringBuilder b = new StringBuilder("\"");
    for (int i = 0; i < s.length(); ) {
      int cp = s.codePointAt(i);
      i += Character.charCount(cp);
      if (Character.isSurrogate((char) cp)) {
        throw new IllegalStateException("a tailoring holds a lone surrogate");
      }
      if (cp == '"' || cp == '\\') {
        b.append('\\').append((char) cp);
      } else if ((cp >= 0x20 && cp < 0x7f) || (cp > 0x7f && Character.isLetterOrDigit(cp)
          && Character.getType(cp) != Character.NON_SPACING_MARK)) {
        b.appendCodePoint(cp);
      } else {
        b.append("\\u{").append(Integer.toHexString(cp)).append('}');
      }
    }
    return b.append('"').toString();
  }

  /** The key LocaleResources.getCollationData reads for {@code locale}. */
  private static String collationKey(Locale locale) {
    String key = "Rule";
    String type = locale.getUnicodeLocaleType("co");
    if (type != null && !type.isEmpty() && !type.equalsIgnoreCase("standard")) {
      key += "." + type;
    }
    return key;
  }

  /** The tailoring the model gives {@code locale} (see the class comment). */
  private static String modelTailoring(Map<String, Map<String, String>> table, Locale locale) {
    List<Locale> candidates = ResourceBundle.Control.getNoFallbackControl(
        ResourceBundle.Control.FORMAT_DEFAULT).getCandidateLocales("", locale);
    for (Locale c : candidates) {
      String tag = c.equals(Locale.ROOT) ? "" : c.toLanguageTag();
      Map<String, String> keys = table.get(tag);
      if (keys != null) {
        return keys.getOrDefault(collationKey(locale), "");
      }
    }
    throw new IllegalStateException("no candidate of " + locale + " has a tailoring");
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
    Path out = Path.of("crates/rucene-analysis-common/src/jdk/collator_data.rs");
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

    // The tailorings: every language tag of the JRE adapter's CollationData
    // bundles, with every key of the bundle LocaleData returns for it.
    JRELocaleProviderAdapter jre = (JRELocaleProviderAdapter) LocaleProviderAdapter.forJRE();
    Map<String, Map<String, String>> table = new TreeMap<>();
    for (String tag : new TreeSet<>(jre.getLanguageTagSet("CollationData"))) {
      Locale locale = tag.isEmpty() ? Locale.ROOT : Locale.forLanguageTag(tag);
      String back = locale.equals(Locale.ROOT) ? "" : locale.toLanguageTag();
      if (!back.equals(tag)) {
        throw new IllegalStateException("tag " + tag + " reads back as " + back);
      }
      ResourceBundle rb = LocaleData.getBundle("sun.text.resources.CollationData", locale);
      Map<String, String> keys = new TreeMap<>();
      for (String key : rb.keySet()) {
        keys.put(key, rb.getString(key));
      }
      table.put(tag, keys);
    }
    if (!table.containsKey("")) {
      throw new IllegalStateException("the root bundle has no tag");
    }
    // Self-check of the resolution model on every available locale and edge
    // cases, plain and with the Unicode extensions Collator.getInstance reads.
    List<Locale> samples = new ArrayList<>(List.of(Locale.getAvailableLocales()));
    for (String tag : table.keySet()) {
      Locale l = tag.isEmpty() ? Locale.ROOT : Locale.forLanguageTag(tag);
      samples.add(l);
      samples.add(new Locale.Builder().setLocale(l).setRegion("ZZ").build());
      samples.add(new Locale.Builder().setLocale(l).setScript("Latn").build());
      samples.add(new Locale.Builder().setLocale(l).setVariant("POSIX").build());
    }
    for (String tag : List.of("nn", "nb-SJ", "nn-NO", "no-NO", "zh-CN", "zh-SG", "zh-HK",
        "zh-MO", "zh-TW", "zh-Hans", "zh-Hant", "zh-Hans-HK", "zh-Hant-CN", "sr-Latn-RS",
        "sr-Cyrl", "sr-Latn-ME", "he", "iw", "id", "in", "yi", "ji", "und", "sv-FI", "pt-BR")) {
      samples.add(Locale.forLanguageTag(tag));
    }
    samples.add(Locale.of("no", "NO", "NY"));
    samples.add(Locale.of("ja", "JP", "JP"));
    samples.add(Locale.of("th", "TH", "TH"));
    samples.add(Locale.of("de", "DE", "a_b_c"));
    String[] extensions = {"", "co-trad", "co-standard", "co-phonebk", "ks-level1", "ks-level2",
        "ks-identic", "ks-LEVEL3", "ks-level4", "kk-true", "kk-false", "co-trad-ks-level1-kk-true"};
    int checked = 0;
    for (Locale base : samples) {
      for (String ext : extensions) {
        Locale l = base;
        if (!ext.isEmpty()) {
          try {
            l = new Locale.Builder().setLocale(base).setExtension('u', ext).build();
          } catch (java.util.IllformedLocaleException e) {
            continue; // a variant the builder refuses (ja_JP_JP, de_DE_a_b_c)
          }
        }
        RuleBasedCollator c = (RuleBasedCollator) Collator.getInstance(l);
        String expected = rules + modelTailoring(table, l);
        if (!c.getRules().equals(expected)) {
          throw new IllegalStateException("rules of " + l.toLanguageTag() + " differ from the model");
        }
        int strength = Collator.TERTIARY;
        String ks = l.getUnicodeLocaleType("ks");
        if (ks != null) {
          switch (ks.toLowerCase(Locale.ROOT)) {
            case "level1" -> strength = Collator.PRIMARY;
            case "level2" -> strength = Collator.SECONDARY;
            case "level3" -> strength = Collator.TERTIARY;
            case "identic" -> strength = Collator.IDENTICAL;
            default -> { }
          }
        }
        int decomposition = Collator.NO_DECOMPOSITION;
        String kk = l.getUnicodeLocaleType("kk");
        if (kk != null) {
          switch (kk.toLowerCase(Locale.ROOT)) {
            case "true" -> decomposition = Collator.CANONICAL_DECOMPOSITION;
            case "false" -> decomposition = Collator.NO_DECOMPOSITION;
            default -> { }
          }
        }
        if (c.getStrength() != strength || c.getDecomposition() != decomposition) {
          throw new IllegalStateException("strength or decomposition of " + l.toLanguageTag());
        }
        checked++;
      }
    }
    // The tailoring strings, each once, in first-use order.
    Map<String, Integer> tailoringIndex = new LinkedHashMap<>();
    for (Map<String, String> keys : table.values()) {
      for (String value : keys.values()) {
        tailoringIndex.putIfAbsent(value, tailoringIndex.size());
      }
    }

    StringBuilder b = new StringBuilder();
    b.append("// @generated by tools/gen/CollatorData.java on ")
        .append(System.getProperty("java.vm.name")).append(' ')
        .append(Runtime.version().feature())
        .append(". Do not edit.\n\n");
    b.append("//! The data `java.text.RuleBasedCollator` reads to build JDK 25's\n");
    b.append("//! `Collator.getInstance(Locale.ROOT)`, and the locale tailorings\n");
    b.append("//! `Collator.getInstance(Locale)` adds to it (see `tools/gen/CollatorData.java`).\n\n");
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
    b.append("];\n\n");
    b.append("/// Every distinct tailoring string of [`TAILORINGS`], which index it.\n");
    b.append("pub(super) static TAILORING_RULES: &[&str] = &[\n");
    for (String value : tailoringIndex.keySet()) {
      b.append("    ").append(rustText(value)).append(",\n");
    }
    b.append("];\n\n");
    b.append("/// `(language tag, key, tailoring)` for every language tag of the JRE\n");
    b.append("/// adapter's `CollationData` bundles (`\"\"` for the root locale) and every key\n");
    b.append("/// of its bundle, ascending; the tailoring indexes [`TAILORING_RULES`].\n");
    b.append("pub(super) static TAILORINGS: &[(&str, &str, u16)] = &[\n");
    for (Map.Entry<String, Map<String, String>> e : table.entrySet()) {
      for (Map.Entry<String, String> k : e.getValue().entrySet()) {
        b.append("    (").append(rustString(e.getKey())).append(", ").append(rustString(k.getKey()))
            .append(", ").append(tailoringIndex.get(k.getValue())).append("),\n");
      }
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
          + composed.size() + " composed characters, " + table.size() + " tailored tags, "
          + tailoringIndex.size() + " tailorings, " + checked + " locales checked");
    }
  }
}
