// Generates crates/rucene-queryparser/src/jdk/format_data.bin, the
// zlib-compressed JDK 25 locale data the emulation of java.text.DateFormat,
// java.text.SimpleDateFormat, java.text.DecimalFormat, java.text.NumberFormat
// and java.util.Calendar in crates/rucene-queryparser/src/jdk/ reads, from the
// running JDK (run it with JDK 25; see tools/translate/README.md for the
// command line, which opens the JDK internals read below).
//
// It extends tools/translate/JdkDateParseData.java (the data of the
// analysis-common SimpleDateFormat parsing emulation) with what formatting
// and computing a date need. What is dumped, and why:
// * the CLDR locale-provider metadata the JDK resolves a locale with
//   (available language tags, language aliases, parent locales, likely
//   scripts), so that the emulation can compute the weeded candidate chain
//   of any locale as sun.util.cldr.CLDRLocaleProviderAdapter and
//   sun.util.resources.LocaleData do;
// * for every distinct weeded chain ("class") reachable from a language tag
//   built from the subtags the JDK knows (plus one unseen value of each
//   subtag), the data a locale of that class gives: the
//   Calendar.getDisplayNames maps (name to value) SimpleDateFormat.parse
//   reads, the Calendar.getDisplayName names (value to name) and the
//   DateFormatSymbols arrays SimpleDateFormat.format reads, the
//   DateFormat.getDateInstance pattern of every style and calendar type, the
//   DecimalFormatSymbols and the number pattern of NumberFormat per
//   numbering system, and DateFormatSymbols.getZoneStrings();
// * the first day of the week and the minimal days in the first week per
//   region (java.util.Calendar's week data);
// * the time-zone IDs TimeZone.getTimeZone knows and the BCP 47 "tz" IDs.
// The data of every class is read through the JDK's public API on a
// representative locale; a validation pass compares, for a sample of
// locales with and without Unicode extensions, the JDK's own formats and
// calendars with the state the dumped data predicts, and the run fails if
// they differ. Deterministic: every collection is sorted.
import java.io.*;
import java.lang.reflect.*;
import java.text.*;
import java.util.*;
import java.util.zip.*;

public class JdkFormatData {
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

  /** A display-names map as a list alternating each name, sorted, and its value. */
  static List<String> names(Map<String, Integer> m) {
    if (m == null) return NULL;
    List<String> r = new ArrayList<>();
    for (Map.Entry<String, Integer> e : new TreeMap<>(m).entrySet()) {
      r.add(e.getKey());
      r.add(String.valueOf(e.getValue()));
    }
    return r;
  }

  // ------------------------------------------------------ formatting data
  /**
   * The (field, style) pairs SimpleDateFormat.subFormat asks
   * Calendar.getDisplayName for: ERA, MONTH, DAY_OF_WEEK and AM_PM in the
   * SHORT and LONG format styles, MONTH also in the standalone styles
   * ('L', and 'M' under forceStandaloneForm).
   */
  static final int[][] DCOMBOS = {
    {Calendar.ERA, Calendar.SHORT_FORMAT}, {Calendar.ERA, Calendar.LONG_FORMAT},
    {Calendar.MONTH, Calendar.SHORT_FORMAT}, {Calendar.MONTH, Calendar.LONG_FORMAT},
    {Calendar.MONTH, Calendar.SHORT_STANDALONE}, {Calendar.MONTH, Calendar.LONG_STANDALONE},
    {Calendar.DAY_OF_WEEK, Calendar.SHORT_FORMAT}, {Calendar.DAY_OF_WEEK, Calendar.LONG_FORMAT},
    {Calendar.AM_PM, Calendar.SHORT_FORMAT}, {Calendar.AM_PM, Calendar.LONG_FORMAT},
  };

  static final TimeZone UTC = TimeZone.getTimeZone("UTC");

  /** Instants at which `field` of a calendar of type `type` takes each of its values. */
  static List<Long> instants(String type, int field) {
    List<Long> r = new ArrayList<>();
    GregorianCalendar g = new GregorianCalendar(UTC, Locale.ROOT);
    switch (field) {
      case Calendar.ERA:
        for (int y : new int[] {-100, 1800, 1880, 1920, 1950, 2000, 2020}) {
          g.clear();
          if (y < 0) { g.set(Calendar.ERA, GregorianCalendar.BC); g.set(-y, 0, 15, 12, 0); }
          else g.set(y, 0, 15, 12, 0);
          r.add(g.getTimeInMillis());
        }
        break;
      case Calendar.MONTH:
        for (int m = 0; m < 12; m++) { g.clear(); g.set(2001, m, 15, 12, 0); r.add(g.getTimeInMillis()); }
        break;
      case Calendar.DAY_OF_WEEK:
        for (int d = 0; d < 7; d++) { g.clear(); g.set(2001, 0, 7 + d, 12, 0); r.add(g.getTimeInMillis()); }
        break;
      case Calendar.AM_PM:
        for (int h : new int[] {6, 18}) { g.clear(); g.set(2001, 0, 15, h, 0); r.add(g.getTimeInMillis()); }
        break;
      default:
        throw new IllegalArgumentException("field " + field);
    }
    return r;
  }

  /**
   * Calendar.getDisplayName(field, style, l) of a calendar of type `type`,
   * per DCOMBOS entry, as a list indexed by field value ("\\u0000" for null and
   * for a value no instant reaches).
   */
  static List<List<String>> displayRecord(String type, Locale l) {
    List<List<String>> r = new ArrayList<>();
    for (int[] c : DCOMBOS) {
      TreeMap<Integer, String> byValue = new TreeMap<>();
      for (long t : instants(type, c[0])) {
        Calendar cal = new Calendar.Builder().setLocale(l).setCalendarType(type).setTimeZone(UTC).setInstant(t).build();
        int v = cal.get(c[0]);
        String name = cal.getDisplayName(c[0], c[1], l);
        String prev = byValue.put(v, name == null ? "\u0000" : name);
        if (prev != null && !prev.equals(byValue.get(v))) throw new IllegalStateException("display name of " + v + " varies " + l);
      }
      List<String> arr = new ArrayList<>();
      int max = byValue.isEmpty() ? -1 : byValue.lastKey();
      for (int v = 0; v <= max; v++) arr.add(byValue.getOrDefault(v, "\u0000"));
      r.add(arr);
    }
    return r;
  }

  /** The index in formatSymbols of the array Calendar.getDisplayName reads for (field, style). */
  static int symbolsIndex(int field, int style) {
    boolean longStyle = (style & Calendar.LONG) != 0;
    switch (field) {
      case Calendar.ERA: return 0;
      case Calendar.MONTH: return longStyle ? 1 : 2;
      case Calendar.DAY_OF_WEEK: return longStyle ? 3 : 4;
      case Calendar.AM_PM: return 5;
      default: throw new IllegalArgumentException("field " + field);
    }
  }

  /** DateFormatSymbols.getEras, getMonths, getShortMonths, getWeekdays, getShortWeekdays, getAmPmStrings. */
  static List<List<String>> formatSymbols(DateFormatSymbols s) {
    return List.of(Arrays.asList(s.getEras()), Arrays.asList(s.getMonths()), Arrays.asList(s.getShortMonths()),
        Arrays.asList(s.getWeekdays()), Arrays.asList(s.getShortWeekdays()), Arrays.asList(s.getAmPmStrings()));
  }

  /**
   * The DecimalFormatSymbols of a number format and the pattern
   * NumberFormat.getNumberInstance gives it: the char symbols as
   * one-character strings, then the string symbols, then the pattern. The
   * currency symbols, which depend on the region rather than on the class
   * and which no number-instance pattern reads, are left out.
   */
  static List<String> numberRecord(NumberFormat nf0) throws Exception {
    DecimalFormat nf = (DecimalFormat) nf0;
    DecimalFormatSymbols s = nf.getDecimalFormatSymbols();
    String p = nf.toPattern();
    // the pattern reproduces the format: the emulation rebuilds it from it
    DecimalFormat rebuilt = new DecimalFormat(p, s);
    if (!rebuilt.equals(nf) || !rebuilt.toPattern().equals(p)) throw new IllegalStateException("pattern does not rebuild " + p);
    return List.of(
        String.valueOf(s.getZeroDigit()), String.valueOf(s.getGroupingSeparator()),
        String.valueOf(s.getDecimalSeparator()), String.valueOf(s.getPerMill()),
        String.valueOf(s.getPercent()), String.valueOf(s.getDigit()),
        String.valueOf(s.getPatternSeparator()), String.valueOf(s.getMinusSign()),
        String.valueOf(s.getMonetaryDecimalSeparator()), String.valueOf(s.getMonetaryGroupingSeparator()),
        String.valueOf((char) (Character) getField(s, DecimalFormatSymbols.class, "exponential")),
        s.getInfinity(), s.getNaN(), s.getExponentSeparator(),
        (String) getField(s, DecimalFormatSymbols.class, "perMillText"),
        (String) getField(s, DecimalFormatSymbols.class, "percentText"),
        (String) getField(s, DecimalFormatSymbols.class, "minusSignText"),
        p);
  }

  /** The number record of NumberFormat.getNumberInstance(l). */
  static List<String> numberRecordOf(Locale l) throws Exception {
    return numberRecord(NumberFormat.getNumberInstance(l));
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
    int[][] patterns = new int[4][3]; // [style][calendar type]: string id + 1, 0 = unreachable
    int number;
    TreeMap<String, Integer> nu = new TreeMap<>();
    int formatSymbols;
    int[] display = new int[3];
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
  /** String arrays (DateFormatSymbols arrays and display-name arrays); "\u0000" is null. */
  static final Map<List<String>, Integer> arrayIds = new HashMap<>();
  static final List<List<String>> arrays = new ArrayList<>();
  /** Records of arrays: the six DateFormatSymbols arrays, or one display-name array per DCOMBOS entry. */
  static final Map<List<Integer>, Integer> arrayRecordIds = new HashMap<>();
  static final List<List<Integer>> arrayRecords = new ArrayList<>();

  static int setId(List<String> s) {
    return intern(setIds, sets, s);
  }

  static int arrayRecord(List<List<String>> rec) {
    List<Integer> ids = new ArrayList<>();
    for (List<String> a : rec) ids.add(intern(arrayIds, arrays, a));
    return intern(arrayRecordIds, arrayRecords, ids);
  }

  static ClassData build(String key, String repTag, boolean extensionsKeepClass) throws Exception {
    Locale rep = repLocale(repTag, "");
    ClassData d = new ClassData();
    d.key = key;
    d.rep = rep;
    d.formatSymbols = arrayRecord(formatSymbols(DateFormatSymbols.getInstance(rep)));
    for (int t = 0; t < 3; t++) {
      Calendar cal = new Calendar.Builder().setLocale(rep).setCalendarType(CAL_TYPES[t]).build();
      d.names[t] = intern(nameIds, nameRecords, providerRecord(cal, rep));
      if (t == 0) d.symbols = intern(nameIds, nameRecords, symbolsRecord(cal, rep));
      d.display[t] = arrayRecord(displayRecord(CAL_TYPES[t], rep));
      Locale withCa = repLocale(repTag, "ca-" + CAL_TYPES[t]);
      for (int style = 0; style < 4; style++) {
        if (extensionsKeepClass) {
          if (!key(withCa).equals(key)) throw new IllegalStateException("ca changes class " + repTag);
          d.patterns[style][t] = sid(((SimpleDateFormat) DateFormat.getDateInstance(style, withCa)).toPattern()) + 1;
        } else {
          // only reachable without extensions: the default calendar type alone
          String own = ((SimpleDateFormat) DateFormat.getDateInstance(style, rep)).toPattern();
          d.patterns[style][t] = calendarType(rep).equals(CAL_TYPES[t]) ? sid(own) + 1 : 0;
        }
      }
    }
    List<String> def = numberRecordOf(rep);
    d.number = intern(numberIds, numberRecords, def);
    if (extensionsKeepClass) {
      for (String nuKey : nuKeys(rep)) {
        Locale n = repLocale(repTag, "nu-" + nuKey);
        if (!key(n).equals(key)) throw new IllegalStateException("nu changes class: " + n);
        List<String> rec = numberRecordOf(n);
        if (!rec.equals(def)) d.nu.put(nuKey, intern(numberIds, numberRecords, rec));
      }
    }
    return d;
  }

  // ------------------------------------------------------------ week data
  /** Region -> {firstDayOfWeek, minimalDaysInFirstWeek}, for every two-letter and three-digit region. */
  static final TreeMap<String, int[]> weekData = new TreeMap<>();
  static int[] weekDefault;

  static int[] week(Locale l) {
    Calendar c = Calendar.getInstance(UTC, l);
    return new int[] {c.getFirstDayOfWeek(), c.getMinimalDaysInFirstWeek()};
  }

  static void buildWeekData() {
    weekDefault = week(Locale.forLanguageTag("und-QQ"));
    List<String> regionCodes = new ArrayList<>();
    for (char a = 'A'; a <= 'Z'; a++) for (char b = 'A'; b <= 'Z'; b++) regionCodes.add("" + a + b);
    for (int i = 0; i < 1000; i++) regionCodes.add(String.format(Locale.ROOT, "%03d", i));
    for (String r : regionCodes) {
      int[] w = week(new Locale.Builder().setLanguageTag("und-" + r).build());
      if (!Arrays.equals(w, weekDefault)) weekData.put(r, w);
    }
    int[] empty = week(Locale.ROOT);
    // a locale without a region takes the US data (CLDRCalendarDataProviderImpl.findValue)
    if (!Arrays.equals(empty, weekData.getOrDefault("US", weekDefault))) throw new IllegalStateException("root week data");
  }

  /** The week data the emulation predicts for `l`: the -u-fw- keyword, then the region override's region. */
  static int[] predictWeek(Locale l) {
    // Calendar.Builder gives an iso8601 calendar the ISO 8601 week definition
    if ("iso8601".equals(l.getUnicodeLocaleType("ca"))) return new int[] {Calendar.MONDAY, 4};
    Locale r = regionOverride(l);
    String region = r.getCountry().isEmpty() ? "US" : r.getCountry();
    int[] w = weekData.getOrDefault(region, weekDefault).clone();
    String fw = l.getUnicodeLocaleType("fw");
    if (fw != null) {
      int i = Arrays.asList("sun", "mon", "tue", "wed", "thu", "fri", "sat").indexOf(fw.toLowerCase(Locale.ROOT));
      if (i >= 0) w[0] = i + 1;
    }
    return w;
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
    String[] fws = {null, "mon", "sat", "qqq"};
    buildWeekData();
    int checked = 0;
    for (int i = 0; i < sample.size(); i++) {
      String baseTag = sample.get(i);
      List<Locale> variantsL = new ArrayList<>(List.of(fromTag(baseTag)));
      String ca = cas[i % cas.length], nu = nus[(i / 3) % nus.length], rg = rgs[(i / 7) % rgs.length], tz = tzs[(i / 11) % tzs.length];
      String fw = fws[(i / 5) % fws.length];
      StringBuilder u = new StringBuilder("u");
      if (ca != null) u.append("-ca-").append(ca);
      if (fw != null) u.append("-fw-").append(fw);
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
          validate(new SimpleDateFormat("", l), l, byKey, zonesByKey, -1);
          checked += 1;
          continue;
        }
        for (int style = 0; style < 4; style++) {
          clearCaches();
          validate((SimpleDateFormat) DateFormat.getDateInstance(style, l), sdfLocale, byKey, zonesByKey, style);
          checked += 1;
        }
        clearCaches();
        validate(new SimpleDateFormat("", l), l, byKey, zonesByKey, -1);
        checked += 1;
        // NumberFormat.getNumberInstance and getIntegerInstance
        Locale numberLocale = l.getUnicodeLocaleType("nu") == null ? sdfLocale : l;
        List<String> predicted = predictNumber(numberLocale, byKey);
        if (!numberRecord(NumberFormat.getNumberInstance(l)).equals(predicted)) throw new IllegalStateException("number instance " + l);
        DecimalFormat integer = (DecimalFormat) NumberFormat.getIntegerInstance(l);
        DecimalFormat expectedInteger = new DecimalFormat(predicted.get(predicted.size() - 1), integer.getDecimalFormatSymbols());
        expectedInteger.setMaximumFractionDigits(0);
        expectedInteger.setDecimalSeparatorAlwaysShown(false);
        expectedInteger.setParseIntegerOnly(true);
        if (!expectedInteger.equals(integer)) throw new IllegalStateException("integer instance " + l);
        if (!NumberFormat.getInstance(l).equals(NumberFormat.getNumberInstance(l))) throw new IllegalStateException("getInstance " + l);
        // week data
        int[] w = week(l);
        if (!Arrays.equals(w, predictWeek(l))) throw new IllegalStateException("week data " + l + " " + Arrays.toString(w) + " vs " + Arrays.toString(predictWeek(l)));
        checked += 1;
      }
    }
    System.err.println("validated " + checked + " formats");

    // ------------------------------------------------------------- write
    ByteArrayOutputStream raw = new ByteArrayOutputStream();
    DataOut out = new DataOut(raw);
    out.bytes("JFD1".getBytes("US-ASCII"));
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
      o.v(1); o.v(x.size() / 2);
      for (int i = 0; i < x.size(); i += 2) { o.v(sid(x.get(i))); o.v(Integer.parseInt(x.get(i + 1))); }
    }
    o.v(COMBOS.length);
    for (int[] c : COMBOS) { o.v(c[0]); o.v(c[1]); }
    o.v(nameRecords.size());
    for (List<List<String>> r : nameRecords) for (List<String> x : r) o.v(setId(x));
    o.v(numberRecords.size());
    for (List<String> r : numberRecords) {
      for (int i = 0; i <= 10; i++) o.v(r.get(i).charAt(0));
      for (int i = 11; i < r.size(); i++) o.v(sid(r.get(i)));
    }
    o.v(rows.size());
    for (List<String> r : rows) { for (int i = 0; i < 6; i++) o.v(sid(r.get(i))); o.v(Integer.parseInt(r.get(6))); }
    o.v(tables.size());
    for (List<Integer> t : tables) { o.v(t.size()); for (int r : t) o.v(r); }
    o.v(arrays.size());
    for (List<String> a : arrays) {
      o.v(a.size());
      for (String x : a) o.v(x.equals("\u0000") ? 0 : sid(x) + 1);
    }
    o.v(DCOMBOS.length);
    for (int[] c : DCOMBOS) { o.v(c[0]); o.v(c[1]); }
    o.v(arrayRecords.size());
    for (List<Integer> r : arrayRecords) { o.v(r.size()); for (int x : r) o.v(x); }
    o.v(weekDefault[0]); o.v(weekDefault[1]);
    o.v(weekData.size());
    for (Map.Entry<String, int[]> e : weekData.entrySet()) { o.v(sid(e.getKey())); o.v(e.getValue()[0]); o.v(e.getValue()[1]); }
    o.v(byKey.size());
    for (ClassData c : byKey.values()) {
      o.v(sid(c.key));
      for (int t = 0; t < 3; t++) o.v(c.names[t]);
      o.v(c.symbols);
      for (int style = 0; style < 4; style++) for (int t = 0; t < 3; t++) o.v(c.patterns[style][t]);
      o.v(c.number);
      o.v(c.nu.size());
      for (Map.Entry<String, Integer> e : c.nu.entrySet()) { o.v(sid(e.getKey())); o.v(e.getValue()); }
      o.v(c.formatSymbols);
      for (int t = 0; t < 3; t++) o.v(c.display[t]);
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
  static void validate(SimpleDateFormat sdf, Locale sdfLocale, Map<String, ClassData> byKey, Map<String, ZoneData> zonesByKey, int dateStyle) throws Exception {
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
      if (!actualNames.get(i).equals(predicted)) throw new IllegalStateException("names " + sdfLocale.toLanguageTag() + " style=" + dateStyle + " t=" + t + " combo " + i + " " + actualNames.get(i) + " vs " + predicted);
    }
    // formatting: the DateFormatSymbols arrays and the calendar's display names
    List<Integer> fs = arrayRecords.get(sym.formatSymbols);
    List<List<String>> actualFs = formatSymbols(sdf.getDateFormatSymbols());
    for (int i = 0; i < 6; i++) {
      if (!arrays.get(fs.get(i)).equals(actualFs.get(i))) throw new IllegalStateException("format symbols " + sdfLocale + " " + i);
    }
    // Calendar.getDisplayName answers the format styles of a Gregorian
    // calendar (but its short era) from DateFormatSymbols, which reads the
    // region override; the other names come from the calendar-name provider
    // of the locale's own class
    List<Integer> disp = arrayRecords.get(c.display[indexOf(t)]);
    List<List<String>> actualDisp = displayRecord(t, sdfLocale);
    for (int i = 0; i < DCOMBOS.length; i++) {
      List<String> actual = actualDisp.get(i);
      if (fromSymbols(t, DCOMBOS[i][0], DCOMBOS[i][1])) {
        List<String> strings = arrays.get(fs.get(symbolsIndex(DCOMBOS[i][0], DCOMBOS[i][1])));
        for (int v = 0; v < actual.size(); v++) {
          String expected = v < strings.size() ? strings.get(v) : "\u0000";
          if (!actual.get(v).equals("\u0000") && !actual.get(v).equals(expected)) throw new IllegalStateException("display names (symbols) " + sdfLocale + " " + i + " " + actual + " vs " + strings);
        }
      } else if (!arrays.get(disp.get(i)).equals(actual)) {
        throw new IllegalStateException("display names " + sdfLocale + " " + i + " " + actual + " vs " + arrays.get(disp.get(i)));
      }
    }
    if (dateStyle >= 0) {
      int p = c.patterns[dateStyle][indexOf(t)];
      if (p == 0) throw new IllegalStateException("unreachable pattern reached " + sdfLocale);
      String expected = strings.get(p - 1);
      if (!sdf.toPattern().equals(expected)) throw new IllegalStateException("pattern " + sdfLocale + " " + sdf.toPattern() + " vs " + expected);
    }
    // numbers: the integer format, grouping off
    Locale numberLocale = sdfLocale.getUnicodeLocaleType("nu") == null ? regionOverride(sdfLocale) : sdfLocale;
    List<String> num = predictNumber(numberLocale, byKey);
    DecimalFormat expectedNumber = new DecimalFormat(num.get(num.size() - 1), ((DecimalFormat) sdf.getNumberFormat()).getDecimalFormatSymbols());
    expectedNumber.setMaximumFractionDigits(0);
    expectedNumber.setDecimalSeparatorAlwaysShown(false);
    expectedNumber.setParseIntegerOnly(true);
    expectedNumber.setGroupingUsed(false);
    if (!expectedNumber.equals(sdf.getNumberFormat())) throw new IllegalStateException("number " + sdfLocale);
    if (!numberRecord(NumberFormat.getNumberInstance(numberLocale)).equals(num)) throw new IllegalStateException("number class " + sdfLocale);
    if (!((DecimalFormat) sdf.getNumberFormat()).getDecimalFormatSymbols().equals(((DecimalFormat) NumberFormat.getNumberInstance(numberLocale)).getDecimalFormatSymbols())) throw new IllegalStateException("number symbols " + sdfLocale);
    // the calendar's week data
    if (!Arrays.equals(new int[] {cal.getFirstDayOfWeek(), cal.getMinimalDaysInFirstWeek()}, predictWeek(sdfLocale))) throw new IllegalStateException("calendar week data " + sdfLocale);
    // zones
    char zero = num.get(0).charAt(0);
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
