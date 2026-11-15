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

//! The bindings of the `queries.function` classes for the Java-oracle
//! harness (`query_oracle`, rmp #478 and #479): the [`registry`] Rucene
//! evaluates the expressions with, Rucene's side of the harness's `values`
//! records ([`values_dumper`]), the seeded index both tests search
//! ([`function_index`]) and the expression builders both sides read.
//!
//! A `values` record dumps, after `createWeight` with
//! `ValueSource.newContext(searcher)`, every `FunctionValues` getter of
//! every document of every leaf: its value (floats as bits) or the
//! exception it throws.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::document::{
    Document, KnnByteVectorField, KnnFloatVectorField, NumericDocValuesField, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField, Store, StringField, TextField,
};
use rucene::index::{Term, VectorSimilarityFunction};
use rucene::search::double_values_source::{self, DoubleValuesSource};
use rucene::search::long_values_source::{self, LongValuesSource};
use rucene::search::similarities::ExplanationValue;
use rucene::search::sorted_numeric_selector::SortedNumericSelectorType;
use rucene::util::mutable::MutableValueObject;
use rucene::util::{BytesRef, BytesRefBuilder, NumericUtils};
use rucene_queries::function::function_match_query::{DoublePredicate, FunctionMatchQuery};
use rucene_queries::function::function_query::FunctionQuery;
use rucene_queries::function::function_range_query::FunctionRangeQuery;
use rucene_queries::function::function_score_query::FunctionScoreQuery;
use rucene_queries::function::index_reader_functions::IndexReaderFunctions;
use rucene_queries::function::value_source::{from_double_values_source, new_context, ValueSource};
use rucene_queries::function::valuesource::byte_knn_vector_field_source::ByteKnnVectorFieldSource;
use rucene_queries::function::valuesource::byte_vector_similarity_function::ByteVectorSimilarityFunction;
use rucene_queries::function::valuesource::bytes_ref_field_source::BytesRefFieldSource;
use rucene_queries::function::valuesource::const_knn_byte_vector_value_source::ConstKnnByteVectorValueSource;
use rucene_queries::function::valuesource::const_knn_float_value_source::ConstKnnFloatValueSource;
use rucene_queries::function::valuesource::const_value_source::ConstValueSource;
use rucene_queries::function::valuesource::def_function::DefFunction;
use rucene_queries::function::valuesource::div_float_function::DivFloatFunction;
use rucene_queries::function::valuesource::doc_freq_value_source::DocFreqValueSource;
use rucene_queries::function::valuesource::double_const_value_source::DoubleConstValueSource;
use rucene_queries::function::valuesource::double_field_source::DoubleFieldSource;
use rucene_queries::function::valuesource::enum_field_source::EnumFieldSource;
use rucene_queries::function::valuesource::float_field_source::FloatFieldSource;
use rucene_queries::function::valuesource::float_knn_vector_field_source::FloatKnnVectorFieldSource;
use rucene_queries::function::valuesource::float_vector_similarity_function::FloatVectorSimilarityFunction;
use rucene_queries::function::valuesource::idf_value_source::IDFValueSource;
use rucene_queries::function::valuesource::if_function::IfFunction;
use rucene_queries::function::valuesource::int_field_source::IntFieldSource;
use rucene_queries::function::valuesource::join_doc_freq_value_source::JoinDocFreqValueSource;
use rucene_queries::function::valuesource::linear_float_function::LinearFloatFunction;
use rucene_queries::function::valuesource::literal_value_source::LiteralValueSource;
use rucene_queries::function::valuesource::long_field_source::LongFieldSource;
use rucene_queries::function::valuesource::max_doc_value_source::MaxDocValueSource;
use rucene_queries::function::valuesource::max_float_function::MaxFloatFunction;
use rucene_queries::function::valuesource::min_float_function::MinFloatFunction;
use rucene_queries::function::valuesource::multi_valued_double_field_source::MultiValuedDoubleFieldSource;
use rucene_queries::function::valuesource::multi_valued_float_field_source::MultiValuedFloatFieldSource;
use rucene_queries::function::valuesource::multi_valued_int_field_source::MultiValuedIntFieldSource;
use rucene_queries::function::valuesource::multi_valued_long_field_source::MultiValuedLongFieldSource;
use rucene_queries::function::valuesource::norm_value_source::NormValueSource;
use rucene_queries::function::valuesource::num_docs_value_source::NumDocsValueSource;
use rucene_queries::function::valuesource::pow_float_function::PowFloatFunction;
use rucene_queries::function::valuesource::product_float_function::ProductFloatFunction;
use rucene_queries::function::valuesource::query_value_source::QueryValueSource;
use rucene_queries::function::valuesource::range_map_float_function::RangeMapFloatFunction;
use rucene_queries::function::valuesource::reciprocal_float_function::ReciprocalFloatFunction;
use rucene_queries::function::valuesource::scale_float_function::ScaleFloatFunction;
use rucene_queries::function::valuesource::sorted_set_field_source::SortedSetFieldSource;
use rucene_queries::function::valuesource::sum_float_function::SumFloatFunction;
use rucene_queries::function::valuesource::sum_total_term_freq_value_source::SumTotalTermFreqValueSource;
use rucene_queries::function::valuesource::term_freq_value_source::TermFreqValueSource;
use rucene_queries::function::valuesource::tf_value_source::TFValueSource;
use rucene_queries::function::valuesource::total_term_freq_value_source::TotalTermFreqValueSource;
use rucene_queries::function::valuesource::vector_value_source::VectorValueSource;

use crate::query_oracle::{
    escape, escape_opt, expr, harness_fail, lit, stage, OracleIndex, Registry, SpansFn, Value,
    VOCABULARY,
};

/// The class of the value sources the registry builds.
pub const VS: &str = "org.apache.lucene.queries.function.ValueSource";
/// The class of the double values sources the registry builds.
pub const DVS: &str = "org.apache.lucene.search.DoubleValuesSource";
/// The class of the long values sources the registry builds.
pub const LVS: &str = "org.apache.lucene.search.LongValuesSource";
const PKG: &str = "org.apache.lucene.queries.function.valuesource";
const ORACLE: &str = "org.apache.lucene.rucene.codec.QueryOracle";
const SELECTOR: &str = "org.apache.lucene.search.SortedNumericSelector$Type";
const SIM: &str = "org.apache.lucene.index.VectorSimilarityFunction";

fn vs_value(v: Arc<dyn ValueSource>) -> Value {
    Value::object(VS, v)
}

/// The value source a value holds.
pub fn vsrc(v: &Value) -> Arc<dyn ValueSource> {
    v.get::<Arc<dyn ValueSource>>()
}

fn vsrcs(v: &Value) -> Vec<Arc<dyn ValueSource>> {
    v.as_values().iter().map(vsrc).collect()
}

fn dvs(v: &Value) -> Arc<dyn DoubleValuesSource> {
    v.get::<Arc<dyn DoubleValuesSource>>()
}

/// A `java.lang.Number` bound of `FunctionRangeQuery`.
fn number(v: &Value) -> Option<ExplanationValue> {
    match v {
        Value::Null => None,
        Value::Int(i) => Some(ExplanationValue::Int(*i)),
        Value::Long(l) => Some(ExplanationValue::Long(*l)),
        Value::Float(f) => Some(ExplanationValue::Float(*f)),
        Value::Double(d) => Some(ExplanationValue::Double(*d)),
        other => harness_fail(format!("expected a Number, found {other:?}")),
    }
}

fn class(name: &str) -> String {
    format!("{PKG}.{name}")
}

/// The bindings of the function queries, the value sources,
/// `IndexReaderFunctions` and the `DoubleValuesSource` factories, over
/// [`Registry::core`].
pub fn registry() -> Registry {
    let mut r = Registry::core();
    r.new_fn("org.apache.lucene.util.BytesRef", 1, |a| {
        Ok(Value::object(
            "org.apache.lucene.util.BytesRef",
            BytesRef::from_char_sequence(a[0].as_str()),
        ))
    });

    // single-field sources
    type FieldCtor = fn(&str) -> Arc<dyn ValueSource>;
    let field_sources: [(&str, FieldCtor); 13] = [
        ("IntFieldSource", |f| Arc::new(IntFieldSource::new(f))),
        ("LongFieldSource", |f| Arc::new(LongFieldSource::new(f))),
        ("FloatFieldSource", |f| Arc::new(FloatFieldSource::new(f))),
        ("DoubleFieldSource", |f| Arc::new(DoubleFieldSource::new(f))),
        ("BytesRefFieldSource", |f| {
            Arc::new(BytesRefFieldSource::new(f))
        }),
        ("SortedSetFieldSource", |f| {
            Arc::new(SortedSetFieldSource::new(f))
        }),
        ("NormValueSource", |f| Arc::new(NormValueSource::new(f))),
        ("SumTotalTermFreqValueSource", |f| {
            Arc::new(SumTotalTermFreqValueSource::new(f))
        }),
        ("LiteralValueSource", |f| {
            Arc::new(LiteralValueSource::new(f))
        }),
        ("FloatKnnVectorFieldSource", |f| {
            Arc::new(FloatKnnVectorFieldSource::new(f))
        }),
        ("ByteKnnVectorFieldSource", |f| {
            Arc::new(ByteKnnVectorFieldSource::new(f))
        }),
        ("JoinDocFreqValueSource", |_| unreachable!("two arguments")),
        ("EnumFieldSource", |_| unreachable!("three arguments")),
    ];
    for (name, ctor) in field_sources.into_iter().take(11) {
        r.new_fn(&class(name), 1, move |a| Ok(vs_value(ctor(a[0].as_str()))));
    }
    r.new_fn(&class("MaxDocValueSource"), 0, |_| {
        Ok(vs_value(Arc::new(MaxDocValueSource::new())))
    });
    r.new_fn(&class("NumDocsValueSource"), 0, |_| {
        Ok(vs_value(Arc::new(NumDocsValueSource::new())))
    });
    r.new_fn(&class("ConstValueSource"), 1, |a| {
        Ok(vs_value(Arc::new(ConstValueSource::new(a[0].as_float()))))
    });
    r.new_fn(&class("DoubleConstValueSource"), 1, |a| {
        Ok(vs_value(Arc::new(DoubleConstValueSource::new(
            a[0].as_double(),
        ))))
    });
    r.new_fn(&class("JoinDocFreqValueSource"), 2, |a| {
        Ok(vs_value(Arc::new(JoinDocFreqValueSource::new(
            a[0].as_str(),
            a[1].as_str(),
        ))))
    });
    r.field_fn(SELECTOR, "MIN", || {
        Value::object(SELECTOR, SortedNumericSelectorType::MIN)
    });
    r.field_fn(SELECTOR, "MAX", || {
        Value::object(SELECTOR, SortedNumericSelectorType::MAX)
    });
    type MvCtor = fn(&str, SortedNumericSelectorType) -> Arc<dyn ValueSource>;
    let mv: [(&str, MvCtor); 4] = [
        ("MultiValuedIntFieldSource", |f, s| {
            Arc::new(MultiValuedIntFieldSource::new(f, s))
        }),
        ("MultiValuedLongFieldSource", |f, s| {
            Arc::new(MultiValuedLongFieldSource::new(f, s))
        }),
        ("MultiValuedFloatFieldSource", |f, s| {
            Arc::new(MultiValuedFloatFieldSource::new(f, s))
        }),
        ("MultiValuedDoubleFieldSource", |f, s| {
            Arc::new(MultiValuedDoubleFieldSource::new(f, s))
        }),
    ];
    for (name, ctor) in mv {
        r.new_fn(&class(name), 2, move |a| {
            Ok(vs_value(ctor(
                a[0].as_str(),
                a[1].get::<SortedNumericSelectorType>(),
            )))
        });
    }
    r.static_fn(ORACLE, "enumIntToString", 1, |a| {
        let names = a[0].as_strings();
        let map: HashMap<i32, String> = names
            .into_iter()
            .enumerate()
            .map(|(i, n)| (i as i32, n))
            .collect();
        Ok(Value::object("java.util.Map", map))
    });
    r.static_fn(ORACLE, "enumStringToInt", 1, |a| {
        let names = a[0].as_strings();
        let map: HashMap<String, i32> = names
            .into_iter()
            .enumerate()
            .map(|(i, n)| (n, i as i32))
            .collect();
        Ok(Value::object("java.util.Map", map))
    });
    r.new_fn(&class("EnumFieldSource"), 3, |a| {
        Ok(vs_value(Arc::new(EnumFieldSource::new(
            a[0].as_str(),
            a[1].get::<HashMap<i32, String>>(),
            a[2].get::<HashMap<String, i32>>(),
        ))))
    });

    // term statistics: (field, val, indexedField, indexedBytes)
    type StatCtor = fn(&str, &str, &str, BytesRef) -> Arc<dyn ValueSource>;
    let stats: [(&str, StatCtor); 5] = [
        ("DocFreqValueSource", |a, b, c, d| {
            Arc::new(DocFreqValueSource::new(a, b, c, d))
        }),
        ("IDFValueSource", |a, b, c, d| {
            Arc::new(IDFValueSource::new(a, b, c, d))
        }),
        ("TFValueSource", |a, b, c, d| {
            Arc::new(TFValueSource::new(a, b, c, d))
        }),
        ("TermFreqValueSource", |a, b, c, d| {
            Arc::new(TermFreqValueSource::new(a, b, c, d))
        }),
        ("TotalTermFreqValueSource", |a, b, c, d| {
            Arc::new(TotalTermFreqValueSource::new(a, b, c, d))
        }),
    ];
    for (name, ctor) in stats {
        r.new_fn(&class(name), 4, move |a| {
            Ok(vs_value(ctor(
                a[0].as_str(),
                a[1].as_str(),
                a[2].as_str(),
                a[3].get::<BytesRef>(),
            )))
        });
    }
    r.new_fn(&class("QueryValueSource"), 2, |a| {
        Ok(vs_value(Arc::new(QueryValueSource::new(
            a[0].as_query(),
            a[1].as_float(),
        ))))
    });

    // functions
    r.new_fn(&class("DivFloatFunction"), 2, |a| {
        Ok(vs_value(Arc::new(DivFloatFunction::new(
            vsrc(&a[0]),
            vsrc(&a[1]),
        ))))
    });
    r.new_fn(&class("PowFloatFunction"), 2, |a| {
        Ok(vs_value(Arc::new(PowFloatFunction::new(
            vsrc(&a[0]),
            vsrc(&a[1]),
        ))))
    });
    r.new_fn(&class("LinearFloatFunction"), 3, |a| {
        Ok(vs_value(Arc::new(LinearFloatFunction::new(
            vsrc(&a[0]),
            a[1].as_float(),
            a[2].as_float(),
        ))))
    });
    r.new_fn(&class("ReciprocalFloatFunction"), 4, |a| {
        Ok(vs_value(Arc::new(ReciprocalFloatFunction::new(
            vsrc(&a[0]),
            a[1].as_float(),
            a[2].as_float(),
            a[3].as_float(),
        ))))
    });
    r.new_fn(&class("ScaleFloatFunction"), 3, |a| {
        Ok(vs_value(Arc::new(ScaleFloatFunction::new(
            vsrc(&a[0]),
            a[1].as_float(),
            a[2].as_float(),
        ))))
    });
    r.new_fn(&class("RangeMapFloatFunction"), 5, |a| {
        Ok(vs_value(match (&a[3], &a[4]) {
            (Value::Float(target), def) => Arc::new(RangeMapFloatFunction::with_floats(
                vsrc(&a[0]),
                a[1].as_float(),
                a[2].as_float(),
                *target,
                match def {
                    Value::Null => None,
                    other => Some(other.as_float()),
                },
            )),
            (target, def) => Arc::new(RangeMapFloatFunction::new(
                vsrc(&a[0]),
                a[1].as_float(),
                a[2].as_float(),
                vsrc(target),
                match def {
                    Value::Null => None,
                    other => Some(vsrc(other)),
                },
            )),
        }))
    });
    type MultiCtor = fn(Vec<Arc<dyn ValueSource>>) -> Arc<dyn ValueSource>;
    let multi: [(&str, MultiCtor); 6] = [
        ("MaxFloatFunction", |v| Arc::new(MaxFloatFunction::new(v))),
        ("MinFloatFunction", |v| Arc::new(MinFloatFunction::new(v))),
        ("SumFloatFunction", |v| Arc::new(SumFloatFunction::new(v))),
        ("ProductFloatFunction", |v| {
            Arc::new(ProductFloatFunction::new(v))
        }),
        ("DefFunction", |v| Arc::new(DefFunction::new(v))),
        ("VectorValueSource", |v| Arc::new(VectorValueSource::new(v))),
    ];
    for (name, ctor) in multi {
        r.new_fn(&class(name), 1, move |a| Ok(vs_value(ctor(vsrcs(&a[0])))));
    }
    r.new_fn(&class("IfFunction"), 3, |a| {
        Ok(vs_value(Arc::new(IfFunction::new(
            vsrc(&a[0]),
            vsrc(&a[1]),
            vsrc(&a[2]),
        ))))
    });

    // vectors
    r.new_fn(&class("ConstKnnFloatValueSource"), 1, |a| {
        let v: Vec<f32> = a[0].as_values().iter().map(Value::as_float).collect();
        Ok(vs_value(Arc::new(ConstKnnFloatValueSource::new(v)?)))
    });
    r.new_fn(&class("ConstKnnByteVectorValueSource"), 1, |a| {
        let v: Vec<u8> = a[0]
            .as_values()
            .iter()
            .map(|b| match b {
                Value::Byte(b) => *b as u8,
                other => harness_fail(format!("expected a byte, found {other:?}")),
            })
            .collect();
        Ok(vs_value(Arc::new(ConstKnnByteVectorValueSource::new(v))))
    });
    for (name, sim) in [
        ("EUCLIDEAN", VectorSimilarityFunction::EUCLIDEAN),
        ("DOT_PRODUCT", VectorSimilarityFunction::DOT_PRODUCT),
        ("COSINE", VectorSimilarityFunction::COSINE),
        (
            "MAXIMUM_INNER_PRODUCT",
            VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
        ),
    ] {
        r.field_fn(SIM, name, move || Value::object(SIM, sim));
    }
    r.new_fn(&class("FloatVectorSimilarityFunction"), 3, |a| {
        Ok(vs_value(Arc::new(FloatVectorSimilarityFunction::new(
            a[0].get::<VectorSimilarityFunction>(),
            vsrc(&a[1]),
            vsrc(&a[2]),
        ))))
    });
    r.new_fn(&class("ByteVectorSimilarityFunction"), 3, |a| {
        Ok(vs_value(Arc::new(ByteVectorSimilarityFunction::new(
            a[0].get::<VectorSimilarityFunction>(),
            vsrc(&a[1]),
            vsrc(&a[2]),
        ))))
    });

    // the queries
    const F: &str = "org.apache.lucene.queries.function";
    r.new_fn(&format!("{F}.FunctionQuery"), 1, |a| {
        Ok(Value::query(Arc::new(FunctionQuery::new(vsrc(&a[0])))))
    });
    r.new_fn(&format!("{F}.FunctionRangeQuery"), 5, |a| {
        let q = match (&a[1], &a[2]) {
            (Value::Str(_), _) | (_, Value::Str(_)) => FunctionRangeQuery::new(
                vsrc(&a[0]),
                a[1].as_opt_str().map(str::to_string),
                a[2].as_opt_str().map(str::to_string),
                a[3].as_bool(),
                a[4].as_bool(),
            ),
            _ => FunctionRangeQuery::from_numbers(
                vsrc(&a[0]),
                number(&a[1]),
                number(&a[2]),
                a[3].as_bool(),
                a[4].as_bool(),
            ),
        };
        Ok(Value::query(Arc::new(q)))
    });
    r.new_fn(&format!("{F}.FunctionScoreQuery"), 2, |a| {
        Ok(Value::query(Arc::new(FunctionScoreQuery::new(
            a[0].as_query(),
            dvs(&a[1]),
        ))))
    });
    r.static_fn(&format!("{F}.FunctionScoreQuery"), "boostByValue", 2, |a| {
        Ok(Value::query(Arc::new(FunctionScoreQuery::boost_by_value(
            a[0].as_query(),
            dvs(&a[1]),
        ))))
    });
    r.static_fn(&format!("{F}.FunctionScoreQuery"), "boostByQuery", 3, |a| {
        Ok(Value::query(Arc::new(FunctionScoreQuery::boost_by_query(
            a[0].as_query(),
            a[1].as_query(),
            a[2].as_float(),
        ))))
    });
    r.static_fn(ORACLE, "between", 2, |a| {
        let (lo, hi) = (a[0].as_double(), a[1].as_double());
        let p: DoublePredicate = Arc::new(move |d| d >= lo && d < hi);
        Ok(Value::object("java.util.function.DoublePredicate", p))
    });
    r.new_fn(&format!("{F}.FunctionMatchQuery"), 2, |a| {
        Ok(Value::query(Arc::new(FunctionMatchQuery::new(
            dvs(&a[0]),
            a[1].get::<DoublePredicate>(),
        ))))
    });
    r.new_fn(&format!("{F}.FunctionMatchQuery"), 3, |a| {
        Ok(Value::query(Arc::new(FunctionMatchQuery::with_match_cost(
            dvs(&a[0]),
            a[1].get::<DoublePredicate>(),
            a[2].as_float(),
        ))))
    });

    // IndexReaderFunctions, the conversions and the DoubleValuesSource factories
    const IRF: &str = "org.apache.lucene.queries.function.IndexReaderFunctions";
    let d = |s: Arc<dyn DoubleValuesSource>| Value::object(DVS, s);
    r.static_fn(IRF, "docFreq", 1, move |a| {
        Ok(d(IndexReaderFunctions::doc_freq(a[0].get::<Term>())))
    });
    r.static_fn(IRF, "maxDoc", 0, move |_| {
        Ok(d(IndexReaderFunctions::max_doc()))
    });
    r.static_fn(IRF, "numDocs", 0, move |_| {
        Ok(d(IndexReaderFunctions::num_docs()))
    });
    r.static_fn(IRF, "numDeletedDocs", 0, move |_| {
        Ok(d(IndexReaderFunctions::num_deleted_docs()))
    });
    r.static_fn(IRF, "termFreq", 1, move |a| {
        Ok(d(IndexReaderFunctions::term_freq(a[0].get::<Term>())))
    });
    r.static_fn(IRF, "totalTermFreq", 1, move |a| {
        Ok(d(IndexReaderFunctions::total_term_freq(a[0].get::<Term>())))
    });
    r.static_fn(IRF, "sumDocFreq", 1, move |a| {
        Ok(d(IndexReaderFunctions::sum_doc_freq(a[0].as_str())))
    });
    r.static_fn(IRF, "docCount", 1, move |a| {
        Ok(d(IndexReaderFunctions::doc_count(a[0].as_str())))
    });
    r.static_fn(IRF, "sumTotalTermFreq", 1, |a| {
        Ok(Value::object(
            LVS,
            IndexReaderFunctions::sum_total_term_freq(a[0].as_str()),
        ))
    });
    r.call_fn(LVS, "toDoubleValuesSource", 0, move |v, _| {
        Ok(d(long_values_source::to_double_values_source(
            v.get::<Arc<dyn LongValuesSource>>(),
        )))
    });
    r.static_fn(VS, "fromDoubleValuesSource", 1, |a| {
        Ok(vs_value(from_double_values_source(dvs(&a[0]))))
    });
    r.call_fn(VS, "asDoubleValuesSource", 0, move |v, _| {
        Ok(d(vsrc(v).as_double_values_source()))
    });
    r.static_fn(DVS, "fromIntField", 1, move |a| {
        Ok(d(double_values_source::from_int_field(a[0].as_str())))
    });
    r.static_fn(DVS, "fromLongField", 1, move |a| {
        Ok(d(double_values_source::from_long_field(a[0].as_str())))
    });
    r.static_fn(DVS, "fromFloatField", 1, move |a| {
        Ok(d(double_values_source::from_float_field(a[0].as_str())))
    });
    r.static_fn(DVS, "fromDoubleField", 1, move |a| {
        Ok(d(double_values_source::from_double_field(a[0].as_str())))
    });
    r.static_fn(DVS, "constant", 1, move |a| {
        Ok(d(double_values_source::constant(a[0].as_double())))
    });
    r.field_fn(DVS, "SCORES", move || d(double_values_source::scores()));
    r
}

fn object_value(o: Option<MutableValueObject>) -> String {
    match o {
        None => "null".to_string(),
        Some(MutableValueObject::Float(f)) => format!("F{:08x}", f.to_bits()),
        Some(MutableValueObject::Double(d)) => format!("D{:016x}", d.to_bits()),
        Some(MutableValueObject::Int(i)) => format!("I{i}"),
        Some(MutableValueObject::Long(l)) => format!("L{l}"),
        Some(MutableValueObject::Bool(b)) => format!("Z{b}"),
        Some(MutableValueObject::Str(s)) => format!("S{}", escape(&s)),
        Some(MutableValueObject::Date(t)) => format!("T{t}"),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn val(
    out: &mut Vec<String>,
    name: &str,
    r: std::result::Result<String, randomizedtesting::jdk::exception::Throwable>,
) {
    match r {
        Ok(v) => out.push(format!("val\t{name}\t{v}")),
        Err(t) => out.push(format!(
            "val\t{name}\terr\t{}\t{}",
            t.class_name(),
            escape_opt(t.message())
        )),
    }
}

/// Rucene's side of the `values` records: see the module documentation and
/// `QueryOracle.values`.
pub fn values_dumper() -> SpansFn {
    Box::new(|v, searcher, _arg, out| {
        let vs = vsrc(v);
        out.push(format!("tostring\t{}", escape(&vs.description())));
        let context = new_context(Some(searcher));
        vs.create_weight(&context, searcher)?;
        let reader = Arc::clone(searcher.get_index_reader());
        for ctx in reader.leaves()? {
            out.push(format!("leaf\t{}\t{}", ctx.ord(), ctx.doc_base()));
            let mut fv = Arc::clone(&vs).get_values(&context, &ctx)?;
            for doc in 0..ctx.leaf_reader().max_doc() {
                out.push(format!("vdoc\t{doc}"));
                val(out, "exists", stage(|| Ok(fv.exists(doc)?.to_string())));
                val(
                    out,
                    "float",
                    stage(|| Ok(format!("F{:08x}", fv.float_val(doc)?.to_bits()))),
                );
                val(
                    out,
                    "double",
                    stage(|| Ok(format!("D{:016x}", fv.double_val(doc)?.to_bits()))),
                );
                val(out, "int", stage(|| Ok(fv.int_val(doc)?.to_string())));
                val(out, "long", stage(|| Ok(fv.long_val(doc)?.to_string())));
                val(out, "bool", stage(|| Ok(fv.bool_val(doc)?.to_string())));
                val(
                    out,
                    "str",
                    stage(|| Ok(escape_opt(fv.str_val(doc)?.as_deref()))),
                );
                val(
                    out,
                    "object",
                    stage(|| Ok(object_value(fv.object_val(doc)?))),
                );
                val(out, "ord", stage(|| Ok(fv.ord_val(doc)?.to_string())));
                val(out, "numord", stage(|| Ok(fv.num_ord()?.to_string())));
                val(
                    out,
                    "fvec",
                    stage(|| {
                        Ok(match fv.float_vector_val(doc)? {
                            None => "null".to_string(),
                            Some(v) => v.iter().map(|x| format!("{:08x},", x.to_bits())).collect(),
                        })
                    }),
                );
                val(
                    out,
                    "bvec",
                    stage(|| {
                        Ok(match fv.byte_vector_val(doc)? {
                            None => "null".to_string(),
                            Some(v) => v.iter().map(|x| format!("{},", *x as i8)).collect(),
                        })
                    }),
                );
                val(
                    out,
                    "bytes",
                    stage(|| {
                        let mut b = BytesRefBuilder::new();
                        let r = fv.bytes_val(doc, &mut b)?;
                        Ok(format!("{r}:{}", hex(b.get().slice())))
                    }),
                );
                val(
                    out,
                    "tostring",
                    stage(|| Ok(escape(&fv.to_doc_string(doc)?))),
                );
            }
        }
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// The words of the `s`, `ss` and enum fields.
pub const COLORS: [&str; 5] = ["red", "green", "blue", "cyan", "the"];

/// An index of `num_docs` documents drawn from `seed`, in three segments,
/// about one in twenty deleted: `id`, `body` (text, 0 to 19
/// [`VOCABULARY`] words), `i` (int doc values, two in three documents), `l`
/// (long), `f` (float bits), `d` (double bits), `imv`/`lmv` (sorted
/// numeric, 0 to 3 values), `fmv`/`dmv` (sortable float and double sorted
/// numeric), `s` (sorted, a [`COLORS`] word or none), `ss` (sorted set, 0
/// to 2 words), `e` (an enum ordinal 0 to 4 as numeric doc values), `fv`
/// (a 3-dimensional float vector, three in four documents) and `bv` (a byte
/// vector).
pub fn function_index(seed: i64, num_docs: usize) -> OracleIndex {
    let mut random = JdkRandom::new(seed);
    let docs: Vec<Document> = (0..num_docs).map(|i| doc(&mut random, i)).collect();
    let deletes: Vec<usize> = (0..num_docs / 20)
        .map(|_| random.next_int_bound(num_docs as i32) as usize)
        .collect();
    OracleIndex::write(&docs, &deletes)
}

fn doc(random: &mut JdkRandom, i: usize) -> Document {
    let mut doc = Document::new();
    doc.add(Box::new(
        StringField::new("id", format!("d{i}"), Store::YES).expect("id"),
    ));
    let body: Vec<&str> = (0..random.next_int_bound(20))
        .map(|_| {
            let r = random.next_double();
            VOCABULARY[(r * r * VOCABULARY.len() as f64) as usize]
        })
        .collect();
    doc.add(Box::new(
        TextField::new("body", body.join(" "), Store::NO).expect("body"),
    ));
    if random.next_int_bound(3) != 0 {
        let v = random.next_int_bound(200) - 50;
        doc.add(Box::new(NumericDocValuesField::new("i", i64::from(v))));
        doc.add(Box::new(NumericDocValuesField::new(
            "l",
            i64::from(v) * 1_000_003,
        )));
        let f = v as f32 / 7.0;
        doc.add(Box::new(NumericDocValuesField::new(
            "f",
            i64::from(f.to_bits() as i32),
        )));
        let d = f64::from(v) / 3.0;
        doc.add(Box::new(NumericDocValuesField::new(
            "d",
            d.to_bits() as i64,
        )));
    }
    for _ in 0..random.next_int_bound(4) {
        let v = random.next_int_bound(1000) - 300;
        doc.add(Box::new(SortedNumericDocValuesField::new(
            "imv",
            i64::from(v),
        )));
        doc.add(Box::new(SortedNumericDocValuesField::new(
            "lmv",
            i64::from(v) << 20,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::new(
            "fmv",
            i64::from(NumericUtils::float_to_sortable_int(v as f32 / 9.0)),
        )));
        doc.add(Box::new(SortedNumericDocValuesField::new(
            "dmv",
            NumericUtils::double_to_sortable_long(f64::from(v) / 11.0),
        )));
    }
    if random.next_int_bound(4) != 0 {
        let c = COLORS[random.next_int_bound(COLORS.len() as i32) as usize];
        doc.add(Box::new(SortedDocValuesField::new(
            "s",
            BytesRef::from_char_sequence(c),
        )));
    }
    for _ in 0..random.next_int_bound(3) {
        let c = COLORS[random.next_int_bound(COLORS.len() as i32) as usize];
        doc.add(Box::new(SortedSetDocValuesField::new(
            "ss",
            BytesRef::from_char_sequence(c),
        )));
    }
    doc.add(Box::new(NumericDocValuesField::new(
        "e",
        i64::from(random.next_int_bound(COLORS.len() as i32)),
    )));
    if random.next_int_bound(4) != 0 {
        let v: Vec<f32> = (0..3).map(|_| random.next_float() * 2.0 - 1.0).collect();
        doc.add(Box::new(
            KnnFloatVectorField::with_euclidean("fv", &v).expect("fv"),
        ));
        let b: Vec<u8> = (0..3)
            .map(|_| (random.next_int_bound(200) - 100) as i8 as u8)
            .collect();
        doc.add(Box::new(
            KnnByteVectorField::with_euclidean("bv", &b).expect("bv"),
        ));
    }
    doc
}

// ---------------------------------------------------------------------------
// Expression builders
// ---------------------------------------------------------------------------

/// `new <valuesource class>(args...)`.
pub fn vs(name: &str, args: &[&str]) -> String {
    let c = class(name);
    let mut all: Vec<&str> = vec![&c];
    all.extend_from_slice(args);
    expr("new", &all)
}

/// `new <valuesource class>(field)`.
pub fn field(name: &str, field: &str) -> String {
    vs(name, &[&lit(field)])
}

/// `new <multi-valued source>(field, SortedNumericSelector.Type.<selector>)`.
pub fn mv(name: &str, field: &str, selector: &str) -> String {
    vs(name, &[&lit(field), &expr("field", &[SELECTOR, selector])])
}

/// `new <term statistics source>(field, val, indexedField, new BytesRef(text))`.
pub fn stat(name: &str, indexed_field: &str, text: &str) -> String {
    let bytes = expr("new", &["org.apache.lucene.util.BytesRef", &lit(text)]);
    vs(
        name,
        &[&lit("bogus"), &lit("bogus"), &lit(indexed_field), &bytes],
    )
}

/// `new ConstValueSource(constant)`.
pub fn cvs(constant: f32) -> String {
    vs("ConstValueSource", &[&format!("f:{constant}")])
}

/// An array of value sources.
pub fn vs_array(sources: &[String]) -> String {
    let mut args: Vec<&str> = vec![VS];
    args.extend(sources.iter().map(String::as_str));
    expr("array", &args)
}

/// A list of value sources.
pub fn vs_list(sources: &[String]) -> String {
    let args: Vec<&str> = sources.iter().map(String::as_str).collect();
    expr("list", &args)
}

/// `new FunctionQuery(source)`.
pub fn function_query(source: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.queries.function.FunctionQuery", source],
    )
}

/// `new FunctionRangeQuery(source, lower, upper, includeLower, includeUpper)`.
pub fn frange(source: &str, lower: &str, upper: &str, il: bool, iu: bool) -> String {
    expr(
        "new",
        &[
            "org.apache.lucene.queries.function.FunctionRangeQuery",
            source,
            lower,
            upper,
            &format!("z:{il}"),
            &format!("z:{iu}"),
        ],
    )
}

/// `new Term(field, text)`.
pub fn term(field: &str, text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.index.Term", &lit(field), &lit(text)],
    )
}

/// `new TermQuery(new Term(field, text))`.
pub fn term_query(field: &str, text: &str) -> String {
    expr(
        "new",
        &["org.apache.lucene.search.TermQuery", &term(field, text)],
    )
}

/// `MatchAllDocsQuery.INSTANCE`.
pub fn match_all() -> String {
    expr(
        "field",
        &["org.apache.lucene.search.MatchAllDocsQuery", "INSTANCE"],
    )
}

/// A static factory call.
pub fn static_call(class: &str, method: &str, args: &[&str]) -> String {
    let mut all: Vec<&str> = vec![class, method];
    all.extend_from_slice(args);
    expr("static", &all)
}

/// `ValueSource.fromDoubleValuesSource(source)`.
pub fn from_dvs(source: &str) -> String {
    static_call(VS, "fromDoubleValuesSource", &[source])
}

/// `QueryOracle.between(lo, hi)`.
pub fn between(lo: f64, hi: f64) -> String {
    static_call(ORACLE, "between", &[&format!("d:{lo}"), &format!("d:{hi}")])
}

/// `VectorSimilarityFunction.<name>`.
pub fn similarity(name: &str) -> String {
    expr("field", &[SIM, name])
}

/// `new EnumFieldSource(field, enumIntToString(COLORS), enumStringToInt(COLORS))`.
pub fn enum_source(field: &str) -> String {
    let names: Vec<String> = COLORS.iter().map(|c| lit(c)).collect();
    let mut arr: Vec<&str> = vec!["java.lang.String"];
    arr.extend(names.iter().map(String::as_str));
    let arr = expr("array", &arr);
    vs(
        "EnumFieldSource",
        &[
            &lit(field),
            &static_call(ORACLE, "enumIntToString", &[&arr]),
            &static_call(ORACLE, "enumStringToInt", &[&arr]),
        ],
    )
}

// ---------------------------------------------------------------------------
// Parity
// ---------------------------------------------------------------------------

/// The oracle over `index` with the function bindings and the `values`
/// dumper.
pub fn oracle(index: &OracleIndex) -> crate::query_oracle::SearchOracle {
    let mut oracle = crate::query_oracle::SearchOracle::new(index, registry());
    oracle.custom_dumper("values", values_dumper());
    oracle
}

fn count_records(dump: &str, kind: &str) -> usize {
    dump.lines()
        .filter(|l| l.starts_with(&format!("{kind}\t")))
        .count()
}

/// Runs both sides and panics with the first difference; `min_values`
/// guards against a vacuous comparison, and a tampered value must make the
/// comparison fail (negative control).
pub fn assert_parity(oracle: &crate::query_oracle::SearchOracle, min_values: usize) {
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let values = count_records(&java, "val");
    println!(
        "{} queries, {} hits, {} values records, {} values, {} dump lines",
        count_records(&java, "query"),
        count_records(&java, "hit"),
        count_records(&java, "custom"),
        values,
        java.lines().count()
    );
    assert!(
        values >= min_values,
        "the dump has {values} values, too few to prove parity"
    );
    if let Err(diff) = crate::query_oracle::compare(&java, &rust) {
        panic!("function parity with Lucene 10.5.0 fails:\n{diff}");
    }
    if values > 0 {
        let tampered = rust.replacen("\nval\t", "\nval\t-", 1);
        assert!(
            crate::query_oracle::compare(&java, &tampered).is_err(),
            "the comparison does not see a changed value"
        );
    }
}

/// A query record (`FunctionQuery`) and a values record for each source.
pub fn add_sources(oracle: &mut crate::query_oracle::SearchOracle, sources: &[(&str, String)]) {
    for (id, source) in sources {
        oracle.query(&format!("fq-{id}"), &function_query(source));
        oracle.custom("values", &format!("vals-{id}"), source, "-");
    }
}
