// Generates crates/rucene-analysis-common/src/jdk/date_format_data.rs from the
// running JDK's public java.text / java.util API (run it with JDK 25):
//
//   java tools/translate/JdkDateFormatData.java \
//     > crates/rucene-analysis-common/src/jdk/date_format_data.rs
//
// For every locale of DateFormat.getAvailableLocales() without extensions it
// records what SimpleDateFormat.parse reads: the DEFAULT date pattern, the
// DateFormatSymbols arrays, the MONTH display names of the four non-narrow
// styles (the 'L' field), and the integer NumberFormat's symbols and affixes.
// Deterministic: the output depends only on the JDK's locale data.
import java.text.*;
import java.util.*;

public class JdkDateFormatData {
  static final List<String> strings = new ArrayList<>();
  static final Map<String, Integer> stringIds = new HashMap<>();
  static final List<String> records = new ArrayList<>();
  static final Map<String, Integer> recordIds = new HashMap<>();

  static int id(String s) {
    return stringIds.computeIfAbsent(s, k -> { strings.add(k); return strings.size() - 1; });
  }

  static String ids(String[] values) {
    StringBuilder sb = new StringBuilder("&[");
    for (int i = 0; i < values.length; i++) {
      if (i > 0) sb.append(", ");
      sb.append(id(values[i] == null ? "" : values[i]));
    }
    return sb.append("]").toString();
  }

  static String rustString(String s) {
    StringBuilder sb = new StringBuilder("\"");
    for (int i = 0; i < s.length(); ) {
      int cp = s.codePointAt(i);
      i += Character.charCount(cp);
      if (cp == '"' || cp == '\\') sb.append('\\').appendCodePoint(cp);
      else if (cp >= 0x20 && cp < 0x7F) sb.appendCodePoint(cp);
      else sb.append(String.format("\\u{%X}", cp));
    }
    return sb.append("\"").toString();
  }

  static String rustChar(char c) {
    return String.format("'\\u{%X}'", (int) c);
  }

  public static void main(String[] args) {
    Locale[] available = DateFormat.getAvailableLocales();
    // getAvailableLocales() has no stable order; sort for a deterministic output.
    Arrays.sort(available, Comparator.comparing(Locale::toString));
    TreeMap<String, Integer> locales = new TreeMap<>();
    for (Locale l : available) {
      if (l.hasExtensions()) continue;
      String key = l.getLanguage() + "_" + l.getScript() + "_" + l.getCountry() + "_" + l.getVariant();
      SimpleDateFormat df = (SimpleDateFormat) DateFormat.getDateInstance(DateFormat.DEFAULT, l);
      if (!(df.getCalendar() instanceof GregorianCalendar)) {
        throw new IllegalStateException("non-Gregorian calendar for " + l + ": " + df.getCalendar());
      }
      DateFormatSymbols s = DateFormatSymbols.getInstance(l);
      Calendar cal = Calendar.getInstance(l);
      TreeSet<String> standalone = new TreeSet<>();
      for (int style : new int[] {Calendar.SHORT_FORMAT, Calendar.SHORT_STANDALONE,
          Calendar.LONG_FORMAT, Calendar.LONG_STANDALONE}) {
        Map<String, Integer> names = cal.getDisplayNames(Calendar.MONTH, style, l);
        if (names != null) standalone.addAll(names.keySet());
      }
      DecimalFormat nf = (DecimalFormat) NumberFormat.getIntegerInstance(l);
      nf.setGroupingUsed(false);
      nf.setDecimalSeparatorAlwaysShown(false);
      nf.setParseIntegerOnly(true);
      nf.setMinimumFractionDigits(0);
      DecimalFormatSymbols sym = nf.getDecimalFormatSymbols();
      // SimpleDateFormat.checkNegativeNumberExpression
      String numberPattern = nf.toPattern();
      boolean hasFollowingMinusSign = false;
      int separatorIndex = numberPattern.indexOf(';');
      if (separatorIndex > -1) {
        int minusIndex = numberPattern.indexOf('-', separatorIndex);
        if (minusIndex > numberPattern.lastIndexOf('0') && minusIndex > numberPattern.lastIndexOf('#')) {
          hasFollowingMinusSign = true;
        }
      }
      // DecimalFormatSymbols.getMinusSignText() is not public: for an integer
      // pattern without a negative subpattern the negative prefix is the minus
      // sign text before the (empty) positive prefix.
      if (!nf.getPositivePrefix().isEmpty() || !nf.getNegativeSuffix().equals(nf.getPositiveSuffix())
          || numberPattern.indexOf(';') > -1) {
        throw new IllegalStateException("unexpected integer pattern for " + l + ": " + numberPattern);
      }
      String minusSignText = nf.getNegativePrefix();
      String record = "    LocaleDateData {\n"
          + "        default_pattern: " + id(df.toPattern()) + ",\n"
          + "        eras: " + ids(s.getEras()) + ",\n"
          + "        months: " + ids(s.getMonths()) + ",\n"
          + "        short_months: " + ids(s.getShortMonths()) + ",\n"
          + "        weekdays: " + ids(s.getWeekdays()) + ",\n"
          + "        short_weekdays: " + ids(s.getShortWeekdays()) + ",\n"
          + "        am_pm: " + ids(s.getAmPmStrings()) + ",\n"
          + "        month_names: " + ids(standalone.toArray(new String[0])) + ",\n"
          + "        zero_digit: " + rustChar(sym.getZeroDigit()) + ",\n"
          + "        decimal_separator: " + rustChar(sym.getDecimalSeparator()) + ",\n"
          + "        minus_sign: " + rustChar(sym.getMinusSign()) + ",\n"
          + "        minus_sign_text: " + id(minusSignText) + ",\n"
          + "        exponent_separator: " + id(sym.getExponentSeparator()) + ",\n"
          + "        nan: " + id(sym.getNaN()) + ",\n"
          + "        infinity: " + id(sym.getInfinity()) + ",\n"
          + "        positive_prefix: " + id(nf.getPositivePrefix()) + ",\n"
          + "        positive_suffix: " + id(nf.getPositiveSuffix()) + ",\n"
          + "        negative_prefix: " + id(nf.getNegativePrefix()) + ",\n"
          + "        negative_suffix: " + id(nf.getNegativeSuffix()) + ",\n"
          + "        has_following_minus_sign: " + hasFollowingMinusSign + ",\n"
          + "    },\n";
      int recordId = recordIds.computeIfAbsent(record, k -> { records.add(k); return records.size() - 1; });
      locales.put(key, recordId);
    }

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
        "//! The locale data `java.text.SimpleDateFormat.parse` reads, for every",
        "//! locale (without extensions) of `DateFormat.getAvailableLocales()`, as",
        "//! [`simple_date_format`](super::simple_date_format) reads it.",
        "//!",
        "//! Generated by `tools/translate/JdkDateFormatData.java` from OpenJDK "
            + Runtime.version() + "",
        "//! (its CLDR locale data, read through the public API); no JDK code is",
        "//! reproduced. Do not edit by hand.",
        "",
        "use super::simple_date_format::LocaleDateData;",
        "",
        "/// Every distinct string the records name, by index.",
        "pub(super) const STRINGS: &[&str] = &[",
        ""));
    for (String s : strings) out.append("    ").append(rustString(s)).append(",\n");
    out.append("];\n\n/// The distinct locale records.\npub(super) const RECORDS: &[LocaleDateData] = &[\n");
    for (String r : records) out.append(r);
    out.append("];\n\n/// The locales, keyed `language_script_country_variant` and sorted, with\n/// the index of their record.\npub(super) const LOCALES: &[(&str, u16)] = &[\n");
    for (Map.Entry<String, Integer> e : locales.entrySet()) {
      out.append("    (").append(rustString(e.getKey())).append(", ").append(e.getValue()).append("),\n");
    }
    out.append("];\n");
    System.out.print(out);
    System.err.println("locales=" + locales.size() + " records=" + records.size() + " strings=" + strings.size());
  }
}
