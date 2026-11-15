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
import java.lang.reflect.Array;
import java.lang.reflect.Constructor;
import java.lang.reflect.Executable;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.DoublePredicate;
import java.util.function.Supplier;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexReader;
import org.apache.lucene.index.LeafReaderContext;
import org.apache.lucene.index.PostingsEnum;
import org.apache.lucene.index.Term;
import org.apache.lucene.queries.function.FunctionValues;
import org.apache.lucene.queries.function.ValueSource;
import org.apache.lucene.queries.intervals.IntervalIterator;
import org.apache.lucene.queries.intervals.IntervalMatchesIterator;
import org.apache.lucene.queries.intervals.IntervalsSource;
import org.apache.lucene.queries.spans.SpanCollector;
import org.apache.lucene.queries.spans.SpanQuery;
import org.apache.lucene.queries.spans.SpanWeight;
import org.apache.lucene.queries.spans.Spans;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.Explanation;
import org.apache.lucene.search.DocIdSetIterator;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.MatchesIterator;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.QueryVisitor;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.ScoreMode;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.similarities.Similarity;
import org.apache.lucene.store.FSDirectory;
import org.apache.lucene.util.BytesRef;
import org.apache.lucene.util.BytesRefBuilder;
import org.apache.lucene.util.automaton.ByteRunAutomaton;

/**
 * The Java side of the query modules' oracle harness (rmp #475; its Rust side is {@code
 * crates/rucene-queries/tests/query_oracle/mod.rs}). It reads one spec file and prints one dump
 * that the Rust side rebuilds with Rucene, line for line.
 *
 * <p>Run as a single source file on the class path {@code lucene-core}, {@code lucene-queries},
 * {@code lucene-queryparser} and {@code lucene-sandbox} 10.5.0: {@code java -cp <jars>
 * QueryOracle.java <spec file>}, with {@code -Dtests.seed} set to the seed the Rust side runs
 * under (it fixes {@code StringHelper.GOOD_FAST_HASH_SEED}, so the hash orders of query clause
 * sets). It writes nothing but standard output, so concurrent runs are independent.
 *
 * <p>Spec file: one tab-separated record per line ({@code #} lines are comments):
 *
 * <ul>
 *   <li>{@code index <path>}: opens {@code DirectoryReader.open(FSDirectory.open(path))} and a
 *       {@code new IndexSearcher(reader)} with the query cache disabled, bound to {@code $searcher}
 *       and {@code $reader};
 *   <li>{@code similarity <expr>}: {@code searcher.setSimilarity(expr)};
 *   <li>{@code let <name> <expr>}: binds {@code $name};
 *   <li>{@code do <expr>}: evaluates the expression for its effect;
 *   <li>{@code query <id> <expr>}: search parity of the query the expression builds;
 *   <li>{@code spans <id> <expr> <postings>}: span parity of the span query the expression
 *       builds, its spans iterated with the {@code SpanWeight.Postings} constant {@code postings};
 *   <li>{@code parse <id> <expr> <input> [<field>]}: parser parity of the expression evaluated
 *       with {@code $input} bound to the (escaped) input.
 * </ul>
 *
 * <p>Expressions are S-expressions: {@code "..."} a {@code String} (escaped as below), {@code
 * null}, {@code i:1} {@code int}, {@code l:1} {@code long}, {@code f:1.5} {@code float}, {@code
 * d:1.5} {@code double}, {@code z:true} {@code boolean}, {@code c:0041} {@code char}, {@code b:-1}
 * {@code byte}, {@code $name} a binding; {@code (new <class> args...)} the public constructor the arguments fit, {@code
 * (static <class> <method> args...)}, {@code (call <method> <receiver> args...)}, {@code (field
 * <class> <name>)} a public static field (an enum constant), {@code (array <component class>
 * elems...)}, {@code (list elems...)} an {@code ArrayList}, and {@code (cast <class> expr)}, which
 * gives an argument the static type an overload is chosen by. Class names are binary names ({@code
 * BooleanClause$Occur}) or primitive names. An overload is the most specific public member whose
 * parameters accept the arguments (boxed values fit their primitives, {@code null} any reference);
 * no other conversion is made.
 *
 * <p>Dump, per {@code query}: {@code query <id>}, then {@code tostring}, {@code classtree}, {@code
 * rewritten}, {@code rclasstree}, {@code count}, {@code hits <total> <relation>}, one {@code hit
 * <doc> <score bits>} per hit of {@code searcher.search(q, max(1, maxDoc))}, and for every document
 * {@code explain <doc>} and one {@code expl <depth> <match> <value> <description>} per node; a
 * failure prints {@code error <stage> <class> <message>} and ends the record. Per {@code parse}:
 * {@code parse <id>}, then {@code tostring} (and {@code tostringf} with the field), {@code
 * classtree} (each, when it throws, {@code error <stage> <class> <message>}), or {@code value
 * <class> <string>} for a result that is no query, or {@code error <class> <message>}. Per {@code spans}: {@code spans <id>}, {@code tostring}, {@code rewritten},
 * then per leaf {@code leaf <ord> null} when {@code getSpans} returns {@code null}, else {@code leaf
 * <ord> <docBase>} followed, for every document the spans iterate, by {@code sdoc <doc>} and, per
 * span, {@code span <start> <end> <width>} and one {@code coll <term> <position> <payload hex or
 * null>} per {@code collectLeaf} call its {@code collect} makes; a failure prints {@code error spans
 * <class> <message>} and ends the record. The class tree is the query's class and the calls {@code
 * Query.visit(QueryVisitor)} makes on a visitor that records every {@code getSubVisitor}, {@code
 * consumeTerms}, {@code consumeTermsMatching} and {@code visitLeaf}, siblings sorted (see {@link
 * Node#render}). Strings are escaped: printable
 * ASCII except the backslash as is, every other UTF-16 unit as {@code \\uXXXX}.
 */
public final class QueryOracle {

  private QueryOracle() {}

  // ---------------------------------------------------------------------------------------------
  // Escaping
  // ---------------------------------------------------------------------------------------------

  /** Printable ASCII except the backslash as is, every other UTF-16 unit as {@code \\uXXXX}. */
  static String escape(CharSequence s) {
    if (s == null) {
      return "null";
    }
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

  // ---------------------------------------------------------------------------------------------
  // Expressions
  // ---------------------------------------------------------------------------------------------

  /** A string literal of an expression. */
  record Str(String value) {}

  /** A value with the static type an overload is chosen by ({@code cast}). */
  record Typed(Object value, Class<?> type) {}

  /** Parses one expression. */
  static Object parse(String text) {
    int[] pos = {0};
    Object node = parseNode(text, pos);
    skipSpaces(text, pos);
    if (pos[0] != text.length()) {
      throw new IllegalArgumentException("trailing text in expression: " + text);
    }
    return node;
  }

  static void skipSpaces(String text, int[] pos) {
    while (pos[0] < text.length() && text.charAt(pos[0]) == ' ') {
      pos[0]++;
    }
  }

  static Object parseNode(String text, int[] pos) {
    skipSpaces(text, pos);
    char ch = text.charAt(pos[0]);
    if (ch == '(') {
      pos[0]++;
      List<Object> list = new ArrayList<>();
      while (true) {
        skipSpaces(text, pos);
        if (text.charAt(pos[0]) == ')') {
          pos[0]++;
          return list;
        }
        list.add(parseNode(text, pos));
      }
    }
    if (ch == '"') {
      int end = text.indexOf('"', pos[0] + 1);
      String value = unescape(text.substring(pos[0] + 1, end));
      pos[0] = end + 1;
      return new Str(value);
    }
    int start = pos[0];
    while (pos[0] < text.length()
        && text.charAt(pos[0]) != ' '
        && text.charAt(pos[0]) != '('
        && text.charAt(pos[0]) != ')') {
      pos[0]++;
    }
    return text.substring(start, pos[0]);
  }

  static Class<?> classOf(String name) throws ClassNotFoundException {
    return switch (name) {
      case "int" -> int.class;
      case "long" -> long.class;
      case "float" -> float.class;
      case "double" -> double.class;
      case "boolean" -> boolean.class;
      case "char" -> char.class;
      case "byte" -> byte.class;
      case "short" -> short.class;
      default -> Class.forName(name);
    };
  }

  static Class<?> box(Class<?> c) {
    if (c == int.class) return Integer.class;
    if (c == long.class) return Long.class;
    if (c == float.class) return Float.class;
    if (c == double.class) return Double.class;
    if (c == boolean.class) return Boolean.class;
    if (c == char.class) return Character.class;
    if (c == byte.class) return Byte.class;
    if (c == short.class) return Short.class;
    return c;
  }

  final Map<String, Object> vars = new HashMap<>();

  Object eval(Object node) throws Throwable {
    if (node instanceof Str s) {
      return s.value();
    }
    if (node instanceof String atom) {
      if (atom.equals("null")) return null;
      if (atom.startsWith("$")) {
        String name = atom.substring(1);
        if (!vars.containsKey(name)) {
          throw new IllegalStateException("harness: unbound $" + name);
        }
        return vars.get(name);
      }
      String value = atom.substring(2);
      return switch (atom.substring(0, 2)) {
        case "i:" -> Integer.parseInt(value);
        case "l:" -> Long.parseLong(value);
        case "f:" -> Float.parseFloat(value);
        case "d:" -> Double.parseDouble(value);
        case "z:" -> Boolean.parseBoolean(value);
        case "c:" -> (char) Integer.parseInt(value, 16);
        case "b:" -> Byte.parseByte(value);
        default -> throw new IllegalStateException("harness: bad atom " + atom);
      };
    }
    @SuppressWarnings("unchecked")
    List<Object> list = (List<Object>) node;
    String head = (String) list.get(0);
    switch (head) {
      case "new":
        {
          Class<?> c = classOf((String) list.get(1));
          Object[] args = evalAll(list, 2);
          Constructor<?> ctor = (Constructor<?>) select(c.getConstructors(), args, false, c + " new");
          return invoke(() -> ctor.newInstance(values(args)));
        }
      case "static":
        {
          Class<?> c = classOf((String) list.get(1));
          String name = (String) list.get(2);
          Object[] args = evalAll(list, 3);
          Method m = (Method) select(named(c.getMethods(), name, true), args, true, c + "." + name);
          m.setAccessible(true);
          return invoke(() -> m.invoke(null, values(args)));
        }
      case "call":
        {
          String name = (String) list.get(1);
          Object recv = value(eval(list.get(2)));
          if (recv == null) {
            throw new NullPointerException(
                "Cannot invoke \"" + name + "()\" because the receiver is null");
          }
          Object[] args = evalAll(list, 3);
          Method m =
              (Method)
                  select(
                      named(recv.getClass().getMethods(), name, false),
                      args,
                      true,
                      recv.getClass() + "." + name);
          m.setAccessible(true);
          return invoke(() -> m.invoke(recv, values(args)));
        }
      case "field":
        {
          Class<?> c = classOf((String) list.get(1));
          Field f = c.getField((String) list.get(2));
          return f.get(null);
        }
      case "array":
        {
          Class<?> component = classOf((String) list.get(1));
          Object[] elems = evalAll(list, 2);
          Object array = Array.newInstance(component, elems.length);
          for (int i = 0; i < elems.length; i++) {
            Array.set(array, i, value(elems[i]));
          }
          return array;
        }
      case "list":
        {
          List<Object> out = new ArrayList<>();
          for (Object o : evalAll(list, 1)) {
            out.add(value(o));
          }
          return out;
        }
      case "cast":
        return new Typed(value(eval(list.get(2))), classOf((String) list.get(1)));
      default:
        throw new IllegalStateException("harness: bad form " + head);
    }
  }

  interface Call {
    Object run() throws Exception;
  }

  static Object invoke(Call call) throws Throwable {
    try {
      return call.run();
    } catch (InvocationTargetException e) {
      throw e.getCause();
    }
  }

  Object[] evalAll(List<Object> list, int from) throws Throwable {
    Object[] out = new Object[list.size() - from];
    for (int i = from; i < list.size(); i++) {
      out[i - from] = eval(list.get(i));
    }
    return out;
  }

  static Object value(Object o) {
    return o instanceof Typed t ? t.value() : o;
  }

  static Object[] values(Object[] args) {
    Object[] out = new Object[args.length];
    for (int i = 0; i < args.length; i++) {
      out[i] = value(args[i]);
    }
    return out;
  }

  static Executable[] named(Method[] methods, String name, boolean isStatic) {
    List<Executable> out = new ArrayList<>();
    for (Method m : methods) {
      if (m.getName().equals(name) && Modifier.isStatic(m.getModifiers()) == isStatic) {
        out.add(m);
      }
    }
    return out.toArray(new Executable[0]);
  }

  static boolean accepts(Class<?> param, Object arg) {
    if (arg == null) {
      return !param.isPrimitive();
    }
    Class<?> type = arg instanceof Typed t ? t.type() : arg.getClass();
    if (param.isPrimitive()) {
      return box(param) == type;
    }
    return param.isAssignableFrom(type);
  }

  /** The most specific of {@code candidates} whose parameters accept {@code args}. */
  static Executable select(Executable[] candidates, Object[] args, boolean method, String what) {
    List<Executable> applicable = new ArrayList<>();
    for (Executable e : candidates) {
      Class<?>[] params = e.getParameterTypes();
      if (params.length != args.length) {
        continue;
      }
      boolean ok = true;
      for (int i = 0; i < params.length && ok; i++) {
        ok = accepts(params[i], args[i]);
      }
      if (ok) {
        applicable.add(e);
      }
    }
    List<Executable> maximal = new ArrayList<>();
    for (Executable e : applicable) {
      boolean dominated = false;
      for (Executable o : applicable) {
        if (o != e && moreSpecific(o, e) && !moreSpecific(e, o)) {
          dominated = true;
          break;
        }
      }
      if (!dominated) {
        maximal.add(e);
      }
    }
    // A bridge or covariant override leaves equal signatures: keep the first.
    if (maximal.size() > 1) {
      Executable first = maximal.get(0);
      boolean same = true;
      for (Executable e : maximal) {
        same &= java.util.Arrays.equals(e.getParameterTypes(), first.getParameterTypes());
      }
      if (same) {
        maximal = List.of(first);
      }
    }
    if (maximal.size() != 1) {
      throw new IllegalStateException(
          "harness: " + maximal.size() + " overloads of " + what + " fit " + args.length + " args");
    }
    return maximal.get(0);
  }

  static boolean moreSpecific(Executable a, Executable b) {
    Class<?>[] pa = a.getParameterTypes();
    Class<?>[] pb = b.getParameterTypes();
    for (int i = 0; i < pa.length; i++) {
      if (!box(pb[i]).isAssignableFrom(box(pa[i]))) {
        return false;
      }
    }
    return true;
  }

  // ---------------------------------------------------------------------------------------------
  // Class tree
  // ---------------------------------------------------------------------------------------------

  /** One call the visitor recorded, with the calls made on the sub-visitor it returned. */
  static final class Node {
    final String label;
    final List<Node> children = new ArrayList<>();

    Node(String label) {
      this.label = label;
    }

    /**
     * The label, then the rendered children in sorted order: {@code BooleanQuery} and {@code
     * DisjunctionMaxQuery} visit hash sets of their clauses, and the hash of a {@code
     * BooleanQuery} mixes {@code Enum.hashCode()} of {@code BooleanClause.Occur}, an identity hash
     * no two JVMs share, so the visit order of nested clauses is not reproducible.
     */
    void render(StringBuilder out) {
      out.append(label);
      if (!children.isEmpty()) {
        List<String> rendered = new ArrayList<>();
        for (Node child : children) {
          StringBuilder b = new StringBuilder();
          child.render(b);
          rendered.add(b.toString());
        }
        java.util.Collections.sort(rendered);
        out.append('{').append(String.join(" ", rendered)).append('}');
      }
    }
  }

  /** Records every call {@code Query.visit} makes. */
  static final class TreeVisitor extends QueryVisitor {
    final List<Node> out;

    TreeVisitor(List<Node> out) {
      this.out = out;
    }

    @Override
    public void consumeTerms(Query query, Term... terms) {
      StringBuilder b = new StringBuilder("terms:").append(query.getClass().getName()).append(":[");
      for (int i = 0; i < terms.length; i++) {
        if (i > 0) b.append(',');
        b.append(escape(terms[i].toString()));
      }
      out.add(new Node(b.append(']').toString()));
    }

    @Override
    public void consumeTermsMatching(
        Query query, String field, Supplier<ByteRunAutomaton> automaton) {
      out.add(new Node("matching:" + query.getClass().getName() + ":" + escape(field)));
    }

    @Override
    public void visitLeaf(Query query) {
      out.add(new Node("leaf:" + query.getClass().getName()));
    }

    @Override
    public boolean acceptField(String field) {
      return true;
    }

    @Override
    public QueryVisitor getSubVisitor(BooleanClause.Occur occur, Query parent) {
      Node node = new Node("sub:" + occur.name() + ":" + parent.getClass().getName());
      out.add(node);
      return new TreeVisitor(node.children);
    }
  }

  static String classTree(Query q) {
    Node root = new Node(q.getClass().getName());
    q.visit(new TreeVisitor(root.children));
    StringBuilder out = new StringBuilder();
    root.render(out);
    return out.toString();
  }

  // ---------------------------------------------------------------------------------------------
  // Dumps
  // ---------------------------------------------------------------------------------------------

  final StringBuilder out = new StringBuilder();
  IndexReader reader;
  IndexSearcher searcher;

  void line(String... fields) {
    out.append(String.join("\t", fields)).append('\n');
  }

  void error(String stage, Throwable t) {
    if (stage == null) {
      line("error", t.getClass().getName(), escape(t.getMessage()));
    } else {
      line("error", stage, t.getClass().getName(), escape(t.getMessage()));
    }
  }

  static String numberValue(Number v) {
    if (v instanceof Float f) {
      return "F" + String.format("%08x", Float.floatToRawIntBits(f));
    }
    if (v instanceof Double d) {
      return "D" + String.format("%016x", Double.doubleToRawLongBits(d));
    }
    if (v instanceof Integer i) {
      return "I" + i;
    }
    if (v instanceof Long l) {
      return "L" + l;
    }
    return "N" + v.getClass().getName() + ":" + v;
  }

  void explanation(Explanation e, int depth) {
    line(
        "expl",
        Integer.toString(depth),
        Boolean.toString(e.isMatch()),
        numberValue(e.getValue()),
        escape(e.getDescription()));
    for (Explanation d : e.getDetails()) {
      explanation(d, depth + 1);
    }
  }

  void query(String id, String expr) {
    line("query", id);
    Query q;
    try {
      q = (Query) eval(parse(expr));
    } catch (Throwable t) {
      error("build", t);
      return;
    }
    String text;
    try {
      text = q.toString();
    } catch (Throwable t) {
      error("tostring", t);
      return;
    }
    line("tostring", escape(text));
    line("classtree", escape(classTree(q)));
    Query rewritten;
    try {
      rewritten = searcher.rewrite(q);
    } catch (Throwable t) {
      error("rewrite", t);
      return;
    }
    line("rewritten", escape(rewritten.toString()));
    line("rclasstree", escape(classTree(rewritten)));
    try {
      line("count", Integer.toString(searcher.count(q)));
    } catch (Throwable t) {
      error("count", t);
      return;
    }
    TopDocs td;
    try {
      td = searcher.search(q, Math.max(1, reader.maxDoc()));
    } catch (Throwable t) {
      error("search", t);
      return;
    }
    line("hits", Long.toString(td.totalHits.value()), td.totalHits.relation().name());
    for (ScoreDoc sd : td.scoreDocs) {
      line("hit", Integer.toString(sd.doc), String.format("%08x", Float.floatToRawIntBits(sd.score)));
    }
    for (int doc = 0; doc < reader.maxDoc(); doc++) {
      line("explain", Integer.toString(doc));
      try {
        explanation(searcher.explain(q, doc), 0);
      } catch (Throwable t) {
        error("explain", t);
        return;
      }
    }
  }

  /** The payload bytes in lower-case hexadecimal, {@code null} for none. */
  static String hex(BytesRef b) {
    if (b == null) {
      return "null";
    }
    StringBuilder out = new StringBuilder();
    for (int i = b.offset; i < b.offset + b.length; i++) {
      out.append(String.format("%02x", b.bytes[i] & 0xff));
    }
    return out.toString();
  }

  void spans(String id, String expr, String postings) {
    line("spans", id);
    Query q;
    try {
      q = (Query) eval(parse(expr));
    } catch (Throwable t) {
      error("build", t);
      return;
    }
    String text;
    try {
      text = q.toString();
    } catch (Throwable t) {
      error("tostring", t);
      return;
    }
    line("tostring", escape(text));
    try {
      Query rewritten = searcher.rewrite(q);
      line("rewritten", escape(rewritten.toString()));
      SpanWeight weight =
          ((SpanQuery) rewritten).createWeight(searcher, ScoreMode.COMPLETE_NO_SCORES, 1f);
      SpanWeight.Postings required = SpanWeight.Postings.valueOf(postings);
      List<String> collected = new ArrayList<>();
      SpanCollector collector =
          new SpanCollector() {
            @Override
            public void collectLeaf(PostingsEnum p, int position, Term term) throws java.io.IOException {
              collected.add(
                  String.join(
                      "\t",
                      "coll",
                      escape(term.toString()),
                      Integer.toString(position),
                      hex(p.getPayload())));
            }

            @Override
            public void reset() {}
          };
      for (LeafReaderContext ctx : reader.leaves()) {
        Spans s = weight.getSpans(ctx, required);
        if (s == null) {
          line("leaf", Integer.toString(ctx.ord), "null");
          continue;
        }
        line("leaf", Integer.toString(ctx.ord), Integer.toString(ctx.docBase));
        while (s.nextDoc() != Spans.NO_MORE_DOCS) {
          line("sdoc", Integer.toString(s.docID()));
          while (s.nextStartPosition() != Spans.NO_MORE_POSITIONS) {
            line(
                "span",
                Integer.toString(s.startPosition()),
                Integer.toString(s.endPosition()),
                Integer.toString(s.width()));
            collected.clear();
            s.collect(collector);
            for (String c : collected) {
              out.append(c).append('\n');
            }
          }
        }
      }
    } catch (Throwable t) {
      error("spans", t);
    }
  }

  // ---------------------------------------------------------------------------------------------
  // Custom records: intervals and function values
  // ---------------------------------------------------------------------------------------------

  /** A {@code DoublePredicate} the expressions can name: {@code lo <= d && d < hi}. */
  public static DoublePredicate between(double lo, double hi) {
    return d -> d >= lo && d < hi;
  }

  /** A payload filter the expressions can name: accepts a {@code null} payload. */
  public static java.util.function.Predicate<BytesRef> nullPayload() {
    return b -> b == null;
  }

  /** The {@code Map<Integer, String>} of an enum field: name {@code i} has value {@code i}. */
  public static Map<Integer, String> enumIntToString(String[] names) {
    Map<Integer, String> out = new HashMap<>();
    for (int i = 0; i < names.length; i++) {
      out.put(i, names[i]);
    }
    return out;
  }

  /** The {@code Map<String, Integer>} of an enum field: name {@code i} has value {@code i}. */
  public static Map<String, Integer> enumStringToInt(String[] names) {
    Map<String, Integer> out = new HashMap<>();
    for (int i = 0; i < names.length; i++) {
      out.put(names[i], i);
    }
    return out;
  }

  void custom(String kind, String id, String expr, String arg) {
    line("custom", kind, id);
    Object v;
    try {
      v = eval(parse(expr));
    } catch (Throwable t) {
      error("build", t);
      return;
    }
    try {
      switch (kind) {
        case "intervals" -> intervals((IntervalsSource) v, arg);
        case "values" -> values((ValueSource) v);
        default -> throw new IllegalStateException("harness: bad custom kind " + kind);
      }
    } catch (Throwable t) {
      error(kind, t);
    }
  }

  /**
   * Per leaf: every document the source's {@code IntervalIterator} visits, every interval (start,
   * end, gaps, width), then the {@code matches} of every document of the leaf (positions, offsets,
   * gaps and width of the top-level matches, and the sub-matches, depth first).
   */
  void intervals(IntervalsSource src, String field) throws java.io.IOException {
    line("tostring", escape(src.toString()));
    line("minextent", Integer.toString(src.minExtent()));
    for (LeafReaderContext ctx : reader.leaves()) {
      IntervalIterator it = src.intervals(field, ctx);
      if (it == null) {
        line("leaf", Integer.toString(ctx.ord), "null");
      } else {
        line("leaf", Integer.toString(ctx.ord), Integer.toString(ctx.docBase));
        int doc;
        while ((doc = it.nextDoc()) != DocIdSetIterator.NO_MORE_DOCS) {
          line("idoc", Integer.toString(doc), Integer.toString(it.start()), Integer.toString(it.end()));
          int pos;
          while ((pos = it.nextInterval()) != IntervalIterator.NO_MORE_INTERVALS) {
            line(
                "iv",
                Integer.toString(pos),
                Integer.toString(it.start()),
                Integer.toString(it.end()),
                Integer.toString(it.gaps()),
                Integer.toString(it.width()));
          }
          line("ivend", Integer.toString(it.start()), Integer.toString(it.end()));
        }
      }
      for (int doc = 0; doc < ctx.reader().maxDoc(); doc++) {
        IntervalMatchesIterator mi = src.matches(field, ctx, doc);
        if (mi == null) {
          continue;
        }
        line("mdoc", Integer.toString(doc));
        while (mi.next()) {
          line(
              "m",
              "0",
              Integer.toString(mi.startPosition()),
              Integer.toString(mi.endPosition()),
              Integer.toString(mi.startOffset()),
              Integer.toString(mi.endOffset()),
              Integer.toString(mi.gaps()),
              Integer.toString(mi.width()));
          subMatches(mi.getSubMatches(), 1);
        }
      }
    }
  }

  void subMatches(MatchesIterator mi, int depth) throws java.io.IOException {
    if (mi == null) {
      return;
    }
    while (mi.next()) {
      line(
          "m",
          Integer.toString(depth),
          Integer.toString(mi.startPosition()),
          Integer.toString(mi.endPosition()),
          Integer.toString(mi.startOffset()),
          Integer.toString(mi.endOffset()));
      subMatches(mi.getSubMatches(), depth + 1);
    }
  }

  interface Getter {
    String get() throws Exception;
  }

  void val(String name, Getter g) {
    try {
      line("val", name, g.get());
    } catch (Throwable t) {
      line("val", name, "err", t.getClass().getName(), escape(t.getMessage()));
    }
  }

  static String objectValue(Object o) {
    if (o == null) {
      return "null";
    }
    if (o instanceof Float f) {
      return "F" + String.format("%08x", Float.floatToRawIntBits(f));
    }
    if (o instanceof Double d) {
      return "D" + String.format("%016x", Double.doubleToRawLongBits(d));
    }
    if (o instanceof Integer i) {
      return "I" + i;
    }
    if (o instanceof Long l) {
      return "L" + l;
    }
    if (o instanceof Boolean b) {
      return "Z" + b;
    }
    if (o instanceof String str) {
      return "S" + escape(str);
    }
    if (o instanceof java.util.Date date) {
      return "T" + date.getTime();
    }
    return o.getClass().getName() + ":" + escape(o.toString());
  }

  /**
   * Per leaf and document: every {@code FunctionValues} getter, its value or the exception it
   * throws, after {@code createWeight} with {@code ValueSource.newContext(searcher)}.
   */
  void values(ValueSource vs) throws java.io.IOException {
    line("tostring", escape(vs.description()));
    Map<Object, Object> context = ValueSource.newContext(searcher);
    vs.createWeight(context, searcher);
    for (LeafReaderContext ctx : reader.leaves()) {
      line("leaf", Integer.toString(ctx.ord), Integer.toString(ctx.docBase));
      FunctionValues fv = vs.getValues(context, ctx);
      for (int d = 0; d < ctx.reader().maxDoc(); d++) {
        final int doc = d;
        line("vdoc", Integer.toString(doc));
        val("exists", () -> Boolean.toString(fv.exists(doc)));
        val("float", () -> "F" + String.format("%08x", Float.floatToRawIntBits(fv.floatVal(doc))));
        val(
            "double",
            () -> "D" + String.format("%016x", Double.doubleToRawLongBits(fv.doubleVal(doc))));
        val("int", () -> Integer.toString(fv.intVal(doc)));
        val("long", () -> Long.toString(fv.longVal(doc)));
        val("bool", () -> Boolean.toString(fv.boolVal(doc)));
        val("str", () -> escape(fv.strVal(doc)));
        val("object", () -> objectValue(fv.objectVal(doc)));
        val("ord", () -> Integer.toString(fv.ordVal(doc)));
        val("numord", () -> Integer.toString(fv.numOrd()));
        val(
            "fvec",
            () -> {
              float[] v = fv.floatVectorVal(doc);
              if (v == null) {
                return "null";
              }
              StringBuilder b = new StringBuilder();
              for (float x : v) {
                b.append(String.format("%08x,", Float.floatToRawIntBits(x)));
              }
              return b.toString();
            });
        val(
            "bvec",
            () -> {
              byte[] v = fv.byteVectorVal(doc);
              if (v == null) {
                return "null";
              }
              StringBuilder b = new StringBuilder();
              for (byte x : v) {
                b.append(x).append(',');
              }
              return b.toString();
            });
        val(
            "bytes",
            () -> {
              BytesRefBuilder b = new BytesRefBuilder();
              boolean r = fv.bytesVal(doc, b);
              return r + ":" + hex(b.get());
            });
        val("tostring", () -> escape(fv.toString(doc)));
      }
    }
  }

  void parseRecord(String id, String expr, String input, String field) {
    line("parse", id);
    vars.put("input", input);
    Object result;
    try {
      result = eval(parse(expr));
    } catch (Throwable t) {
      error(null, t);
      return;
    }
    if (result instanceof Query q) {
      // Each stage may throw (a term of a null field makes TermQuery.toString
      // throw a NullPointerException): its exception is recorded and the next
      // stage runs.
      try {
        line("tostring", escape(q.toString()));
      } catch (Throwable t) {
        error("tostring", t);
      }
      if (field != null) {
        try {
          line("tostringf", escape(q.toString(field)));
        } catch (Throwable t) {
          error("tostringf", t);
        }
      }
      try {
        line("classtree", escape(classTree(q)));
      } catch (Throwable t) {
        error("classtree", t);
      }
    } else {
      line(
          "value",
          result == null ? "null" : result.getClass().getName(),
          escape(String.valueOf(result)));
    }
  }

  void run(List<String> spec) throws Throwable {
    for (String l : spec) {
      if (l.isEmpty() || l.startsWith("#")) {
        continue;
      }
      String[] f = l.split("\t", -1);
      switch (f[0]) {
        case "index" -> {
          reader = DirectoryReader.open(FSDirectory.open(Paths.get(unescape(f[1]))));
          searcher = new IndexSearcher(reader);
          searcher.setQueryCache(null);
          vars.put("reader", reader);
          vars.put("searcher", searcher);
        }
        case "similarity" -> searcher.setSimilarity((Similarity) eval(parse(f[1])));
        case "let" -> vars.put(f[1], eval(parse(f[2])));
        case "do" -> eval(parse(f[1]));
        case "query" -> query(f[1], f[2]);
        case "spans" -> spans(f[1], f[2], f[3]);
        case "custom" -> custom(f[1], f[2], f[3], f[4]);
        case "parse" -> parseRecord(f[1], f[2], unescape(f[3]), f.length > 4 ? unescape(f[4]) : null);
        default -> throw new IllegalStateException("harness: bad spec line " + l);
      }
    }
    if (reader != null) {
      reader.close();
    }
  }

  public static void main(String[] args) throws Throwable {
    List<String> spec = Files.readAllLines(Paths.get(args[0]), StandardCharsets.UTF_8);
    QueryOracle oracle = new QueryOracle();
    oracle.run(spec);
    try (PrintWriter w =
        new PrintWriter(
            new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8)))) {
      w.print(oracle.out);
    }
  }
}
