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

import java.io.BufferedWriter;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.text.BreakIterator;
import java.util.List;
import java.util.Locale;
import org.apache.lucene.analysis.util.CharArrayIterator;

/**
 * The JDK side of the {@code java.text.BreakIterator} oracle of rucene-analysis-common (rmp #447).
 * Run as a single source file on the class path lucene-core + lucene-analysis-common 10.5.0.
 *
 * <ul>
 *   <li>{@code ops <file>}: each line {@code <kind> <text> <begin> <length> <ops>}; kind {@code w}
 *       (word, root locale), {@code s} (sentence, root locale) or {@code t} (word, Thai); text the
 *       UTF-16 units as 4-digit hex ({@code -} when empty); begin -1 for {@code setText(String)},
 *       otherwise the text is set as {@code CharArrayIterator.setText(chars, begin, length)}; ops
 *       comma-separated: F first, L last, N next, P previous, C current, X&lt;n&gt; next(n),
 *       W&lt;k&gt; following(k), V&lt;k&gt; preceding(k), B&lt;k&gt; isBoundary(k) (1/0), A every
 *       boundary from first() on (joined by ':'). Prints one line of comma-separated results per
 *       case; a call that throws prints {@code E<SimpleName>}.
 * </ul>
 */
public class BreakIteratorOracle {
  static String unhex(String s) {
    if (s.equals("-")) return "";
    StringBuilder b = new StringBuilder();
    for (int i = 0; i < s.length(); i += 4) b.append((char) Integer.parseInt(s.substring(i, i + 4), 16));
    return b.toString();
  }

  static String hex(String s) {
    StringBuilder b = new StringBuilder();
    for (int i = 0; i < s.length(); i++) b.append(String.format("%04x", (int) s.charAt(i)));
    return b.toString();
  }

  static BreakIterator instance(String kind) {
    switch (kind) {
      case "w":
        return BreakIterator.getWordInstance(Locale.ROOT);
      case "s":
        return BreakIterator.getSentenceInstance(Locale.ROOT);
      case "t":
        return BreakIterator.getWordInstance(new Locale.Builder().setLanguageTag("th").build());
      default:
        throw new IllegalArgumentException(kind);
    }
  }

  static String op(BreakIterator bi, String op) {
    try {
      char c = op.charAt(0);
      String arg = op.substring(1);
      switch (c) {
        case 'F':
          return Integer.toString(bi.first());
        case 'L':
          return Integer.toString(bi.last());
        case 'N':
          return Integer.toString(bi.next());
        case 'P':
          return Integer.toString(bi.previous());
        case 'C':
          return Integer.toString(bi.current());
        case 'X':
          return Integer.toString(bi.next(Integer.parseInt(arg)));
        case 'W':
          return Integer.toString(bi.following(Integer.parseInt(arg)));
        case 'V':
          return Integer.toString(bi.preceding(Integer.parseInt(arg)));
        case 'B':
          return bi.isBoundary(Integer.parseInt(arg)) ? "1" : "0";
        case 'A':
          {
            StringBuilder b = new StringBuilder();
            b.append(bi.first());
            for (int p = bi.next(); p != BreakIterator.DONE; p = bi.next()) b.append(':').append(p);
            return b.toString();
          }
        default:
          throw new IllegalStateException(op);
      }
    } catch (RuntimeException e) {
      return "E" + e.getClass().getSimpleName();
    }
  }

  public static void main(String[] args) throws Exception {
    PrintWriter out =
        new PrintWriter(
            new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8), 1 << 16));
    {
      List<String> lines = Files.readAllLines(Paths.get(args[1]), StandardCharsets.UTF_8);
      BreakIterator w = instance("w"), s = instance("s"), t = instance("t");
      for (String line : lines) {
        if (line.isEmpty()) continue;
        String[] f = line.split(" ", -1);
        BreakIterator bi = f[0].equals("w") ? w : f[0].equals("s") ? s : t;
        String text = unhex(f[1]);
        int begin = Integer.parseInt(f[2]);
        if (begin < 0) {
          bi.setText(text);
        } else {
          CharArrayIterator it = CharArrayIterator.newWordInstance();
          it.setText(text.toCharArray(), begin, Integer.parseInt(f[3]));
          bi.setText(it);
        }
        StringBuilder b = new StringBuilder();
        for (String op : f[4].split(",")) {
          if (b.length() > 0) b.append(',');
          b.append(op(bi, op));
        }
        out.println(b);
      }
    }
    out.flush();
  }
}
