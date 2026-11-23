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

import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.Random;
import java.util.ArrayList;
import java.util.List;
import org.apache.lucene.facet.FacetsConfig;

/**
 * The Java side of the sprint-17 oracle harness for {@code lucene/facet} (rmp #628,
 * {@code tests/sprint17_oracle/mod.rs}): {@code main(seed, scenario, input, args...)} prints the
 * dump of the scenario, which Rucene's test computes from the same seed.
 */
public class FacetOracle {
  /** The seeded generators of the harness (`tests/sprint17_oracle/mod.rs`, `corpus`). */
  static final class Corpus {
    static final String[] VOCABULARY = {
      "the", "of", "and", "to", "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
      "hotel", "india", "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec",
      "romeo", "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu"
    };

    static String word(Random r) {
      return VOCABULARY[r.nextInt(VOCABULARY.length)];
    }

    static String text(Random r, int n) {
      StringBuilder sb = new StringBuilder();
      for (int i = 0; i < n; i++) {
        if (i > 0) sb.append(' ');
        sb.append(word(r));
      }
      return sb.toString();
    }
  }

  /** Printable ASCII except the backslash as is, every other UTF-16 unit as a u-escape. */
  static String escape(String s) {
    if (s == null) return "null";
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char c = s.charAt(i);
      if (c >= 0x20 && c <= 0x7e && c != '\\') sb.append(c);
      else sb.append(String.format("\\u%04X", (int) c));
    }
    return sb.toString();
  }

  static String bits(float f) {
    return Integer.toHexString(Float.floatToRawIntBits(f));
  }

  static String bits(double d) {
    return Long.toHexString(Double.doubleToRawLongBits(d));
  }

  static String input(String path) throws IOException {
    return path.equals("-") ? null : Files.readString(Path.of(path), StandardCharsets.UTF_8);
  }

  public static void main(String[] args) throws Exception {
    long seed = Long.parseLong(args[0]);
    String scenario = args[1];
    String input = input(args[2]);
    String[] rest = Arrays.copyOfRange(args, 3, args.length);
    PrintStream out =
        new PrintStream(new FileOutputStream(FileDescriptor.out), false, StandardCharsets.UTF_8);
    run(scenario, new Random(seed), input, rest, out);
    out.flush();
  }

  static IllegalArgumentException unknown(String scenario) {
    return new IllegalArgumentException("unknown scenario: " + scenario);
  }

  static void run(String scenario, Random r, String input, String[] args, PrintStream out)
      throws Exception {
    switch (scenario) {
      case "smoke" -> smoke(r, out);
      default -> throw unknown(scenario);
    }
  }

  /**
   * Twenty seeded paths (a dimension and one to four components, each a word into which the
   * delimiter and escape characters may be spliced): {@code FacetsConfig.pathToString} of each,
   * then {@code stringToPath} of the result.
   */
  static void smoke(Random r, PrintStream out) throws Exception {
    for (int i = 0; i < 20; i++) {
      List<String> path = new ArrayList<>();
      path.add("dim" + r.nextInt(3));
      int n = 1 + r.nextInt(4);
      for (int j = 0; j < n; j++) {
        StringBuilder component = new StringBuilder(Corpus.word(r));
        int splice = r.nextInt(4);
        if (splice == 1) component.insert(r.nextInt(component.length() + 1), FacetsConfig.DELIM_CHAR);
        if (splice == 2) component.insert(r.nextInt(component.length() + 1), '\u001E');
        path.add(component.toString());
      }
      String encoded = FacetsConfig.pathToString(path.toArray(new String[0]));
      out.println("path " + escape(String.join("/", path)));
      out.println("encoded " + escape(encoded));
      out.println("decoded " + escape(String.join("/", FacetsConfig.stringToPath(encoded))));
    }
  }
}
