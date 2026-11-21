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

//! The bindings of the `queries.intervals` classes for the Java-oracle
//! harness (`query_oracle`, rmp #482): the [`registry`] Rucene evaluates the
//! expressions with, Rucene's side of the harness's `intervals` records
//! ([`intervals_dumper`]), and the expression builders both sides read.
//!
//! An `intervals` record dumps, per leaf, every document the source's
//! `IntervalIterator` visits with every interval (start, end, gaps, width),
//! then the `matches` of every document of the leaf: positions, offsets,
//! gaps and width of each top-level match and the sub-matches, depth first.
//! `MatchesIterator.getQuery()` is not dumped: several interval iterators
//! return `null` there, which the port's infallible core signature cannot
//! (the declared divergence of the package).

#![allow(dead_code)]

use std::sync::Arc;

use rucene::analysis::Analyzer;
use rucene::index::IndexReader;
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::matches::MatchesIterator;
use rucene::util::automaton::{CompiledAutomaton, Operations, RegExp};
use rucene::util::BytesRef;
use rucene_queries::intervals::interval_iterator::NO_MORE_INTERVALS;
use rucene_queries::intervals::interval_query::IntervalQuery;
use rucene_queries::intervals::intervals::Intervals;
use rucene_queries::intervals::intervals_source::IntervalsSource;
use rucene_queries::intervals::payload_filtered_term_intervals_source::PayloadFilter;

use crate::query_oracle::{escape, expr, harness_fail, lit, Registry, SpansFn, Value};

/// The class of the interval sources the registry builds.
pub const SOURCE: &str = "org.apache.lucene.queries.intervals.IntervalsSource";
const INTERVALS: &str = "org.apache.lucene.queries.intervals.Intervals";
const QUERY: &str = "org.apache.lucene.queries.intervals.IntervalQuery";
const AUTOMATON: &str = "org.apache.lucene.util.automaton.Automaton";
const REGEXP: &str = "org.apache.lucene.util.automaton.RegExp";
const COMPILED: &str = "org.apache.lucene.util.automaton.CompiledAutomaton";
const PREDICATE: &str = "java.util.function.Predicate";

fn src_value(s: Arc<dyn IntervalsSource>) -> Value {
    Value::object(SOURCE, s)
}

fn src(v: &Value) -> Arc<dyn IntervalsSource> {
    v.get::<Arc<dyn IntervalsSource>>()
}

fn srcs(v: &Value) -> Vec<Arc<dyn IntervalsSource>> {
    v.as_values().iter().map(src).collect()
}

/// The `BytesRef` of a `BytesRef` object or `null`.
fn bytes(v: &Value) -> Option<BytesRef> {
    match v {
        Value::Null => None,
        other => Some(other.get::<BytesRef>()),
    }
}

/// The bindings of `Intervals`, `IntervalQuery` and the automaton classes
/// `Intervals.multiterm` takes, over [`Registry::core`].
pub fn registry() -> Registry {
    let mut r = Registry::core();
    r.static_fn(INTERVALS, "term", 1, |a| {
        Ok(src_value(match &a[0] {
            Value::Str(s) => Intervals::term_str(s),
            other => Intervals::term(other.get::<BytesRef>()),
        }))
    });
    r.static_fn(INTERVALS, "term", 2, |a| {
        let filter = a[1].get::<PayloadFilter>();
        Ok(src_value(match &a[0] {
            Value::Str(s) => Intervals::term_str_with_payload_filter(s, filter),
            other => Intervals::term_with_payload_filter(other.get::<BytesRef>(), filter),
        }))
    });
    r.static_fn(INTERVALS, "phrase", 1, |a| {
        Ok(src_value(match &a[0] {
            Value::Array(c, _) if c == "java.lang.String" => {
                let terms = a[0].as_strings();
                let terms: Vec<&str> = terms.iter().map(String::as_str).collect();
                Intervals::phrase_terms(&terms)?
            }
            other => Intervals::phrase(srcs(other))?,
        }))
    });
    r.static_fn(INTERVALS, "or", 1, |a| {
        Ok(src_value(Intervals::or(srcs(&a[0]))?))
    });
    r.static_fn(INTERVALS, "or", 2, |a| {
        Ok(src_value(Intervals::or_with_rewrite(
            a[0].as_bool(),
            srcs(&a[1]),
        )?))
    });
    let one_bytes = |f: fn(&BytesRef) -> rucene::error::Result<Arc<dyn IntervalsSource>>| {
        move |a: &[Value]| Ok(src_value(f(&a[0].get::<BytesRef>())?))
    };
    let two_bytes = |f: fn(&BytesRef, i32) -> rucene::error::Result<Arc<dyn IntervalsSource>>| {
        move |a: &[Value]| Ok(src_value(f(&a[0].get::<BytesRef>(), a[1].as_int())?))
    };
    r.static_fn(INTERVALS, "prefix", 1, one_bytes(Intervals::prefix));
    r.static_fn(
        INTERVALS,
        "prefix",
        2,
        two_bytes(Intervals::prefix_with_max_expansions),
    );
    r.static_fn(INTERVALS, "wildcard", 1, one_bytes(Intervals::wildcard));
    r.static_fn(
        INTERVALS,
        "wildcard",
        2,
        two_bytes(Intervals::wildcard_with_max_expansions),
    );
    r.static_fn(INTERVALS, "regexp", 1, one_bytes(Intervals::regexp));
    r.static_fn(
        INTERVALS,
        "regexp",
        2,
        two_bytes(Intervals::regexp_with_max_expansions),
    );
    r.static_fn(INTERVALS, "range", 4, |a| {
        let (lower, upper) = (bytes(&a[0]), bytes(&a[1]));
        Ok(src_value(Intervals::range(
            lower.as_ref(),
            upper.as_ref(),
            a[2].as_bool(),
            a[3].as_bool(),
        )?))
    });
    r.static_fn(INTERVALS, "range", 5, |a| {
        let (lower, upper) = (bytes(&a[0]), bytes(&a[1]));
        Ok(src_value(Intervals::range_with_max_expansions(
            lower.as_ref(),
            upper.as_ref(),
            a[2].as_bool(),
            a[3].as_bool(),
            a[4].as_int(),
        )?))
    });
    r.static_fn(INTERVALS, "fuzzyTerm", 2, |a| {
        Ok(src_value(Intervals::fuzzy_term(
            a[0].as_str(),
            a[1].as_int(),
        )?))
    });
    r.static_fn(INTERVALS, "fuzzyTerm", 5, |a| {
        Ok(src_value(Intervals::fuzzy_term_with(
            a[0].as_str(),
            a[1].as_int(),
            a[2].as_int(),
            a[3].as_bool(),
            a[4].as_int(),
        )?))
    });
    r.static_fn(INTERVALS, "multiterm", 2, |a| {
        Ok(src_value(Intervals::multiterm(
            a[0].get::<CompiledAutomaton>(),
            a[1].as_str(),
        )?))
    });
    r.static_fn(INTERVALS, "multiterm", 3, |a| {
        Ok(src_value(Intervals::multiterm_with_max_expansions(
            a[0].get::<CompiledAutomaton>(),
            a[1].as_int(),
            a[2].as_str(),
        )?))
    });
    r.static_fn(INTERVALS, "maxwidth", 2, |a| {
        Ok(src_value(Intervals::maxwidth(a[0].as_int(), src(&a[1]))))
    });
    r.static_fn(INTERVALS, "maxgaps", 2, |a| {
        Ok(src_value(Intervals::maxgaps(a[0].as_int(), src(&a[1]))?))
    });
    r.static_fn(INTERVALS, "extend", 3, |a| {
        Ok(src_value(Intervals::extend(
            src(&a[0]),
            a[1].as_int(),
            a[2].as_int(),
        )))
    });
    r.static_fn(INTERVALS, "ordered", 1, |a| {
        Ok(src_value(Intervals::ordered(srcs(&a[0]))))
    });
    r.static_fn(INTERVALS, "unordered", 1, |a| {
        Ok(src_value(Intervals::unordered(srcs(&a[0]))))
    });
    type Pair = fn(
        Arc<dyn IntervalsSource>,
        Arc<dyn IntervalsSource>,
    ) -> rucene::error::Result<Arc<dyn IntervalsSource>>;
    let pair = |f: Pair| move |a: &[Value]| Ok(src_value(f(src(&a[0]), src(&a[1]))?));
    r.static_fn(
        INTERVALS,
        "unorderedNoOverlaps",
        2,
        pair(Intervals::unordered_no_overlaps),
    );
    r.static_fn(
        INTERVALS,
        "notContaining",
        2,
        pair(Intervals::not_containing),
    );
    r.static_fn(INTERVALS, "containing", 2, pair(Intervals::containing));
    r.static_fn(
        INTERVALS,
        "notContainedBy",
        2,
        pair(Intervals::not_contained_by),
    );
    r.static_fn(INTERVALS, "containedBy", 2, pair(Intervals::contained_by));
    r.static_fn(INTERVALS, "before", 2, pair(Intervals::before));
    r.static_fn(INTERVALS, "after", 2, pair(Intervals::after));
    r.static_fn(INTERVALS, "nonOverlapping", 2, |a| {
        Ok(src_value(Intervals::non_overlapping(
            src(&a[0]),
            src(&a[1]),
        )))
    });
    r.static_fn(INTERVALS, "overlapping", 2, |a| {
        Ok(src_value(Intervals::overlapping(src(&a[0]), src(&a[1]))))
    });
    r.static_fn(INTERVALS, "fixField", 2, |a| {
        Ok(src_value(Intervals::fix_field(a[0].as_str(), src(&a[1]))))
    });
    r.static_fn(INTERVALS, "notWithin", 3, |a| {
        Ok(src_value(Intervals::not_within(
            src(&a[0]),
            a[1].as_int(),
            src(&a[2]),
        )))
    });
    r.static_fn(INTERVALS, "within", 3, |a| {
        Ok(src_value(Intervals::within(
            src(&a[0]),
            a[1].as_int(),
            src(&a[2]),
        )?))
    });
    r.static_fn(INTERVALS, "atLeast", 2, |a| {
        Ok(src_value(Intervals::at_least(a[0].as_int(), srcs(&a[1]))))
    });
    r.static_fn(INTERVALS, "noIntervals", 1, |a| {
        Ok(src_value(Intervals::no_intervals(a[0].as_str())))
    });
    r.static_fn(INTERVALS, "analyzedText", 5, |a| {
        let analyzer = a[1].get::<Arc<dyn Analyzer>>();
        Ok(src_value(Intervals::analyzed_text(
            a[0].as_str(),
            &*analyzer,
            a[2].as_str(),
            a[3].as_int(),
            a[4].as_bool(),
        )?))
    });
    // `QueryOracle.nullPayload()`: a payload filter accepting a `null` payload
    r.static_fn(
        "org.apache.lucene.rucene.codec.QueryOracle",
        "nullPayload",
        0,
        |_| {
            let filter: PayloadFilter = Arc::new(|p: Option<&[u8]>| p.is_none());
            Ok(Value::object(PREDICATE, filter))
        },
    );
    r.new_fn(QUERY, 2, |a| {
        Ok(Value::query(Arc::new(IntervalQuery::new(
            a[0].as_str(),
            src(&a[1]),
        ))))
    });
    r.new_fn(QUERY, 3, |a| {
        Ok(Value::query(Arc::new(IntervalQuery::with_pivot(
            a[0].as_str(),
            src(&a[1]),
            a[2].as_float(),
        )?)))
    });
    r.new_fn(QUERY, 4, |a| {
        Ok(Value::query(Arc::new(IntervalQuery::with_pivot_and_exp(
            a[0].as_str(),
            src(&a[1]),
            a[2].as_float(),
            a[3].as_float(),
        )?)))
    });
    r.new_fn(REGEXP, 1, |a| {
        Ok(Value::object(REGEXP, RegExp::new(a[0].as_str())?))
    });
    r.call_fn(REGEXP, "toAutomaton", 0, |re, _| {
        let automaton = re.with_mut(|re: &mut RegExp| re.to_automaton())?;
        Ok(Value::object(AUTOMATON, automaton))
    });
    r.call_fn(REGEXP, "toString", 0, |re, _| {
        Ok(Value::Str(re.with_mut(|re: &mut RegExp| re.to_string())))
    });
    r.static_fn(
        "org.apache.lucene.util.automaton.Operations",
        "determinize",
        2,
        |a| {
            let automaton = a[0].with_mut(|x: &mut rucene::util::automaton::Automaton| {
                Operations::determinize(x, a[1].as_int())
            });
            Ok(Value::object(
                AUTOMATON,
                automaton.map_err(rucene::error::LuceneError::from)?,
            ))
        },
    );
    r.new_fn(COMPILED, 1, |a| {
        let automaton = a[0].get::<rucene::util::automaton::Automaton>();
        Ok(Value::object(
            COMPILED,
            CompiledAutomaton::from_automaton(automaton)?,
        ))
    });
    r
}

/// Rucene's side of the `intervals` records: see the module documentation
/// and `QueryOracle.intervals`.
pub fn intervals_dumper() -> SpansFn {
    Box::new(|v, searcher, field, out| {
        let source = src(v);
        out.push(format!("tostring\t{}", escape(&source.to_source_string())));
        out.push(format!("minextent\t{}", source.min_extent()));
        let reader = Arc::clone(searcher.get_index_reader());
        for ctx in reader.leaves()? {
            match source.intervals(field, &ctx)? {
                None => out.push(format!("leaf\t{}\tnull", ctx.ord())),
                Some(mut it) => {
                    out.push(format!("leaf\t{}\t{}", ctx.ord(), ctx.doc_base()));
                    loop {
                        let doc = it.next_doc()?;
                        if doc == NO_MORE_DOCS {
                            break;
                        }
                        out.push(format!("idoc\t{doc}\t{}\t{}", it.start(), it.end()));
                        loop {
                            let pos = it.next_interval()?;
                            if pos == NO_MORE_INTERVALS {
                                break;
                            }
                            out.push(format!(
                                "iv\t{pos}\t{}\t{}\t{}\t{}",
                                it.start(),
                                it.end(),
                                it.gaps(),
                                it.width()
                            ));
                        }
                        out.push(format!("ivend\t{}\t{}", it.start(), it.end()));
                    }
                }
            }
            for doc in 0..ctx.leaf_reader().max_doc() {
                let Some(mut mi) = source.matches(field, &ctx, doc)? else {
                    continue;
                };
                out.push(format!("mdoc\t{doc}"));
                while mi.next()? {
                    out.push(format!(
                        "m\t0\t{}\t{}\t{}\t{}\t{}\t{}",
                        mi.start_position(),
                        mi.end_position(),
                        mi.start_offset()?,
                        mi.end_offset()?,
                        mi.gaps(),
                        mi.width()
                    ));
                    sub_matches(mi.get_sub_matches()?, 1, out)?;
                }
            }
        }
        Ok(())
    })
}

fn sub_matches(
    mi: Option<Box<dyn MatchesIterator>>,
    depth: usize,
    out: &mut Vec<String>,
) -> rucene::error::Result<()> {
    let Some(mut mi) = mi else {
        return Ok(());
    };
    while mi.next()? {
        out.push(format!(
            "m\t{depth}\t{}\t{}\t{}\t{}",
            mi.start_position(),
            mi.end_position(),
            mi.start_offset()?,
            mi.end_offset()?
        ));
        sub_matches(mi.get_sub_matches()?, depth + 1, out)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Expression builders
// ---------------------------------------------------------------------------

/// `Intervals.term(text)`.
pub fn term(text: &str) -> String {
    expr("static", &[INTERVALS, "term", &lit(text)])
}

/// `Intervals.term(new BytesRef(text))`.
pub fn term_bytes(text: &str) -> String {
    expr("static", &[INTERVALS, "term", &bref(text)])
}

/// `Intervals.term(text, QueryOracle.nullPayload())`.
pub fn term_payload(text: &str) -> String {
    let filter = expr(
        "static",
        &["org.apache.lucene.rucene.codec.QueryOracle", "nullPayload"],
    );
    expr("static", &[INTERVALS, "term", &lit(text), &filter])
}

/// `new BytesRef(text)`.
pub fn bref(text: &str) -> String {
    expr("new", &["org.apache.lucene.util.BytesRef", &lit(text)])
}

fn array(sources: &[String]) -> String {
    let mut args: Vec<&str> = vec![SOURCE];
    args.extend(sources.iter().map(String::as_str));
    expr("array", &args)
}

/// A static factory of `Intervals` over an array of sources.
pub fn vararg(method: &str, sources: &[String]) -> String {
    expr("static", &[INTERVALS, method, &array(sources)])
}

/// `Intervals.phrase(String...)`.
pub fn phrase_terms(terms: &[&str]) -> String {
    let lits: Vec<String> = terms.iter().map(|t| lit(t)).collect();
    let mut args: Vec<&str> = vec!["java.lang.String"];
    args.extend(lits.iter().map(String::as_str));
    expr("static", &[INTERVALS, "phrase", &expr("array", &args)])
}

/// `Intervals.or(false, sources...)`.
pub fn or_no_rewrite(sources: &[String]) -> String {
    expr("static", &[INTERVALS, "or", "z:false", &array(sources)])
}

/// `Intervals.atLeast(min, sources...)`.
pub fn at_least(min: i32, sources: &[String]) -> String {
    expr(
        "static",
        &[INTERVALS, "atLeast", &format!("i:{min}"), &array(sources)],
    )
}

/// A static factory of `Intervals` with the given arguments.
pub fn call(method: &str, args: &[&str]) -> String {
    let mut all: Vec<&str> = vec![INTERVALS, method];
    all.extend_from_slice(args);
    expr("static", &all)
}

/// `new IntervalQuery(field, source)`.
pub fn query(field: &str, source: &str) -> String {
    expr("new", &[QUERY, &lit(field), source])
}

/// `new IntervalQuery(field, source, pivot)`.
pub fn query_pivot(field: &str, source: &str, pivot: f32) -> String {
    expr("new", &[QUERY, &lit(field), source, &format!("f:{pivot}")])
}

/// `new IntervalQuery(field, source, pivot, exp)`.
pub fn query_pivot_exp(field: &str, source: &str, pivot: f32, exp: f32) -> String {
    expr(
        "new",
        &[
            QUERY,
            &lit(field),
            source,
            &format!("f:{pivot}"),
            &format!("f:{exp}"),
        ],
    )
}

/// `new CompiledAutomaton(Operations.determinize(new RegExp(re).toAutomaton(), 10000))`.
pub fn compiled(re: &str) -> String {
    let automaton = expr("call", &["toAutomaton", &expr("new", &[REGEXP, &lit(re)])]);
    let det = expr(
        "static",
        &[
            "org.apache.lucene.util.automaton.Operations",
            "determinize",
            &automaton,
            "i:10000",
        ],
    );
    expr("new", &[COMPILED, &det])
}

/// `new RegExp(re).toString()`.
pub fn regexp_string(re: &str) -> String {
    expr("call", &["toString", &expr("new", &[REGEXP, &lit(re)])])
}

/// The reader's `maxDoc()` (silences an unused import on some builds).
pub fn max_doc(reader: &dyn IndexReader) -> i32 {
    reader.max_doc()
}

/// Panics as a harness failure.
pub fn fail(message: &str) -> ! {
    harness_fail(message.to_string())
}
