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

package org.apache.lucene.rucene.codec;

import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import org.apache.lucene.analysis.compound.hyphenation.Hyphenation;
import org.apache.lucene.analysis.compound.hyphenation.HyphenationTree;
import org.apache.lucene.analysis.compound.hyphenation.PatternConsumer;
import org.apache.lucene.analysis.compound.hyphenation.PatternParser;
import org.xml.sax.InputSource;

/**
 * The Java side of the hyphenation oracle of rmp task #461 (Rust counterpart {@code
 * crates/rucene-analysis-common/tests/analysis_oracle_hunspell_compound_morph_collation.rs}): what
 * {@code PatternParser} hands its consumer for each grammar, read through the JDK's SAX parser,
 * and the hyphenation points of the resulting {@code HyphenationTree} for a word corpus.
 *
 * <p><b>Spec file</b> (UTF-8, TAB-separated): {@code xml <path>} grammars and {@code word <word>}
 * corpus lines (escaped as {@code AnalysisOracle.escape}).
 *
 * <p><b>Dump</b>: per grammar {@code xml <path>}, the consumer calls ({@code class <chargroup>},
 * {@code exception <word> <parts>}, {@code pattern <pattern> <values>}) or {@code parse-error
 * <simple class>} of {@code PatternParser.parse(String)}, then {@code tree} with the size of the
 * tree {@code loadPatterns} builds (or {@code tree-error <simple class>}), and per word and
 * {@code (remain, push)} pair {@code h <remain> <push> <word> <points>} ({@code null} when {@code
 * hyphenate} returns none), then {@code find <pattern> <values>} for every pattern parsed.
 */
public final class HyphenationOracle {

  private HyphenationOracle() {}

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

  static String unescape(String s) {
    StringBuilder out = new StringBuilder();
    int i = 0;
    while (i < s.length()) {
      char ch = s.charAt(i);
      if (ch == '\\') {
        out.append((char) Integer.parseInt(s.substring(i + 2, i + 6), 16));
        i += 6;
      } else {
        out.append(ch);
        i++;
      }
    }
    return out.toString();
  }

  static final int[][] PARAMS = {{1, 1}, {2, 2}, {2, 3}, {3, 1}};

  public static void main(String[] args) throws Exception {
    List<String> xmls = new ArrayList<>();
    List<String> words = new ArrayList<>();
    for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
      String[] p = line.split("\t", -1);
      switch (p[0]) {
        case "xml" -> xmls.add(p[1]);
        case "word" -> words.add(unescape(p[1]));
        default -> throw new IllegalArgumentException(line);
      }
    }
    PrintWriter out =
        new PrintWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8), false);
    for (String xml : xmls) {
      out.println("xml " + xml);
      List<String> patterns = new ArrayList<>();
      PatternConsumer logger =
          new PatternConsumer() {
            @Override
            public void addClass(String chargroup) {
              out.println("class " + escape(chargroup));
            }

            @Override
            public void addException(String word, ArrayList<Object> hyphenatedword) {
              StringBuilder b = new StringBuilder("[");
              for (int i = 0; i < hyphenatedword.size(); i++) {
                if (i > 0) b.append(", ");
                Object o = hyphenatedword.get(i);
                b.append(o instanceof String ? "s:" : "h:").append(escape(String.valueOf(o)));
              }
              out.println("exception " + escape(word) + " " + b.append(']'));
            }

            @Override
            public void addPattern(String pattern, String values) {
              patterns.add(pattern);
              out.println("pattern " + escape(pattern) + " " + escape(values));
            }
          };
      try {
        new PatternParser(logger).parse(xml);
      } catch (Throwable t) {
        out.println("parse-error " + t.getClass().getSimpleName());
      }
      HyphenationTree tree = new HyphenationTree();
      try {
        tree.loadPatterns(new InputSource(xml));
      } catch (Throwable t) {
        out.println("tree-error " + t.getClass().getSimpleName());
        continue;
      }
      out.println("tree " + tree.size());
      for (String word : words) {
        for (int[] p : PARAMS) {
          String points;
          try {
            Hyphenation h = tree.hyphenate(word, p[0], p[1]);
            if (h == null) {
              points = "null";
            } else {
              StringBuilder b = new StringBuilder("[");
              int[] hp = h.getHyphenationPoints();
              for (int i = 0; i < hp.length; i++) {
                if (i > 0) b.append(", ");
                b.append(hp[i]);
              }
              points = b.append(']').toString();
            }
          } catch (Throwable t) {
            points = "!" + t.getClass().getSimpleName();
          }
          out.println("h " + p[0] + " " + p[1] + " " + escape(word) + " " + points);
        }
      }
      for (String pattern : patterns) {
        String found;
        try {
          found = escape(tree.findPattern(pattern));
        } catch (Throwable t) {
          found = "!" + t.getClass().getSimpleName();
        }
        out.println("find " + escape(pattern) + " " + found);
      }
    }
    out.flush();
  }
}
