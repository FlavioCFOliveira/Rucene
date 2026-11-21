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
// Regenerates the simple case-mapping tables of src/util/automaton/case_folding.rs
// (SIMPLE_UPPERCASE and SIMPLE_LOWERCASE) from the running JDK, which must be
// JDK 25: every code point c of 0..0x10FFFF for which Character.toUpperCase(int)
// (resp. toLowerCase(int)) returns a different code point, paired with that code
// point, sorted by c. The body of each table (between its `= &[` line and its
// `];` line) is replaced in place, every entry as `(0x%04X, 0x%04X),`, entries
// joined by one space on lines of at most 93 characters (4-space indent), the
// layout of the tables this replaces. Deterministic: rerunning it on its own
// output changes nothing. Usage (see tools/translate/README.md):
//
//   java tools/translate/JdkSimpleCaseData.java src/util/automaton/case_folding.rs
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.function.IntUnaryOperator;

public class JdkSimpleCaseData {
  static final int WIDTH = 93;

  static String body(IntUnaryOperator map, int[] count) {
    List<String> items = new ArrayList<>();
    for (int c = 0; c <= Character.MAX_CODE_POINT; c++) {
      int m = map.applyAsInt(c);
      if (m != c) {
        items.add(String.format("(0x%04X, 0x%04X),", c, m));
      }
    }
    count[0] = items.size();
    StringBuilder out = new StringBuilder();
    StringBuilder line = new StringBuilder();
    for (String item : items) {
      if (line.length() > 0 && line.length() + 1 + item.length() > WIDTH) {
        out.append(line).append('\n');
        line.setLength(0);
      }
      line.append(line.length() == 0 ? "    " : " ").append(item);
    }
    if (line.length() > 0) {
      out.append(line).append('\n');
    }
    return out.toString();
  }

  static String replace(String source, String table, String body) {
    int start = source.indexOf("static " + table + ": &[(u32, u32)] = &[\n");
    if (start < 0) {
      throw new IllegalStateException("table not found: " + table);
    }
    int from = source.indexOf("= &[\n", start) + "= &[\n".length();
    int to = source.indexOf("];", from);
    return source.substring(0, from) + body + source.substring(to);
  }

  public static void main(String[] args) throws Exception {
    if (Runtime.version().feature() != 25) {
      throw new IllegalStateException("run with JDK 25, not " + Runtime.version());
    }
    Path path = Path.of(args[0]);
    String source = Files.readString(path, StandardCharsets.UTF_8);
    int[] upper = new int[1];
    int[] lower = new int[1];
    source = replace(source, "SIMPLE_UPPERCASE", body(Character::toUpperCase, upper));
    source = replace(source, "SIMPLE_LOWERCASE", body(Character::toLowerCase, lower));
    Files.writeString(path, source, StandardCharsets.UTF_8);
    System.err.println(
        "JDK " + Runtime.version() + ": " + upper[0] + " uppercase, " + lower[0] + " lowercase mappings");
  }
}
