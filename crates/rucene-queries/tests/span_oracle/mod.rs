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

//! The bindings of the span and payload query classes for the Java-oracle
//! harness (`query_oracle`), shared by the span and payload oracle tests
//! (rmp #480, #481): the [`registry`] Rucene evaluates their expressions
//! with, Rucene's side of the harness's `spans` records ([`spans_dumper`]),
//! and the expression builders both sides read.
//!
//! **Span values.** The harness holds every query as an `Arc<dyn Query>`;
//! Rust cannot cast it back to `dyn SpanQuery`, which the span constructors
//! and the spans dump need. Each span query the [`registry`] builds is
//! therefore also recorded, by the address of its allocation, in a
//! per-thread table ([`span_value`], [`as_span`]); the harness evaluates and
//! dumps on the test's thread.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::{PostingsEnum, Term};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::fuzzy_query::FuzzyQuery;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::multi_term_query::MultiTermQuery;
use rucene::search::prefix_query::PrefixQuery;
use rucene::search::query::{query_to_string, Query};
use rucene::search::regexp_query::RegexpQuery;
use rucene::search::score_mode::ScoreMode;
use rucene::search::wildcard_query::WildcardQuery;
use rucene::util::BytesRef;
use rucene_queries::payloads::average_payload_function::AveragePayloadFunction;
use rucene_queries::payloads::max_payload_function::MaxPayloadFunction;
use rucene_queries::payloads::min_payload_function::MinPayloadFunction;
use rucene_queries::payloads::payload_decoder::{float_decoder, PayloadDecoder};
use rucene_queries::payloads::payload_function::PayloadFunction;
use rucene_queries::payloads::payload_score_query::PayloadScoreQuery;
use rucene_queries::payloads::span_payload_check_query::{
    MatchOperation, PayloadType, SpanPayloadCheckQuery,
};
use rucene_queries::payloads::sum_payload_function::SumPayloadFunction;
use rucene_queries::spans::field_masking_span_query::FieldMaskingSpanQuery;
use rucene_queries::spans::span_collector::SpanCollector;
use rucene_queries::spans::span_containing_query::SpanContainingQuery;
use rucene_queries::spans::span_first_query::SpanFirstQuery;
use rucene_queries::spans::span_multi_term_query_wrapper::SpanMultiTermQueryWrapper;
use rucene_queries::spans::span_near_query::{Builder as SpanNearBuilder, SpanNearQuery};
use rucene_queries::spans::span_not_query::SpanNotQuery;
use rucene_queries::spans::span_or_query::SpanOrQuery;
use rucene_queries::spans::span_position_range_query::SpanPositionRangeQuery;
use rucene_queries::spans::span_query::SpanQuery;
use rucene_queries::spans::span_term_query::SpanTermQuery;
use rucene_queries::spans::span_weight::Postings;
use rucene_queries::spans::span_within_query::SpanWithinQuery;
use rucene_queries::spans::spans::NO_MORE_POSITIONS;

use crate::query_oracle::{escape, expr, harness_fail, lit, Registry, SpansFn, Value};

// ---------------------------------------------------------------------------
// Span values
// ---------------------------------------------------------------------------

thread_local! {
    static SPANS: RefCell<HashMap<usize, Arc<dyn SpanQuery>>> = RefCell::new(HashMap::new());
}

/// The address of the allocation a query lives in, whatever trait object
/// points to it.
fn address<T: ?Sized>(q: &Arc<T>) -> usize {
    Arc::as_ptr(q) as *const () as usize
}

/// A span query as a harness value (of its Java class), recorded so that
/// [`as_span`] finds it again.
pub fn span_value(q: Arc<dyn SpanQuery>) -> Value {
    SPANS.with(|t| t.borrow_mut().insert(address(&q), Arc::clone(&q)));
    let class = q.java_class_name().to_string();
    Value::object(&class, q as Arc<dyn Query>)
}

/// The span query a harness value holds.
pub fn as_span(v: &Value) -> Arc<dyn SpanQuery> {
    let q = v.as_query();
    SPANS
        .with(|t| t.borrow().get(&address(&q)).cloned())
        .unwrap_or_else(|| harness_fail(format!("{v:?} is no span query the registry built")))
}

/// The span queries of an array or list value.
pub fn as_spans(v: &Value) -> Vec<Arc<dyn SpanQuery>> {
    v.as_values().iter().map(as_span).collect()
}

/// The `byte[]` or `String` argument of `new BytesRef(...)`.
fn bytes_ref_of(v: &Value) -> BytesRef {
    match v {
        Value::Array(_, elems) => BytesRef::new(
            elems
                .iter()
                .map(|e| match e {
                    Value::Byte(b) => *b as u8,
                    other => harness_fail(format!("expected a byte, found {other:?}")),
                })
                .collect(),
        ),
        other => BytesRef::from_char_sequence(other.as_str()),
    }
}

/// The `List<BytesRef>` argument of `SpanPayloadCheckQuery`, `null`
/// elements kept.
fn payload_list(v: &Value) -> Vec<Option<BytesRef>> {
    v.as_values()
        .iter()
        .map(|e| match e {
            Value::Null => None,
            other => Some(other.get::<BytesRef>()),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The registry
// ---------------------------------------------------------------------------

const SPANS_PKG: &str = "org.apache.lucene.queries.spans.";
const PAYLOADS_PKG: &str = "org.apache.lucene.queries.payloads.";

fn spans_class(name: &str) -> String {
    format!("{SPANS_PKG}{name}")
}

fn payloads_class(name: &str) -> String {
    format!("{PAYLOADS_PKG}{name}")
}

/// [`Registry::core`] with the classes of `queries.spans` and
/// `queries.payloads` and the multi-term queries the span wrapper takes, and
/// `new BytesRef(byte[])` beside `new BytesRef(CharSequence)`.
pub fn registry() -> Registry {
    let mut r = Registry::core();
    r.new_fn("org.apache.lucene.util.BytesRef", 1, |a| {
        Ok(Value::object(
            "org.apache.lucene.util.BytesRef",
            bytes_ref_of(&a[0]),
        ))
    });

    // Multi-term queries, held as `Arc<dyn MultiTermQuery>`.
    let mtq = |class: &str, q: Arc<dyn MultiTermQuery>| Value::object(class, q);
    r.new_fn("org.apache.lucene.search.WildcardQuery", 1, move |a| {
        Ok(mtq(
            "org.apache.lucene.search.WildcardQuery",
            Arc::new(WildcardQuery::new(a[0].get::<Term>())?),
        ))
    });
    r.new_fn("org.apache.lucene.search.PrefixQuery", 1, move |a| {
        Ok(mtq(
            "org.apache.lucene.search.PrefixQuery",
            Arc::new(PrefixQuery::new(a[0].get::<Term>())?),
        ))
    });
    r.new_fn("org.apache.lucene.search.RegexpQuery", 1, move |a| {
        Ok(mtq(
            "org.apache.lucene.search.RegexpQuery",
            Arc::new(RegexpQuery::new(a[0].get::<Term>())?),
        ))
    });
    r.new_fn("org.apache.lucene.search.FuzzyQuery", 1, move |a| {
        Ok(mtq(
            "org.apache.lucene.search.FuzzyQuery",
            Arc::new(FuzzyQuery::new(a[0].get::<Term>())?),
        ))
    });
    r.new_fn("org.apache.lucene.search.FuzzyQuery", 3, move |a| {
        Ok(mtq(
            "org.apache.lucene.search.FuzzyQuery",
            Arc::new(FuzzyQuery::with_prefix_length(
                a[0].get::<Term>(),
                a[1].as_int(),
                a[2].as_int(),
            )?),
        ))
    });
    r.new_fn("org.apache.lucene.search.FuzzyQuery", 5, move |a| {
        Ok(mtq(
            "org.apache.lucene.search.FuzzyQuery",
            Arc::new(FuzzyQuery::with_expansions(
                a[0].get::<Term>(),
                a[1].as_int(),
                a[2].as_int(),
                a[3].as_int(),
                a[4].as_bool(),
            )?),
        ))
    });

    // queries.spans
    r.new_fn(&spans_class("SpanTermQuery"), 1, |a| {
        Ok(span_value(Arc::new(SpanTermQuery::new(a[0].get::<Term>()))))
    });
    r.new_fn(&spans_class("SpanOrQuery"), 1, |a| {
        Ok(span_value(Arc::new(SpanOrQuery::new(as_spans(&a[0]))?)))
    });
    r.new_fn(&spans_class("SpanNearQuery"), 3, |a| {
        Ok(span_value(Arc::new(SpanNearQuery::new(
            as_spans(&a[0]),
            a[1].as_int(),
            a[2].as_bool(),
        )?)))
    });
    let builder_class = spans_class("SpanNearQuery$Builder");
    {
        let class = builder_class.clone();
        r.static_fn(
            &spans_class("SpanNearQuery"),
            "newOrderedNearQuery",
            1,
            move |a| {
                Ok(Value::object(
                    &class,
                    SpanNearQuery::new_ordered_near_query(a[0].as_str()),
                ))
            },
        );
    }
    {
        let class = builder_class.clone();
        r.static_fn(
            &spans_class("SpanNearQuery"),
            "newUnorderedNearQuery",
            1,
            move |a| {
                Ok(Value::object(
                    &class,
                    SpanNearQuery::new_unordered_near_query(a[0].as_str()),
                ))
            },
        );
    }
    r.call_fn(&builder_class, "addClause", 1, |recv, a| {
        let clause = as_span(&a[0]);
        recv.with_mut(|b: &mut SpanNearBuilder| b.add_clause(clause).map(|_| ()))?;
        Ok(recv.clone())
    });
    r.call_fn(&builder_class, "addGap", 1, |recv, a| {
        let width = a[0].as_int();
        recv.with_mut(|b: &mut SpanNearBuilder| b.add_gap(width).map(|_| ()))?;
        Ok(recv.clone())
    });
    r.call_fn(&builder_class, "setSlop", 1, |recv, a| {
        let slop = a[0].as_int();
        recv.with_mut(|b: &mut SpanNearBuilder| {
            b.set_slop(slop);
        });
        Ok(recv.clone())
    });
    r.call_fn(&builder_class, "build", 0, |recv, _a| {
        let q = recv.with_mut(|b: &mut SpanNearBuilder| b.build())?;
        Ok(span_value(Arc::new(q)))
    });
    r.new_fn(&spans_class("SpanNotQuery"), 2, |a| {
        Ok(span_value(Arc::new(SpanNotQuery::new(
            as_span(&a[0]),
            as_span(&a[1]),
        )?)))
    });
    r.new_fn(&spans_class("SpanNotQuery"), 3, |a| {
        Ok(span_value(Arc::new(SpanNotQuery::with_dist(
            as_span(&a[0]),
            as_span(&a[1]),
            a[2].as_int(),
        )?)))
    });
    r.new_fn(&spans_class("SpanNotQuery"), 4, |a| {
        Ok(span_value(Arc::new(SpanNotQuery::with_pre_post(
            as_span(&a[0]),
            as_span(&a[1]),
            a[2].as_int(),
            a[3].as_int(),
        )?)))
    });
    r.new_fn(&spans_class("SpanFirstQuery"), 2, |a| {
        Ok(span_value(Arc::new(SpanFirstQuery::new(
            as_span(&a[0]),
            a[1].as_int(),
        ))))
    });
    r.new_fn(&spans_class("SpanPositionRangeQuery"), 3, |a| {
        Ok(span_value(Arc::new(SpanPositionRangeQuery::new(
            as_span(&a[0]),
            a[1].as_int(),
            a[2].as_int(),
        ))))
    });
    r.new_fn(&spans_class("SpanContainingQuery"), 2, |a| {
        Ok(span_value(Arc::new(SpanContainingQuery::new(
            as_span(&a[0]),
            as_span(&a[1]),
        )?)))
    });
    r.new_fn(&spans_class("SpanWithinQuery"), 2, |a| {
        Ok(span_value(Arc::new(SpanWithinQuery::new(
            as_span(&a[0]),
            as_span(&a[1]),
        )?)))
    });
    r.new_fn(&spans_class("FieldMaskingSpanQuery"), 2, |a| {
        Ok(span_value(Arc::new(FieldMaskingSpanQuery::new(
            as_span(&a[0]),
            a[1].as_str(),
        ))))
    });
    r.new_fn(&spans_class("SpanMultiTermQueryWrapper"), 1, |a| {
        Ok(span_value(Arc::new(SpanMultiTermQueryWrapper::new(
            a[0].get::<Arc<dyn MultiTermQuery>>(),
        ))))
    });

    // queries.payloads
    let function = |class: String, f: fn() -> Arc<dyn PayloadFunction>| {
        move |_a: &[Value]| Ok(Value::object(&class, f()))
    };
    r.new_fn(
        &payloads_class("AveragePayloadFunction"),
        0,
        function(payloads_class("AveragePayloadFunction"), || {
            Arc::new(AveragePayloadFunction::new())
        }),
    );
    r.new_fn(
        &payloads_class("MaxPayloadFunction"),
        0,
        function(payloads_class("MaxPayloadFunction"), || {
            Arc::new(MaxPayloadFunction::new())
        }),
    );
    r.new_fn(
        &payloads_class("MinPayloadFunction"),
        0,
        function(payloads_class("MinPayloadFunction"), || {
            Arc::new(MinPayloadFunction::new())
        }),
    );
    r.new_fn(
        &payloads_class("SumPayloadFunction"),
        0,
        function(payloads_class("SumPayloadFunction"), || {
            Arc::new(SumPayloadFunction::new())
        }),
    );
    r.field_fn(&payloads_class("PayloadDecoder"), "FLOAT_DECODER", || {
        Value::object(
            "org.apache.lucene.queries.payloads.PayloadDecoder$$Lambda",
            float_decoder(),
        )
    });
    r.new_fn(&payloads_class("PayloadScoreQuery"), 3, |a| {
        Ok(span_value(Arc::new(PayloadScoreQuery::with_span_score(
            as_span(&a[0]),
            a[1].get::<Arc<dyn PayloadFunction>>(),
            a[2].get::<Arc<dyn PayloadDecoder>>(),
        ))))
    });
    r.new_fn(&payloads_class("PayloadScoreQuery"), 4, |a| {
        Ok(span_value(Arc::new(PayloadScoreQuery::new(
            as_span(&a[0]),
            a[1].get::<Arc<dyn PayloadFunction>>(),
            a[2].get::<Arc<dyn PayloadDecoder>>(),
            a[3].as_bool(),
        ))))
    });
    let type_class = payloads_class("SpanPayloadCheckQuery$PayloadType");
    for (name, t) in [
        ("INT", PayloadType::Int),
        ("FLOAT", PayloadType::Float),
        ("STRING", PayloadType::String),
    ] {
        let class = type_class.clone();
        r.field_fn(&type_class, name, move || Value::object(&class, t));
    }
    let op_class = payloads_class("SpanPayloadCheckQuery$MatchOperation");
    for (name, op) in [
        ("EQ", MatchOperation::Eq),
        ("GT", MatchOperation::Gt),
        ("GTE", MatchOperation::Gte),
        ("LT", MatchOperation::Lt),
        ("LTE", MatchOperation::Lte),
    ] {
        let class = op_class.clone();
        r.field_fn(&op_class, name, move || Value::object(&class, op));
    }
    r.new_fn(&payloads_class("SpanPayloadCheckQuery"), 2, |a| {
        Ok(span_value(Arc::new(SpanPayloadCheckQuery::new(
            as_span(&a[0]),
            payload_list(&a[1]),
        ))))
    });
    r.new_fn(&payloads_class("SpanPayloadCheckQuery"), 4, |a| {
        Ok(span_value(Arc::new(
            SpanPayloadCheckQuery::with_type_and_operation(
                as_span(&a[0]),
                payload_list(&a[1]),
                a[2].get::<PayloadType>(),
                a[3].get::<MatchOperation>(),
            ),
        )))
    });
    r
}

// ---------------------------------------------------------------------------
// The spans dump
// ---------------------------------------------------------------------------

/// The payload bytes in lower-case hexadecimal, `null` for none
/// (`QueryOracle.hex`).
fn hex(payload: Option<&[u8]>) -> String {
    match payload {
        None => "null".to_string(),
        Some(bytes) => bytes.iter().map(|b| format!("{b:02x}")).collect(),
    }
}

/// The anonymous `SpanCollector` of `QueryOracle.spans`: one `coll` line per
/// `collectLeaf` call.
struct LineCollector {
    lines: Vec<String>,
}

impl SpanCollector for LineCollector {
    fn collect_leaf(
        &mut self,
        postings: &dyn PostingsEnum,
        position: i32,
        term: &Term,
    ) -> Result<()> {
        self.lines.push(format!(
            "coll\t{}\t{position}\t{}",
            escape(&term.to_string()),
            hex(postings.get_payload()?)
        ));
        Ok(())
    }

    fn reset(&mut self) {}
}

/// `searcher.rewrite(q)` over the span query type: Java's loop
/// `query.rewrite(this)` until it returns its argument, each step a
/// `SpanQuery` (`rewrite_span` returns `None` for "itself").
fn rewrite_span_fully(
    q: Arc<dyn SpanQuery>,
    searcher: &IndexSearcher,
) -> Result<Arc<dyn SpanQuery>> {
    let mut q = q;
    while let Some(rewritten) = q.rewrite_span(searcher)? {
        q = rewritten;
    }
    Ok(q)
}

/// Rucene's side of the `spans` records, as `QueryOracle.spans` prints
/// Lucene's: `rewritten` (from `searcher.rewrite`), then per leaf of the
/// searcher the spans of the rewritten query's
/// `createWeight(searcher, COMPLETE_NO_SCORES, 1f)`.
pub fn spans_dumper() -> SpansFn {
    Box::new(
        |v: &Value, searcher: &IndexSearcher, postings: &str, lines: &mut Vec<String>| {
            let rewritten = searcher.rewrite(v.as_query())?;
            lines.push(format!(
                "rewritten\t{}",
                escape(&query_to_string(&*rewritten))
            ));
            let sq = rewrite_span_fully(as_span(v), searcher)?;
            let weight = sq.create_span_weight(searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
            let required = match postings {
                "POSITIONS" => Postings::Positions,
                "PAYLOADS" => Postings::Payloads,
                "OFFSETS" => Postings::Offsets,
                other => harness_fail(format!("no SpanWeight.Postings constant {other}")),
            };
            for ctx in searcher.get_leaf_contexts() {
                let Some(mut s) = weight.get_spans(ctx, required)? else {
                    lines.push(format!("leaf\t{}\tnull", ctx.ord()));
                    continue;
                };
                lines.push(format!("leaf\t{}\t{}", ctx.ord(), ctx.doc_base()));
                while s.next_doc()? != NO_MORE_DOCS {
                    lines.push(format!("sdoc\t{}", s.doc_id()));
                    while s.next_start_position()? != NO_MORE_POSITIONS {
                        lines.push(format!(
                            "span\t{}\t{}\t{}",
                            s.start_position(),
                            s.end_position(),
                            s.width()
                        ));
                        let mut collector = LineCollector { lines: Vec::new() };
                        s.collect(&mut collector)?;
                        lines.extend(collector.lines);
                    }
                }
            }
            Ok(())
        },
    )
}

// ---------------------------------------------------------------------------
// Expression builders
// ---------------------------------------------------------------------------

/// `new Term(field, text)`.
pub fn term(field: &str, text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.index.Term", &lit(field), &lit(text)],
    )
}

/// `new SpanTermQuery(new Term(field, text))`.
pub fn st(field: &str, text: &str) -> String {
    expr("new", &[&spans_class("SpanTermQuery"), &term(field, text)])
}

/// `(array SpanQuery ...)`.
fn span_array(clauses: &[String]) -> String {
    let class = spans_class("SpanQuery");
    let mut args = vec![class.as_str()];
    args.extend(clauses.iter().map(String::as_str));
    expr("array", &args)
}

/// `new SpanOrQuery(clauses...)`.
pub fn sor(clauses: &[String]) -> String {
    expr("new", &[&spans_class("SpanOrQuery"), &span_array(clauses)])
}

/// `new SpanNearQuery(clauses, slop, inOrder)`.
pub fn snear(clauses: &[String], slop: i32, in_order: bool) -> String {
    expr(
        "new",
        &[
            &spans_class("SpanNearQuery"),
            &span_array(clauses),
            &format!("i:{slop}"),
            &format!("z:{in_order}"),
        ],
    )
}

/// A step of a `SpanNearQuery.Builder` chain.
pub enum NearStep {
    /// `addClause(q)`.
    Clause(String),
    /// `addGap(width)`.
    Gap(i32),
    /// `setSlop(slop)`.
    Slop(i32),
}

/// `SpanNearQuery.new(Un)orderedNearQuery(field)`, the steps, `build()`.
pub fn snear_builder(field: &str, ordered: bool, steps: &[NearStep]) -> String {
    let factory = if ordered {
        "newOrderedNearQuery"
    } else {
        "newUnorderedNearQuery"
    };
    let mut b = expr(
        "static",
        &[&spans_class("SpanNearQuery"), factory, &lit(field)],
    );
    for step in steps {
        b = match step {
            NearStep::Clause(q) => expr("call", &["addClause", &b, q]),
            NearStep::Gap(w) => expr("call", &["addGap", &b, &format!("i:{w}")]),
            NearStep::Slop(s) => expr("call", &["setSlop", &b, &format!("i:{s}")]),
        };
    }
    expr("call", &["build", &b])
}

/// `new SpanNotQuery(include, exclude)`.
pub fn snot(include: &str, exclude: &str) -> String {
    expr("new", &[&spans_class("SpanNotQuery"), include, exclude])
}

/// `new SpanNotQuery(include, exclude, dist)`.
pub fn snot_dist(include: &str, exclude: &str, dist: i32) -> String {
    expr(
        "new",
        &[
            &spans_class("SpanNotQuery"),
            include,
            exclude,
            &format!("i:{dist}"),
        ],
    )
}

/// `new SpanNotQuery(include, exclude, pre, post)`.
pub fn snot_pre_post(include: &str, exclude: &str, pre: i32, post: i32) -> String {
    expr(
        "new",
        &[
            &spans_class("SpanNotQuery"),
            include,
            exclude,
            &format!("i:{pre}"),
            &format!("i:{post}"),
        ],
    )
}

/// `new SpanFirstQuery(match, end)`.
pub fn sfirst(m: &str, end: i32) -> String {
    expr(
        "new",
        &[&spans_class("SpanFirstQuery"), m, &format!("i:{end}")],
    )
}

/// `new SpanPositionRangeQuery(match, start, end)`.
pub fn srange(m: &str, start: i32, end: i32) -> String {
    expr(
        "new",
        &[
            &spans_class("SpanPositionRangeQuery"),
            m,
            &format!("i:{start}"),
            &format!("i:{end}"),
        ],
    )
}

/// `new SpanContainingQuery(big, little)`.
pub fn scontaining(big: &str, little: &str) -> String {
    expr("new", &[&spans_class("SpanContainingQuery"), big, little])
}

/// `new SpanWithinQuery(big, little)`.
pub fn swithin(big: &str, little: &str) -> String {
    expr("new", &[&spans_class("SpanWithinQuery"), big, little])
}

/// `new FieldMaskingSpanQuery(query, field)`.
pub fn smask(q: &str, field: &str) -> String {
    expr(
        "new",
        &[&spans_class("FieldMaskingSpanQuery"), q, &lit(field)],
    )
}

/// `new SpanMultiTermQueryWrapper<>(query)`.
pub fn smtq(mtq: &str) -> String {
    expr("new", &[&spans_class("SpanMultiTermQueryWrapper"), mtq])
}

/// `new WildcardQuery(new Term(field, text))`.
pub fn wildcard(field: &str, text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.search.WildcardQuery", &term(field, text)],
    )
}

/// `new PrefixQuery(new Term(field, text))`.
pub fn prefix(field: &str, text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.search.PrefixQuery", &term(field, text)],
    )
}

/// `new RegexpQuery(new Term(field, text))`.
pub fn regexp(field: &str, text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.search.RegexpQuery", &term(field, text)],
    )
}

/// `new FuzzyQuery(new Term(field, text))`.
pub fn fuzzy(field: &str, text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.search.FuzzyQuery", &term(field, text)],
    )
}

/// `new FuzzyQuery(term, maxEdits, prefixLength, maxExpansions,
/// transpositions)`.
pub fn fuzzy_full(
    field: &str,
    text: &str,
    max_edits: i32,
    prefix_length: i32,
    max_expansions: i32,
    transpositions: bool,
) -> String {
    expr(
        "new",
        &[
            "org.apache.lucene.search.FuzzyQuery",
            &term(field, text),
            &format!("i:{max_edits}"),
            &format!("i:{prefix_length}"),
            &format!("i:{max_expansions}"),
            &format!("z:{transpositions}"),
        ],
    )
}

/// `new BoostQuery(query, boost)`.
pub fn boost(q: &str, b: f32) -> String {
    expr(
        "new",
        &["org.apache.lucene.search.BoostQuery", q, &format!("f:{b}")],
    )
}

/// `new BooleanQuery.Builder()`, an `add` per clause, `build()`.
pub fn boolean(clauses: &[(String, &str)]) -> String {
    let mut b = expr("new", &["org.apache.lucene.search.BooleanQuery$Builder"]);
    for (q, o) in clauses {
        let occur = expr(
            "field",
            &["org.apache.lucene.search.BooleanClause$Occur", o],
        );
        b = expr("call", &["add", &b, q, &occur]);
    }
    expr("call", &["build", &b])
}

/// `new <PayloadFunction class>()`.
pub fn function(name: &str) -> String {
    expr("new", &[&payloads_class(name)])
}

/// `PayloadDecoder.FLOAT_DECODER`.
pub fn float_decoder_expr() -> String {
    expr(
        "field",
        &[&payloads_class("PayloadDecoder"), "FLOAT_DECODER"],
    )
}

/// `new PayloadScoreQuery(query, function, FLOAT_DECODER[, includeSpanScore])`.
pub fn payload_score(q: &str, function_name: &str, include_span_score: Option<bool>) -> String {
    let f = function(function_name);
    let d = float_decoder_expr();
    match include_span_score {
        None => expr("new", &[&payloads_class("PayloadScoreQuery"), q, &f, &d]),
        Some(include) => expr(
            "new",
            &[
                &payloads_class("PayloadScoreQuery"),
                q,
                &f,
                &d,
                &format!("z:{include}"),
            ],
        ),
    }
}

/// A payload of a `SpanPayloadCheckQuery` list.
pub enum Payload {
    /// `null`.
    Null,
    /// `new BytesRef(CharSequence)`.
    Text(String),
    /// `new BytesRef(byte[])`.
    Bytes(Vec<u8>),
}

fn payload_expr(p: &Payload) -> String {
    match p {
        Payload::Null => "null".to_string(),
        Payload::Text(s) => expr("new", &["org.apache.lucene.util.BytesRef", &lit(s)]),
        Payload::Bytes(bytes) => {
            let elems: Vec<String> = bytes.iter().map(|b| format!("b:{}", *b as i8)).collect();
            let mut args = vec!["byte"];
            args.extend(elems.iter().map(String::as_str));
            expr(
                "new",
                &["org.apache.lucene.util.BytesRef", &expr("array", &args)],
            )
        }
    }
}

/// `new SpanPayloadCheckQuery(match, payloads[, type, operation])`.
pub fn payload_check(m: &str, payloads: &[Payload], type_op: Option<(&str, &str)>) -> String {
    let list: Vec<String> = payloads.iter().map(payload_expr).collect();
    let list = expr("list", &list.iter().map(String::as_str).collect::<Vec<_>>());
    match type_op {
        None => expr("new", &[&payloads_class("SpanPayloadCheckQuery"), m, &list]),
        Some((t, op)) => expr(
            "new",
            &[
                &payloads_class("SpanPayloadCheckQuery"),
                m,
                &list,
                &expr(
                    "field",
                    &[&payloads_class("SpanPayloadCheckQuery$PayloadType"), t],
                ),
                &expr(
                    "field",
                    &[&payloads_class("SpanPayloadCheckQuery$MatchOperation"), op],
                ),
            ],
        ),
    }
}

/// Changes the first line of record `header` in `dump` that starts with
/// `prefix` (its last field gets a `*` appended), for the positive controls.
pub fn perturb(dump: &str, header: &str, prefix: &str) -> String {
    let mut out = String::new();
    let mut in_record = false;
    let mut done = false;
    for l in dump.lines() {
        if l.starts_with("query\t") || l.starts_with("spans\t") || l.starts_with("parse\t") {
            in_record = l == header;
        }
        if in_record && !done && l.starts_with(prefix) {
            out.push_str(l);
            out.push('*');
            done = true;
        } else {
            out.push_str(l);
        }
        out.push('\n');
    }
    assert!(done, "record `{header}` has a `{prefix}` line to perturb");
    out
}
