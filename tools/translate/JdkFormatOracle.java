/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
// The JDK 25 oracle of the locale-sensitive formats the query parsers call
// (crates/rucene-queryparser/src/jdk/, rmp #476), run by
// crates/rucene-queryparser/tests/jdk_format_oracle.rs as a single source file
// on the public API only (no --add-exports), and writing nothing but standard
// output:
//
//   java JdkFormatOracle.java <seed> <shard> <shards>
//
// For every locale of Locale.getAvailableLocales() (sorted by toString())
// whose index modulo <shards> is <shard>, with a java.util.Random seeded from
// <seed> and the index, it prints:
//
//   locale <tag> <round trip> <language> <script> <country> <variant>
//       the locale: toLanguageTag(), whether Locale.Builder().setLanguageTag
//       (tag) gives an equal locale, and its fields;
//   dpattern <pattern> | dpattern-error <class> <message>
//       DateFormat.getDateInstance(DateFormat.SHORT, l): toPattern();
//   dzone <id>
//       the time zone set on that format (setTimeZone, seeded);
//   dformat <millis> <text>
//       format(new Date(millis));
//   dparse <lenient> <input> <index> <error index> <millis or null>
//       setLenient(lenient), then parse(input, new ParsePosition(0));
//   nformat <kind> <number> <text>
//       NumberFormat.getNumberInstance(l) (kind "number") or
//       getIntegerInstance(l) (kind "integer"): format(number), the number as
//       L:<long>, I:<int>, D:<double bits> or F:<float bits>;
//   nparse <kind> <input> <index> <error index> <result>
//       parse(input, new ParsePosition(0)): L:<long>, D:<double bits> or null;
//   nformat-error / nparse-error <kind> ... <class> <message>
//       an exception instead;
//   lower <input> <output>
//       input.toLowerCase(l).
//
// Strings are escaped: printable ASCII except the backslash as is, every
// other UTF-16 unit as a backslash, `u` and four upper-case hex digits.
import java.text.DateFormat;
import java.text.NumberFormat;
import java.text.ParsePosition;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.Date;
import java.util.List;
import java.util.Locale;
import java.util.Random;
import java.util.TimeZone;

public class JdkFormatOracle {
  static final String[] ZONES = {
    "UTC", "America/New_York", "Asia/Tokyo", "Europe/Lisbon", "Australia/Lord_Howe",
    "Asia/Kolkata", "Pacific/Chatham", "America/St_Johns", "Africa/Casablanca",
    "Pacific/Kiritimati", "America/Sao_Paulo", "Asia/Tehran",
  };

  static final long[] FIXED_DATES = {
    0L, -1L, 86_399_999L, 946_684_800_000L, 951_782_400_000L, 1_700_000_000_000L,
    -12_219_292_800_000L, -12_219_292_800_001L, -62_135_596_800_000L, 253_402_300_799_999L,
    -100_000_000_000_000L, 4_102_444_800_000L,
  };

  static final String[] FIXED_DATE_INPUTS = {
    "", " ", "1/2/3", "12/31/1999", "2024-01-15", "15.01.2024", "15/01/2024", "2024/1/15",
    "abc", "0/0/0", "13/13/13", "1/1/1", "99/99/99", "31.12.99", "1.1.2000 ", "١/٢/٣",
  };

  static final long[] FIXED_LONGS = {
    0L, 1L, -1L, 7L, 12L, 1234L, -98765L, 1_000_000L, 1_234_567_890_123L, Long.MAX_VALUE,
    Long.MIN_VALUE, Integer.MAX_VALUE, Integer.MIN_VALUE,
  };

  static final double[] FIXED_DOUBLES = {
    0.0, -0.0, 0.5, 1.5, 2.5, -2.5, 0.125, 0.0005, 0.0015, 1234.5678, -0.0001, 1e-10, 1e15, 1e20,
    123456789.987654321, 9.995, 0.1 + 0.2, Double.NaN, Double.POSITIVE_INFINITY,
    Double.NEGATIVE_INFINITY, Double.MAX_VALUE, Double.MIN_VALUE, 1e300, -123456.0, 999.9995,
  };

  static final float[] FIXED_FLOATS = {1.1f, -0.0f, 3.4028235e38f, 1.4e-45f, 0.3f, 16777217f};

  static final String[] FIXED_NUMBER_INPUTS = {
    "", " ", " 12", "12 ", "abc", "-", "+5", "--5", "5-", "(5)", "12%", "12‰", "1E3",
    "1.5E-2", "1e3", "0x10", "1,234.5", "1.234,5", "1 234,5", "1 234,5", "1 234,5",
    "1'234.5", "∞", "-∞", "NaN", "�", "٣٤٥", "１２３",
    "۱۲", "๑๒", "9223372036854775807", "9223372036854775808",
    "-9223372036854775808", "-9223372036854775809", "-0", "0", "-0.0", "00012", ".5", "5.",
    "1" + "0".repeat(400), "0." + "0".repeat(400) + "1", "1.000000000000000000001", "−" + "5",
    "1,2,3", "12,", ",5",
  };

  static final String[] FIXED_LOWER = {
    "", "ABC", "TITLE Case", "ΟΔΟΣ", "ΑΣ", "Σ", "ΑΣ.",
    "ΑΣΑ", "Α Σ", "ΑΣ́", "ᾼΣ",
    "Α­Σ", "xΣy", "ΣΣ", "กΣ", "กขΣ ค",
    "İSTANBUL", "İ", "İ̀", "Ì̇", "ÌÍĨ",
    "J́", "Į́", "ẞ", "𐐀𐐁", "ǅǄǆ",
    "KKÅ", "ႠᲐ", "ᎠᏵ", "𞤀", "IJ", "Iİıi",
  };

  static final int[] LOWER_POOL = {
    'A', 'B', 'I', 'J', 'Z', 'a', 'i', ' ', '.', '\'', '-', '1', 0x00ad, 0x00cc, 0x00cd, 0x0128,
    0x012e, 0x0130, 0x0131, 0x0300, 0x0301, 0x0307, 0x0345, 0x0391, 0x03a3, 0x03aa, 0x03c3,
    0x03c2, 0x1f08, 0x00df, 0x1e9e, 0x2126, 0x212a, 0x212b, 0x1c90, 0x13a0, 0x10400, 0x1e900,
    0x0e01, 0x0531, 0x01c5, 0x01c4, 0xfb00, 0x0400, 0x2160, 0x24b6, 0x10d0,
  };

  static String escape(String s) {
    if (s == null) {
      return "null";
    }
    StringBuilder b = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (c >= 0x20 && c <= 0x7e && c != '\\') {
        b.append(c);
      } else {
        b.append(String.format("\\u%04X", (int) c));
      }
    }
    return b.toString();
  }

  static final StringBuilder OUT = new StringBuilder();

  static void line(Object... fields) {
    for (int i = 0; i < fields.length; i++) {
      if (i > 0) OUT.append('\t');
      OUT.append(fields[i]);
    }
    OUT.append('\n');
  }

  static String number(Object n) {
    if (n == null) return "null";
    if (n instanceof Long l) return "L:" + l;
    if (n instanceof Integer i) return "I:" + i;
    if (n instanceof Double d) return "D:" + Long.toHexString(Double.doubleToRawLongBits(d));
    if (n instanceof Float f) return "F:" + Integer.toHexString(Float.floatToRawIntBits(f));
    return n.getClass().getName() + ":" + n;
  }

  /** Replaces the first (or last) run of ASCII digits of s with r; s when it has none. */
  static String replaceDigits(String s, boolean last, String r) {
    int start = -1, end = -1;
    for (int i = 0; i < s.length(); i++) {
      boolean digit = s.charAt(i) >= '0' && s.charAt(i) <= '9';
      if (digit && (i == 0 || !(s.charAt(i - 1) >= '0' && s.charAt(i - 1) <= '9'))) {
        if (start < 0 || last) {
          start = i;
          end = i;
          while (end < s.length() && s.charAt(end) >= '0' && s.charAt(end) <= '9') end++;
          if (!last) break;
        }
      }
    }
    return start < 0 ? s : s.substring(0, start) + r + s.substring(end);
  }

  /** s with its ASCII digits mapped to the digits starting at zero. */
  static String mapDigits(String s, char zero) {
    StringBuilder b = new StringBuilder();
    for (char c : s.toCharArray()) {
      b.append(c >= '0' && c <= '9' ? (char) (zero + (c - '0')) : c);
    }
    return b.toString();
  }

  static List<String> dateInputs(String s) {
    List<String> out = new ArrayList<>();
    out.add(s);
    out.add(s + " trailing");
    out.add(s.substring(0, s.length() / 2));
    out.add(replaceDigits(s, true, "1999"));
    out.add(replaceDigits(s, true, "07"));
    out.add(replaceDigits(s, false, "0"));
    out.add(replaceDigits(s, false, "13"));
    out.add(replaceDigits(s, false, "32"));
    out.add(s.toUpperCase(Locale.ROOT));
    out.add(" " + s);
    out.add(mapDigits(s, '0'));
    out.add(mapDigits(s, '٠'));
    out.add(mapDigits(s, '０'));
    return out;
  }

  static List<String> numberInputs(String s, Random r) {
    List<String> out = new ArrayList<>();
    out.add(s);
    out.add(s + "abc");
    out.add("-" + s);
    out.add(s.replace(",", "").replace(".", "").replace(" ", "").replace(" ", ""));
    out.add(s.replace(',', '.'));
    out.add(s.replace('.', ','));
    out.add(s + "E3");
    out.add(s.substring(0, s.length() / 2));
    out.add(mapDigits(s, '٠'));
    if (r.nextBoolean()) out.add(" " + s);
    return out;
  }

  static String randomLower(Random r) {
    StringBuilder b = new StringBuilder();
    int n = r.nextInt(12);
    for (int i = 0; i < n; i++) {
      b.appendCodePoint(LOWER_POOL[r.nextInt(LOWER_POOL.length)]);
    }
    return b.toString();
  }

  static void dates(Locale l, Random r) {
    DateFormat df;
    try {
      df = DateFormat.getDateInstance(DateFormat.SHORT, l);
    } catch (RuntimeException e) {
      line("dpattern-error", e.getClass().getName(), escape(e.getMessage()));
      return;
    }
    line("dpattern", escape(((java.text.SimpleDateFormat) df).toPattern()));
    String zone = ZONES[r.nextInt(ZONES.length)];
    df.setTimeZone(TimeZone.getTimeZone(zone));
    line("dzone", zone);
    List<Long> millis = new ArrayList<>();
    for (long m : FIXED_DATES) millis.add(m);
    for (int i = 0; i < 6; i++) {
      millis.add(-62_135_596_800_000L + (long) (r.nextDouble() * 64_000_000_000_000.0));
    }
    List<String> formatted = new ArrayList<>();
    for (long m : millis) {
      String s = df.format(new Date(m));
      formatted.add(s);
      line("dformat", m, escape(s));
    }
    List<String> inputs = new ArrayList<>();
    for (int i = 0; i < formatted.size(); i++) {
      if (i < 6 || i >= FIXED_DATES.length) {
        inputs.addAll(dateInputs(formatted.get(i)));
      } else {
        inputs.add(formatted.get(i));
      }
    }
    inputs.addAll(Arrays.asList(FIXED_DATE_INPUTS));
    for (boolean lenient : new boolean[] {true, false}) {
      df.setLenient(lenient);
      for (String in : inputs) {
        ParsePosition pos = new ParsePosition(0);
        Date d = df.parse(in, pos);
        line(
            "dparse",
            lenient,
            escape(in),
            pos.getIndex(),
            pos.getErrorIndex(),
            d == null ? "null" : Long.toString(d.getTime()));
      }
    }
  }

  static void numbers(Locale l, Random r, String kind) {
    NumberFormat nf;
    try {
      nf = kind.equals("number") ? NumberFormat.getNumberInstance(l) : NumberFormat.getIntegerInstance(l);
    } catch (RuntimeException e) {
      line("nformat-error", kind, "-", e.getClass().getName(), escape(e.getMessage()));
      return;
    }
    List<Object> values = new ArrayList<>();
    for (long v : FIXED_LONGS) values.add(v);
    for (double v : FIXED_DOUBLES) values.add(v);
    for (float v : FIXED_FLOATS) values.add(v);
    for (int i = 0; i < 4; i++) {
      values.add(r.nextInt());
      values.add(r.nextLong() >> r.nextInt(63));
      values.add(r.nextGaussian() * Math.pow(10, r.nextInt(16) - 6));
      values.add(r.nextFloat() * (float) Math.pow(10, r.nextInt(8) - 3));
    }
    List<String> inputs = new ArrayList<>();
    for (Object v : values) {
      String s;
      try {
        s = nf.format(v);
      } catch (RuntimeException e) {
        line("nformat-error", kind, number(v), e.getClass().getName(), escape(e.getMessage()));
        continue;
      }
      line("nformat", kind, number(v), escape(s));
      if (r.nextInt(3) == 0) {
        inputs.addAll(numberInputs(s, r));
      } else {
        inputs.add(s);
      }
    }
    inputs.addAll(Arrays.asList(FIXED_NUMBER_INPUTS));
    for (String in : inputs) {
      ParsePosition pos = new ParsePosition(0);
      Number n;
      try {
        n = nf.parse(in, pos);
      } catch (RuntimeException e) {
        line("nparse-error", kind, escape(in), e.getClass().getName(), escape(e.getMessage()));
        continue;
      }
      line("nparse", kind, escape(in), pos.getIndex(), pos.getErrorIndex(), number(n));
    }
  }

  public static void main(String[] args) {
    long seed = Long.parseLong(args[0]);
    int shard = Integer.parseInt(args[1]);
    int shards = Integer.parseInt(args[2]);
    Locale[] locales = Locale.getAvailableLocales();
    Arrays.sort(locales, Comparator.comparing(Locale::toString));
    for (int i = 0; i < locales.length; i++) {
      if (i % shards != shard) {
        continue;
      }
      Locale l = locales[i];
      Random r = new Random(seed * 31 + i);
      String tag = l.toLanguageTag();
      boolean roundTrip;
      try {
        roundTrip = new Locale.Builder().setLanguageTag(tag).build().equals(l);
      } catch (RuntimeException e) {
        roundTrip = false;
      }
      line(
          "locale",
          escape(tag),
          roundTrip,
          escape(l.getLanguage()),
          escape(l.getScript()),
          escape(l.getCountry()),
          escape(l.getVariant()));
      dates(l, r);
      numbers(l, r, "number");
      numbers(l, r, "integer");
      List<String> lower = new ArrayList<>(Arrays.asList(FIXED_LOWER));
      for (int k = 0; k < 8; k++) lower.add(randomLower(r));
      for (String s : lower) {
        line("lower", escape(s), escape(s.toLowerCase(l)));
      }
    }
    System.out.print(OUT);
    System.out.flush();
  }
}
