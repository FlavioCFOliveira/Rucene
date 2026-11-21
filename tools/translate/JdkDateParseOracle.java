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
// The JDK 25 oracle of the SimpleDateFormat parsing emulation
// (crates/rucene-analysis-common/src/jdk/simple_date_format.rs, locale_data.rs),
// run by crates/rucene-analysis-common/tests/date_parse_jdk_oracle.rs.
//
// It is launched as a multi-file source program next to JdkDateParseData.java,
// whose locale-class computation (key, zoneKey) and cache clearing
// (clearCaches) it reuses, with the --add-exports/--add-opens flags that file
// needs (tools/translate/README.md):
//
//   --keys
//       For every locale of DateFormat.getAvailableLocales(), alone and with
//       each Unicode extension of EXTENSIONS, and for EXTRA_TAGS: the language
//       tag, then `ill` when Locale.Builder refuses it, else the class key and
//       the zone key JdkDateParseData computes for
//       new Locale.Builder().setLanguageTag(tag).build().
//   --parse <seed> <locales>
//       For a seeded sample of <locales> available locales plus FIXED_TAGS,
//       and every pattern of PATTERNS (DEFAULT is
//       DateFormat.getDateInstance(DEFAULT, locale)): a `format` line (the
//       tag, the pattern, then `error <class>` or the compiled pattern and the
//       time-zone ID), then one `parse` line per input (the input, escaped;
//       ParsePosition index and error index; whether parse returned a Date;
//       the format's time-zone ID after the parse). The inputs are dates the
//       format itself prints (seeded), their case and spacing variants, the
//       same with a random zone name of the locale in place of the zone, and
//       fixed strings. Caches are cleared before each format is built, so each
//       parse sees the state of a JVM where that locale is the first queried
//       (the state the generated data holds).
import java.text.*;
import java.util.*;

public class JdkDateParseOracle {
  static final String[] EXTENSIONS = {
    "u-ca-japanese", "u-ca-buddhist", "u-ca-islamic", "u-nu-arab", "u-nu-thai", "u-rg-gbzzzz",
    "u-tz-usnyc", "u-ca-gregory-nu-latn-rg-uszzzz",
  };

  static final String[] EXTRA_TAGS = {
    "", "und", "en", "en-US", "en-US-POSIX", "en-US-x-lvariant-POSIX", "no-NO", "no-NO-x-lvariant-NY",
    "nb", "nn-NO", "ja-JP-u-ca-japanese-x-lvariant-JP", "th-TH-u-nu-thai-x-lvariant-TH",
    "sr-Latn-RS", "zh-Hant-TW", "zh-TW", "zh-HK", "zh-SG", "zh-Hans-HK", "es-419", "pt-AO",
    "en-150", "en-001", "qaa", "qaa-QQ", "xx-Qaaa-AA", "iw", "in-ID", "ji", "he-IL", "fil-PH",
    "tl", "en-GB-u-rg-uszzzz", "de-CH-u-rg-atzzzz", "ar-SA-u-nu-latn", "fa-IR-u-nu-arabext",
    "hi-IN-u-nu-deva", "und-u-ca-japanese", "und-Latn-US", "en-US-u-rg-gbzzzz-x-lvariant-ab",
  };

  static final String[] FIXED_TAGS = {
    "en", "en-US", "en-GB", "de-DE", "fr-FR", "fr-CA", "es-ES", "it-IT", "pt-BR", "ru-RU", "pl-PL",
    "nl-NL", "sv-SE", "fi-FI", "da-DK", "nb-NO", "no-NO", "tr-TR", "el-GR", "he-IL", "ar-EG",
    "ar-SA-u-nu-arab", "fa-IR", "hi-IN", "hi-IN-u-nu-deva", "bn-BD", "th-TH", "th-TH-u-nu-thai",
    "th-TH-u-ca-buddhist", "ja-JP", "ja-JP-u-ca-japanese", "zh-CN", "zh-TW", "ko-KR", "vi-VN",
    "en-US-u-tz-usnyc", "en-GB-u-rg-uszzzz", "de-CH-u-rg-atzzzz", "und", "",
  };

  static final String DEFAULT = "DEFAULT";

  static final String[] PATTERNS = {
    DEFAULT,
    "MM/dd/yyyy",
    "d MMM yyyy",
    "EEEE, d MMMM y G",
    "yyyy-MM-dd HH:mm:ss Z",
    "dd.MM.yy HH:mm z",
    "EEE, d MMM yyyy HH:mm:ss zzzz",
    "h:mm a",
    "yyyy-MM-dd'T'HH:mm:ssXXX",
    "LLLL yyyy",
    "YYYY-'W'ww-u",
    "GGGG y MMMMM",
    "K:mm a, z",
    "yyyyMMdd",
    "q",
  };

  static final String[] FIXED_INPUTS = {
    "", "abc", "7/11/2006", "Jul 11, 2006", "11.07.2006", "2006-07-11", "July 11, 2006 AD",
    "  7/11/2006", "07/11/2006 junk", "-5", "NaN", "1E2/1/2000", "∞", "12:30 PM GMT+02:00",
    "2006-07-11T10:20:30+02:00", "2006-07-11T10:20:30Z", "11 juil. 2006", "11. Juli 2006",
  };

  static String escape(String s) {
    StringBuilder b = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (c == '\\') {
        b.append("\\\\");
      } else if (c < 0x20 || c == 0x7f || (c >= 0x80 && c < 0xa0) || Character.isSurrogate(c)) {
        b.append(String.format("\\u%04x", (int) c));
      } else {
        b.append(c);
      }
    }
    return b.toString();
  }

  static Locale fromTag(String tag) {
    return new Locale.Builder().setLanguageTag(tag).build();
  }

  static String withExtension(String tag, String ext) {
    if (tag.isEmpty()) return "und-" + ext;
    return JdkDateParseData.withExtension(tag, ext);
  }

  public static void main(String[] args) throws Exception {
    JdkDateParseData.setup();
    StringBuilder out = new StringBuilder();
    if (args[0].equals("--keys")) {
      TreeSet<String> tags = new TreeSet<>(Arrays.asList(EXTRA_TAGS));
      for (Locale l : DateFormat.getAvailableLocales()) {
        String tag = l.toLanguageTag();
        tags.add(tag);
        for (String ext : EXTENSIONS) {
          tags.add(withExtension(tag, ext));
        }
      }
      for (String tag : tags) {
        out.append(escape(tag)).append('\t');
        Locale l;
        try {
          l = fromTag(tag);
        } catch (IllformedLocaleException e) {
          out.append("ill\n");
          continue;
        }
        out.append(escape(JdkDateParseData.key(l))).append('\t');
        out.append(escape(JdkDateParseData.zoneKey(l))).append('\n');
      }
    } else {
      long seed = Long.parseLong(args[1]);
      int sample = Integer.parseInt(args[2]);
      Random r = new Random(seed);
      List<String> available = new ArrayList<>();
      for (Locale l : DateFormat.getAvailableLocales()) available.add(l.toLanguageTag());
      Collections.sort(available);
      LinkedHashSet<String> tags = new LinkedHashSet<>(Arrays.asList(FIXED_TAGS));
      while (tags.size() < FIXED_TAGS.length + sample) {
        tags.add(available.get(r.nextInt(available.size())));
      }
      for (String tag : tags) {
        Locale l;
        try {
          l = fromTag(tag);
        } catch (IllformedLocaleException e) {
          continue;
        }
        for (String pattern : PATTERNS) {
          // 1. the inputs, from a format of its own
          List<String> inputs = new ArrayList<>();
          JdkDateParseData.clearCaches();
          DateFormat gen;
          try {
            gen =
                pattern.equals(DEFAULT)
                    ? DateFormat.getDateInstance(DateFormat.DEFAULT, l)
                    : new SimpleDateFormat(pattern, l);
          } catch (RuntimeException e) {
            gen = null;
          }
          if (gen != null) {
            String[][] zones = new DateFormatSymbols(l).getZoneStrings();
            for (int i = 0; i < 3; i++) {
              long millis = -2208988800000L + (long) (r.nextDouble() * 6311390400000L);
              String s = gen.format(new Date(millis));
              inputs.add(s);
              inputs.add(s.toUpperCase(Locale.ROOT));
              inputs.add(s.toLowerCase(Locale.ROOT));
              inputs.add(" " + s.replace(' ', '\t'));
              inputs.add(s.substring(0, s.length() / 2));
              inputs.add(s + "x");
              if (zones.length > 0 && (pattern.indexOf('z') >= 0)) {
                String[] row = zones[r.nextInt(zones.length)];
                String name = row[1 + r.nextInt(4)];
                if (name != null) {
                  String zoneId = gen.getTimeZone().getID();
                  String own = gen.getTimeZone().getDisplayName(false, TimeZone.SHORT, l);
                  String ownLong = gen.getTimeZone().getDisplayName(false, TimeZone.LONG, l);
                  String replaced = s.replace(ownLong, name).replace(own, name);
                  inputs.add(replaced);
                  inputs.add(replaced.toLowerCase(Locale.ROOT));
                }
              }
            }
          }
          inputs.addAll(Arrays.asList(FIXED_INPUTS));
          // 2. a fresh format parses them
          JdkDateParseData.clearCaches();
          out.append("format\t").append(escape(tag)).append('\t').append(escape(pattern)).append('\t');
          SimpleDateFormat f;
          try {
            f =
                pattern.equals(DEFAULT)
                    ? (SimpleDateFormat) DateFormat.getDateInstance(DateFormat.DEFAULT, l)
                    : new SimpleDateFormat(pattern, l);
          } catch (RuntimeException e) {
            out.append("error ").append(e.getClass().getSimpleName()).append('\n');
            continue;
          }
          out.append(escape(f.toPattern())).append('\t').append(f.getTimeZone().getID()).append('\n');
          for (String input : inputs) {
            ParsePosition pos = new ParsePosition(0);
            Date d = f.parse(input, pos);
            out.append("parse\t")
                .append(escape(input))
                .append('\t')
                .append(pos.getIndex())
                .append('\t')
                .append(pos.getErrorIndex())
                .append('\t')
                .append(d != null)
                .append('\t')
                .append(f.getTimeZone().getID())
                .append('\n');
          }
        }
      }
    }
    System.out.write(out.toString().getBytes("UTF-8"));
    System.out.flush();
  }
}
