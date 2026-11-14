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
import java.io.IOException;
import java.io.OutputStreamWriter;
import java.io.PrintWriter;
import java.io.Reader;
import java.io.StringReader;
import java.lang.reflect.InvocationTargetException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import org.apache.lucene.analysis.Analyzer;
import org.apache.lucene.analysis.CharArraySet;
import org.apache.lucene.analysis.CharFilterFactory;
import org.apache.lucene.analysis.TokenFilterFactory;
import org.apache.lucene.analysis.TokenStream;
import org.apache.lucene.analysis.Tokenizer;
import org.apache.lucene.analysis.TokenizerFactory;
import org.apache.lucene.analysis.custom.CustomAnalyzer;
import org.apache.lucene.analysis.tokenattributes.BytesTermAttribute;
import org.apache.lucene.analysis.tokenattributes.CharTermAttribute;
import org.apache.lucene.analysis.tokenattributes.FlagsAttribute;
import org.apache.lucene.analysis.tokenattributes.KeywordAttribute;
import org.apache.lucene.analysis.tokenattributes.OffsetAttribute;
import org.apache.lucene.analysis.tokenattributes.PayloadAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionIncrementAttribute;
import org.apache.lucene.analysis.tokenattributes.PositionLengthAttribute;
import org.apache.lucene.analysis.tokenattributes.TermFrequencyAttribute;
import org.apache.lucene.analysis.tokenattributes.TermToBytesRefAttribute;
import org.apache.lucene.analysis.tokenattributes.TypeAttribute;
import org.apache.lucene.analysis.util.FilesystemResourceLoader;
import org.apache.lucene.util.Attribute;
import org.apache.lucene.util.AttributeImpl;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.ClasspathResourceLoader;
import org.apache.lucene.util.ResourceLoader;
import org.apache.lucene.util.ResourceLoaderAware;
import org.apache.lucene.analysis.LowerCaseFilter;
import org.apache.lucene.analysis.charfilter.HTMLStripCharFilterFactory;
import org.apache.lucene.analysis.core.LowerCaseFilterFactory;
import org.apache.lucene.analysis.core.WhitespaceAnalyzer;
import org.apache.lucene.analysis.core.WhitespaceTokenizer;
import org.apache.lucene.analysis.miscellaneous.ASCIIFoldingFilterFactory;
import org.apache.lucene.analysis.query.QueryAutoStopWordAnalyzer;
import org.apache.lucene.analysis.reverse.ReverseStringFilterFactory;
import org.apache.lucene.analysis.sinks.TeeSinkTokenFilter;
import org.apache.lucene.analysis.standard.StandardAnalyzer;
import org.apache.lucene.analysis.standard.StandardTokenizer;
import org.apache.lucene.analysis.standard.StandardTokenizerFactory;
import org.apache.lucene.document.Document;
import org.apache.lucene.document.Field;
import org.apache.lucene.document.TextField;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexReader;
import org.apache.lucene.index.IndexWriter;
import org.apache.lucene.index.IndexWriterConfig;
import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.util.Version;

/**
 * The Java side of the analysis oracle harness of {@code lucene-analysis-common} (rmp #446). Its
 * Rust counterpart is {@code crates/rucene-analysis-common/tests/analysis_oracle/mod.rs}, which
 * writes the spec file this program reads, launches it once ({@code java -cp
 * lucene-core.jar:lucene-analysis-common.jar AnalysisOracle.java <spec-file>}), builds the same
 * dump with Rucene and compares the two.
 *
 * <p><b>Spec file</b> (UTF-8, one record per line, fields separated by TAB; every text field is
 * escaped by {@link #escape}): {@code input <text>} lines form the corpus, in order; a {@code spec
 * <id> analyzer <class>} record builds that analyzer through its public constructor, with the
 * {@code arg <type> <value>...} lines that follow as arguments and the {@code call <method> <type>
 * <value>...} lines as setter calls ({@code type} is {@code int}, {@code boolean}, {@code string},
 * {@code charset} or {@code charset_ic}, the last two a {@link CharArraySet} of the remaining fields
 * without or with ignoreCase); a {@code spec <id> custom} record builds a {@link CustomAnalyzer} and
 * a {@code spec <id> factories} record a chain of factories created per input, both from the {@code
 * configdir <path>}, {@code charfilter <name> <key=value>...}, {@code tokenizer <name>
 * <key=value>...} and {@code filter <name> <key=value>...} lines that follow, the factories looked
 * up by SPI name.
 *
 * <p>A {@code spec <id> scenario <name>} record builds the analyzer of a fixed scenario ({@link
 * #scenario}): analyzers that need an index built from the corpus or a hand-made chain.
 *
 * <p>With the single argument {@code --spi} it prints the factory registry instead ({@link
 * #spi}).
 *
 * <p><b>Dump</b> (stdout): {@code spec <id>}, then per input {@code input <n>}, one {@code tok}
 * line per token and an {@code end} line with the attributes after {@code end()}, both printing
 * {@link #attributes}, and an {@code attrs} line with the simple names of the attribute interfaces
 * the stream holds, sorted; an exception prints {@code error <simple class name>} instead of the
 * rest of the input. A spec that cannot be built prints {@code spec-error <simple class name>}.
 * Every stream is consumed with the full workflow ({@code reset}, {@code incrementToken}, {@code
 * end}, {@code close}) after adding the term, offset, position increment and length, type, flags,
 * payload, keyword and term frequency attributes, as {@code BaseTokenStreamTestCase} adds them.
 */
public final class AnalysisOracle {

  private AnalysisOracle() {}

  /** The field name every analyzer is asked for. */
  static final String FIELD = "field";

  /** Printable ASCII except the backslash as is, every other UTF-16 unit as {@code \\uXXXX}. */
  static String escape(CharSequence s) {
    StringBuilder out = new StringBuilder();
    for (int i = 0; i < s.length(); i++) {
      char ch = s.charAt(i);
      if (ch >= 0x20 && ch <= 0x7e && ch != '\\') {
        out.append(ch);
      } else {
        out.append(String.format("\\u%04X", (int) ch));
      }
    }
    return out.toString();
  }

  /** The inverse of {@link #escape}. */
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

  /** The bytes of {@code ref} in hexadecimal, {@code -} when empty, {@code null} for none. */
  static String hex(BytesRef ref) {
    if (ref == null) {
      return "null";
    }
    if (ref.length == 0) {
      return "-";
    }
    StringBuilder out = new StringBuilder();
    for (int i = 0; i < ref.length; i++) {
      out.append(String.format("%02x", ref.bytes[ref.offset + i] & 0xff));
    }
    return out.toString();
  }

  /** The attribute interfaces whose values {@link #attributes} prints by name. */
  static final Set<Class<? extends Attribute>> KNOWN =
      Set.of(
          CharTermAttribute.class,
          TermToBytesRefAttribute.class,
          BytesTermAttribute.class,
          TypeAttribute.class,
          PositionIncrementAttribute.class,
          PositionLengthAttribute.class,
          OffsetAttribute.class,
          FlagsAttribute.class,
          PayloadAttribute.class,
          KeywordAttribute.class,
          TermFrequencyAttribute.class);

  /**
   * The attributes of {@code ts}: {@code term=} (escaped), {@code bytes=} (the {@link
   * TermToBytesRefAttribute} bytes), {@code type=}, {@code posInc=}, {@code posLen=}, {@code
   * offset=start-end}, {@code flags=}, {@code payload=}, {@code keyword=} and {@code tf=}, each only
   * when its attribute is present, and {@code extra=} with every property reflected for any other
   * attribute interface, {@code Interface#key=value} sorted and joined by {@code ;}.
   */
  static String attributes(TokenStream ts) {
    StringBuilder b = new StringBuilder();
    if (ts.hasAttribute(CharTermAttribute.class)) {
      b.append(" term=").append(escape(ts.getAttribute(CharTermAttribute.class)));
    }
    if (ts.hasAttribute(TermToBytesRefAttribute.class)) {
      b.append(" bytes=").append(hex(ts.getAttribute(TermToBytesRefAttribute.class).getBytesRef()));
    }
    if (ts.hasAttribute(TypeAttribute.class)) {
      b.append(" type=").append(escape(ts.getAttribute(TypeAttribute.class).type()));
    }
    if (ts.hasAttribute(PositionIncrementAttribute.class)) {
      b.append(" posInc=")
          .append(ts.getAttribute(PositionIncrementAttribute.class).getPositionIncrement());
    }
    if (ts.hasAttribute(PositionLengthAttribute.class)) {
      b.append(" posLen=")
          .append(ts.getAttribute(PositionLengthAttribute.class).getPositionLength());
    }
    if (ts.hasAttribute(OffsetAttribute.class)) {
      OffsetAttribute o = ts.getAttribute(OffsetAttribute.class);
      b.append(" offset=").append(o.startOffset()).append('-').append(o.endOffset());
    }
    if (ts.hasAttribute(FlagsAttribute.class)) {
      b.append(" flags=").append(ts.getAttribute(FlagsAttribute.class).getFlags());
    }
    if (ts.hasAttribute(PayloadAttribute.class)) {
      b.append(" payload=").append(hex(ts.getAttribute(PayloadAttribute.class).getPayload()));
    }
    if (ts.hasAttribute(KeywordAttribute.class)) {
      b.append(" keyword=").append(ts.getAttribute(KeywordAttribute.class).isKeyword());
    }
    if (ts.hasAttribute(TermFrequencyAttribute.class)) {
      b.append(" tf=").append(ts.getAttribute(TermFrequencyAttribute.class).getTermFrequency());
    }
    TreeSet<String> extra = new TreeSet<>();
    for (Iterator<AttributeImpl> it = ts.getAttributeImplsIterator(); it.hasNext(); ) {
      it.next()
          .reflectWith(
              (attClass, key, value) -> {
                if (!KNOWN.contains(attClass)) {
                  String v;
                  if (value == null) {
                    v = "null";
                  } else if (value instanceof BytesRef) {
                    v = hex((BytesRef) value);
                  } else if (value instanceof CharSequence) {
                    v = escape((CharSequence) value);
                  } else {
                    v = escape(String.valueOf(value));
                  }
                  extra.add(attClass.getSimpleName() + "#" + key + "=" + v);
                }
              });
    }
    if (!extra.isEmpty()) {
      b.append(" extra=").append(String.join(";", extra));
    }
    return b.toString();
  }

  /** The simple names of the attribute interfaces {@code ts} holds, sorted, joined by commas. */
  static String attributeClasses(TokenStream ts) {
    TreeSet<String> names = new TreeSet<>();
    for (Iterator<Class<? extends Attribute>> it = ts.getAttributeClassesIterator();
        it.hasNext(); ) {
      names.add(it.next().getSimpleName());
    }
    return String.join(",", names);
  }

  /** The full workflow over {@code ts}, printing every token and the end state. */
  static void consume(TokenStream ts, PrintWriter out) throws IOException {
    ts.addAttribute(CharTermAttribute.class);
    ts.addAttribute(OffsetAttribute.class);
    ts.addAttribute(PositionIncrementAttribute.class);
    ts.addAttribute(PositionLengthAttribute.class);
    ts.addAttribute(TypeAttribute.class);
    ts.addAttribute(FlagsAttribute.class);
    ts.addAttribute(PayloadAttribute.class);
    ts.addAttribute(KeywordAttribute.class);
    ts.addAttribute(TermFrequencyAttribute.class);
    ts.reset();
    while (ts.incrementToken()) {
      out.println("tok" + attributes(ts));
    }
    ts.end();
    out.println("end" + attributes(ts));
    out.println("attrs " + attributeClasses(ts));
    ts.close();
  }

  /** The simple name of the exception a failure reports, through reflective wrapping. */
  static String failure(Throwable t) {
    while (t instanceof InvocationTargetException && t.getCause() != null) {
      t = t.getCause();
    }
    return t.getClass().getSimpleName();
  }

  // ---------------------------------------------------------------------------------------------
  // Specs
  // ---------------------------------------------------------------------------------------------

  /** One spec of the spec file. */
  static final class Spec {
    final String id;
    final String kind;
    final String target;
    final List<String[]> args = new ArrayList<>();
    final List<String[]> calls = new ArrayList<>();
    final List<Object[]> chain = new ArrayList<>(); // {kind, name, Map<String,String>}
    String configDir;

    Spec(String id, String kind, String target) {
      this.id = id;
      this.kind = kind;
      this.target = target;
    }
  }

  static Class<?> argType(String type) {
    switch (type) {
      case "int":
        return int.class;
      case "boolean":
        return boolean.class;
      case "string":
        return String.class;
      case "charset":
      case "charset_ic":
        return CharArraySet.class;
      case "analyzer":
        return Analyzer.class;
      case "analyzermap":
        return Map.class;
      default:
        throw new IllegalArgumentException("unknown argument type " + type);
    }
  }

  /** A new instance of the analyzer class {@code name}, through its public no-args constructor. */
  static Analyzer newAnalyzer(String name) {
    try {
      return Class.forName(name).asSubclass(Analyzer.class).getConstructor().newInstance();
    } catch (ReflectiveOperationException e) {
      throw new IllegalArgumentException("cannot instantiate analyzer " + name, e);
    }
  }

  static Object argValue(String[] fields, int from) {
    String type = fields[from];
    switch (type) {
      case "int":
        return Integer.parseInt(fields[from + 1]);
      case "boolean":
        return Boolean.parseBoolean(fields[from + 1]);
      case "string":
        return unescape(fields[from + 1]);
      case "charset":
      case "charset_ic":
        List<String> words = new ArrayList<>();
        for (int i = from + 1; i < fields.length; i++) {
          words.add(unescape(fields[i]));
        }
        return new CharArraySet(words, type.equals("charset_ic"));
      case "analyzer":
        return newAnalyzer(unescape(fields[from + 1]));
      case "analyzermap":
        Map<String, Analyzer> analyzers = new HashMap<>();
        for (int i = from + 1; i < fields.length; i++) {
          String entry = unescape(fields[i]);
          int eq = entry.indexOf('=');
          analyzers.put(entry.substring(0, eq), newAnalyzer(entry.substring(eq + 1)));
        }
        return analyzers;
      default:
        throw new IllegalArgumentException("unknown argument type " + type);
    }
  }

  static Map<String, String> params(String[] fields) {
    Map<String, String> params = new LinkedHashMap<>();
    for (int i = 2; i < fields.length; i++) {
      int eq = fields[i].indexOf('=');
      params.put(unescape(fields[i].substring(0, eq)), unescape(fields[i].substring(eq + 1)));
    }
    return params;
  }

  static Analyzer analyzer(Spec spec) throws Exception {
    Class<?>[] types = new Class<?>[spec.args.size()];
    Object[] values = new Object[spec.args.size()];
    for (int i = 0; i < types.length; i++) {
      types[i] = argType(spec.args.get(i)[1]);
      values[i] = argValue(spec.args.get(i), 1);
    }
    Analyzer analyzer =
        Class.forName(spec.target)
            .asSubclass(Analyzer.class)
            .getConstructor(types)
            .newInstance(values);
    for (String[] call : spec.calls) {
      analyzer
          .getClass()
          .getMethod(call[1], argType(call[2]))
          .invoke(analyzer, argValue(call, 2));
    }
    return analyzer;
  }

  /**
   * A stream over a {@link TeeSinkTokenFilter}'s sink that consumes the tee first: {@code reset}
   * resets the tee, consumes all its tokens and resets the sink; the attributes are the sink's.
   */
  static final class SinkAfterTee extends TokenStream {
    private final TeeSinkTokenFilter tee;
    private final TokenStream sink;

    SinkAfterTee(TeeSinkTokenFilter tee, TokenStream sink) {
      super(sink);
      this.tee = tee;
      this.sink = sink;
    }

    @Override
    public boolean incrementToken() throws IOException {
      return sink.incrementToken();
    }

    @Override
    public void reset() throws IOException {
      tee.reset();
      tee.consumeAllTokens();
      sink.reset();
    }

    @Override
    public void end() throws IOException {
      sink.end();
    }

    @Override
    public void close() throws IOException {
      tee.close();
      sink.close();
    }
  }

  /** A new instance of the delegate analyzer {@code name}: {@code whitespace} or {@code standard}. */
  static Analyzer delegate(String name) {
    switch (name) {
      case "whitespace":
        return new WhitespaceAnalyzer();
      case "standard":
        return new StandardAnalyzer();
      default:
        throw new IllegalArgumentException("unknown delegate " + name);
    }
  }

  /** A new tokenizer {@code name}: {@code whitespace} or {@code standard}. */
  static Tokenizer tokenizer(String name) {
    switch (name) {
      case "whitespace":
        return new WhitespaceTokenizer();
      case "standard":
        return new StandardTokenizer();
      default:
        throw new IllegalArgumentException("unknown tokenizer " + name);
    }
  }

  /**
   * The analyzer of scenario {@code name}:
   *
   * <ul>
   *   <li>{@code qasw:<delegate>:<mode>:<value>[:<fields>]}: a {@link QueryAutoStopWordAnalyzer}
   *       over a new {@code <delegate>} analyzer and a reader of an index holding one document per
   *       corpus input, its text in the fields {@code field} and {@code body} (indexed with another
   *       {@code <delegate>}); {@code <mode>} {@code default} (no value), {@code percent} or {@code
   *       docfreq} selects the constructor, {@code <fields>} (comma-separated) the field list;
   *   <li>{@code tee:<tokenizer>:<stream>}: a {@code <tokenizer>} under a {@link
   *       TeeSinkTokenFilter}; {@code <stream>} {@code source} hands out the tee, {@code sink} its
   *       sink after the tee is consumed, {@code sink-lowercase} that sink under a {@link
   *       LowerCaseFilter};
   *   <li>{@code custom:when-term}, {@code custom:when-protected}, {@code custom:classes}: {@link
   *       CustomAnalyzer} builder paths the {@code custom} spec does not take.
   *   <li>{@code collation:<tag>:<strength>:<decomposition>}: a {@code CollationKeyAnalyzer} over
   *       {@code Collator.getInstance} of the locale of language tag {@code <tag>}, with that
   *       strength and decomposition.
   * </ul>
   */
  static Analyzer scenario(String name, List<String> corpus) throws Exception {
    String[] p = name.split(":", -1);
    switch (p[0]) {
      case "qasw":
        {
          ByteBuffersDirectory dir = new ByteBuffersDirectory();
          try (IndexWriter w = new IndexWriter(dir, new IndexWriterConfig(delegate(p[1])))) {
            for (String text : corpus) {
              Document doc = new Document();
              doc.add(new TextField("field", text, Field.Store.NO));
              doc.add(new TextField("body", text, Field.Store.NO));
              w.addDocument(doc);
            }
          }
          IndexReader reader = DirectoryReader.open(dir);
          Analyzer d = delegate(p[1]);
          List<String> fields = p.length > 4 ? Arrays.asList(p[4].split(",")) : null;
          switch (p[2]) {
            case "default":
              return new QueryAutoStopWordAnalyzer(d, reader);
            case "percent":
              return fields == null
                  ? new QueryAutoStopWordAnalyzer(d, reader, Float.parseFloat(p[3]))
                  : new QueryAutoStopWordAnalyzer(d, reader, fields, Float.parseFloat(p[3]));
            case "docfreq":
              return fields == null
                  ? new QueryAutoStopWordAnalyzer(d, reader, Integer.parseInt(p[3]))
                  : new QueryAutoStopWordAnalyzer(d, reader, fields, Integer.parseInt(p[3]));
            default:
              throw new IllegalArgumentException("unknown qasw mode " + p[2]);
          }
        }
      case "tee":
        return new Analyzer() {
          @Override
          protected TokenStreamComponents createComponents(String fieldName) {
            Tokenizer source = tokenizer(p[1]);
            TeeSinkTokenFilter tee = new TeeSinkTokenFilter(source);
            switch (p[2]) {
              case "source":
                return new TokenStreamComponents(source, tee);
              case "sink":
                return new TokenStreamComponents(
                    source, new SinkAfterTee(tee, tee.newSinkTokenStream()));
              case "sink-lowercase":
                return new TokenStreamComponents(
                    source, new LowerCaseFilter(new SinkAfterTee(tee, tee.newSinkTokenStream())));
              default:
                throw new IllegalArgumentException("unknown tee stream " + p[2]);
            }
          }
        };
      case "custom":
        switch (p[1]) {
          case "when-term":
            return CustomAnalyzer.builder()
                .withTokenizer("whitespace")
                .addTokenFilter("lowercase")
                .whenTerm(t -> t.toString().contains("o"))
                .addTokenFilter("uppercase")
                .addTokenFilter(ReverseStringFilterFactory.class)
                .endwhen()
                .addTokenFilter("asciifolding")
                .build();
          case "when-protected":
            return CustomAnalyzer.builder()
                .withTokenizer("standard")
                .addTokenFilter("lowercase")
                .when(
                    "protectedterm",
                    "protected",
                    "org/apache/lucene/analysis/snowball/english_stop.txt")
                .addTokenFilter("reversestring")
                .endwhen()
                .build();
          case "classes":
            return CustomAnalyzer.builder()
                .withDefaultMatchVersion(Version.LUCENE_9_0_0)
                .addCharFilter(HTMLStripCharFilterFactory.class)
                .withTokenizer(StandardTokenizerFactory.class)
                .addTokenFilter(ASCIIFoldingFilterFactory.class, "preserveOriginal", "true")
                .addTokenFilter(LowerCaseFilterFactory.class)
                .withPositionIncrementGap(100)
                .withOffsetGap(1000)
                .build();
          default:
            throw new IllegalArgumentException("unknown custom scenario " + p[1]);
        }
      case "collation":
        {
          java.text.Collator collator =
              java.text.Collator.getInstance(new java.util.Locale.Builder().setLanguageTag(p[1]).build());
          collator.setStrength(Integer.parseInt(p[2]));
          collator.setDecomposition(Integer.parseInt(p[3]));
          return new org.apache.lucene.collation.CollationKeyAnalyzer(collator);
        }
      default:
        throw new IllegalArgumentException("unknown scenario " + name);
    }
  }

  static ResourceLoader loader(Spec spec) {
    ClassLoader classLoader = CustomAnalyzer.class.getClassLoader();
    return spec.configDir == null
        ? new ClasspathResourceLoader(classLoader)
        : new FilesystemResourceLoader(Paths.get(spec.configDir), classLoader);
  }

  @SuppressWarnings("unchecked")
  static Analyzer custom(Spec spec) throws IOException {
    CustomAnalyzer.Builder builder = CustomAnalyzer.builder(loader(spec));
    for (Object[] element : spec.chain) {
      Map<String, String> params = new HashMap<>((Map<String, String>) element[2]);
      switch ((String) element[0]) {
        case "charfilter":
          builder.addCharFilter((String) element[1], params);
          break;
        case "tokenizer":
          builder.withTokenizer((String) element[1], params);
          break;
        default:
          builder.addTokenFilter((String) element[1], params);
          break;
      }
    }
    return builder.build();
  }

  /** The factories of a {@code factories} spec, built and informed once. */
  static final class Factories {
    final List<CharFilterFactory> charFilters = new ArrayList<>();
    TokenizerFactory tokenizer;
    final List<TokenFilterFactory> filters = new ArrayList<>();

    TokenStream create(String text) {
      Reader reader = new StringReader(text);
      for (CharFilterFactory f : charFilters) {
        reader = f.create(reader);
      }
      Tokenizer source = tokenizer.create();
      source.setReader(reader);
      TokenStream ts = source;
      for (TokenFilterFactory f : filters) {
        ts = f.create(ts);
      }
      return ts;
    }
  }

  @SuppressWarnings("unchecked")
  static Factories factories(Spec spec) throws IOException {
    ResourceLoader loader = loader(spec);
    Factories factories = new Factories();
    for (Object[] element : spec.chain) {
      Map<String, String> params = new HashMap<>((Map<String, String>) element[2]);
      Object factory;
      switch ((String) element[0]) {
        case "charfilter":
          factory = CharFilterFactory.forName((String) element[1], params);
          factories.charFilters.add((CharFilterFactory) factory);
          break;
        case "tokenizer":
          factory = TokenizerFactory.forName((String) element[1], params);
          factories.tokenizer = (TokenizerFactory) factory;
          break;
        default:
          factory = TokenFilterFactory.forName((String) element[1], params);
          factories.filters.add((TokenFilterFactory) factory);
          break;
      }
      if (factory instanceof ResourceLoaderAware) {
        ((ResourceLoaderAware) factory).inform(loader);
      }
    }
    return factories;
  }

  // ---------------------------------------------------------------------------------------------
  // Main
  // ---------------------------------------------------------------------------------------------

  /**
   * The {@code --spi} mode: every name of {@code availableCharFilters()}, {@code
   * availableTokenizers()} and {@code availableTokenFilters()}, in the order the loaders list them,
   * with the class {@code lookupClass} finds for it: {@code charfilter|tokenizer|filter <name>
   * <class>}.
   */
  static void spi(PrintWriter out) {
    for (String name : CharFilterFactory.availableCharFilters()) {
      out.println("charfilter " + name + " " + CharFilterFactory.lookupClass(name).getName());
    }
    for (String name : TokenizerFactory.availableTokenizers()) {
      out.println("tokenizer " + name + " " + TokenizerFactory.lookupClass(name).getName());
    }
    for (String name : TokenFilterFactory.availableTokenFilters()) {
      out.println("filter " + name + " " + TokenFilterFactory.lookupClass(name).getName());
    }
  }

  public static void main(String[] args) throws Exception {
    if (args[0].equals("--spi")) {
      PrintWriter out =
          new PrintWriter(
              new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8)));
      spi(out);
      out.flush();
      return;
    }
    List<String> corpus = new ArrayList<>();
    List<Spec> specs = new ArrayList<>();
    Spec current = null;
    for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
      if (line.isEmpty() || line.startsWith("#")) {
        continue;
      }
      String[] f = line.split("\t", -1);
      switch (f[0]) {
        case "input":
          corpus.add(unescape(f[1]));
          break;
        case "spec":
          current = new Spec(f[1], f[2], f.length > 3 ? f[3] : null);
          specs.add(current);
          break;
        case "arg":
          current.args.add(f);
          break;
        case "call":
          current.calls.add(f);
          break;
        case "configdir":
          current.configDir = unescape(f[1]);
          break;
        case "charfilter":
        case "tokenizer":
        case "filter":
          current.chain.add(new Object[] {f[0], unescape(f[1]), params(f)});
          break;
        default:
          throw new IllegalArgumentException("unknown spec record: " + Arrays.toString(f));
      }
    }

    PrintWriter out =
        new PrintWriter(
            new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8)));
    for (Spec spec : specs) {
      out.println("spec " + spec.id);
      Analyzer analyzer = null;
      Factories factories = null;
      try {
        switch (spec.kind) {
          case "analyzer":
            analyzer = analyzer(spec);
            break;
          case "custom":
            analyzer = custom(spec);
            break;
          case "factories":
            factories = factories(spec);
            break;
          case "scenario":
            analyzer = scenario(unescape(spec.target), corpus);
            break;
          default:
            throw new IllegalArgumentException("unknown spec kind " + spec.kind);
        }
      } catch (Exception e) {
        out.println("spec-error " + failure(e));
        continue;
      }
      for (int i = 0; i < corpus.size(); i++) {
        out.println("input " + i);
        TokenStream ts = null;
        try {
          ts =
              analyzer != null
                  ? analyzer.tokenStream(FIELD, corpus.get(i))
                  : factories.create(corpus.get(i));
          consume(ts, out);
        } catch (Exception e) {
          out.println("error " + failure(e));
          if (ts != null) {
            try {
              ts.close();
            } catch (Exception ignored) {
              // The stream is already broken; the error is reported above.
            }
          }
        }
      }
      if (analyzer != null) {
        analyzer.close();
      }
    }
    out.flush();
  }
}
