// Generates crates/rucene-analysis-common/src/jdk/date_parse_data.bin, the
// zlib-compressed data the emulation of java.text.SimpleDateFormat parsing
// (crates/rucene-analysis-common/src/jdk/simple_date_format.rs) reads, from the
// running JDK (run it with JDK 25; see tools/translate/README.md for the
// command line, which opens the JDK internals read below).
//
// What is dumped, and why:
// * the CLDR locale-provider metadata the JDK resolves a locale with
//   (available language tags, language aliases, parent locales, likely
//   scripts), so that the emulation can compute the weeded candidate chain
//   of any locale as sun.util.cldr.CLDRLocaleProviderAdapter and
//   sun.util.resources.LocaleData do;
// * for every distinct weeded chain ("class") reachable from a language tag
//   built from the subtags the JDK knows (plus one unseen value of each
//   subtag), the data SimpleDateFormat.parse reads for a locale of that
//   class: Calendar display names per calendar type, the DEFAULT date
//   pattern per calendar type, the integer NumberFormat per numbering
//   system, and DateFormatSymbols.getZoneStrings();
// * the time-zone IDs TimeZone.getTimeZone knows and the BCP 47 "tz" IDs.
// The data of every class is read through the JDK's public API on a
// representative locale; a validation pass compares, for a sample of
// locales with and without Unicode extensions, the JDK's own
// SimpleDateFormat state with the state the dumped data predicts, and the
// run fails if they differ. Deterministic: every collection is sorted.
import java.io.*;
import java.lang.reflect.*;
import java.text.*;
import java.util.*;
import java.util.zip.*;

public class JdkDateParseData {
  static final String[] CAL_TYPES = {"gregory", "buddhist", "japanese"};

  static Object cldr, fallback;
  static Method getAdapter, getLookupLocales, findProviders;
  static Object namePool;
  static Set<String> fallbackTags, fallbackZoneTags;
  /** The provider classes LocaleProviderAdapter.getAdapter is asked for along a parse. */
  static final String[] PROVIDERS = {
    "java.text.spi.DateFormatProvider", "java.text.spi.DateFormatSymbolsProvider",
    "java.text.spi.NumberFormatProvider", "java.text.spi.DecimalFormatSymbolsProvider",
    "java.util.spi.TimeZoneNameProvider",
  };
  static Method getCandidateLocales, isSupportedProviderLocale, getLocaleResources, getDateTimePattern, getLocaleData, getNumberFormatData;
  static Set<String> langtags;
  static Map<String, String> aliases;
  static Map<String, String> tzCanonical;

  // ---------------------------------------------------------------- strings
  static final Map<String, Integer> stringIds = new HashMap<>();
  static final List<String> strings = new ArrayList<>();
  static int sid(String s) {
    return stringIds.computeIfAbsent(s, k -> { strings.add(k); return strings.size() - 1; });
  }

  static <T> int intern(Map<T, Integer> ids, List<T> list, T value) {
    return ids.computeIfAbsent(value, k -> { list.add(k); return list.size() - 1; });
  }

  // ------------------------------------------------------------ resolution
  @SuppressWarnings("unchecked")
  static List<Locale> chain(Locale l) throws Exception {
    List<Locale> c = new ArrayList<>((List<Locale>) getCandidateLocales.invoke(cldr, "", l));
    c.removeIf(x -> {
      try { return !(Boolean) isSupportedProviderLocale.invoke(cldr, x, langtags); }
      catch (Exception e) { throw new RuntimeException(e); }
    });
    return c;
  }

  @SuppressWarnings("unchecked")
  static List<Locale> fallbackChain(Locale l) throws Exception {
    List<Locale> c = new ArrayList<>((List<Locale>) getCandidateLocales.invoke(fallback, "", l));
    c.removeIf(x -> {
      try { return !(Boolean) isSupportedProviderLocale.invoke(fallback, x, fallbackTags); }
      catch (Exception e) { throw new RuntimeException(e); }
    });
    return c;
  }

  static String join(List<Locale> ls) {
    StringBuilder sb = new StringBuilder();
    for (Locale x : ls) {
      if (sb.length() > 0) sb.append('|');
      sb.append(x.toString());
    }
    return sb.toString();
  }

  /** The adapter types LocaleProviderAdapter.getAdapter picks for the PROVIDERS. */
  static String adapters(Locale l) throws Exception {
    StringBuilder sb = new StringBuilder();
    for (String p : PROVIDERS) {
      Object a = getAdapter.invoke(null, Class.forName(p), l);
      sb.append(a == cldr ? 'C' : a == fallback ? 'F' : '?');
    }
    return sb.toString();
  }

  /**
   * The class of a locale: the adapters picked for it, its weeded CLDR chain,
   * and its weeded FALLBACK chain (which a name the CLDR provider lacks comes
   * from).
   */
  static String key(Locale l) throws Exception {
    return adapters(l) + ";" + namePoolProviders(l) + ";" + join(chain(l)) + ";" + join(fallbackChain(l));
  }

  /**
   * The providers LocaleServiceProviderPool asks for calendar names, per
   * lookup locale of `l`: Calendar.getDisplayNames takes the first non-null
   * answer.
   */
  @SuppressWarnings("unchecked")
  static String namePoolProviders(Locale l) throws Exception {
    StringBuilder sb = new StringBuilder();
    for (Locale c : (List<Locale>) getLookupLocales.invoke(null, l)) {
      for (Object p : (List<Object>) findProviders.invoke(namePool, c, false)) {
        String n = p.getClass().getName();
        Object type = getField(p, Class.forName("sun.util.locale.provider.CalendarNameProviderImpl"), "type");
        sb.append(type.toString().charAt(0));
      }
      sb.append(',');
    }
    return sb.toString();
  }

  /**
   * The key of the time-zone names of a locale: its own weeded chain, then the
   * weeded chain of every other CLDR candidate of it, which the CLDR provider
   * consults, in order, for a name the locale lacks.
   */
  @SuppressWarnings("unchecked")
  static String zoneKey(Locale l) throws Exception {
    StringBuilder sb = new StringBuilder(key(l));
    List<Locale> cands = (List<Locale>) getCandidateLocales.invoke(cldr, "", l);
    for (int i = 1; i < cands.size(); i++) sb.append("||").append(key(cands.get(i)));
    return sb.toString();
  }

  static Locale withKeyword(Locale l, String key, String value) {
    return new Locale.Builder().setLocale(l).setUnicodeLocaleKeyword(key, value).build();
  }

  /** The tag `tag` with the extension subtags `ext` (such as "u-nu-latn"), placed before a private use. */
  static String withExtension(String tag, String ext) {
    int x = tag.indexOf("-x-");
    return x < 0 ? tag + "-" + ext : tag.substring(0, x) + "-" + ext + tag.substring(x);
  }

  static Locale fromTag(String tag) {
    return new Locale.Builder().setLanguageTag(tag).build();
  }

  /** A representative: a base tag, and whether it carries the neutral keyword va-qqqq. */
  static String rep(String base, boolean va) {
    return va ? base + "#va" : base;
  }

  /** The locale of representative `rep` with the extra Unicode keywords `kw` ("nu-latn" or ""). */
  static Locale repLocale(String rep, String kw) {
    boolean va = rep.endsWith("#va");
    String base = va ? rep.substring(0, rep.length() - 3) : rep;
    StringBuilder u = new StringBuilder();
    if (!kw.isEmpty()) u.append('-').append(kw);
    if (va) u.append("-va-qqqq");
    return fromTag(u.length() == 0 ? base : withExtension(base, "u" + u));
  }

  /** CalendarDataUtility.findRegionOverride. */
  static Locale regionOverride(Locale l) {
    String rg = l.getUnicodeLocaleType("rg");
    if (rg != null && rg.length() == 6) {
      rg = rg.toUpperCase(Locale.ROOT);
      if (rg.charAt(0) >= 'A' && rg.charAt(0) <= 'Z' && rg.charAt(1) >= 'A' && rg.charAt(1) <= 'Z'
          && rg.substring(2).equals("ZZZZ")) {
        return new Locale.Builder().setLocale(l).setRegion(rg.substring(0, 2)).build();
      }
    }
    return l;
  }

  /** The calendar type Calendar.getInstance(locale) creates. */
  static String calendarType(Locale l) {
    String type = l.getUnicodeLocaleType("ca");
    if (type == null) {
      return l.getCountry().equals("TH") && l.getLanguage().equals("th") ? "buddhist" : "gregory";
    }
    switch (type) {
      case "gregory": case "iso8601": return "gregory";
      case "buddhist": return "buddhist";
      case "japanese": return "japanese";
      default:
        if (l.getLanguage().equals("th") && l.getCountry().equals("TH")) return "buddhist";
        if (l.getVariant().equals("JP") && l.getLanguage().equals("ja") && l.getCountry().equals("JP")) return "japanese";
        return "gregory";
    }
  }

  // ------------------------------------------------------------------ data
  /**
   * The (field, style) pairs SimpleDateFormat.subParse asks Calendar.getDisplayNames
   * for: ERA, MONTH and AM_PM in the four non-narrow styles, DAY_OF_WEEK in LONG
   * and SHORT, YEAR in LONG and SHORT.
   */
  static final int[][] COMBOS = {
    {Calendar.ERA, Calendar.SHORT_FORMAT}, {Calendar.ERA, Calendar.SHORT_STANDALONE},
    {Calendar.ERA, Calendar.LONG_FORMAT}, {Calendar.ERA, Calendar.LONG_STANDALONE},
    {Calendar.MONTH, Calendar.SHORT_FORMAT}, {Calendar.MONTH, Calendar.SHORT_STANDALONE},
    {Calendar.MONTH, Calendar.LONG_FORMAT}, {Calendar.MONTH, Calendar.LONG_STANDALONE},
    {Calendar.DAY_OF_WEEK, Calendar.LONG}, {Calendar.DAY_OF_WEEK, Calendar.SHORT},
    {Calendar.AM_PM, Calendar.SHORT_FORMAT}, {Calendar.AM_PM, Calendar.SHORT_STANDALONE},
    {Calendar.AM_PM, Calendar.LONG_FORMAT}, {Calendar.AM_PM, Calendar.LONG_STANDALONE},
    {Calendar.YEAR, Calendar.LONG}, {Calendar.YEAR, Calendar.SHORT},
  };

  /**
   * Whether Calendar.getDisplayNames answers (field, style) from
   * DateFormatSymbols (the SHORT and LONG format styles, except the short
   * era) rather than from the calendar name provider; JapaneseImperialCalendar
   * always asks the provider, and BuddhistCalendar does for the eras.
   */
  static boolean fromSymbols(String type, int field, int style) {
    if (type.equals("japanese")) return false;
    if (type.equals("buddhist") && field == Calendar.ERA) return false;
    boolean format = style == Calendar.SHORT_FORMAT || style == Calendar.LONG_FORMAT;
    return format && !(field == Calendar.ERA && style == Calendar.SHORT_FORMAT) && field != Calendar.YEAR;
  }

  static final List<String> NULL = List.of("\u0000NULL");

  static List<String> names(Map<String, Integer> m) {
    if (m == null) return NULL;
    return new ArrayList<>(new TreeSet<>(m.keySet()));
  }

  /** The provider maps of a calendar type, NULL where the symbols answer. */
  static List<List<String>> providerRecord(Calendar cal, Locale l) {
    List<List<String>> r = new ArrayList<>();
    for (int[] c : COMBOS) {
      r.add(fromSymbols(cal.getCalendarType(), c[0], c[1]) ? NULL : names(cal.getDisplayNames(c[0], c[1], l)));
    }
    return r;
  }

  /** The maps a Gregorian calendar takes from DateFormatSymbols, NULL elsewhere. */
  static List<List<String>> symbolsRecord(Calendar gregorian, Locale l) {
    List<List<String>> r = new ArrayList<>();
    for (int[] c : COMBOS) {
      r.add(fromSymbols("gregory", c[0], c[1]) ? names(gregorian.getDisplayNames(c[0], c[1], l)) : NULL);
    }
    return r;
  }

  /** Every map a calendar answers for the combos, as SimpleDateFormat sees them. */
  static List<List<String>> actualRecord(Calendar cal, Locale l) {
    List<List<String>> r = new ArrayList<>();
    for (int[] c : COMBOS) r.add(names(cal.getDisplayNames(c[0], c[1], l)));
    return r;
  }

  static Object getField(Object o, Class<?> c, String name) throws Exception {
    Field f = c.getDeclaredField(name);
    f.setAccessible(true);
    return f.get(o);
  }

  /** The parse-relevant state of the integer NumberFormat SimpleDateFormat uses. */
  static List<String> numberRecord(NumberFormat nf0) throws Exception {
    DecimalFormat nf = (DecimalFormat) nf0.clone();
    nf.setGroupingUsed(false);
    DecimalFormatSymbols s = nf.getDecimalFormatSymbols();
    String p = nf.toPattern();
    boolean hasFollowingMinusSign = false;
    int separatorIndex = p.indexOf(';');
    if (separatorIndex > -1) {
      int minusIndex = p.indexOf('-', separatorIndex);
      if (minusIndex > p.lastIndexOf('0') && minusIndex > p.lastIndexOf('#')) hasFollowingMinusSign = true;
    }
    if (!nf.isParseIntegerOnly() || nf.isParseBigDecimal() || nf.getMultiplier() != 1 || nf.isStrict()) {
      throw new IllegalStateException("unexpected integer format " + p);
    }
    return List.of(
        String.valueOf(s.getZeroDigit()), String.valueOf(s.getDecimalSeparator()),
        String.valueOf(s.getMinusSign()), (String) getField(s, DecimalFormatSymbols.class, "minusSignText"),
        s.getExponentSeparator(), s.getNaN(), s.getInfinity(),
        nf.getPositivePrefix(), nf.getPositiveSuffix(), nf.getNegativePrefix(), nf.getNegativeSuffix(),
        hasFollowingMinusSign ? "1" : "0");
  }

  /**
   * Empties the JDK caches that time-zone names pass through. The CLDR
   * provider fills the missing names of a zone into the array LocaleResources
   * caches for a locale, so a locale computed after another one can inherit
   * that one's derived names; clearing the caches gives every locale the names
   * a JVM computes when that locale is the first it formats.
   */
  @SuppressWarnings("unchecked")
  static void clearCaches() throws Exception {
    ((Map<?, ?>) getField(cldr, Class.forName("sun.util.locale.provider.JRELocaleProviderAdapter"), "localeResourcesMap")).clear();
    Class<?> tznu = Class.forName("sun.util.locale.provider.TimeZoneNameUtility");
    ((Map<?, ?>) getField(null, tznu, "cachedZoneData")).clear();
    ((Map<?, ?>) getField(null, tznu, "cachedDisplayNames")).clear();
    ((Map<?, ?>) getField(null, DateFormatSymbols.class, "cachedInstances")).clear();
  }

  static String[][] zoneStrings(Locale l) throws Exception {
    clearCaches();
    return DateFormatSymbols.getInstance(l).getZoneStrings();
  }

  static String translit(String s, char zero) {
    StringBuilder sb = new StringBuilder(s.length());
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      sb.append(c >= '0' && c <= '9' ? (char) (zero + (c - '0')) : c);
    }
    return sb.toString();
  }

  // ------------------------------------------------------------ the classes
  static final class ClassData {
    String key;
    Locale rep;
    int[] names = new int[3];
    int symbols;
    int[] patterns = new int[3]; // string id + 1, 0 = MissingResourceException
    int number;
    TreeMap<String, Integer> nu = new TreeMap<>();
  }

  static final Map<List<List<String>>, Integer> nameIds = new HashMap<>();
  static final List<List<List<String>>> nameRecords = new ArrayList<>();
  static final Map<List<String>, Integer> setIds = new HashMap<>();
  static final List<List<String>> sets = new ArrayList<>();
  static final Map<List<String>, Integer> numberIds = new HashMap<>();
  static final List<List<String>> numberRecords = new ArrayList<>();
  static final Map<List<String>, Integer> rowIds = new HashMap<>();
  static final List<List<String>> rows = new ArrayList<>();
  static final Map<List<Integer>, Integer> tableIds = new HashMap<>();
  static final List<List<Integer>> tables = new ArrayList<>();

  static int setId(List<String> s) {
    return intern(setIds, sets, s);
  }

  static ClassData build(String key, String repTag, boolean extensionsKeepClass) throws Exception {
    Locale rep = repLocale(repTag, "");
    ClassData d = new ClassData();
    d.key = key;
    d.rep = rep;
    for (int t = 0; t < 3; t++) {
      Calendar cal = new Calendar.Builder().setLocale(rep).setCalendarType(CAL_TYPES[t]).build();
      d.names[t] = intern(nameIds, nameRecords, providerRecord(cal, rep));
      if (t == 0) d.symbols = intern(nameIds, nameRecords, symbolsRecord(cal, rep));
      Locale withCa = repLocale(repTag, "ca-" + CAL_TYPES[t]);
      if (extensionsKeepClass) {
        if (!key(withCa).equals(key)) throw new IllegalStateException("ca changes class " + repTag);
        d.patterns[t] = sid(((SimpleDateFormat) DateFormat.getDateInstance(DateFormat.DEFAULT, withCa)).toPattern()) + 1;
      } else {
        // only reachable without extensions: the default calendar type alone
        String own = ((SimpleDateFormat) DateFormat.getDateInstance(DateFormat.DEFAULT, rep)).toPattern();
        d.patterns[t] = calendarType(rep).equals(CAL_TYPES[t]) ? sid(own) + 1 : 0;
      }
    }
    List<String> def = numberRecord(NumberFormat.getIntegerInstance(rep));
    d.number = intern(numberIds, numberRecords, def);
    if (extensionsKeepClass) {
      for (String nuKey : nuKeys(rep)) {
        Locale n = repLocale(repTag, "nu-" + nuKey);
        if (!key(n).equals(key)) throw new IllegalStateException("nu changes class: " + n);
        List<String> rec = numberRecord(NumberFormat.getIntegerInstance(n));
        if (!rec.equals(def)) d.nu.put(nuKey, intern(numberIds, numberRecords, rec));
      }
    }
    return d;
  }

  static final class ZoneData {
    String key;
    String rep;
    int table;
    boolean latin; // masked entries hold Latin digits
  }

  static ZoneData buildZones(String key, String repTag, boolean extensionsKeepClass) throws Exception {
    Locale rep = repLocale(repTag, "");
    ZoneData d = new ZoneData();
    d.key = key;
    d.rep = repTag + (extensionsKeepClass ? "" : " (no extension)");
    String[][] zDefault = zoneStrings(rep);
    String[][] zStored = zDefault;
    int[] masks = new int[zDefault.length];
    if (extensionsKeepClass) {
      Locale latn = repLocale(repTag, "nu-latn"), arab = repLocale(repTag, "nu-arab");
      if (!zoneKey(latn).equals(key) || !zoneKey(arab).equals(key)) throw new IllegalStateException("nu changes zone class " + repTag);
      String[][] zLatn = zoneStrings(latn);
      String[][] zArab = zoneStrings(arab);
      if (zLatn.length != zDefault.length || zArab.length != zDefault.length) throw new IllegalStateException("zone rows differ " + rep);
      char zero = DecimalFormatSymbols.getInstance(rep).getZeroDigit();
      for (int r = 0; r < zLatn.length; r++) {
        for (int i = 0; i < zLatn[r].length; i++) {
          if (!zLatn[r][i].equals(zArab[r][i])) {
            if (i < 1 || i > 4) continue; // only names 1..4 are matched
            masks[r] |= 1 << (i - 1);
          }
        }
        for (int i = 1; i <= 4; i++) {
          String expected = (masks[r] & (1 << (i - 1))) != 0 ? translit(zLatn[r][i], zero) : zLatn[r][i];
          if (!expected.equals(zDefault[r][i])) throw new IllegalStateException("digit model fails " + rep + " " + Arrays.toString(zDefault[r]));
        }
      }
      zStored = zLatn;
      d.latin = true;
    }
    List<Integer> table = new ArrayList<>();
    for (int r = 0; r < zStored.length; r++) {
      String[] row = zStored[r];
      for (int i = 1; i <= 4; i++) if (row[i] == null || row[i].isEmpty()) throw new IllegalStateException("empty zone name " + rep + " " + Arrays.toString(row));
      String resolved = TimeZone.getTimeZone(row[0]).getID();
      table.add(intern(rowIds, rows, List.of(row[0], resolved, row[1], row[2], row[3], row[4], String.valueOf(masks[r]))));
    }
    d.table = intern(tableIds, tables, table);
    return d;
  }

  @SuppressWarnings("unchecked")
  static SortedSet<String> nuKeys(Locale l) throws Exception {
    Object ld = getLocaleData.invoke(cldr);
    ResourceBundle rb = (ResourceBundle) getNumberFormatData.invoke(ld, l);
    TreeSet<String> out = new TreeSet<>();
    for (String k : rb.keySet()) {
      if (k.endsWith(".NumberElements")) out.add(k.substring(0, k.length() - ".NumberElements".length()));
    }
    return out;
  }

  // ------------------------------------------------------------------ main
  @SuppressWarnings("unchecked")
  public static void main(String[] args) throws Exception {
    setup();
    run(args);
  }

  @SuppressWarnings("unchecked")
  static void setup() throws Exception {
    Class<?> lpa = Class.forName("sun.util.locale.provider.LocaleProviderAdapter");
    Class<?> typeC = Class.forName("sun.util.locale.provider.LocaleProviderAdapter$Type");
    Object cldrType = null;
    for (Object c : typeC.getEnumConstants()) if (c.toString().equals("CLDR")) cldrType = c;
    cldr = lpa.getMethod("forType", typeC).invoke(null, cldrType);
    for (Object c : typeC.getEnumConstants()) if (c.toString().equals("FALLBACK")) fallback = lpa.getMethod("forType", typeC).invoke(null, c);
    Class<?> jre = Class.forName("sun.util.locale.provider.JRELocaleProviderAdapter");
    getCandidateLocales = jre.getMethod("getCandidateLocales", String.class, Locale.class);
    isSupportedProviderLocale = lpa.getMethod("isSupportedProviderLocale", Locale.class, Set.class);
    getAdapter = lpa.getMethod("getAdapter", Class.class, Locale.class);
    Class<?> pool = Class.forName("sun.util.locale.provider.LocaleServiceProviderPool");
    getLookupLocales = pool.getDeclaredMethod("getLookupLocales", Locale.class);
    getLookupLocales.setAccessible(true);
    findProviders = pool.getDeclaredMethod("findProviders", Locale.class, boolean.class);
    findProviders.setAccessible(true);
    namePool = pool.getMethod("getPool", Class.class).invoke(null, java.util.spi.CalendarNameProvider.class);
    getLocaleResources = lpa.getMethod("getLocaleResources", Locale.class);
    Class<?> lrC = Class.forName("sun.util.locale.provider.LocaleResources");
    getDateTimePattern = lrC.getMethod("getDateTimePattern", int.class, int.class, Calendar.class);
    getLocaleData = cldr.getClass().getMethod("getLocaleData");
    getNumberFormatData = Class.forName("sun.util.resources.LocaleData").getMethod("getNumberFormatData", Locale.class);
    langtags = (Set<String>) jre.getMethod("getLanguageTagSet", String.class).invoke(cldr, "AvailableLocales");
    fallbackTags = (Set<String>) jre.getMethod("getLanguageTagSet", String.class).invoke(fallback, "FormatData");
    fallbackZoneTags = (Set<String>) jre.getMethod("getLanguageTagSet", String.class).invoke(fallback, "TimeZoneNames");
    System.err.println("fallback tags " + new TreeSet<>(fallbackTags) + " zones " + new TreeSet<>(fallbackZoneTags));
    Object meta = getField(null, cldr.getClass(), "baseMetaInfo");
    aliases = (Map<String, String>) meta.getClass().getMethod("getLanguageAliasMap").invoke(meta);
    tzCanonical = (Map<String, String>) meta.getClass().getMethod("tzCanonicalIDs").invoke(meta);
  }

  @SuppressWarnings("unchecked")
  static void run(String[] args) throws Exception {
    Object meta = getField(null, cldr.getClass(), "baseMetaInfo");
    Map<Locale, String[]> parents = (Map<Locale, String[]>) meta.getClass().getMethod("parentLocales").invoke(meta);
    Method likelyM = meta.getClass().getDeclaredMethod("likelyScriptMap");
    likelyM.setAccessible(true);
    Map<String, String> likely = (Map<String, String>) likelyM.invoke(meta);
    boolean nonlikely = (Boolean) getField(null, meta.getClass(), "nonlikelyScript");
    Class<?> zif = Class.forName("sun.util.calendar.ZoneInfoFile");
    String[] regions = (String[]) getField(null, zif, "regions");
    Map<String, String> zoneAliases = (Map<String, String>) getField(null, zif, "aliases");

    // ------------------------------------------------- enumerate the tags
    TreeMap<String, TreeSet<String>> scripts = new TreeMap<>(), regs = new TreeMap<>(), variants = new TreeMap<>();
    for (String t : new TreeSet<>(langtags)) {
      Locale l = Locale.forLanguageTag(t);
      String lang = l.getLanguage();
      scripts.computeIfAbsent(lang, k -> new TreeSet<>()).add(l.getScript());
      regs.computeIfAbsent(lang, k -> new TreeSet<>()).add(l.getCountry());
      variants.computeIfAbsent(lang, k -> new TreeSet<>()).add(l.getVariant());
    }
    // the subtags of the parent-locale children, which a membership test names too
    for (String[] children : parents.values()) {
      for (String t : children) {
        Locale l = Locale.forLanguageTag(t);
        scripts.computeIfAbsent(l.getLanguage(), k -> new TreeSet<>()).add(l.getScript());
        regs.computeIfAbsent(l.getLanguage(), k -> new TreeSet<>()).add(l.getCountry());
        variants.computeIfAbsent(l.getLanguage(), k -> new TreeSet<>()).add(l.getVariant());
      }
    }
    // the languages of the likely-script table
    for (String langs : likely.values()) {
      for (String lang : langs.trim().split("\\s+")) {
        scripts.computeIfAbsent(lang, k -> new TreeSet<>());
        regs.computeIfAbsent(lang, k -> new TreeSet<>());
        variants.computeIfAbsent(lang, k -> new TreeSet<>());
      }
    }
    TreeSet<String> tags = new TreeSet<>(aliases.keySet());
    tags.add("qaa");
    tags.add("und");
    // the root language with each kind of subtag
    for (String sc : new String[] {"", "Latn", "Qaaa"}) for (String r : new String[] {"", "US", "AA"}) for (String v : new String[] {"", "POSIX", "QQQQQ"}) {
      StringBuilder t = new StringBuilder("und");
      if (!sc.isEmpty()) t.append('-').append(sc);
      if (!r.isEmpty()) t.append('-').append(r);
      if (!v.isEmpty()) t.append('-').append(v);
      tags.add(t.toString());
    }
    for (String lang : new TreeSet<>(scripts.keySet())) {
      if (lang.isEmpty()) continue;
      TreeSet<String> sc = new TreeSet<>(scripts.get(lang));
      sc.add("");
      sc.add("Qaaa");
      for (Map.Entry<String, String> e : likely.entrySet()) if (e.getValue().contains(" " + lang + " ")) sc.add(e.getKey());
      TreeSet<String> rg = new TreeSet<>(regs.get(lang));
      rg.add("");
      rg.add("AA");
      if (lang.equals("zh")) rg.addAll(List.of("TW", "HK", "MO", "CN", "SG"));
      TreeSet<String> va = new TreeSet<>();
      for (String v : variants.get(lang)) { va.add(v); va.add(v.toLowerCase(Locale.ROOT)); }
      va.add("");
      va.add("QQQQQ");
      if (lang.equals("ja")) va.add("JP");
      if (lang.equals("th")) va.add("TH");
      if (lang.equals("no")) va.add("NY");
      for (String s : sc) for (String r : rg) for (String v : va) {
        StringBuilder t = new StringBuilder(lang);
        if (!s.isEmpty()) t.append('-').append(s);
        if (!r.isEmpty()) t.append('-').append(r);
        if (!v.isEmpty()) {
          if (v.length() >= 5 || Character.isDigit(v.charAt(0)) && v.length() == 4) t.append('-').append(v);
          else t.append("-x-lvariant-").append(v);
        }
        tags.add(t.toString());
      }
    }
    // classes: plain tags, and the same tags carrying an extension
    TreeMap<String, Locale> reps = new TreeMap<>();
    TreeMap<String, Boolean> repKeepsClass = new TreeMap<>();
    TreeMap<String, String> repTags = new TreeMap<>();
    TreeMap<String, String> zoneRepTags = new TreeMap<>();
    TreeMap<String, Boolean> zoneRepKeeps = new TreeMap<>();
    List<String> sample = new ArrayList<>();
    int n = 0;
    for (String t0 : tags) {
      String t = t0;
      Locale l;
      try {
        l = fromTag(t0);
      } catch (IllformedLocaleException e) {
        continue; // not a tag Locale.Builder accepts
      }
      t = l.toLanguageTag();
      String k = key(l);
      boolean aliased = aliases.containsKey(t);
      Locale ext;
      try {
        ext = fromTag(withExtension(t, "u-va-qqqq"));
      } catch (IllformedLocaleException e) {
        ext = null; // a legacy tag, which takes no extension
      }
      String kx = ext == null ? k : key(ext);
      // an extension leaves the class of a locale unchanged unless the locale is
      // a language alias (aliases apply to extension-free tags only) or the root
      // locale (whose extended forms the FALLBACK adapter serves)
      boolean keeps = !aliased && kx.equals(k);
      if (!aliased && !kx.equals(k) && !l.stripExtensions().equals(Locale.ROOT)) throw new IllegalStateException("extension changes class of " + t);
      String zk = zoneKey(l);
      String zkx = ext == null ? zk : zoneKey(ext);
      boolean zkeeps = !aliased && zkx.equals(zk);
      if (!aliased && !zkx.equals(zk) && !l.stripExtensions().equals(Locale.ROOT)) throw new IllegalStateException("extension changes zone class of " + t);
      if (!zoneRepTags.containsKey(zk) || (zkeeps && zoneRepKeeps.get(zk) == Boolean.FALSE)) {
        zoneRepTags.put(zk, t);
        zoneRepKeeps.put(zk, zkeeps);
      }
      if (ext != null && !zoneRepTags.containsKey(zkx)) {
        zoneRepTags.put(zkx, rep(t, true));
        zoneRepKeeps.put(zkx, true);
      }
      if (!reps.containsKey(k) || (keeps && repKeepsClass.get(k) == Boolean.FALSE)) {
        reps.put(k, l);
        repKeepsClass.put(k, keeps);
        repTags.put(k, t);
      }
      if (ext != null && !reps.containsKey(kx)) {
        reps.put(kx, ext);
        repKeepsClass.put(kx, true);
        repTags.put(kx, rep(t, true));
      }
      if (n++ % 37 == 0) sample.add(t);
    }
    System.err.println("tags=" + tags.size() + " classes=" + reps.size() + " zone classes=" + zoneRepTags.size());

    List<ClassData> classes = new ArrayList<>();
    for (Map.Entry<String, Locale> e : reps.entrySet()) {
      classes.add(build(e.getKey(), repTags.get(e.getKey()), repKeepsClass.get(e.getKey())));
    }
    TreeMap<String, ClassData> byKey = new TreeMap<>();
    for (ClassData c : classes) byKey.put(c.key, c);
    TreeMap<String, ZoneData> zonesByKey = new TreeMap<>();
    for (Map.Entry<String, String> e : zoneRepTags.entrySet()) {
      zonesByKey.put(e.getKey(), buildZones(e.getKey(), e.getValue(), zoneRepKeeps.get(e.getKey())));
    }
    System.err.println("names=" + nameRecords.size() + " numbers=" + numberRecords.size()
        + " rows=" + rows.size() + " tables=" + tables.size());

    // --------------------------------------------------------- validation
    String[] cas = {null, "gregory", "iso8601", "buddhist", "japanese", "islamic"};
    String[] nus = {null, "latn", "arab", "thai", "qqqq"};
    String[] rgs = {null, "gbzzzz", "thzzzz", "sazzzz", "uszz"};
    String[] tzs = {null, "usnyc", "gblon", "qqqqq"};
    int checked = 0;
    for (int i = 0; i < sample.size(); i++) {
      String baseTag = sample.get(i);
      List<Locale> variantsL = new ArrayList<>(List.of(fromTag(baseTag)));
      String ca = cas[i % cas.length], nu = nus[(i / 3) % nus.length], rg = rgs[(i / 7) % rgs.length], tz = tzs[(i / 11) % tzs.length];
      StringBuilder u = new StringBuilder("u");
      if (ca != null) u.append("-ca-").append(ca);
      if (nu != null) u.append("-nu-").append(nu);
      if (rg != null) u.append("-rg-").append(rg);
      if (tz != null) u.append("-tz-").append(tz);
      if (u.length() > 1) {
        try {
          variantsL.add(fromTag(withExtension(baseTag, u.toString())));
        } catch (IllformedLocaleException e) {
          // a legacy tag, which takes no extension
        }
      }
      for (Locale l : variantsL) {
        Locale sdfLocale;
        try {
          sdfLocale = regionOverride(l);
        } catch (IllformedLocaleException e) {
          // DateFormat.getDateInstance throws it too
          try { DateFormat.getDateInstance(DateFormat.DEFAULT, l); throw new IllegalStateException("expected throw " + l); }
          catch (IllformedLocaleException expected) { }
          clearCaches();
          validate(new SimpleDateFormat("", l), l, byKey, zonesByKey, false);
          checked += 1;
          continue;
        }
        clearCaches();
        validate((SimpleDateFormat) DateFormat.getDateInstance(DateFormat.DEFAULT, l), sdfLocale, byKey, zonesByKey, true);
        clearCaches();
        validate(new SimpleDateFormat("", l), l, byKey, zonesByKey, false);
        checked += 2;
      }
    }
    System.err.println("validated " + checked + " formats");

    // ------------------------------------------------------------- write
    ByteArrayOutputStream raw = new ByteArrayOutputStream();
    DataOut out = new DataOut(raw);
    out.bytes("JDP1".getBytes("US-ASCII"));
    // reserve string ids for every name first, in a stable order
    TreeSet<String> lt = new TreeSet<>(langtags);
    TreeMap<String, String> al = new TreeMap<>(aliases);
    TreeMap<String, String[]> par = new TreeMap<>();
    for (Map.Entry<Locale, String[]> e : parents.entrySet()) {
      String[] ch = e.getValue().clone();
      par.put(e.getKey().toLanguageTag(), ch);
    }
    TreeMap<String, String> lk = new TreeMap<>(likely);
    TreeMap<String, String> tzc = new TreeMap<>(tzCanonical);
    TreeSet<String> zoneIds = new TreeSet<>(Arrays.asList(regions));
    zoneIds.addAll(zoneAliases.keySet());

    ByteArrayOutputStream body = new ByteArrayOutputStream();
    DataOut o = new DataOut(body);
    o.v(lt.size()); for (String s : lt) o.v(sid(s));
    TreeSet<String> ft = new TreeSet<>(fallbackTags);
    o.v(ft.size()); for (String s : ft) o.v(sid(s));
    TreeSet<String> fz = new TreeSet<>(fallbackZoneTags);
    o.v(fz.size()); for (String s : fz) o.v(sid(s));
    o.v(al.size()); for (Map.Entry<String, String> e : al.entrySet()) { o.v(sid(e.getKey())); o.v(sid(e.getValue())); }
    o.v(par.size());
    for (Map.Entry<String, String[]> e : par.entrySet()) {
      o.v(sid(e.getKey()));
      o.v(e.getValue().length);
      for (String c : e.getValue()) o.v(sid(c));
    }
    o.v(lk.size()); for (Map.Entry<String, String> e : lk.entrySet()) { o.v(sid(e.getKey())); o.v(sid(e.getValue())); }
    o.v(nonlikely ? 1 : 0);
    o.v(tzc.size()); for (Map.Entry<String, String> e : tzc.entrySet()) { o.v(sid(e.getKey())); o.v(sid(e.getValue())); }
    o.v(zoneIds.size());
    for (String z : zoneIds) { o.v(sid(z)); o.v(sid(TimeZone.getTimeZone(z).getID())); }
    for (String t : CAL_TYPES) o.v(new Calendar.Builder().setCalendarType(t).build().isWeekDateSupported() ? 1 : 0);
    TreeSet<String> shortIds = new TreeSet<>(java.time.ZoneId.SHORT_IDS.keySet());
    o.v(shortIds.size()); for (String x : shortIds) o.v(sid(x));
    for (List<List<String>> r : nameRecords) for (List<String> x : r) setId(x);
    o.v(sets.size());
    for (List<String> x : sets) {
      if (x.equals(NULL)) { o.v(0); continue; }
      o.v(1); o.v(x.size()); for (String y : x) o.v(sid(y));
    }
    o.v(COMBOS.length);
    for (int[] c : COMBOS) { o.v(c[0]); o.v(c[1]); }
    o.v(nameRecords.size());
    for (List<List<String>> r : nameRecords) for (List<String> x : r) o.v(setId(x));
    o.v(numberRecords.size());
    for (List<String> r : numberRecords) {
      o.v(r.get(0).charAt(0)); o.v(r.get(1).charAt(0)); o.v(r.get(2).charAt(0));
      for (int i = 3; i <= 10; i++) o.v(sid(r.get(i)));
      o.v(Integer.parseInt(r.get(11)));
    }
    o.v(rows.size());
    for (List<String> r : rows) { for (int i = 0; i < 6; i++) o.v(sid(r.get(i))); o.v(Integer.parseInt(r.get(6))); }
    o.v(tables.size());
    for (List<Integer> t : tables) { o.v(t.size()); for (int r : t) o.v(r); }
    o.v(byKey.size());
    for (ClassData c : byKey.values()) {
      o.v(sid(c.key));
      for (int t = 0; t < 3; t++) o.v(c.names[t]);
      o.v(c.symbols);
      for (int t = 0; t < 3; t++) o.v(c.patterns[t]);
      o.v(c.number);
      o.v(c.nu.size());
      for (Map.Entry<String, Integer> e : c.nu.entrySet()) { o.v(sid(e.getKey())); o.v(e.getValue()); }
    }
    o.v(zonesByKey.size());
    for (ZoneData z : zonesByKey.values()) {
      o.v(sid(z.key));
      o.v(z.table);
      o.v(z.latin ? 1 : 0);
    }
    // strings last in the body order, first in the file
    out.v(strings.size());
    for (String s : strings) { byte[] u = s.getBytes("UTF-8"); out.v(u.length); out.bytes(u); }
    out.bytes(body.toByteArray());
    byte[] data = raw.toByteArray();
    Deflater def = new Deflater(Deflater.BEST_COMPRESSION);
    def.setInput(data);
    def.finish();
    ByteArrayOutputStream z = new ByteArrayOutputStream();
    byte[] buf = new byte[1 << 16];
    while (!def.finished()) z.write(buf, 0, def.deflate(buf));
    def.end();
    try (FileOutputStream f = new FileOutputStream(args[0])) { f.write(z.toByteArray()); }
    System.err.println("strings=" + strings.size() + " raw=" + data.length + " compressed=" + z.size());
  }

  // -------------------------------------------------------- the validation
  static void validate(SimpleDateFormat sdf, Locale sdfLocale, Map<String, ClassData> byKey, Map<String, ZoneData> zonesByKey, boolean dateInstance) throws Exception {
    String k = key(sdfLocale);
    ClassData c = byKey.get(k);
    if (c == null) throw new IllegalStateException("no class " + k + " for " + sdfLocale);
    String t = calendarType(sdfLocale);
    Calendar cal = sdf.getCalendar();
    if (!cal.getCalendarType().equals(t)) throw new IllegalStateException("calendar " + sdfLocale + " " + cal.getCalendarType() + " vs " + t);
    ClassData sym = byKey.get(key(regionOverride(sdfLocale)));
    List<List<String>> provider = nameRecords.get(c.names[indexOf(t)]);
    List<List<String>> symbols = nameRecords.get(sym.symbols);
    List<List<String>> actualNames = actualRecord(cal, sdfLocale);
    for (int i = 0; i < COMBOS.length; i++) {
      List<String> predicted = fromSymbols(t, COMBOS[i][0], COMBOS[i][1]) ? symbols.get(i) : provider.get(i);
      if (COMBOS[i][0] == Calendar.YEAR && !t.equals("japanese")) continue; // never asked
      if (!actualNames.get(i).equals(predicted)) throw new IllegalStateException("names " + sdfLocale.toLanguageTag() + " dateInstance=" + dateInstance + " t=" + t + " combo " + i + " " + actualNames.get(i) + " vs " + predicted);
    }
    if (dateInstance) {
      int p = c.patterns[indexOf(t)];
      if (p == 0) throw new IllegalStateException("unreachable pattern reached " + sdfLocale);
      String expected = strings.get(p - 1);
      if (!sdf.toPattern().equals(expected)) throw new IllegalStateException("pattern " + sdfLocale + " " + sdf.toPattern() + " vs " + expected);
    }
    // numbers
    Locale numberLocale = sdfLocale.getUnicodeLocaleType("nu") == null ? regionOverride(sdfLocale) : sdfLocale;
    List<String> num = predictNumber(numberLocale, byKey);
    if (!numberRecord(sdf.getNumberFormat()).equals(num)) throw new IllegalStateException("number " + sdfLocale);
    // zones
    char zero = predictNumber(sdfLocale.getUnicodeLocaleType("nu") == null ? regionOverride(sdfLocale) : sdfLocale, byKey).get(0).charAt(0);
    String[][] actual = sdf.getDateFormatSymbols().getZoneStrings();
    ZoneData zd = zonesByKey.get(zoneKey(sdfLocale));
    if (zd == null) throw new IllegalStateException("no zone class for " + sdfLocale);
    List<Integer> table = tables.get(zd.table);
    if (actual.length != table.size()) throw new IllegalStateException("zones " + sdfLocale);
    for (int r = 0; r < actual.length; r++) {
      List<String> row = rows.get(table.get(r));
      int mask = Integer.parseInt(row.get(6));
      if (!actual[r][0].equals(row.get(0))) throw new IllegalStateException("zone id " + sdfLocale);
      for (int i = 1; i <= 4; i++) {
        String expected = zd.latin && (mask & (1 << (i - 1))) != 0 ? translit(row.get(i + 1), zero) : row.get(i + 1);
        if (!actual[r][i].equals(expected)) throw new IllegalStateException("zone name " + sdfLocale + " " + Arrays.toString(actual[r]) + " vs " + expected + " rep " + zd.rep + " mask " + mask + " latin " + zd.latin + " key " + zd.key);
      }
    }
    // time zone
    String tz = sdfLocale.getUnicodeLocaleType("tz");
    String expectedTz = tz != null && tzCanonical.containsKey(tz) ? TimeZone.getTimeZone(tzCanonical.get(tz)).getID() : TimeZone.getDefault().getID();
    if (!sdf.getTimeZone().getID().equals(expectedTz)) throw new IllegalStateException("tz " + sdfLocale + " " + sdf.getTimeZone().getID());
  }

  static List<String> predictNumber(Locale l, Map<String, ClassData> byKey) throws Exception {
    ClassData c = byKey.get(key(l));
    if (c == null) throw new IllegalStateException("no number class for " + l);
    String nu = l.getUnicodeLocaleType("nu");
    Integer id = nu == null ? null : c.nu.get(nu);
    return numberRecords.get(id == null ? c.number : id);
  }

  static int indexOf(String t) {
    for (int i = 0; i < 3; i++) if (CAL_TYPES[i].equals(t)) return i;
    throw new IllegalArgumentException(t);
  }

  static final class DataOut {
    final OutputStream out;
    DataOut(OutputStream out) { this.out = out; }
    void v(long x) throws IOException {
      while ((x & ~0x7FL) != 0) { out.write((int) ((x & 0x7F) | 0x80)); x >>>= 7; }
      out.write((int) x);
    }
    void bytes(byte[] b) throws IOException { out.write(b); }
  }
}
