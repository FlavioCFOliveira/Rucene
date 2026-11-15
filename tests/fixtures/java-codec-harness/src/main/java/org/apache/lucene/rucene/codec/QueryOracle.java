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
import java.util.function.Supplier;
import org.apache.lucene.index.DirectoryReader;
import org.apache.lucene.index.IndexReader;
import org.apache.lucene.index.Term;
import org.apache.lucene.search.BooleanClause;
import org.apache.lucene.search.Explanation;
import org.apache.lucene.search.IndexSearcher;
import org.apache.lucene.search.Query;
import org.apache.lucene.search.QueryVisitor;
import org.apache.lucene.search.ScoreDoc;
import org.apache.lucene.search.TopDocs;
import org.apache.lucene.search.similarities.Similarity;
import org.apache.lucene.store.FSDirectory;
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
 *   <li>{@code parse <id> <expr> <input> [<field>]}: parser parity of the expression evaluated
 *       with {@code $input} bound to the (escaped) input.
 * </ul>
 *
 * <p>Expressions are S-expressions: {@code "..."} a {@code String} (escaped as below), {@code
 * null}, {@code i:1} {@code int}, {@code l:1} {@code long}, {@code f:1.5} {@code float}, {@code
 * d:1.5} {@code double}, {@code z:true} {@code boolean}, {@code c:0041} {@code char}, {@code $name}
 * a binding; {@code (new <class> args...)} the public constructor the arguments fit, {@code
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
 * classtree}, or {@code value <class> <string>} for a result that is no query, or {@code error
 * <class> <message>}. The class tree is the query's class and the calls {@code
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
    line("tostring", escape(q.toString()));
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
      line("tostring", escape(q.toString()));
      if (field != null) {
        line("tostringf", escape(q.toString(field)));
      }
      line("classtree", escape(classTree(q)));
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
