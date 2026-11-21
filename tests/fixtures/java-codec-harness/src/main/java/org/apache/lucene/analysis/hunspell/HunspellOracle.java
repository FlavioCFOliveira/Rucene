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

package org.apache.lucene.analysis.hunspell;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.nio.charset.Charset;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import org.apache.lucene.util.CharsRef;

/**
 * The Java side of the hunspell oracle of rmp tasks #462/#463 (Rust counterpart {@code
 * crates/rucene-analysis-common/tests/analysis_oracle_hunspell_compound_morph_collation.rs}). It
 * lives in the package of {@code Stemmer}, which is package-private, and is therefore compiled
 * with {@code javac} onto the class path of the Lucene jars rather than launched as a single
 * source file (a source-launched class has its own class loader, hence another runtime package).
 *
 * <p><b>Spec file</b> (UTF-8, TAB-separated): {@code synthetic <dir>} writes the synthetic
 * multi-byte-charset dictionaries into {@code <dir>} and adds them; {@code dict <name> <aff> <dic>
 * <mode>} adds a dictionary ({@code mode} {@code plain} or {@code tolerant}, the latter with the
 * affix/REP/conversion tolerances of {@code TestDictionary.loadForgivingDictionary}).
 *
 * <p><b>Dump</b> (stdout): per dictionary {@code dict <name> <aff> <dic> <mode>}, then either
 * {@code load-error <simple class> <message>} or {@code charset <name>} and the words it checks:
 * {@code w <flags> <word>} (flags: {@code S} suggest, {@code F} expand) followed by one line per
 * result: {@code spell}, {@code stem} ({@code Stemmer.stem}), {@code stemIC} (the same over the
 * dictionary loaded with {@code ignoreCase}), {@code roots} ({@code Hunspell.getRoots}), {@code
 * analyze} ({@code analyzeSimpleWord}), {@code sug} ({@code Suggester.suggestNoTimeout}), {@code
 * forms} ({@code getAllWordForms}) and {@code compress} ({@code compress} of the forms). Text is
 * escaped as {@code AnalysisOracle.escape} does; an exception prints {@code !<simple class>}.
 */
public final class HunspellOracle {

  private HunspellOracle() {}

  static String escape(CharSequence s) {
    StringBuilder out = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char ch = s.charAt(i);
      if (ch >= 0x20 && ch <= 0x7e && ch != '\\') {
        out.append(ch);
      } else {
        out.append(String.format(Locale.ROOT, "\\u%04X", (int) ch));
      }
    }
    return out.toString();
  }

  static String list(List<?> items) {
    StringBuilder b = new StringBuilder("[");
    for (int i = 0; i < items.size(); i++) {
      if (i > 0) b.append(", ");
      b.append(escape(String.valueOf(items.get(i))));
    }
    return b.append(']').toString();
  }

  interface Op {
    Object run() throws Exception;
  }

  static String safe(Op op) {
    try {
      Object r = op.run();
      if (r instanceof List<?> l) return list(l);
      return r == null ? "null" : escape(String.valueOf(r));
    } catch (Throwable t) {
      return "!" + t.getClass().getSimpleName();
    }
  }

  static Dictionary load(Path aff, Path dic, String mode, boolean ignoreCase) throws Exception {
    try (InputStream a = Files.newInputStream(aff);
        InputStream d = Files.newInputStream(dic)) {
      if (mode.equals("tolerant")) {
        return new Dictionary(a, List.of(d), ignoreCase, SortingStrategy.inMemory()) {
          @Override
          protected boolean tolerateAffixRuleCountMismatches() {
            return true;
          }

          @Override
          protected boolean tolerateRepRuleCountMismatches() {
            return true;
          }

          @Override
          protected boolean tolerateDuplicateConversionMappings() {
            return true;
          }
        };
      }
      return new Dictionary(a, List.of(d), ignoreCase, SortingStrategy.inMemory());
    }
  }

  /** The lines of {@code p} decoded with {@code cs} (malformed input replaced), or none. */
  static List<String> lines(Path p, Charset cs) throws Exception {
    if (!Files.exists(p)) return List.of();
    String text = new String(Files.readAllBytes(p), cs);
    return text.lines().toList();
  }

  static String sibling(Path aff, String ext) {
    String s = aff.toString();
    return s.substring(0, s.length() - 4) + ext;
  }

  /** The words checked for one dictionary, with their flags, in order. */
  static Map<String, String> words(Dictionary dictionary, Path aff, Path dic) throws Exception {
    Map<String, String> words = new LinkedHashMap<>();
    Charset cs = dictionary.decoder.charset();
    List<String> dicWords = new ArrayList<>();
    List<String> dicLines = lines(dic, cs);
    for (int i = 1; i < dicLines.size(); i++) {
      String line = dicLines.get(i);
      int len = (int) line.chars().takeWhile(c -> !Character.isWhitespace(c) && c != '/').count();
      String word = line.substring(0, len).trim();
      if (word.isEmpty()) continue;
      dicWords.add(word);
      words.merge(word, word.contains("\\") ? "" : "F", (x, y) -> x + y);
    }
    for (String ext : new String[] {".good", ".wrong"}) {
      for (String line : lines(Path.of(sibling(aff, ext)), StandardCharsets.UTF_8)) {
        String w = line.trim();
        if (w.isEmpty()) continue;
        words.merge(w, ext.equals(".wrong") ? "S" : "", (x, y) -> x + y);
      }
    }
    for (String line : lines(Path.of(sibling(aff, ".sug")), StandardCharsets.UTF_8)) {
      for (String s : line.split(", ")) {
        String w = s.trim();
        if (!w.isEmpty()) words.merge(w, "", (x, y) -> x + y);
      }
    }
    int n = 0;
    for (String w : dicWords) {
      if (n++ >= 40) break;
      words.merge(w.toUpperCase(Locale.ROOT), "", (x, y) -> x + y);
      if (!w.isEmpty()) {
        words.merge(
            Character.toUpperCase(w.charAt(0)) + w.substring(1), "", (x, y) -> x + y);
      }
      words.merge(w + "s", "", (x, y) -> x + y);
      words.merge("re" + w, "", (x, y) -> x + y);
      if (w.length() > 1) {
        words.merge(w.substring(0, w.length() - 1), n <= 15 ? "S" : "", (x, y) -> x + y);
        words.merge(w.substring(1) + w.charAt(0), n <= 15 ? "S" : "", (x, y) -> x + y);
      }
    }
    // the port's public API takes Rust strings: no unpaired surrogate
    words.keySet().removeIf(HunspellOracle::hasUnpairedSurrogate);
    words.replaceAll((k, v) -> v.chars().distinct().sorted().collect(
        StringBuilder::new, StringBuilder::appendCodePoint, StringBuilder::append).toString());
    return words;
  }

  static boolean hasUnpairedSurrogate(String s) {
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (Character.isHighSurrogate(c)) {
        if (i + 1 < s.length() && Character.isLowSurrogate(s.charAt(i + 1))) {
          i++;
        } else {
          return true;
        }
      } else if (Character.isLowSurrogate(c)) {
        return true;
      }
    }
    return false;
  }

  static void check(PrintWriter out, String name, Path aff, Path dic, String mode) {
    out.println("dict " + name + "\t" + aff + "\t" + dic + "\t" + mode);
    Dictionary dictionary;
    Dictionary ignoreCase;
    try {
      dictionary = load(aff, dic, mode, false);
      ignoreCase = load(aff, dic, mode, true);
    } catch (Throwable t) {
      out.println(
          "load-error " + t.getClass().getSimpleName() + " " + escape(String.valueOf(t.getMessage())));
      return;
    }
    out.println("charset " + dictionary.decoder.charset().name());
    Map<String, String> words;
    try {
      words = words(dictionary, aff, dic);
    } catch (Throwable t) {
      out.println("words-error " + t.getClass().getSimpleName());
      return;
    }
    Stemmer stemmer = new Stemmer(dictionary);
    Stemmer stemmerIC = new Stemmer(ignoreCase);
    Hunspell hunspell = new Hunspell(dictionary, TimeoutPolicy.NO_TIMEOUT, () -> {});
    Suggester suggester = new Suggester(dictionary);
    for (Map.Entry<String, String> e : words.entrySet()) {
      String w = e.getKey();
      String flags = e.getValue();
      out.println("w " + (flags.isEmpty() ? "-" : flags) + " " + escape(w));
      out.println(" spell " + safe(() -> hunspell.spell(w)));
      out.println(" stem " + safe(() -> stems(stemmer.stem(w))));
      out.println(" stemIC " + safe(() -> stems(stemmerIC.stem(w))));
      out.println(" roots " + safe(() -> hunspell.getRoots(w)));
      out.println(" analyze " + safe(() -> hunspell.analyzeSimpleWord(w)));
      if (flags.contains("S")) {
        out.println(" sug " + safe(() -> suggester.suggestNoTimeout(w, () -> {})));
      }
      if (flags.contains("F")) {
        out.println(" forms " + safe(() -> hunspell.getAllWordForms(w)));
        out.println(
            " compress "
                + safe(
                    () -> {
                      List<String> forms =
                          hunspell.getAllWordForms(w).stream().map(AffixedWord::getWord).toList();
                      EntrySuggestion s = hunspell.compress(forms);
                      return s == null ? null : s.internalsToString();
                    }));
      }
    }
  }

  static List<String> stems(List<CharsRef> stems) {
    return stems.stream().map(CharsRef::toString).toList();
  }

  /** One synthetic dictionary per multi-byte charset: name, charset, stems, suffix. */
  static final String[][] SYNTHETIC = {
    {"Big5", "中國,臺灣,學生", "們"},
    {"Big5-HKSCS", "中國,香港,學生", "們"},
    {"EUC-JP", "日本,東京,学生,カタカナ", "語"},
    {"Shift_JIS", "日本,東京,学生,ｶﾀｶﾅ", "語"},
    {"windows-31j", "日本,東京,学生,①②", "語"},
    {"ISO-2022-JP", "日本,東京,学生", "語"},
    {"EUC-KR", "한국,서울,학생", "어"},
    {"x-windows-949", "한국,서울,학생", "어"},
    {"GB2312", "中国,北京,学生", "人"},
    {"GBK", "中国,北京,學生", "人"},
    {"GB18030", "中国,北京,学生,𠀀", "人"},
    {"x-EUC-TW", "中國,臺灣,學生", "們"},
    {"UTF-16", "abc,def,été", "s"},
    {"UTF-16LE", "abc,def,été", "s"},
    {"x-IBM942", "日本,東京", "語"},
    {"x-IBM970", "한국,서울", "어"},
  };

  static List<String[]> synthetic(Path dir) throws Exception {
    Files.createDirectories(dir);
    List<String[]> dicts = new ArrayList<>();
    for (String[] s : SYNTHETIC) {
      Charset cs = Charset.forName(s[0]);
      String name = "syn-" + s[0];
      String[] stems = s[1].split(",");
      String aff =
          "SET "
              + s[0]
              + "\nTRY "
              + s[1].replace(",", "")
              + "\nSFX A Y 1\nSFX A 0 "
              + s[2]
              + " .\nPFX B Y 1\nPFX B 0 "
              + stems[0]
              + " .\n";
      StringBuilder dic = new StringBuilder().append(stems.length).append('\n');
      for (String stem : stems) dic.append(stem).append("/AB\n");
      Path a = dir.resolve(name + ".aff");
      Path d = dir.resolve(name + ".dic");
      Files.write(a, encode(aff, cs));
      Files.write(d, encode(dic.toString(), cs));
      // the words the speller checks: every stem with and without the affixes
      StringBuilder good = new StringBuilder();
      for (String stem : stems) {
        good.append(stem).append('\n').append(stem).append(s[2]).append('\n');
        good.append(stems[0]).append(stem).append('\n');
      }
      Files.write(dir.resolve(name + ".good"), good.toString().getBytes(StandardCharsets.UTF_8));
      dicts.add(new String[] {name, a.toString(), d.toString(), "plain"});
    }
    return dicts;
  }

  static byte[] encode(String s, Charset cs) throws Exception {
    ByteArrayOutputStream out = new ByteArrayOutputStream();
    try (OutputStreamWriter w = new OutputStreamWriter(out, cs)) {
      w.write(s);
    }
    return out.toByteArray();
  }

  public static void main(String[] args) throws Exception {
    List<String[]> dicts = new ArrayList<>();
    for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
      String[] p = line.split("\t", -1);
      switch (p[0]) {
        case "synthetic" -> dicts.addAll(synthetic(Path.of(p[1])));
        case "dict" -> dicts.add(new String[] {p[1], p[2], p[3], p[4]});
        default -> throw new IllegalArgumentException(line);
      }
    }
    PrintWriter out =
        new PrintWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8), false);
    for (String[] d : dicts) {
      check(out, d[0], Path.of(d[1]), Path.of(d[2]), d[3]);
    }
    out.flush();
  }
}
