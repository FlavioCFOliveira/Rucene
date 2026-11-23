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
import java.io.StringReader;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Properties;
import java.util.TreeMap;
import java.util.stream.Stream;
import org.apache.lucene.benchmark.byTask.Benchmark;
import org.apache.lucene.benchmark.byTask.PerfRunData;
import org.apache.lucene.benchmark.byTask.feeds.DemoHTMLParser;
import org.apache.lucene.benchmark.byTask.feeds.DocData;
import org.apache.lucene.benchmark.byTask.feeds.LineDocSource;
import org.apache.lucene.benchmark.byTask.feeds.NoMoreDataException;
import org.apache.lucene.benchmark.byTask.feeds.TrecContentSource;
import org.apache.lucene.benchmark.byTask.stats.Report;
import org.apache.lucene.benchmark.byTask.tasks.RepSumByNameTask;
import org.apache.lucene.benchmark.byTask.utils.Algorithm;
import org.apache.lucene.benchmark.byTask.utils.Config;
import org.apache.lucene.benchmark.byTask.utils.Format;

/**
 * The Java side of the sprint-17 oracle harness for {@code lucene/benchmark} (rmp #628,
 * {@code tests/sprint17_oracle/mod.rs}): {@code main(seed, scenario, input, args...)} prints the
 * dump of the scenario, which Rucene's test computes from the same seed.
 */
public class BenchmarkOracle {
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
    // The benchmark's own System.out chatter (config properties, task logs, reports) is not part
    // of the dump; Rucene's test prints it to its own stdout.
    System.setOut(new PrintStream(java.io.OutputStream.nullOutputStream()));
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
      case "alg" -> alg(input, args, out);
      case "html" -> html(args, out);
      case "trec" -> trec(args, out);
      case "linedoc" -> linedoc(args, out);
      case "report" -> report(args, out);
      default -> throw unknown(scenario);
    }
  }

  /**
   * Twenty seeded rows of {@code byTask.utils.Format}: a double and a float with 0 to 2 fraction
   * digits (padded left and right), an int and a word, in a column of 0 to 11 spaces.
   */
  static void smoke(Random r, PrintStream out) throws Exception {
    for (int i = 0; i < 20; i++) {
      double d = (r.nextDouble() - 0.25) * 100000;
      int digits = r.nextInt(3);
      String col = " ".repeat(r.nextInt(12));
      int n = r.nextInt(2000000) - 1000000;
      String word = Corpus.word(r);
      out.println(
          "row |" + Format.format(digits, d, col) + "|" + Format.format(digits, (float) d, col)
              + "|" + Format.formatPaddRight(digits, d, col) + "|"
              + Format.formatPaddRight(digits, (float) d, col) + "|" + Format.format(n, col)
              + "|" + Format.format(word, col) + "|" + Format.formatPaddLeft(word, col) + "|");
    }
  }

  /**
   * {@code new Algorithm(new PerfRunData(config)).toString()} of every {@code *.alg} of a dir, with the
   * config edits of {@code TestPerfTasksParse.testParseExamples} (the content source and query
   * maker set to harmless shipped classes: the source launcher's classes are not visible to {@code
   * Class.forName}); args: conf dir, work dir, line file.
   */
  static void alg(String text, String[] args, PrintStream out) throws Exception {
    List<Path> files = new ArrayList<>();
    try (Stream<Path> st = Files.list(Path.of(args[0]))) {
      st.filter(p -> p.getFileName().toString().endsWith(".alg")).sorted().forEach(files::add);
    }
    for (Path f : files) {
      Config config = new Config(Files.newBufferedReader(f, StandardCharsets.UTF_8));
      config.set("work.dir", args[1]);
      config.set("content.source", "org.apache.lucene.benchmark.byTask.feeds.SingleDocSource");
      config.set("directory", "ByteBuffersDirectory");
      if (config.get("line.file.out", null) != null) {
        config.set("line.file.out", args[2]);
      }
      if (config.get("query.maker", null) != null) {
        config.set("query.maker", "org.apache.lucene.benchmark.byTask.feeds.SimpleQueryMaker");
      }
      Algorithm alg = new Algorithm(new PerfRunData(config));
      out.println("ALG " + f.getFileName());
      out.println(escape(alg.toString()));
      alg.close();
    }
  }

  /** ISO-8859-1 text of a file (each byte one char). */
  static String latin1(Path p) throws IOException {
    return new String(Files.readAllBytes(p), StandardCharsets.ISO_8859_1);
  }

  /** {@code DemoHTMLParser.Parser} title, body and sorted meta tags of every *.html of a dir. */
  static void html(String[] args, PrintStream out) throws Exception {
    List<Path> files = new ArrayList<>();
    try (Stream<Path> s = Files.list(Path.of(args[0]))) {
      s.filter(p -> p.getFileName().toString().endsWith(".html")).sorted().forEach(files::add);
    }
    for (Path f : files) {
      out.println("FILE " + f.getFileName());
      DemoHTMLParser.Parser p;
      try {
        p = new DemoHTMLParser.Parser(new StringReader(latin1(f)));
      } catch (Exception e) {
        out.println("ERROR " + e.getClass().getName() + ": " + escape(e.getMessage()));
        continue;
      }
      out.println("TITLE " + escape(p.title));
      out.println("BODY " + escape(p.body));
      TreeMap<String, String> meta = new TreeMap<>();
      for (Map.Entry<Object, Object> e : p.metaTags.entrySet()) {
        meta.put((String) e.getKey(), (String) e.getValue());
      }
      for (Map.Entry<String, String> e : meta.entrySet()) {
        out.println("META " + escape(e.getKey()) + "=" + escape(e.getValue()));
      }
    }
  }

  static void dumpDocData(DocData dd, PrintStream out) {
    out.println(
        "DOC name=" + escape(dd.getName()) + " title=" + escape(dd.getTitle()) + " date="
            + escape(dd.getDate()) + " id=" + dd.getID());
    out.println("BODY " + escape(dd.getBody()));
    Properties props = dd.getProps();
    if (props != null) {
      TreeMap<String, String> sorted = new TreeMap<>();
      for (String k : props.stringPropertyNames()) sorted.put(k, props.getProperty(k));
      for (Map.Entry<String, String> e : sorted.entrySet()) {
        out.println("PROP " + escape(e.getKey()) + "=" + escape(e.getValue()));
      }
    }
  }

  /** {@code TrecContentSource} with {@code TrecParserByPath} over a dir: every doc. */
  static void trec(String[] args, PrintStream out) throws Exception {
    TrecContentSource tcs = new TrecContentSource();
    Properties props = new Properties();
    props.setProperty("print.props", "false");
    props.setProperty("content.source.verbose", "false");
    props.setProperty("content.source.excludeIteration", "true");
    props.setProperty("docs.dir", args[0]);
    props.setProperty(
        "trec.doc.parser", "org.apache.lucene.benchmark.byTask.feeds.TrecParserByPath");
    props.setProperty("content.source.forever", "false");
    tcs.setConfig(new Config(props));
    tcs.resetInputs();
    DocData dd = new DocData();
    try {
      while (true) {
        dd = tcs.getNextDocData(dd);
        dumpDocData(dd, out);
      }
    } catch (NoMoreDataException e) {
      out.println("END");
    }
    tcs.close();
  }

  /** {@code LineDocSource} over a line file (args: file, line.parser or -): every doc. */
  static void linedoc(String[] args, PrintStream out) throws Exception {
    LineDocSource src = new LineDocSource();
    Properties props = new Properties();
    props.setProperty("docs.file", args[0]);
    props.setProperty("content.source.forever", "false");
    if (!args[1].equals("-")) props.setProperty("line.parser", args[1]);
    src.setConfig(new Config(props));
    src.resetInputs();
    try {
      while (true) {
        DocData dd = src.getNextDocData(new DocData());
        dumpDocData(dd, out);
      }
    } catch (NoMoreDataException e) {
      out.println("END");
    }
    src.close();
  }

  /** Exposes the protected {@code reportSumByName}. */
  static final class SumByName extends RepSumByNameTask {
    SumByName(PerfRunData runData) {
      super(runData);
    }

    Report report() {
      return reportSumByName(getRunData().getPoints().taskStats());
    }
  }

  /**
   * The RepSumByName report text of a seeded run of an alg text; the columns after recsPerRun
   * (rates, times, memory) are masked.
   */
  static void report(String[] args, PrintStream out) throws Exception {
    Benchmark b = new Benchmark(new StringReader(args[0]));
    b.execute();
    Report rep = new SumByName(b.getRunData()).report();
    out.println("outOf=" + rep.getOutOf() + " size=" + rep.getSize() + " reported=" + rep.getReported());
    for (String line : rep.getText().split("\n", -1)) {
      out.println(escape(maskReportLine(line)));
    }
    b.getRunData().close();
  }

  /**
   * A report data line (its last two non-padding tokens are the memory columns): the rate and
   * elapsed tokens (with a fraction) and the two memory tokens become {@code #}; the {@code -}
   * padding (its width follows the masked values) is dropped; the counts stay.
   */
  static String maskReportLine(String line) {
    String[] t = line.trim().split("\\s+");
    List<Integer> nums = new ArrayList<>();
    for (int i = 0; i < t.length; i++) {
      if (!t[i].equals("-")) nums.add(i);
    }
    int k = nums.size();
    if (k < 4
        || !t[nums.get(k - 1)].matches("[\\d,]+")
        || !t[nums.get(k - 2)].matches("[\\d,]+")) {
      return line;
    }
    StringBuilder sb = new StringBuilder();
    for (int i = 0; i < t.length; i++) {
      if (t[i].equals("-")) continue;
      if (sb.length() > 0) sb.append(' ');
      boolean mask =
          i == nums.get(k - 1) || i == nums.get(k - 2) || t[i].matches("[\\d,]*\\.\\d+");
      sb.append(mask ? "#" : t[i]);
    }
    return sb.toString();
  }


}
