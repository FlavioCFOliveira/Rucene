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

//! The Java-oracle harness of the query modules (rmp #475): Lucene 10.5.0 and
//! Rucene build the same queries and parse the same query strings, and their
//! dumps are compared line for line.
//!
//! **Where it lives.** This module is shared by the tests of
//! `rucene-queries` (`mod query_oracle;`), `rucene-sandbox` and
//! `rucene-queryparser` (`#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
//! mod query_oracle;`). It depends only on `rucene`, `rucene-test-framework`
//! and `randomizedtesting`; the classes of a module are bound by the test
//! that uses them ([`Registry`]). Its Java side is `QueryOracle.java`
//! (`tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/`),
//! run once per comparison as a single source file on the class path
//! `lucene-core`, `lucene-queries`, `lucene-queryparser` and `lucene-sandbox`
//! 10.5.0 from `~/.m2`, each jar SHA-1 checked against Maven Central's
//! `.sha1` ([`lucene_jars`]). Every JVM runs in its own working directory
//! under `TMPDIR` ([`WorkDir`]), so comparisons run in parallel safely. Both
//! sides run under one `tests.seed` ([`pin_hash_seed`]), which fixes the
//! `BytesRef` hash orders Lucene's queries iterate in.
//!
//! **Specs are data both sides build from.** A query, a parser or any
//! argument is an S-expression ([`parse_expr`]): `"text"`, `null`, `i:1`
//! (`int`), `l:1` (`long`), `f:1.5` (`float`), `d:1.5` (`double`), `z:true`,
//! `c:0041` (`char`), `$name`, `(new <class> args…)`, `(static <class>
//! <method> args…)`, `(call <method> <receiver> args…)`, `(field <class>
//! <name>)`, `(array <component class> elems…)`, `(list elems…)` and `(cast
//! <class> expr)`. Java evaluates it by reflection (the most specific public
//! member the arguments fit); Rust evaluates it through the bindings of a
//! [`Registry`], keyed by class, member name and arity ([`Registry::core`]
//! binds the core classes). [`expr`] builds the text.
//!
//! **Search parity** ([`SearchOracle`]): over one index written by Rucene
//! ([`OracleIndex`]) and read by both sides, each query gives the same
//! `toString()`, class tree (the query's class and every call
//! `Query.visit(QueryVisitor)` makes, siblings sorted: see
//! [`class_tree`]), rewritten query and its class tree,
//! `count`, total hits and relation, hits (document and the bits of the
//! `float` score), and `Explanation` of every document (match flag, value
//! with its class and bits, description, details); a failure gives the same
//! stage, exception class and message.
//!
//! **Parser parity** ([`ParseOracle`]): each query string, evaluated with
//! `$input` bound to it, gives the same `toString()` (and `toString(field)`)
//! and class tree, or the same exception class and message.
//!
//! [`compare`] reports the first differing line with its record, both lines
//! and the line before; [`corpus`] holds the seeded generators.

#![allow(dead_code)]

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::codecs::{register_codec, Lucene104Codec};
use rucene::document::{
    Document, IntPoint, NumericDocValuesField, SortedDocValuesField, Store, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{IndexReader, Term};
use rucene::search::boolean_clause::Occur;
use rucene::search::boolean_query::Builder as BooleanQueryBuilder;
use rucene::search::boost_query::BoostQuery;
use rucene::search::constant_score_query::ConstantScoreQuery;
use rucene::search::disjunction_max_query::DisjunctionMaxQuery;
use rucene::search::match_all_docs_query::MatchAllDocsQuery;
use rucene::search::match_no_docs_query::MatchNoDocsQuery;
use rucene::search::phrase_query::{Builder as PhraseQueryBuilder, PhraseQuery};
use rucene::search::query::{query_to_string, Query};
use rucene::search::query_visitor::QueryVisitor;
use rucene::search::similarities::{BM25Similarity, ClassicSimilarity, Explanation, Similarity};
use rucene::search::{ExplanationValue, IndexSearcher, TermQuery};
use rucene::store::{Directory, FSDirectory};
use rucene::util::automaton::ByteRunAutomaton;
use rucene::util::java_collections::java_string_hash_code;
use rucene::util::{BytesRef, StringHelper};
use rucene_test_framework::util::java_exceptions::java_throwable;

// ---------------------------------------------------------------------------
// Escaping
// ---------------------------------------------------------------------------

/// `QueryOracle.escape` over UTF-16 units: printable ASCII except the
/// backslash as is, every other unit as `\uXXXX`.
pub fn escape_units(units: &[u16]) -> String {
    let mut out = String::new();
    for &unit in units {
        if (0x20..=0x7e).contains(&unit) && unit != u16::from(b'\\') {
            out.push(char::from(unit as u8));
        } else {
            write!(out, "\\u{unit:04X}").unwrap();
        }
    }
    out
}

/// [`escape_units`] of a string's UTF-16 encoding.
pub fn escape(s: &str) -> String {
    escape_units(&s.encode_utf16().collect::<Vec<_>>())
}

/// [`escape`] of an optional string, `null` for none (Java's
/// `escape(null)`).
pub fn escape_opt(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_string(), escape)
}

/// The inverse of [`escape_units`].
pub fn unescape_units(s: &str) -> Vec<u16> {
    let mut out = Vec::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let u: String = chars.by_ref().skip(1).take(4).collect();
            out.push(u16::from_str_radix(&u, 16).expect("a \\u escape"));
        } else {
            let mut buf = [0u16; 2];
            out.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    out
}

/// The inverse of [`escape`]; lone surrogates are replaced.
pub fn unescape(s: &str) -> String {
    String::from_utf16_lossy(&unescape_units(s))
}

/// A string literal of an expression: `"` + [`escape`] + `"`, the quote
/// itself escaped.
pub fn lit(s: &str) -> String {
    format!("\"{}\"", escape(s).replace('"', "\\u0022"))
}

/// Builds an expression's text: `expr("new", &[...])` is `(new ...)`.
pub fn expr(head: &str, args: &[&str]) -> String {
    let mut out = format!("({head}");
    for a in args {
        out.push(' ');
        out.push_str(a);
    }
    out.push(')');
    out
}

// ---------------------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------------------

/// A parsed expression.
#[derive(Debug, Clone)]
pub enum Node {
    /// A string literal.
    Str(String),
    /// An atom: `null`, a typed literal, `$name`, a class or member name.
    Atom(String),
    /// A form.
    List(Vec<Node>),
}

impl Node {
    fn atom(&self) -> &str {
        match self {
            Node::Atom(a) => a,
            other => harness_fail(format!("expected an atom, found {other:?}")),
        }
    }
}

/// Parses one expression, as `QueryOracle.parse` does.
pub fn parse_expr(text: &str) -> Node {
    let units: Vec<char> = text.chars().collect();
    let mut pos = 0;
    let node = parse_node(&units, &mut pos);
    while pos < units.len() && units[pos] == ' ' {
        pos += 1;
    }
    if pos != units.len() {
        harness_fail(format!("trailing text in expression: {text}"));
    }
    node
}

fn parse_node(text: &[char], pos: &mut usize) -> Node {
    while text[*pos] == ' ' {
        *pos += 1;
    }
    match text[*pos] {
        '(' => {
            *pos += 1;
            let mut list = Vec::new();
            loop {
                while text[*pos] == ' ' {
                    *pos += 1;
                }
                if text[*pos] == ')' {
                    *pos += 1;
                    return Node::List(list);
                }
                list.push(parse_node(text, pos));
            }
        }
        '"' => {
            let start = *pos + 1;
            let mut end = start;
            while text[end] != '"' {
                end += 1;
            }
            *pos = end + 1;
            Node::Str(unescape(&text[start..end].iter().collect::<String>()))
        }
        _ => {
            let start = *pos;
            while *pos < text.len() && !matches!(text[*pos], ' ' | '(' | ')') {
                *pos += 1;
            }
            Node::Atom(text[start..*pos].iter().collect())
        }
    }
}

/// A Java object the Rust side holds: its class's binary name and the Rust
/// value standing for it.
#[derive(Clone)]
pub struct Object {
    /// `getClass().getName()`.
    pub class: String,
    cell: Rc<RefCell<Box<dyn Any>>>,
}

/// A value of an expression.
#[derive(Clone)]
pub enum Value {
    /// `null`.
    Null,
    /// A `String`.
    Str(String),
    /// An `Integer`.
    Int(i32),
    /// A `Long`.
    Long(i64),
    /// A `Float`.
    Float(f32),
    /// A `Double`.
    Double(f64),
    /// A `Boolean`.
    Bool(bool),
    /// A `Character`.
    Char(u16),
    /// An array: its component class and elements.
    Array(String, Vec<Value>),
    /// An `ArrayList`.
    List(Vec<Value>),
    /// Any other object.
    Object(Object),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Int(v) => write!(f, "i:{v}"),
            Value::Long(v) => write!(f, "l:{v}"),
            Value::Float(v) => write!(f, "f:{v}"),
            Value::Double(v) => write!(f, "d:{v}"),
            Value::Bool(v) => write!(f, "z:{v}"),
            Value::Char(v) => write!(f, "c:{v:04x}"),
            Value::Array(c, v) => write!(f, "{c}[]{v:?}"),
            Value::List(v) => write!(f, "list{v:?}"),
            Value::Object(o) => write!(f, "<{}>", o.class),
        }
    }
}

impl Value {
    /// An object of class `class` held as `value`.
    pub fn object<T: Any>(class: &str, value: T) -> Value {
        Value::Object(Object {
            class: class.to_string(),
            cell: Rc::new(RefCell::new(Box::new(value))),
        })
    }

    /// A query, of the class [`Query::java_class_name`] names.
    pub fn query(query: Arc<dyn Query>) -> Value {
        let class = query.java_class_name().to_string();
        Value::object(&class, query)
    }

    /// `getClass().getName()` of the value.
    pub fn class_name(&self) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Str(_) => "java.lang.String".into(),
            Value::Int(_) => "java.lang.Integer".into(),
            Value::Long(_) => "java.lang.Long".into(),
            Value::Float(_) => "java.lang.Float".into(),
            Value::Double(_) => "java.lang.Double".into(),
            Value::Bool(_) => "java.lang.Boolean".into(),
            Value::Char(_) => "java.lang.Character".into(),
            Value::Array(c, _) => format!("[L{c};"),
            Value::List(_) => "java.util.ArrayList".into(),
            Value::Object(o) => o.class.clone(),
        }
    }

    /// The `String`; a harness failure for any other value.
    pub fn as_str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            other => harness_fail(format!("expected a String, found {other:?}")),
        }
    }

    /// The `String` or `null`.
    pub fn as_opt_str(&self) -> Option<&str> {
        match self {
            Value::Null => None,
            other => Some(other.as_str()),
        }
    }

    /// The `int`.
    pub fn as_int(&self) -> i32 {
        match self {
            Value::Int(v) => *v,
            other => harness_fail(format!("expected an int, found {other:?}")),
        }
    }

    /// The `long`.
    pub fn as_long(&self) -> i64 {
        match self {
            Value::Long(v) => *v,
            other => harness_fail(format!("expected a long, found {other:?}")),
        }
    }

    /// The `float`.
    pub fn as_float(&self) -> f32 {
        match self {
            Value::Float(v) => *v,
            other => harness_fail(format!("expected a float, found {other:?}")),
        }
    }

    /// The `double`.
    pub fn as_double(&self) -> f64 {
        match self {
            Value::Double(v) => *v,
            other => harness_fail(format!("expected a double, found {other:?}")),
        }
    }

    /// The `boolean`.
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(v) => *v,
            other => harness_fail(format!("expected a boolean, found {other:?}")),
        }
    }

    /// The elements of an array or a list.
    pub fn as_values(&self) -> &[Value] {
        match self {
            Value::Array(_, v) | Value::List(v) => v,
            other => harness_fail(format!("expected an array or list, found {other:?}")),
        }
    }

    /// The `String` elements of an array or a list.
    pub fn as_strings(&self) -> Vec<String> {
        self.as_values()
            .iter()
            .map(|v| v.as_str().to_string())
            .collect()
    }

    /// Whether the value is an object held as a `T`.
    pub fn is<T: Any>(&self) -> bool {
        matches!(self, Value::Object(o) if o.cell.borrow().is::<T>())
    }

    /// A clone of the `T` the object holds.
    pub fn get<T: Any + Clone>(&self) -> T {
        self.with_mut(|v: &mut T| v.clone())
    }

    /// Runs `f` on the `T` the object holds.
    pub fn with_mut<T: Any, R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        match self {
            Value::Object(o) => {
                let mut cell = o.cell.borrow_mut();
                match cell.downcast_mut::<T>() {
                    Some(v) => f(v),
                    None => harness_fail(format!(
                        "the {} object is no {}",
                        o.class,
                        std::any::type_name::<T>()
                    )),
                }
            }
            other => harness_fail(format!(
                "expected an object of {}, found {other:?}",
                std::any::type_name::<T>()
            )),
        }
    }

    /// The query an object holds.
    pub fn as_query(&self) -> Arc<dyn Query> {
        self.get::<Arc<dyn Query>>()
    }

    /// The queries of an array or a list.
    pub fn as_queries(&self) -> Vec<Arc<dyn Query>> {
        self.as_values().iter().map(Value::as_query).collect()
    }
}

/// A harness failure (a missing binding, a malformed expression): it aborts
/// the comparison instead of being dumped as a Java exception.
pub struct HarnessError(pub String);

/// Panics with a [`HarnessError`].
pub fn harness_fail(message: String) -> ! {
    std::panic::panic_any(HarnessError(message))
}

/// A constructor or static method binding.
pub type StaticFn = Box<dyn Fn(&[Value]) -> Result<Value>>;
/// An instance method binding: the receiver, then the arguments.
pub type CallFn = Box<dyn Fn(&Value, &[Value]) -> Result<Value>>;
/// A static field binding.
pub type FieldFn = Box<dyn Fn() -> Value>;

/// The Rust bindings of the Java members expressions name.
#[derive(Default)]
pub struct Registry {
    news: HashMap<(String, usize), StaticFn>,
    statics: HashMap<(String, String, usize), StaticFn>,
    calls: HashMap<(String, String, usize), CallFn>,
    fields: HashMap<(String, String), FieldFn>,
}

impl Registry {
    /// No binding.
    pub fn empty() -> Registry {
        Registry::default()
    }

    /// Binds the constructor of `class` that takes `arity` arguments.
    pub fn new_fn(
        &mut self,
        class: &str,
        arity: usize,
        f: impl Fn(&[Value]) -> Result<Value> + 'static,
    ) -> &mut Registry {
        self.news.insert((class.to_string(), arity), Box::new(f));
        self
    }

    /// Binds the static method `class.method` of `arity` arguments.
    pub fn static_fn(
        &mut self,
        class: &str,
        method: &str,
        arity: usize,
        f: impl Fn(&[Value]) -> Result<Value> + 'static,
    ) -> &mut Registry {
        self.statics
            .insert((class.to_string(), method.to_string(), arity), Box::new(f));
        self
    }

    /// Binds the instance method `method` of `arity` arguments on receivers
    /// of class `class`.
    pub fn call_fn(
        &mut self,
        class: &str,
        method: &str,
        arity: usize,
        f: impl Fn(&Value, &[Value]) -> Result<Value> + 'static,
    ) -> &mut Registry {
        self.calls
            .insert((class.to_string(), method.to_string(), arity), Box::new(f));
        self
    }

    /// Binds the static field `class.name`.
    pub fn field_fn(
        &mut self,
        class: &str,
        name: &str,
        f: impl Fn() -> Value + 'static,
    ) -> &mut Registry {
        self.fields
            .insert((class.to_string(), name.to_string()), Box::new(f));
        self
    }

    /// The bindings of the core classes the harness's own tests use:
    /// `Term`, `BytesRef`, `TermQuery`, `BooleanQuery.Builder`,
    /// `BooleanClause.Occur`, `PhraseQuery` (and its builder), `BoostQuery`,
    /// `ConstantScoreQuery`, `DisjunctionMaxQuery`, `MatchAllDocsQuery`,
    /// `MatchNoDocsQuery`, `BM25Similarity`, `ClassicSimilarity` and
    /// `StandardAnalyzer`.
    pub fn core() -> Registry {
        let mut r = Registry::empty();
        r.new_fn("org.apache.lucene.index.Term", 2, |a| {
            Ok(Value::object(
                "org.apache.lucene.index.Term",
                Term::new(a[0].as_str(), BytesRef::from_char_sequence(a[1].as_str())),
            ))
        });
        r.new_fn("org.apache.lucene.util.BytesRef", 1, |a| {
            Ok(Value::object(
                "org.apache.lucene.util.BytesRef",
                BytesRef::from_char_sequence(a[0].as_str()),
            ))
        });
        r.new_fn("org.apache.lucene.search.TermQuery", 1, |a| {
            Ok(Value::query(Arc::new(TermQuery::new(a[0].get::<Term>()))))
        });
        for occur in [Occur::MUST, Occur::FILTER, Occur::SHOULD, Occur::MUST_NOT] {
            r.field_fn(
                "org.apache.lucene.search.BooleanClause$Occur",
                &format!("{occur:?}"),
                move || Value::object("org.apache.lucene.search.BooleanClause$Occur", occur),
            );
        }
        const BQB: &str = "org.apache.lucene.search.BooleanQuery$Builder";
        r.new_fn(BQB, 0, |_| {
            Ok(Value::object(BQB, BooleanQueryBuilder::new()))
        });
        r.call_fn(BQB, "add", 2, |b, a| {
            let (q, occur) = (a[0].as_query(), a[1].get::<Occur>());
            b.with_mut(|b: &mut BooleanQueryBuilder| b.add(q, occur).map(|_| ()))?;
            Ok(b.clone())
        });
        r.call_fn(BQB, "setMinimumNumberShouldMatch", 1, |b, a| {
            let min = a[0].as_int();
            b.with_mut(|b: &mut BooleanQueryBuilder| {
                b.set_minimum_number_should_match(min);
            });
            Ok(b.clone())
        });
        r.call_fn(BQB, "build", 0, |b, _| {
            Ok(Value::query(Arc::new(
                b.with_mut(|b: &mut BooleanQueryBuilder| b.build()),
            )))
        });
        const PQ: &str = "org.apache.lucene.search.PhraseQuery";
        r.new_fn(PQ, 2, |a| {
            let terms = a[1].as_strings();
            let terms: Vec<&str> = terms.iter().map(String::as_str).collect();
            Ok(Value::query(Arc::new(PhraseQuery::with_slop(
                0,
                a[0].as_str(),
                &terms,
            )?)))
        });
        r.new_fn(PQ, 3, |a| {
            let terms = a[2].as_strings();
            let terms: Vec<&str> = terms.iter().map(String::as_str).collect();
            Ok(Value::query(Arc::new(PhraseQuery::with_slop(
                a[0].as_int(),
                a[1].as_str(),
                &terms,
            )?)))
        });
        const PQB: &str = "org.apache.lucene.search.PhraseQuery$Builder";
        r.new_fn(PQB, 0, |_| {
            Ok(Value::object(PQB, PhraseQueryBuilder::new()))
        });
        r.call_fn(PQB, "add", 1, |b, a| {
            let term = a[0].get::<Term>();
            b.with_mut(|b: &mut PhraseQueryBuilder| b.add(term).map(|_| ()))?;
            Ok(b.clone())
        });
        r.call_fn(PQB, "add", 2, |b, a| {
            let (term, position) = (a[0].get::<Term>(), a[1].as_int());
            b.with_mut(|b: &mut PhraseQueryBuilder| b.add_at(term, position).map(|_| ()))?;
            Ok(b.clone())
        });
        r.call_fn(PQB, "setSlop", 1, |b, a| {
            let slop = a[0].as_int();
            b.with_mut(|b: &mut PhraseQueryBuilder| {
                b.set_slop(slop);
            });
            Ok(b.clone())
        });
        r.call_fn(PQB, "build", 0, |b, _| {
            let q = b.with_mut(|b: &mut PhraseQueryBuilder| b.build())?;
            Ok(Value::query(Arc::new(q)))
        });
        r.new_fn("org.apache.lucene.search.BoostQuery", 2, |a| {
            Ok(Value::query(Arc::new(BoostQuery::new(
                a[0].as_query(),
                a[1].as_float(),
            )?)))
        });
        r.new_fn("org.apache.lucene.search.ConstantScoreQuery", 1, |a| {
            Ok(Value::query(Arc::new(ConstantScoreQuery::new(
                a[0].as_query(),
            ))))
        });
        r.new_fn("org.apache.lucene.search.DisjunctionMaxQuery", 2, |a| {
            Ok(Value::query(Arc::new(DisjunctionMaxQuery::new(
                a[0].as_queries(),
                a[1].as_float(),
            )?)))
        });
        r.field_fn(
            "org.apache.lucene.search.MatchAllDocsQuery",
            "INSTANCE",
            || Value::query(Arc::new(MatchAllDocsQuery::instance())),
        );
        r.new_fn("org.apache.lucene.search.MatchNoDocsQuery", 1, |a| {
            Ok(Value::query(Arc::new(MatchNoDocsQuery::new(a[0].as_str()))))
        });
        const BM25: &str = "org.apache.lucene.search.similarities.BM25Similarity";
        r.new_fn(BM25, 0, |_| {
            Ok(Value::object(
                BM25,
                Arc::new(BM25Similarity::new()) as Arc<dyn Similarity>,
            ))
        });
        r.new_fn(BM25, 2, |a| {
            Ok(Value::object(
                BM25,
                Arc::new(BM25Similarity::with_parameters(
                    a[0].as_float(),
                    a[1].as_float(),
                )?) as Arc<dyn Similarity>,
            ))
        });
        const CLASSIC: &str = "org.apache.lucene.search.similarities.ClassicSimilarity";
        r.new_fn(CLASSIC, 0, |_| {
            Ok(Value::object(
                CLASSIC,
                Arc::new(ClassicSimilarity::new()) as Arc<dyn Similarity>,
            ))
        });
        const STANDARD: &str = "org.apache.lucene.analysis.standard.StandardAnalyzer";
        r.new_fn(STANDARD, 0, |_| {
            Ok(Value::object(
                STANDARD,
                Arc::new(rucene::analysis::standard::StandardAnalyzer::new())
                    as Arc<dyn rucene::analysis::Analyzer>,
            ))
        });
        r
    }

    fn eval(&self, node: &Node, vars: &HashMap<String, Value>) -> Result<Value> {
        let list = match node {
            Node::Str(s) => return Ok(Value::Str(s.clone())),
            Node::Atom(atom) => return Ok(eval_atom(atom, vars)),
            Node::List(list) => list,
        };
        let args = |from: usize| -> Result<Vec<Value>> {
            list[from..].iter().map(|n| self.eval(n, vars)).collect()
        };
        match list[0].atom() {
            "new" => {
                let class = list[1].atom();
                let args = args(2)?;
                let f = self
                    .news
                    .get(&(class.to_string(), args.len()))
                    .unwrap_or_else(|| {
                        harness_fail(format!("no Rust binding for new {class}/{}", args.len()))
                    });
                f(&args)
            }
            "static" => {
                let (class, method) = (list[1].atom(), list[2].atom());
                let args = args(3)?;
                let f = self
                    .statics
                    .get(&(class.to_string(), method.to_string(), args.len()))
                    .unwrap_or_else(|| {
                        harness_fail(format!(
                            "no Rust binding for static {class}.{method}/{}",
                            args.len()
                        ))
                    });
                f(&args)
            }
            "call" => {
                let method = list[1].atom();
                let receiver = self.eval(&list[2], vars)?;
                if let Value::Null = receiver {
                    randomizedtesting::jdk::exception::throw(
                        "java.lang.NullPointerException",
                        format!("Cannot invoke \"{method}()\" because the receiver is null"),
                    );
                }
                let args = args(3)?;
                let class = receiver.class_name();
                let f = self
                    .calls
                    .get(&(class.clone(), method.to_string(), args.len()))
                    .unwrap_or_else(|| {
                        harness_fail(format!(
                            "no Rust binding for call {class}.{method}/{}",
                            args.len()
                        ))
                    });
                f(&receiver, &args)
            }
            "field" => {
                let (class, name) = (list[1].atom(), list[2].atom());
                let f = self
                    .fields
                    .get(&(class.to_string(), name.to_string()))
                    .unwrap_or_else(|| {
                        harness_fail(format!("no Rust binding for field {class}.{name}"))
                    });
                Ok(f())
            }
            "array" => Ok(Value::Array(list[1].atom().to_string(), args(2)?)),
            "list" => Ok(Value::List(args(1)?)),
            "cast" => self.eval(&list[2], vars),
            other => harness_fail(format!("bad form {other}")),
        }
    }

    /// Evaluates the expression `text` with the bindings `vars`.
    ///
    /// # Errors
    ///
    /// The errors of the bound members.
    pub fn eval_text(&self, text: &str, vars: &HashMap<String, Value>) -> Result<Value> {
        self.eval(&parse_expr(text), vars)
    }

    /// `toString()` of a value that is no query: the bound `toString`
    /// method of its class, or the Java string of a literal.
    fn to_java_string(&self, value: &Value) -> Result<String> {
        Ok(match value {
            Value::Null => "null".into(),
            Value::Str(s) => s.clone(),
            Value::Int(v) => v.to_string(),
            Value::Long(v) => v.to_string(),
            Value::Bool(v) => v.to_string(),
            Value::Float(v) => randomizedtesting::jdk::lang::float_to_string(*v),
            Value::Double(v) => randomizedtesting::jdk::lang::double_to_string(*v),
            other => {
                let class = other.class_name();
                let f = self
                    .calls
                    .get(&(class.clone(), "toString".to_string(), 0))
                    .unwrap_or_else(|| {
                        harness_fail(format!("no Rust binding for call {class}.toString/0"))
                    });
                f(other, &[])?.as_str().to_string()
            }
        })
    }
}

fn eval_atom(atom: &str, vars: &HashMap<String, Value>) -> Value {
    if atom == "null" {
        return Value::Null;
    }
    if let Some(name) = atom.strip_prefix('$') {
        return vars
            .get(name)
            .cloned()
            .unwrap_or_else(|| harness_fail(format!("unbound ${name}")));
    }
    fn bad(atom: &str) -> ! {
        harness_fail(format!("bad atom {atom}"))
    }
    if atom.len() < 2 {
        bad(atom);
    }
    let (kind, value) = atom.split_at(2);
    match kind {
        "i:" => Value::Int(value.parse().unwrap_or_else(|_| bad(atom))),
        "l:" => Value::Long(value.parse().unwrap_or_else(|_| bad(atom))),
        "f:" => Value::Float(value.parse().unwrap_or_else(|_| bad(atom))),
        "d:" => Value::Double(value.parse().unwrap_or_else(|_| bad(atom))),
        "z:" => Value::Bool(value.parse().unwrap_or_else(|_| bad(atom))),
        "c:" => Value::Char(u16::from_str_radix(value, 16).unwrap_or_else(|_| bad(atom))),
        _ => bad(atom),
    }
}

/// Runs one stage of a dump: the Java exception it throws, as a
/// `Throwable`, whether it returns a Lucene error or panics with a Java
/// exception; a [`HarnessError`] is re-raised.
fn stage<T>(f: impl FnOnce() -> Result<T>) -> std::result::Result<T, Throwable> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(java_throwable(&e)),
        Err(payload) => {
            if let Some(h) = payload.downcast_ref::<HarnessError>() {
                panic!("query oracle harness: {}", h.0);
            }
            Err(Throwable::from_panic(payload))
        }
    }
}

// ---------------------------------------------------------------------------
// Class tree
// ---------------------------------------------------------------------------

/// One call the visitor recorded, with the calls made on the sub-visitor it
/// returned (`QueryOracle.Node`).
struct TreeNode {
    label: String,
    children: Vec<TreeNode>,
}

impl TreeNode {
    /// The label, then the rendered children in sorted order
    /// (`QueryOracle.Node.render`): `BooleanQuery` and `DisjunctionMaxQuery`
    /// visit hash sets of their clauses, and Java's `BooleanQuery.hashCode`
    /// mixes `Enum.hashCode()` of `BooleanClause.Occur`, an identity hash no
    /// two JVMs share (observed: two runs of the same spec visit nested
    /// clauses in different orders), so that order is not reproducible. The
    /// rendered text is ASCII, so Java's `String` order is byte order.
    fn render(&self, out: &mut String) {
        out.push_str(&self.label);
        if !self.children.is_empty() {
            let mut rendered: Vec<String> = self
                .children
                .iter()
                .map(|c| {
                    let mut b = String::new();
                    c.render(&mut b);
                    b
                })
                .collect();
            rendered.sort();
            out.push('{');
            out.push_str(&rendered.join(" "));
            out.push('}');
        }
    }
}

/// Records every call `Query::visit` makes (`QueryOracle.TreeVisitor`).
struct TreeVisitor<'a> {
    out: &'a mut Vec<TreeNode>,
}

impl QueryVisitor for TreeVisitor<'_> {
    fn consume_terms(&mut self, query: &dyn Query, terms: &[Term]) {
        let terms: Vec<String> = terms.iter().map(|t| escape(&t.to_string())).collect();
        self.out.push(TreeNode {
            label: format!("terms:{}:[{}]", query.java_class_name(), terms.join(",")),
            children: Vec::new(),
        });
    }

    fn consume_terms_matching(
        &mut self,
        query: &dyn Query,
        field: &str,
        _automaton: &dyn Fn() -> ByteRunAutomaton,
    ) {
        self.out.push(TreeNode {
            label: format!("matching:{}:{}", query.java_class_name(), escape(field)),
            children: Vec::new(),
        });
    }

    fn visit_leaf(&mut self, query: &dyn Query) {
        self.out.push(TreeNode {
            label: format!("leaf:{}", query.java_class_name()),
            children: Vec::new(),
        });
    }

    fn accept_field(&self, _field: &str) -> bool {
        true
    }

    fn get_sub_visitor<'b>(
        &'b mut self,
        occur: Occur,
        parent: &dyn Query,
    ) -> Box<dyn QueryVisitor + 'b>
    where
        Self: 'b,
    {
        self.out.push(TreeNode {
            label: format!("sub:{occur:?}:{}", parent.java_class_name()),
            children: Vec::new(),
        });
        let node = self.out.last_mut().expect("just pushed");
        Box::new(TreeVisitor {
            out: &mut node.children,
        })
    }
}

/// The class tree of `query` (`QueryOracle.classTree`): its class, then the
/// calls `Query::visit` makes on a recording visitor, siblings sorted (see
/// `TreeNode::render`).
pub fn class_tree(query: &dyn Query) -> String {
    let mut root = TreeNode {
        label: query.java_class_name().to_string(),
        children: Vec::new(),
    };
    query.visit(&mut TreeVisitor {
        out: &mut root.children,
    });
    let mut out = String::new();
    root.render(&mut out);
    out
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// A directory of its own under `TMPDIR`, removed on drop.
pub struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    /// A new, empty directory named after `prefix`, the process and a
    /// counter.
    pub fn new(prefix: &str) -> WorkDir {
        static N: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = std::env::temp_dir().join(format!(
            "rucene-{prefix}-{}-{}-{nanos}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create the work directory");
        WorkDir { path }
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The words of the standard documents, most frequent first.
pub const VOCABULARY: [&str; 30] = [
    "the", "of", "and", "to", "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
    "hotel", "india", "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec",
    "romeo", "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
];

/// The tags of the standard documents.
pub const TAGS: [&str; 4] = ["red", "green", "blue", "cyan"];

/// An index written by Rucene that both sides read.
pub struct OracleIndex {
    dir: WorkDir,
    max_doc: i32,
}

impl OracleIndex {
    /// The standard index: `num_docs` documents drawn from `seed`, flushed
    /// in three segments (`NoMergePolicy`, a commit after each third), then
    /// about one in twenty deleted by `id` and committed. Each document has
    /// `id` (`StringField`, stored, `d<i>`), `body` (`TextField`, 0 to 39
    /// [`VOCABULARY`] words, low ranks more frequent), `title` (`TextField`,
    /// stored, 1 to 5 words), `tag` (`StringField`, 0 to 3 of [`TAGS`]),
    /// `num` (`IntPoint` and `NumericDocValuesField`, 0 to 99) and `cat`
    /// (`StringField` and `SortedDocValuesField`, `c0` to `c4`).
    ///
    /// # Panics
    ///
    /// When Rucene fails to write the index.
    pub fn standard(seed: i64, num_docs: usize) -> OracleIndex {
        let mut random = JdkRandom::new(seed);
        let docs: Vec<Document> = (0..num_docs)
            .map(|i| standard_doc(&mut random, i))
            .collect();
        let deletes: Vec<usize> = (0..num_docs / 20)
            .map(|_| random.next_int_bound(num_docs as i32) as usize)
            .collect();
        OracleIndex::write(&docs, &deletes)
    }

    /// An index of `docs`, flushed in three segments, with the documents
    /// whose positions `deletes` lists deleted by their `id` field.
    ///
    /// # Panics
    ///
    /// When Rucene fails to write the index.
    pub fn write(docs: &[Document], deletes: &[usize]) -> OracleIndex {
        pin_hash_seed();
        let _ = register_codec("Lucene104", Lucene104Codec::new());
        let work = WorkDir::new("query-oracle-index");
        let dir: Arc<dyn Directory> =
            Arc::new(FSDirectory::open(work.path()).expect("open the index directory"));
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(NoMergePolicy::instance()))
            .expect("set the merge policy");
        let writer = IndexWriter::new(Arc::clone(&dir), config).expect("open the writer");
        let chunk = docs.len().div_ceil(3).max(1);
        for (i, doc) in docs.iter().enumerate() {
            writer.add_document(doc).expect("add a document");
            if (i + 1) % chunk == 0 {
                writer.commit().expect("commit");
            }
        }
        if !deletes.is_empty() {
            let terms = deletes
                .iter()
                .map(|&d| Term::new("id", BytesRef::from_char_sequence(&format!("d{d}"))))
                .collect();
            writer.delete_documents_by_terms(terms).expect("delete");
        }
        writer.commit().expect("commit");
        writer.close().expect("close the writer");
        OracleIndex {
            dir: work,
            max_doc: docs.len() as i32,
        }
    }

    /// The index directory.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// `maxDoc()` of the index.
    pub fn max_doc(&self) -> i32 {
        self.max_doc
    }
}

/// A word of [`VOCABULARY`], low ranks more frequent.
fn word(random: &mut JdkRandom) -> &'static str {
    let r = random.next_double();
    VOCABULARY[((r * r) * VOCABULARY.len() as f64) as usize]
}

fn standard_doc(random: &mut JdkRandom, i: usize) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", format!("d{i}"), Store::YES).expect("id"),
    ));
    let body: Vec<&str> = (0..random.next_int_bound(40))
        .map(|_| word(random))
        .collect();
    doc.add(Box::new(
        TextField::new("body", body.join(" "), Store::NO).expect("body"),
    ));
    let title: Vec<&str> = (0..1 + random.next_int_bound(5))
        .map(|_| word(random))
        .collect();
    doc.add(Box::new(
        TextField::new("title", title.join(" "), Store::YES).expect("title"),
    ));
    for _ in 0..random.next_int_bound(4) {
        let tag = TAGS[random.next_int_bound(TAGS.len() as i32) as usize];
        doc.add(Box::new(
            StringField::new("tag", tag.to_string(), Store::NO).expect("tag"),
        ));
    }
    let num = random.next_int_bound(100);
    doc.add(Box::new(IntPoint::new("num", &[num]).expect("num")));
    doc.add(Box::new(NumericDocValuesField::new("num", i64::from(num))));
    let cat = format!("c{}", random.next_int_bound(5));
    doc.add(Box::new(
        StringField::new("cat", cat.clone(), Store::NO).expect("cat"),
    ));
    doc.add(Box::new(SortedDocValuesField::new(
        "cat",
        BytesRef::from_char_sequence(&cat),
    )));
    doc
}

// ---------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------

/// The jars of the class path and the SHA-1 of each in Maven Central's
/// `.sha1` file for 10.5.0.
pub const LUCENE_JARS: [(&str, &str); 4] = [
    ("lucene-core", "383deeee9b44ef15daaf7541d57ea5183d52a7b5"),
    ("lucene-queries", "c96555ec32a411d860527ea13add388d72a614b2"),
    (
        "lucene-queryparser",
        "dea0c58101825f086127abf5adce13ced0bd0fc2",
    ),
    ("lucene-sandbox", "928527693dc814ba0125659b07316e260713e38f"),
];

/// The Lucene 10.5.0 jars of the local Maven repository, each checked once
/// per process: its SHA-1 must equal both [`LUCENE_JARS`] and the `.sha1`
/// file Maven downloaded beside it.
///
/// # Panics
///
/// When a jar is missing or its digest differs.
pub fn lucene_jars() -> &'static [PathBuf] {
    static JARS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    JARS.get_or_init(|| {
        let home = std::env::var("HOME").expect("HOME is set");
        LUCENE_JARS
            .iter()
            .map(|(artifact, sha1_expected)| {
                let jar = Path::new(&home).join(format!(
                    ".m2/repository/org/apache/lucene/{artifact}/10.5.0/{artifact}-10.5.0.jar"
                ));
                let bytes = std::fs::read(&jar).unwrap_or_else(|e| {
                    panic!("the Lucene 10.5.0 jar is required: {}: {e}", jar.display())
                });
                let digest = sha1(&bytes);
                assert_eq!(&digest, sha1_expected, "SHA-1 of {}", jar.display());
                let sidecar = jar.with_extension("jar.sha1");
                let published = std::fs::read_to_string(&sidecar)
                    .unwrap_or_else(|e| panic!("{}: {e}", sidecar.display()));
                let published = published.split_whitespace().next().unwrap_or("");
                assert_eq!(digest, published, "SHA-1 of {} vs its .sha1", jar.display());
                jar
            })
            .collect()
    })
}

/// SHA-1 (FIPS 180-4) in lower-case hexadecimal.
pub fn sha1(data: &[u8]) -> String {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    let mut message = data.to_vec();
    let bit_length = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

/// The `tests.seed` both sides run under when the environment sets none.
pub const HASH_SEED: &str = "DEADBEEF";

/// Pins the `tests.seed` both sides run under and returns it: the
/// environment's `tests.seed`, else [`HASH_SEED`] (set in the environment,
/// where Rucene reads it). The JVM gets it as `-Dtests.seed`.
///
/// Lucene seeds `StringHelper.GOOD_FAST_HASH_SEED` (the `BytesRef` hash, so
/// the `HashMap` orders of `BooleanQuery`'s clause sets, which
/// `Query.visit` and `toString` walk), `StringHelper.randomId` and
/// `BitUtil.NATIVE_BYTE_ORDER` from that property, and from the clock
/// without it (`StringHelper.java:143,376`, `BitUtil.java:43`); one seed on
/// both sides makes those orders reproducible and equal.
///
/// # Panics
///
/// When `GOOD_FAST_HASH_SEED` was computed before the pin.
pub fn pin_hash_seed() -> String {
    static SEED: OnceLock<String> = OnceLock::new();
    SEED.get_or_init(|| {
        let seed = std::env::var("tests.seed").unwrap_or_else(|_| {
            std::env::set_var("tests.seed", HASH_SEED);
            HASH_SEED.to_string()
        });
        assert_eq!(
            StringHelper::good_fast_hash_seed(),
            java_string_hash_code(&seed),
            "GOOD_FAST_HASH_SEED was computed before the harness pinned tests.seed"
        );
        seed
    })
    .clone()
}

/// `QueryOracle.java`, the Java side of the harness.
pub fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../tests/fixtures/java-codec-harness/src/main/java/org/apache/lucene/rucene/codec/\
         QueryOracle.java",
    )
}

/// Runs `QueryOracle.java` over `spec` in a working directory of its own and
/// returns its dump.
///
/// # Panics
///
/// When the JVM cannot run or fails.
pub fn run_java(spec: &str) -> String {
    let seed = pin_hash_seed();
    let jars = lucene_jars();
    let work = WorkDir::new("query-oracle-jvm");
    let spec_path = work.path().join("spec.tsv");
    std::fs::write(&spec_path, spec).expect("write the spec file");
    let class_path = std::env::join_paths(jars).expect("a class path");
    let output = Command::new("java")
        .current_dir(work.path())
        .arg(format!("-Djava.io.tmpdir={}", work.path().display()))
        .arg("-Xmx1g")
        .arg(format!("-Dtests.seed={seed}"))
        .arg("-cp")
        .arg(class_path)
        .arg(fixture())
        .arg(&spec_path)
        .output()
        .expect("a JDK is required to run the Lucene oracle");
    assert!(
        output.status.success(),
        "QueryOracle.java failed:\n{}",
        java_errors(&String::from_utf8_lossy(&output.stderr))
    );
    String::from_utf8(output.stdout).expect("the oracle prints UTF-8")
}

/// The stderr lines of a Java run without the JVM's start-up warnings.
pub fn java_errors(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| {
            !(l.starts_with("WARNING:")
                || l.contains("VectorizationProvider lookup")
                || l.trim().is_empty())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Dumps and their comparison
// ---------------------------------------------------------------------------

fn line(out: &mut String, fields: &[&str]) {
    out.push_str(&fields.join("\t"));
    out.push('\n');
}

fn error_line(out: &mut String, stage: Option<&str>, t: &Throwable) {
    let message = escape_opt(t.message());
    match stage {
        Some(stage) => line(out, &["error", stage, t.class_name(), &message]),
        None => line(out, &["error", t.class_name(), &message]),
    }
}

fn explanation_value(v: ExplanationValue) -> String {
    match v {
        ExplanationValue::Float(f) => format!("F{:08x}", f.to_bits()),
        ExplanationValue::Double(d) => format!("D{:016x}", d.to_bits()),
        ExplanationValue::Int(i) => format!("I{i}"),
        ExplanationValue::Long(l) => format!("L{l}"),
    }
}

fn explanation(out: &mut String, e: &Explanation, depth: usize) {
    line(
        out,
        &[
            "expl",
            &depth.to_string(),
            &e.is_match().to_string(),
            &explanation_value(e.value()),
            &escape(e.description()),
        ],
    );
    for d in e.details() {
        explanation(out, d, depth + 1);
    }
}

/// One record of a spec, in order.
#[derive(Debug, Clone)]
enum Stmt {
    Let(String, String),
    Do(String),
    Query(String, String),
    Parse(String, String, Vec<u16>, Option<String>),
}

fn write_stmts(out: &mut String, stmts: &[Stmt]) {
    for s in stmts {
        match s {
            Stmt::Let(name, e) => line(out, &["let", name, e]),
            Stmt::Do(e) => line(out, &["do", e]),
            Stmt::Query(id, e) => line(out, &["query", id, e]),
            Stmt::Parse(id, e, input, field) => {
                let input = escape_units(input);
                match field {
                    Some(f) => line(out, &["parse", id, e, &input, &escape(f)]),
                    None => line(out, &["parse", id, e, &input]),
                }
            }
        }
    }
}

/// The records of a dump: each starts at a `query` or `parse` line.
fn records(dump: &str) -> Vec<Vec<&str>> {
    let mut out: Vec<Vec<&str>> = Vec::new();
    for l in dump.lines() {
        if l.starts_with("query\t") || l.starts_with("parse\t") || out.is_empty() {
            out.push(Vec::new());
        }
        out.last_mut().expect("a record").push(l);
    }
    out
}

/// Compares a Java dump with a Rust dump: `Err` with the first differing
/// record (its header), the line of the difference, both lines and the line
/// before.
pub fn compare(java: &str, rust: &str) -> std::result::Result<(), String> {
    let (j, r) = (records(java), records(rust));
    for (jr, rr) in j.iter().zip(&r) {
        let header = jr.first().copied().unwrap_or("");
        for i in 0..jr.len().max(rr.len()) {
            let (jl, rl) = (jr.get(i).copied(), rr.get(i).copied());
            if jl != rl {
                let before = if i > 0 { jr[i - 1] } else { "" };
                return Err(format!(
                    "record `{header}` differs at line {i}:\n  before {before}\n  Java   {}\n  Rust   {}",
                    jl.unwrap_or("<none>"),
                    rl.unwrap_or("<none>")
                ));
            }
        }
    }
    if j.len() != r.len() {
        return Err(format!(
            "the Java dump has {} records, the Rust dump {}",
            j.len(),
            r.len()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Search parity
// ---------------------------------------------------------------------------

/// Search parity over one [`OracleIndex`]: see the module documentation.
pub struct SearchOracle {
    index: PathBuf,
    similarity: Option<String>,
    stmts: Vec<Stmt>,
    registry: Registry,
}

impl SearchOracle {
    /// An oracle over `index`, whose expressions `registry` binds.
    pub fn new(index: &OracleIndex, registry: Registry) -> SearchOracle {
        pin_hash_seed();
        SearchOracle {
            index: index.path().to_path_buf(),
            similarity: None,
            stmts: Vec::new(),
            registry,
        }
    }

    /// `searcher.setSimilarity(expr)`.
    pub fn similarity(&mut self, expr: &str) -> &mut SearchOracle {
        self.similarity = Some(expr.to_string());
        self
    }

    /// Binds `$name` to `expr`.
    pub fn bind(&mut self, name: &str, expr: &str) -> &mut SearchOracle {
        self.stmts.push(Stmt::Let(name.into(), expr.into()));
        self
    }

    /// A query record: `expr` builds the query.
    pub fn query(&mut self, id: &str, expr: &str) -> &mut SearchOracle {
        self.stmts.push(Stmt::Query(id.into(), expr.into()));
        self
    }

    /// The spec file `QueryOracle.java` reads.
    pub fn spec_file(&self) -> String {
        let mut out = String::from("# rucene query oracle spec file (search)\n");
        line(&mut out, &["index", &escape(&self.index.to_string_lossy())]);
        if let Some(s) = &self.similarity {
            line(&mut out, &["similarity", s]);
        }
        write_stmts(&mut out, &self.stmts);
        out
    }

    /// Lucene's dump.
    pub fn java_dump(&self) -> String {
        run_java(&self.spec_file())
    }

    /// Rucene's dump, built as `QueryOracle.run` builds Lucene's.
    ///
    /// # Panics
    ///
    /// When the index cannot be opened, or on a harness failure.
    pub fn rust_dump(&self) -> String {
        let dir: Arc<dyn Directory> =
            Arc::new(FSDirectory::open(&self.index).expect("open the index directory"));
        let reader: Arc<dyn IndexReader> =
            directory_reader::open(dir).expect("open the index") as Arc<dyn IndexReader>;
        let mut searcher = IndexSearcher::new(Arc::clone(&reader)).expect("a searcher");
        searcher.set_query_cache(None);
        let mut vars: HashMap<String, Value> = HashMap::new();
        if let Some(s) = &self.similarity {
            let sim = self
                .registry
                .eval_text(s, &vars)
                .expect("build the similarity");
            searcher.set_similarity(sim.get::<Arc<dyn Similarity>>());
        }
        let searcher = Arc::new(searcher);
        vars.insert(
            "searcher".into(),
            Value::object(
                "org.apache.lucene.search.IndexSearcher",
                Arc::clone(&searcher),
            ),
        );
        vars.insert(
            "reader".into(),
            Value::object(
                "org.apache.lucene.index.StandardDirectoryReader",
                Arc::clone(&reader),
            ),
        );
        let mut out = String::new();
        for s in &self.stmts {
            match s {
                Stmt::Let(name, e) => {
                    let v = self.registry.eval_text(e, &vars).expect("a binding");
                    vars.insert(name.clone(), v);
                }
                Stmt::Do(e) => {
                    self.registry.eval_text(e, &vars).expect("a statement");
                }
                Stmt::Query(id, e) => self.dump_query(&mut out, &searcher, &*reader, &vars, id, e),
                Stmt::Parse(..) => unreachable!("no parse record in a search oracle"),
            }
        }
        out
    }

    fn dump_query(
        &self,
        out: &mut String,
        searcher: &IndexSearcher,
        reader: &dyn IndexReader,
        vars: &HashMap<String, Value>,
        id: &str,
        e: &str,
    ) {
        line(out, &["query", id]);
        let q = match stage(|| Ok(self.registry.eval_text(e, vars)?.as_query())) {
            Ok(q) => q,
            Err(t) => return error_line(out, Some("build"), &t),
        };
        line(out, &["tostring", &escape(&query_to_string(&*q))]);
        line(out, &["classtree", &escape(&class_tree(&*q))]);
        let rewritten = match stage(|| searcher.rewrite(Arc::clone(&q))) {
            Ok(r) => r,
            Err(t) => return error_line(out, Some("rewrite"), &t),
        };
        line(out, &["rewritten", &escape(&query_to_string(&*rewritten))]);
        line(out, &["rclasstree", &escape(&class_tree(&*rewritten))]);
        match stage(|| searcher.count(Arc::clone(&q))) {
            Ok(c) => line(out, &["count", &c.to_string()]),
            Err(t) => return error_line(out, Some("count"), &t),
        }
        let max_doc = reader.max_doc();
        let td = match stage(|| searcher.search(Arc::clone(&q), max_doc.max(1))) {
            Ok(td) => td,
            Err(t) => return error_line(out, Some("search"), &t),
        };
        line(
            out,
            &[
                "hits",
                &td.total_hits.value().to_string(),
                &format!("{:?}", td.total_hits.relation()),
            ],
        );
        for sd in &td.score_docs {
            line(
                out,
                &[
                    "hit",
                    &sd.doc.to_string(),
                    &format!("{:08x}", sd.score.to_bits()),
                ],
            );
        }
        for doc in 0..max_doc {
            line(out, &["explain", &doc.to_string()]);
            match stage(|| searcher.explain(Arc::clone(&q), doc)) {
                Ok(ex) => explanation(out, &ex, 0),
                Err(t) => return error_line(out, Some("explain"), &t),
            }
        }
    }

    /// Runs both sides and panics with the first difference.
    pub fn assert_matches(&self) {
        let java = self.java_dump();
        let rust = self.rust_dump();
        if let Err(diff) = compare(&java, &rust) {
            panic!("search parity with Lucene 10.5.0 fails:\n{diff}");
        }
    }
}

// ---------------------------------------------------------------------------
// Parser parity
// ---------------------------------------------------------------------------

/// Parser parity: see the module documentation.
pub struct ParseOracle {
    stmts: Vec<Stmt>,
    registry: Registry,
}

impl ParseOracle {
    /// An oracle whose expressions `registry` binds.
    pub fn new(registry: Registry) -> ParseOracle {
        pin_hash_seed();
        ParseOracle {
            stmts: Vec::new(),
            registry,
        }
    }

    /// Binds `$name` to `expr`.
    pub fn bind(&mut self, name: &str, expr: &str) -> &mut ParseOracle {
        self.stmts.push(Stmt::Let(name.into(), expr.into()));
        self
    }

    /// Evaluates `expr` for its effect (a setter call).
    pub fn run(&mut self, expr: &str) -> &mut ParseOracle {
        self.stmts.push(Stmt::Do(expr.into()));
        self
    }

    /// A parse record: `expr` evaluated with `$input` bound to `input`; with
    /// `field`, `toString(field)` is compared too.
    pub fn parse(
        &mut self,
        id: &str,
        expr: &str,
        input: &[u16],
        field: Option<&str>,
    ) -> &mut ParseOracle {
        self.stmts.push(Stmt::Parse(
            id.into(),
            expr.into(),
            input.to_vec(),
            field.map(str::to_string),
        ));
        self
    }

    /// The spec file `QueryOracle.java` reads.
    pub fn spec_file(&self) -> String {
        let mut out = String::from("# rucene query oracle spec file (parse)\n");
        write_stmts(&mut out, &self.stmts);
        out
    }

    /// Lucene's dump.
    pub fn java_dump(&self) -> String {
        run_java(&self.spec_file())
    }

    /// Rucene's dump, built as `QueryOracle.run` builds Lucene's.
    ///
    /// # Panics
    ///
    /// On a harness failure, or when a binding or statement fails.
    pub fn rust_dump(&self) -> String {
        let mut vars: HashMap<String, Value> = HashMap::new();
        let mut out = String::new();
        for s in &self.stmts {
            match s {
                Stmt::Let(name, e) => {
                    let v = self.registry.eval_text(e, &vars).expect("a binding");
                    vars.insert(name.clone(), v);
                }
                Stmt::Do(e) => {
                    self.registry.eval_text(e, &vars).expect("a statement");
                }
                Stmt::Parse(id, e, input, field) => {
                    line(&mut out, &["parse", id]);
                    vars.insert(
                        "input".into(),
                        Value::Str(String::from_utf16(input).expect(
                            "a parse input is valid UTF-16: Rust strings hold no lone surrogate",
                        )),
                    );
                    let result = match stage(|| self.registry.eval_text(e, &vars)) {
                        Ok(v) => v,
                        Err(t) => {
                            error_line(&mut out, None, &t);
                            continue;
                        }
                    };
                    if result.is::<Arc<dyn Query>>() {
                        let q = result.as_query();
                        line(&mut out, &["tostring", &escape(&query_to_string(&*q))]);
                        if let Some(f) = field {
                            line(&mut out, &["tostringf", &escape(&q.to_query_string(f))]);
                        }
                        line(&mut out, &["classtree", &escape(&class_tree(&*q))]);
                    } else {
                        let s = self
                            .registry
                            .to_java_string(&result)
                            .expect("toString() of the result");
                        line(&mut out, &["value", &result.class_name(), &escape(&s)]);
                    }
                }
                Stmt::Query(..) => unreachable!("no query record in a parse oracle"),
            }
        }
        out
    }

    /// Runs both sides and panics with the first difference.
    pub fn assert_matches(&self) {
        let java = self.java_dump();
        let rust = self.rust_dump();
        if let Err(diff) = compare(&java, &rust) {
            panic!("parser parity with Lucene 10.5.0 fails:\n{diff}");
        }
    }
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

/// The seeded generators of query strings.
pub mod corpus {
    use randomizedtesting::jdk::random::{JdkRandom, Random};

    use super::{TAGS, VOCABULARY};

    /// Fixed edge cases of the classic query syntax: empty and blank
    /// strings, operators alone, unbalanced and nested groups, quotes, slop,
    /// boosts, fields, wildcards, prefixes, fuzzy terms, ranges (open,
    /// inclusive, exclusive, dates), regular expressions, escapes, `*:*`,
    /// keywords as terms, non-ASCII text, and malformed inputs.
    pub fn classic_edge_cases() -> Vec<String> {
        [
            "",
            " ",
            "alpha",
            "Alpha Bravo",
            "alpha bravo charlie",
            "+alpha -bravo",
            "alpha AND bravo",
            "alpha OR bravo",
            "alpha NOT bravo",
            "alpha && bravo || charlie",
            "!alpha",
            "NOT alpha",
            "AND",
            "OR",
            "NOT",
            "alpha AND",
            "(alpha bravo)",
            "(alpha (bravo charlie))",
            "((alpha))",
            "(alpha",
            "alpha)",
            "()",
            "\"alpha bravo\"",
            "\"alpha bravo\"~2",
            "\"alpha\"",
            "\"\"",
            "\"alpha bravo",
            "alpha^2",
            "alpha^0.5",
            "(alpha bravo)^3",
            "\"alpha bravo\"^2.5",
            "alpha^",
            "title:alpha",
            "title:(alpha bravo)",
            "title:\"alpha bravo\"",
            "tag:red",
            "title:",
            ":alpha",
            "alp*",
            "a?pha",
            "al*ha",
            "*",
            "*:*",
            "*pha",
            "?lpha",
            "alpha~",
            "alpha~1",
            "alpha~0.8",
            "alpha~3",
            "[alpha TO bravo]",
            "{alpha TO bravo}",
            "[alpha TO bravo}",
            "[* TO bravo]",
            "[alpha TO *]",
            "num:[1 TO 5]",
            "[1/1/2020 TO 12/31/2020]",
            "[alpha TO]",
            "/al.*a/",
            "/[ab]+/",
            "title:/br.vo/",
            "/unterminated",
            "alpha\\:bravo",
            "\\(alpha\\)",
            "alpha\\",
            "\\*",
            "alpha-bravo",
            "alpha_bravo",
            "1.5",
            "-1",
            "+-alpha",
            "alpha:bravo:charlie",
            "café naïve",
            "日本語",
            "ΟΔΟΣ",
            "İstanbul",
            "a\u{1F600}b",
            "alpha\tbravo\ncharlie",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    /// `n` query strings drawn from `seed`: clauses of words, fields,
    /// operators, groups, phrases, boosts, wildcards, fuzzy terms, ranges and
    /// stray syntax characters.
    pub fn classic_random(seed: i64, n: usize) -> Vec<String> {
        let mut random = JdkRandom::new(seed);
        (0..n).map(|_| random_query(&mut random, 0)).collect()
    }

    fn pick<'a>(random: &mut JdkRandom, items: &[&'a str]) -> &'a str {
        items[random.next_int_bound(items.len() as i32) as usize]
    }

    fn random_term(random: &mut JdkRandom) -> String {
        let w = pick(random, &VOCABULARY);
        match random.next_int_bound(12) {
            0 => format!("{}*", &w[..1.max(w.len() / 2)]),
            1 => format!("{w}~"),
            2 => format!("{w}~{}", random.next_int_bound(3)),
            3 => w.replacen(&w[1..2], "?", 1),
            4 => w.to_uppercase(),
            5 => format!("{w}^{}", 1 + random.next_int_bound(4)),
            6 => format!("{w}^0.{}", random.next_int_bound(10)),
            _ => w.to_string(),
        }
    }

    fn random_query(random: &mut JdkRandom, depth: u32) -> String {
        let clauses = 1 + random.next_int_bound(4);
        let mut parts = Vec::new();
        for i in 0..clauses {
            if i > 0 {
                parts.push(
                    pick(
                        random,
                        &[" ", " ", " AND ", " OR ", " && ", " || ", " NOT "],
                    )
                    .to_string(),
                );
            }
            let prefix = pick(random, &["", "", "", "+", "-", "!"]);
            let field = pick(random, &["", "", "", "title:", "body:", "tag:"]);
            let clause = match random.next_int_bound(10) {
                0 if depth < 2 => format!("({})", random_query(random, depth + 1)),
                1 => {
                    let words: Vec<&str> = (0..1 + random.next_int_bound(3))
                        .map(|_| pick(random, &VOCABULARY))
                        .collect();
                    let slop = if random.next_boolean() {
                        format!("~{}", random.next_int_bound(4))
                    } else {
                        String::new()
                    };
                    format!("\"{}\"{slop}", words.join(" "))
                }
                2 => {
                    let (a, b) = (pick(random, &VOCABULARY), pick(random, &VOCABULARY));
                    let open = pick(random, &["[", "{"]);
                    let close = pick(random, &["]", "}"]);
                    format!("{open}{a} TO {b}{close}")
                }
                3 => pick(random, &TAGS).to_string(),
                4 => pick(
                    random,
                    &["(", ")", "\"", "^", "~", ":", "*", "?", "\\", "/", "[", "]"],
                )
                .to_string(),
                _ => random_term(random),
            };
            parts.push(format!("{prefix}{field}{clause}"));
        }
        parts.concat()
    }
}
