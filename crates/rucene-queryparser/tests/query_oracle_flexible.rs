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

//! The parity of the flexible query parser framework (rmp #486-#491):
//! Lucene 10.5.0 and Rucene evaluate the same specs and their dumps are
//! compared line for line (the harness of rmp #475, `query_oracle`).
//!
//! * #486 `messages_*`: `NLS.getLocalizedMessage` and `MessageImpl` of every
//!   `QueryParserMessages` key (and a missing one) with seeded arguments
//!   (strings, numbers, characters, booleans, `null`), in `Locale.ROOT` and
//!   `ja`; and the SHA-1 of the test bundles `MessagesTestBundle*.properties`.
//! * #487 `core_nodes_*`: seeded trees of the core query nodes: per node its
//!   class, `isLeaf()`, parent, tags, `toString()` and `toQueryString()`,
//!   then the same of `cloneTree()` (`QueryOracle.nodeDump`);
//!   `query_tree_builder_*`: a core `QueryTreeBuilder` with class and field
//!   builders over seeded trees (the built `Query`, or the conversion
//!   exception).
//! * #488 `standard_nodes_*`, `interval_functions_*`: every standard node and
//!   interval function, the function's `toString()` and the
//!   `IntervalsSource` it builds; `number_date_format_*`; `config_handlers_*`
//!   (`StandardQueryConfigHandler.getFieldConfig` through its three
//!   listeners, `PointsConfig.equals`).
//! * #489 `syntax_parser_*`: a seeded corpus, valid and invalid, through
//!   `StandardSyntaxParser.parse`: the node tree, or the exception class and
//!   message.
//! * #490 `processing_*`: per processor of the standard (and precedence)
//!   pipeline the tree it returns (`QueryOracle.processTrace`), and the
//!   `Query` the `StandardQueryTreeBuilder` builds from the processed tree,
//!   under seeded parser configurations.
//! * #491 `standard_query_parser_*`, `precedence_query_parser_*`: the corpus
//!   through `StandardQueryParser.parse` and `PrecedenceQueryParser.parse`
//!   under randomised configurations (analyzer, operator, points, date
//!   resolution and map, locale, time zone, multi fields and boosts, fuzzy
//!   settings, slop, rewrite method, position increments, leading
//!   wildcards): the `Query` (`toString()`, `toString(field)`, class tree)
//!   or the exception.
//!
//! Every comparison spawns its own JVM in its own directory; the shards run
//! in parallel.

#[path = "../../rucene-queries/tests/query_oracle/mod.rs"]
mod query_oracle;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use query_oracle::{compare, corpus, expr, harness_fail, lit, sha1, ParseOracle, Registry, Value};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::standard::StandardAnalyzer;
use rucene::analysis::{Analyzer, CharArraySet};
use rucene::document::Resolution;
use rucene::error::{LuceneError, Result};
use rucene::search::multi_term_query::{
    constant_score_blended_rewrite, constant_score_boolean_rewrite, constant_score_rewrite,
    scoring_boolean_rewrite, RewriteMethod,
};
use rucene_queryparser::flexible::core::builders::query_builder::QueryBuilder;
use rucene_queryparser::flexible::core::builders::query_tree_builder::QueryTreeBuilder;
use rucene_queryparser::flexible::core::config::field_config::FieldConfig;
use rucene_queryparser::flexible::core::config::query_config_handler::QueryConfigHandler;
use rucene_queryparser::flexible::core::messages::query_parser_messages::QueryParserMessages;
use rucene_queryparser::flexible::core::nodes::and_query_node::AndQueryNode;
use rucene_queryparser::flexible::core::nodes::any_query_node::AnyQueryNode;
use rucene_queryparser::flexible::core::nodes::boolean_query_node::{
    BooleanQueryNode, BOOLEAN_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::boost_query_node::{
    BoostQueryNode, BOOST_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::deleted_query_node::DeletedQueryNode;
use rucene_queryparser::flexible::core::nodes::field_query_node::{
    FieldQueryNode, FIELD_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::fuzzy_query_node::{
    FuzzyQueryNode, FUZZY_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::group_query_node::{
    GroupQueryNode, GROUP_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::match_all_docs_query_node::{
    MatchAllDocsQueryNode, MATCH_ALL_DOCS_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::match_no_docs_query_node::{
    MatchNoDocsQueryNode, MATCH_NO_DOCS_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::modifier_query_node::{
    Modifier, ModifierQueryNode, MODIFIER_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::no_token_found_query_node::NoTokenFoundQueryNode;
use rucene_queryparser::flexible::core::nodes::opaque_query_node::OpaqueQueryNode;
use rucene_queryparser::flexible::core::nodes::or_query_node::OrQueryNode;
use rucene_queryparser::flexible::core::nodes::path_query_node::{PathQueryNode, QueryText};
use rucene_queryparser::flexible::core::nodes::phrase_slop_query_node::PhraseSlopQueryNode;
use rucene_queryparser::flexible::core::nodes::proximity_query_node::{
    ProximityQueryNode, Type as ProximityType,
};
use rucene_queryparser::flexible::core::nodes::query_node::QueryNodeRef;
use rucene_queryparser::flexible::core::nodes::quoted_field_query_node::QuotedFieldQueryNode;
use rucene_queryparser::flexible::core::nodes::slop_query_node::{
    SlopQueryNode, SLOP_QUERY_NODE_CLASS,
};
use rucene_queryparser::flexible::core::nodes::tokenized_phrase_query_node::TokenizedPhraseQueryNode;
use rucene_queryparser::flexible::core::parser::syntax_parser::SyntaxParser;
use rucene_queryparser::flexible::core::processors::query_node_processor::QueryNodeProcessor;
use rucene_queryparser::flexible::core::processors::query_node_processor_pipeline::QueryNodeProcessorPipeline;
use rucene_queryparser::flexible::messages::message::Message;
use rucene_queryparser::flexible::messages::message_impl::MessageImpl;
use rucene_queryparser::flexible::messages::nls::{NlsField, NLS};
use rucene_queryparser::flexible::precedence::precedence_query_parser::PrecedenceQueryParser;
use rucene_queryparser::flexible::precedence::processors::precedence_query_node_processor_pipeline::PrecedenceQueryNodeProcessorPipeline;
use rucene_queryparser::flexible::standard::builders::boolean_query_node_builder::BooleanQueryNodeBuilder;
use rucene_queryparser::flexible::standard::builders::boost_query_node_builder::BoostQueryNodeBuilder;
use rucene_queryparser::flexible::standard::builders::field_query_node_builder::FieldQueryNodeBuilder;
use rucene_queryparser::flexible::standard::builders::fuzzy_query_node_builder::FuzzyQueryNodeBuilder;
use rucene_queryparser::flexible::standard::builders::group_query_node_builder::GroupQueryNodeBuilder;
use rucene_queryparser::flexible::standard::builders::match_all_docs_query_node_builder::MatchAllDocsQueryNodeBuilder;
use rucene_queryparser::flexible::standard::builders::match_no_docs_query_node_builder::MatchNoDocsQueryNodeBuilder;
use rucene_queryparser::flexible::standard::builders::modifier_query_node_builder::ModifierQueryNodeBuilder;
use rucene_queryparser::flexible::standard::builders::slop_query_node_builder::SlopQueryNodeBuilder;
use rucene_queryparser::flexible::standard::builders::standard_query_builder::query_of;
use rucene_queryparser::flexible::standard::builders::standard_query_tree_builder::StandardQueryTreeBuilder;
use rucene_queryparser::flexible::standard::common_query_parser_configuration::CommonQueryParserConfiguration;
use rucene_queryparser::flexible::standard::config::number_date_format::NumberDateFormat;
use rucene_queryparser::flexible::standard::config::points_config::PointsConfig;
use rucene_queryparser::flexible::standard::config::standard_query_config_handler::{
    ConfigurationKeys, FieldBoostMap, FieldDateResolutionMap, Operator, PointsConfigMap,
    StandardQueryConfigHandler,
};
use rucene_queryparser::flexible::standard::nodes::boolean_modifier_node::BooleanModifierNode;
use rucene_queryparser::flexible::standard::nodes::interval_query_node::IntervalQueryNode;
use rucene_queryparser::flexible::standard::nodes::intervalfn::after::After;
use rucene_queryparser::flexible::standard::nodes::intervalfn::analyzed_text::AnalyzedText;
use rucene_queryparser::flexible::standard::nodes::intervalfn::at_least::AtLeast;
use rucene_queryparser::flexible::standard::nodes::intervalfn::before::Before;
use rucene_queryparser::flexible::standard::nodes::intervalfn::contained_by::ContainedBy;
use rucene_queryparser::flexible::standard::nodes::intervalfn::containing::Containing;
use rucene_queryparser::flexible::standard::nodes::intervalfn::extend::Extend;
use rucene_queryparser::flexible::standard::nodes::intervalfn::fuzzy_term::FuzzyTerm;
use rucene_queryparser::flexible::standard::nodes::intervalfn::interval_function::IntervalFunction;
use rucene_queryparser::flexible::standard::nodes::intervalfn::max_gaps::MaxGaps;
use rucene_queryparser::flexible::standard::nodes::intervalfn::max_width::MaxWidth;
use rucene_queryparser::flexible::standard::nodes::intervalfn::non_overlapping::NonOverlapping;
use rucene_queryparser::flexible::standard::nodes::intervalfn::not_contained_by::NotContainedBy;
use rucene_queryparser::flexible::standard::nodes::intervalfn::not_containing::NotContaining;
use rucene_queryparser::flexible::standard::nodes::intervalfn::not_within::NotWithin;
use rucene_queryparser::flexible::standard::nodes::intervalfn::or::Or;
use rucene_queryparser::flexible::standard::nodes::intervalfn::ordered::Ordered;
use rucene_queryparser::flexible::standard::nodes::intervalfn::overlapping::Overlapping;
use rucene_queryparser::flexible::standard::nodes::intervalfn::phrase::Phrase;
use rucene_queryparser::flexible::standard::nodes::intervalfn::unordered::Unordered;
use rucene_queryparser::flexible::standard::nodes::intervalfn::unordered_no_overlaps::UnorderedNoOverlaps;
use rucene_queryparser::flexible::standard::nodes::intervalfn::wildcard::Wildcard;
use rucene_queryparser::flexible::standard::nodes::intervalfn::within::Within;
use rucene_queryparser::flexible::standard::nodes::min_should_match_node::MinShouldMatchNode;
use rucene_queryparser::flexible::standard::nodes::multi_phrase_query_node::MultiPhraseQueryNode;
use rucene_queryparser::flexible::standard::nodes::point_query_node::PointQueryNode;
use rucene_queryparser::flexible::standard::nodes::point_range_query_node::PointRangeQueryNode;
use rucene_queryparser::flexible::standard::nodes::prefix_wildcard_query_node::PrefixWildcardQueryNode;
use rucene_queryparser::flexible::standard::nodes::regexp_query_node::RegexpQueryNode;
use rucene_queryparser::flexible::standard::nodes::synonym_query_node::SynonymQueryNode;
use rucene_queryparser::flexible::standard::nodes::term_range_query_node::TermRangeQueryNode;
use rucene_queryparser::flexible::standard::nodes::wildcard_query_node::WildcardQueryNode;
use rucene_queryparser::flexible::standard::parser::escape_query_syntax_impl::EscapeQuerySyntaxImpl;
use rucene_queryparser::flexible::standard::parser::standard_syntax_parser::StandardSyntaxParser;
use rucene_queryparser::flexible::standard::processors::standard_query_node_processor_pipeline::StandardQueryNodeProcessorPipeline;
use rucene_queryparser::flexible::standard::standard_query_parser::StandardQueryParser;
use rucene_queryparser::jdk::char_sequence::CharSequence;
use rucene_queryparser::jdk::class::{JavaClass, DOUBLE, FLOAT, INTEGER, LONG, SHORT};
use rucene_queryparser::jdk::date_format;
use rucene_queryparser::jdk::format::JavaNumber;
use rucene_queryparser::jdk::locale::Locale;
use rucene_queryparser::jdk::number_format::{
    get_integer_instance, get_number_instance, NumberFormat,
};
use rucene_queryparser::jdk::object::JavaObject;
use rucene_queryparser::jdk::simple_date_format::SimpleDateFormat;
use rucene_queryparser::jdk::time_zone::TimeZone;

// ---------------------------------------------------------------------------
// Class names
// ---------------------------------------------------------------------------

const ORACLE: &str = "org.apache.lucene.rucene.codec.QueryOracle";
const CORE: &str = "org.apache.lucene.queryparser.flexible.core.nodes.";
const STD: &str = "org.apache.lucene.queryparser.flexible.standard.nodes.";
const FN: &str = "org.apache.lucene.queryparser.flexible.standard.nodes.intervalfn.";
const MODIFIER: &str =
    "org.apache.lucene.queryparser.flexible.core.nodes.ModifierQueryNode$Modifier";
const PROXIMITY_TYPE: &str =
    "org.apache.lucene.queryparser.flexible.core.nodes.ProximityQueryNode$Type";
const QUERY_TEXT: &str =
    "org.apache.lucene.queryparser.flexible.core.nodes.PathQueryNode$QueryText";
const NLS_CLASS: &str = "org.apache.lucene.queryparser.flexible.messages.NLS";
const MESSAGE_IMPL: &str = "org.apache.lucene.queryparser.flexible.messages.MessageImpl";
const MESSAGES: &str = "org.apache.lucene.queryparser.flexible.core.messages.QueryParserMessages";
const LOCALE: &str = "java.util.Locale";
const TIME_ZONE: &str = "java.util.TimeZone";
const CLASS: &str = "java.lang.Class";
const DECIMAL_FORMAT: &str = "java.text.DecimalFormat";
const SIMPLE_DATE_FORMAT: &str = "java.text.SimpleDateFormat";
const NUMBER_DATE_FORMAT: &str =
    "org.apache.lucene.queryparser.flexible.standard.config.NumberDateFormat";
const POINTS_CONFIG: &str = "org.apache.lucene.queryparser.flexible.standard.config.PointsConfig";
const SQP: &str = "org.apache.lucene.queryparser.flexible.standard.StandardQueryParser";
const PQP: &str = "org.apache.lucene.queryparser.flexible.precedence.PrecedenceQueryParser";
const SSP: &str = "org.apache.lucene.queryparser.flexible.standard.parser.StandardSyntaxParser";
const STD_PIPELINE: &str =
    "org.apache.lucene.queryparser.flexible.standard.processors.StandardQueryNodeProcessorPipeline";
const PRECEDENCE_PIPELINE: &str = "org.apache.lucene.queryparser.flexible.precedence.processors.PrecedenceQueryNodeProcessorPipeline";
const QTB: &str = "org.apache.lucene.queryparser.flexible.core.builders.QueryTreeBuilder";
const SQTB: &str =
    "org.apache.lucene.queryparser.flexible.standard.builders.StandardQueryTreeBuilder";
const BUILDERS: &str = "org.apache.lucene.queryparser.flexible.standard.builders.";
const SQCH: &str =
    "org.apache.lucene.queryparser.flexible.standard.config.StandardQueryConfigHandler";
const KEYS: &str = "org.apache.lucene.queryparser.flexible.standard.config.StandardQueryConfigHandler$ConfigurationKeys";
const OPERATOR: &str =
    "org.apache.lucene.queryparser.flexible.standard.config.StandardQueryConfigHandler$Operator";
const FIELD_CONFIG: &str = "org.apache.lucene.queryparser.flexible.core.config.FieldConfig";
const RESOLUTION: &str = "org.apache.lucene.document.DateTools$Resolution";
const MTQ: &str = "org.apache.lucene.search.MultiTermQuery";
const STANDARD: &str = "org.apache.lucene.analysis.standard.StandardAnalyzer";
const CHAR_ARRAY_SET: &str = "org.apache.lucene.analysis.CharArraySet";
const HASH_MAP: &str = "java.util.HashMap";

// ---------------------------------------------------------------------------
// The Rust side of the QueryOracle flexible helpers
// ---------------------------------------------------------------------------

/// `QueryOracle.shown`: the value, or `!<class>: <message>` of what it
/// throws.
fn shown(f: impl FnOnce() -> Result<String>) -> String {
    match query_oracle::stage(f) {
        Ok(s) => s,
        Err(t) => format!("!{}: {}", t.class_name(), t.message().unwrap_or("null")),
    }
}

fn same(a: &QueryNodeRef, b: &QueryNodeRef) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

/// `QueryOracle.collectNodes`.
fn collect_nodes(n: &QueryNodeRef, out: &mut Vec<QueryNodeRef>) {
    out.push(Arc::clone(n));
    if let Some(children) = n.get_children() {
        for c in &children {
            collect_nodes(c, out);
        }
    }
}

/// `QueryOracle.dumpNodes`.
fn dump_nodes(
    b: &mut String,
    n: &QueryNodeRef,
    from: Option<&QueryNodeRef>,
    depth: usize,
    all: &[QueryNodeRef],
) {
    let p = match n.get_parent() {
        None => "null".to_string(),
        Some(parent) if from.is_some_and(|f| same(f, &parent)) => "^".to_string(),
        Some(parent) => match all.iter().position(|x| same(x, &parent)) {
            Some(i) => format!("#{i}"),
            None => "?".to_string(),
        },
    };
    let mut tags: Vec<String> = n.get_tag_map().keys().cloned().collect();
    tags.sort();
    b.push_str(&format!(
        "{depth} {} leaf={} parent={p} tags=[{}] ts={} qs={}\n",
        n.java_class().get_name(),
        n.is_leaf(),
        tags.join(", "),
        shown(|| n.to_java_string()),
        shown(|| Ok(n
            .to_query_string(&EscapeQuerySyntaxImpl::new())?
            .to_string())),
    ));
    if let Some(children) = n.get_children() {
        for c in &children {
            dump_nodes(b, c, Some(n), depth + 1, all);
        }
    }
}

/// `QueryOracle.maskNodes`.
fn mask_nodes(mut s: String, all: &[QueryNodeRef]) -> String {
    for (i, n) in all.iter().enumerate() {
        let name = n.java_class().get_name();
        s = s.replace(
            &n.query_node_impl().object_to_string(),
            &format!("{name}@#{i}"),
        );
    }
    s
}

/// `QueryOracle.treeDump`.
fn tree_dump(root: &QueryNodeRef) -> String {
    let mut all = Vec::new();
    collect_nodes(root, &mut all);
    let mut b = String::new();
    dump_nodes(&mut b, root, None, 0, &all);
    mask_nodes(b, &all)
}

/// `QueryOracle.nodeDump`.
fn node_dump(root: &QueryNodeRef) -> String {
    let mut all = Vec::new();
    collect_nodes(root, &mut all);
    let clone = query_oracle::stage(|| root.clone_tree());
    if let Ok(c) = &clone {
        collect_nodes(c, &mut all);
    }
    let mut b = String::new();
    dump_nodes(&mut b, root, None, 0, &all);
    match &clone {
        Err(t) => b.push_str(&format!(
            "clone !{}: {}\n",
            t.class_name(),
            t.message().unwrap_or("null")
        )),
        Ok(c) => {
            b.push_str(if same(c, root) {
                "clone same\n"
            } else {
                "clone new\n"
            });
            dump_nodes(&mut b, c, None, 0, &all);
        }
    }
    mask_nodes(b, &all)
}

/// `QueryOracle.intervalDump`.
fn interval_dump(f: &Arc<dyn IntervalFunction>, field: &str, analyzer: &dyn Analyzer) -> String {
    let mut b = format!("fn={}", f.to_java_string());
    match query_oracle::stage(|| f.to_interval_source(field, analyzer)) {
        Err(t) => {
            b.push_str(&format!(
                " src=!{}: {}",
                t.class_name(),
                t.message().unwrap_or("null")
            ));
        }
        Ok(src) => {
            b.push_str(&format!(" src={}", shown(|| Ok(src.to_string()))));
            b.push_str(&format!(
                " min={}",
                shown(|| Ok(src.min_extent().to_string()))
            ));
        }
    }
    b
}

/// The `QueryNodeProcessorPipeline` a processor value is.
fn pipeline_of(p: &Arc<dyn QueryNodeProcessor>) -> &QueryNodeProcessorPipeline {
    let any = p.as_any();
    if let Some(s) = any.downcast_ref::<StandardQueryNodeProcessorPipeline>() {
        return s.query_node_processor_pipeline();
    }
    if let Some(s) = any.downcast_ref::<PrecedenceQueryNodeProcessorPipeline>() {
        return s
            .standard_query_node_processor_pipeline()
            .query_node_processor_pipeline();
    }
    if let Some(s) = any.downcast_ref::<QueryNodeProcessorPipeline>() {
        return s;
    }
    harness_fail("the processor is no pipeline".to_string())
}

/// `QueryOracle.processTrace`.
fn process_trace(pipeline: &QueryNodeProcessorPipeline, mut tree: QueryNodeRef) -> String {
    let mut b = String::new();
    for (i, proc) in pipeline.iterator().enumerate() {
        match query_oracle::stage(|| proc.process(Arc::clone(&tree))) {
            Err(t) => {
                b.push_str(&format!(
                    "after {i} !{}: {}\n",
                    t.class_name(),
                    t.message().unwrap_or("null")
                ));
                return b;
            }
            Ok(next) => tree = next,
        }
        b.push_str(&format!("after {i}\n"));
        b.push_str(&tree_dump(&tree));
    }
    b
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

fn node(v: &Value) -> QueryNodeRef {
    v.get::<QueryNodeRef>()
}

fn opt_node(v: &Value) -> Option<QueryNodeRef> {
    match v {
        Value::Null => None,
        v => Some(node(v)),
    }
}

fn opt_nodes(v: &Value) -> Option<Vec<QueryNodeRef>> {
    match v {
        Value::Null => None,
        v => Some(v.as_values().iter().map(node).collect()),
    }
}

fn opt_cs(v: &Value) -> Option<CharSequence> {
    v.as_opt_str().map(CharSequence::from)
}

fn func(v: &Value) -> Option<Arc<dyn IntervalFunction>> {
    match v {
        Value::Null => None,
        v => Some(v.get::<Arc<dyn IntervalFunction>>()),
    }
}

fn funcs(v: &Value) -> Option<Vec<Arc<dyn IntervalFunction>>> {
    match v {
        Value::Null => None,
        v => Some(
            v.as_values()
                .iter()
                .map(|f| f.get::<Arc<dyn IntervalFunction>>())
                .collect(),
        ),
    }
}

fn opt_int(v: &Value) -> Option<i32> {
    match v {
        Value::Null => None,
        v => Some(v.as_int()),
    }
}

fn number(v: &Value) -> Option<JavaNumber> {
    match v {
        Value::Null => None,
        Value::Int(x) => Some(JavaNumber::Integer(*x)),
        Value::Long(x) => Some(JavaNumber::Long(*x)),
        Value::Float(x) => Some(JavaNumber::Float(*x)),
        Value::Double(x) => Some(JavaNumber::Double(*x)),
        other => harness_fail(format!("expected a Number, found {other:?}")),
    }
}

fn number_value(n: JavaNumber) -> Value {
    match n {
        JavaNumber::Integer(x) => Value::Int(x),
        JavaNumber::Long(x) => Value::Long(x),
        JavaNumber::Float(x) => Value::Float(x),
        JavaNumber::Double(x) => Value::Double(x),
    }
}

fn number_format(v: &Value) -> Option<Arc<dyn NumberFormat>> {
    match v {
        Value::Null => None,
        v => Some(v.get::<Arc<dyn NumberFormat>>()),
    }
}

fn opt_analyzer(v: &Value) -> Option<Arc<dyn Analyzer>> {
    match v {
        Value::Null => None,
        v => Some(v.get::<Arc<dyn Analyzer>>()),
    }
}

fn node_value(n: QueryNodeRef) -> Value {
    let class = n.java_class().get_name();
    Value::object(class, n)
}

fn java_object(v: &Value) -> JavaObject {
    match v {
        Value::Null => JavaObject::Null,
        Value::Str(s) => JavaObject::String(s.clone()),
        Value::Int(x) => JavaObject::Number(JavaNumber::Integer(*x)),
        Value::Long(x) => JavaObject::Number(JavaNumber::Long(*x)),
        Value::Float(x) => JavaObject::Number(JavaNumber::Float(*x)),
        Value::Double(x) => JavaObject::Number(JavaNumber::Double(*x)),
        Value::Char(c) => JavaObject::character(*c),
        Value::Bool(b) => JavaObject::Other(b.to_string()),
        other => harness_fail(format!("no Java object for {other:?}")),
    }
}

/// The node classes a test names by `Class.forName`, and the number classes.
fn class_for_name(name: &str) -> &'static JavaClass {
    let classes: [&'static JavaClass; 14] = [
        &FIELD_QUERY_NODE_CLASS,
        &BOOLEAN_QUERY_NODE_CLASS,
        &GROUP_QUERY_NODE_CLASS,
        &BOOST_QUERY_NODE_CLASS,
        &MODIFIER_QUERY_NODE_CLASS,
        &MATCH_ALL_DOCS_QUERY_NODE_CLASS,
        &MATCH_NO_DOCS_QUERY_NODE_CLASS,
        &FUZZY_QUERY_NODE_CLASS,
        &SLOP_QUERY_NODE_CLASS,
        &INTEGER,
        &LONG,
        &FLOAT,
        &DOUBLE,
        &SHORT,
    ];
    classes
        .into_iter()
        .find(|c| c.get_name() == name)
        .unwrap_or_else(|| harness_fail(format!("no class {name} bound")))
}

/// The `QueryParserMessages` keys, in the order the class declares them.
const MESSAGE_KEYS: [&str; 19] = [
    "INVALID_SYNTAX",
    "INVALID_SYNTAX_CANNOT_PARSE",
    "INVALID_SYNTAX_FUZZY_LIMITS",
    "INVALID_SYNTAX_FUZZY_EDITS",
    "INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION",
    "INVALID_SYNTAX_ESCAPE_CHARACTER",
    "INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE",
    "NODE_ACTION_NOT_SUPPORTED",
    "PARAMETER_VALUE_NOT_SUPPORTED",
    "LUCENE_QUERY_CONVERSION_ERROR",
    "EMPTY_MESSAGE",
    "WILDCARD_NOT_SUPPORTED",
    "TOO_MANY_BOOLEAN_CLAUSES",
    "LEADING_WILDCARD_NOT_ALLOWED",
    "COULD_NOT_PARSE_NUMBER",
    "NUMBER_CLASS_NOT_SUPPORTED_BY_NUMERIC_RANGE_QUERY",
    "UNSUPPORTED_NUMERIC_DATA_TYPE",
    "NUMERIC_CANNOT_BE_EMPTY",
    "ANALYZER_REQUIRED",
];

fn message_field(name: &str) -> &'static NlsField {
    match name {
        "INVALID_SYNTAX" => QueryParserMessages::INVALID_SYNTAX,
        "INVALID_SYNTAX_CANNOT_PARSE" => QueryParserMessages::INVALID_SYNTAX_CANNOT_PARSE,
        "INVALID_SYNTAX_FUZZY_LIMITS" => QueryParserMessages::INVALID_SYNTAX_FUZZY_LIMITS,
        "INVALID_SYNTAX_FUZZY_EDITS" => QueryParserMessages::INVALID_SYNTAX_FUZZY_EDITS,
        "INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION" => {
            QueryParserMessages::INVALID_SYNTAX_ESCAPE_UNICODE_TRUNCATION
        }
        "INVALID_SYNTAX_ESCAPE_CHARACTER" => QueryParserMessages::INVALID_SYNTAX_ESCAPE_CHARACTER,
        "INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE" => {
            QueryParserMessages::INVALID_SYNTAX_ESCAPE_NONE_HEX_UNICODE
        }
        "NODE_ACTION_NOT_SUPPORTED" => QueryParserMessages::NODE_ACTION_NOT_SUPPORTED,
        "PARAMETER_VALUE_NOT_SUPPORTED" => QueryParserMessages::PARAMETER_VALUE_NOT_SUPPORTED,
        "LUCENE_QUERY_CONVERSION_ERROR" => QueryParserMessages::LUCENE_QUERY_CONVERSION_ERROR,
        "EMPTY_MESSAGE" => QueryParserMessages::EMPTY_MESSAGE,
        "WILDCARD_NOT_SUPPORTED" => QueryParserMessages::WILDCARD_NOT_SUPPORTED,
        "TOO_MANY_BOOLEAN_CLAUSES" => QueryParserMessages::TOO_MANY_BOOLEAN_CLAUSES,
        "LEADING_WILDCARD_NOT_ALLOWED" => QueryParserMessages::LEADING_WILDCARD_NOT_ALLOWED,
        "COULD_NOT_PARSE_NUMBER" => QueryParserMessages::COULD_NOT_PARSE_NUMBER,
        "NUMBER_CLASS_NOT_SUPPORTED_BY_NUMERIC_RANGE_QUERY" => {
            QueryParserMessages::NUMBER_CLASS_NOT_SUPPORTED_BY_NUMERIC_RANGE_QUERY
        }
        "UNSUPPORTED_NUMERIC_DATA_TYPE" => QueryParserMessages::UNSUPPORTED_NUMERIC_DATA_TYPE,
        "NUMERIC_CANNOT_BE_EMPTY" => QueryParserMessages::NUMERIC_CANNOT_BE_EMPTY,
        "ANALYZER_REQUIRED" => QueryParserMessages::ANALYZER_REQUIRED,
        other => harness_fail(format!("no message key {other}")),
    }
}

/// A `java.util.HashMap` the specs fill: typed by the first value put in
/// it, or by the setter it is handed to (Java's map is untyped and held by
/// reference, so later puts reach the parser).
#[derive(Default)]
struct JMap {
    boosts: Option<FieldBoostMap>,
    dates: Option<FieldDateResolutionMap>,
    points: Option<PointsConfigMap>,
}

impl JMap {
    fn boosts(&mut self) -> FieldBoostMap {
        Arc::clone(
            self.boosts
                .get_or_insert_with(|| Arc::new(RwLock::new(HashMap::new()))),
        )
    }

    fn dates(&mut self) -> FieldDateResolutionMap {
        Arc::clone(
            self.dates
                .get_or_insert_with(|| Arc::new(RwLock::new(HashMap::new()))),
        )
    }

    fn points(&mut self) -> PointsConfigMap {
        Arc::clone(
            self.points
                .get_or_insert_with(|| Arc::new(RwLock::new(HashMap::new()))),
        )
    }
}

/// A key of `StandardQueryConfigHandler.ConfigurationKeys`.
#[derive(Clone, Copy, Debug)]
enum Key {
    Boost,
    DateResolution,
    PointsConfig,
}

/// The parsers of the specs: a `StandardQueryParser` or one of its
/// subclasses.
trait Sqp: 'static {
    fn sqp(&mut self) -> &mut StandardQueryParser;
}

impl Sqp for StandardQueryParser {
    fn sqp(&mut self) -> &mut StandardQueryParser {
        self
    }
}

impl Sqp for PrecedenceQueryParser {
    fn sqp(&mut self) -> &mut StandardQueryParser {
        self
    }
}

/// The members of `StandardQueryParser` the specs call, on receivers of
/// class `class` held as `T`.
fn register_parser<T: Sqp>(r: &mut Registry, class: &'static str) {
    r.call_fn(class, "parse", 2, |p, a| {
        let input = a[0].as_str().to_string();
        let field = a[1].as_opt_str().map(str::to_string);
        let q = p.with_mut(|p: &mut T| p.sqp().parse(&input, field.as_deref()))?;
        Ok(q.map_or(Value::Null, Value::query))
    });
    r.call_fn(class, "setAnalyzer", 1, |p, a| {
        let v = opt_analyzer(&a[0]);
        p.with_mut(|p: &mut T| p.sqp().set_analyzer(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setDefaultOperator", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<Operator>()),
        };
        p.with_mut(|p: &mut T| p.sqp().set_default_operator(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setAllowLeadingWildcard", 1, |p, a| {
        let v = a[0].as_bool();
        p.with_mut(|p: &mut T| p.sqp().set_allow_leading_wildcard(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setEnablePositionIncrements", 1, |p, a| {
        let v = a[0].as_bool();
        p.with_mut(|p: &mut T| p.sqp().set_enable_position_increments(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setPhraseSlop", 1, |p, a| {
        let v = a[0].as_int();
        p.with_mut(|p: &mut T| p.sqp().set_phrase_slop(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setFuzzyMinSim", 1, |p, a| {
        let v = a[0].as_float();
        p.with_mut(|p: &mut T| p.sqp().set_fuzzy_min_sim(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setFuzzyPrefixLength", 1, |p, a| {
        let v = a[0].as_int();
        p.with_mut(|p: &mut T| p.sqp().set_fuzzy_prefix_length(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setLocale", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<Locale>()),
        };
        p.with_mut(|p: &mut T| p.sqp().set_locale(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setTimeZone", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<TimeZone>()),
        };
        p.with_mut(|p: &mut T| p.sqp().set_time_zone(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setDateResolution", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<Resolution>()),
        };
        p.with_mut(|p: &mut T| CommonQueryParserConfiguration::set_date_resolution(p.sqp(), v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setMultiTermRewriteMethod", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.get::<Arc<dyn RewriteMethod>>()),
        };
        p.with_mut(|p: &mut T| p.sqp().set_multi_term_rewrite_method(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setMultiFields", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(Arc::new(RwLock::new(
                v.as_values().iter().map(opt_cs).collect::<Vec<_>>(),
            ))),
        };
        p.with_mut(|p: &mut T| p.sqp().set_multi_fields(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setFieldsBoost", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.with_mut(|m: &mut JMap| m.boosts())),
        };
        p.with_mut(|p: &mut T| p.sqp().set_fields_boost(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setDateResolutionMap", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.with_mut(|m: &mut JMap| m.dates())),
        };
        p.with_mut(|p: &mut T| p.sqp().set_date_resolution_map(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "setPointsConfigMap", 1, |p, a| {
        let v = match &a[0] {
            Value::Null => None,
            v => Some(v.with_mut(|m: &mut JMap| m.points())),
        };
        p.with_mut(|p: &mut T| p.sqp().set_points_config_map(v))?;
        Ok(Value::Null)
    });
    r.call_fn(class, "getQueryNodeProcessor", 0, |p, _| {
        let proc = p.with_mut(|p: &mut T| p.sqp().get_query_node_processor());
        Ok(match proc {
            None => Value::Null,
            Some(proc) => {
                let class = if proc.as_any().is::<PrecedenceQueryNodeProcessorPipeline>() {
                    PRECEDENCE_PIPELINE
                } else {
                    STD_PIPELINE
                };
                Value::object(class, proc)
            }
        })
    });
    r.call_fn(class, "getSyntaxParser", 0, |p, _| {
        let sp = p.with_mut(|p: &mut T| p.sqp().get_syntax_parser());
        Ok(sp.map_or(Value::Null, |sp| Value::object(SSP, sp)))
    });
    r.call_fn(class, "getQueryBuilder", 0, |p, _| {
        let b = p.with_mut(|p: &mut T| p.sqp().get_query_builder());
        Ok(b.map_or(Value::Null, |b| Value::object(SQTB, b)))
    });
    r.call_fn(class, "getQueryConfigHandler", 0, |p, _| {
        let h = p.with_mut(|p: &mut T| p.sqp().get_query_config_handler());
        Ok(h.map_or(Value::Null, |h| Value::object(SQCH, h)))
    });
}

/// The standard node builders a `QueryTreeBuilder` of the specs is given.
fn builder(name: &str) -> Arc<dyn QueryBuilder> {
    match name {
        "FieldQueryNodeBuilder" => Arc::new(FieldQueryNodeBuilder::new()),
        "BooleanQueryNodeBuilder" => Arc::new(BooleanQueryNodeBuilder::new()),
        "GroupQueryNodeBuilder" => Arc::new(GroupQueryNodeBuilder::new()),
        "BoostQueryNodeBuilder" => Arc::new(BoostQueryNodeBuilder::new()),
        "ModifierQueryNodeBuilder" => Arc::new(ModifierQueryNodeBuilder::new()),
        "MatchAllDocsQueryNodeBuilder" => Arc::new(MatchAllDocsQueryNodeBuilder::new()),
        "MatchNoDocsQueryNodeBuilder" => Arc::new(MatchNoDocsQueryNodeBuilder::new()),
        "FuzzyQueryNodeBuilder" => Arc::new(FuzzyQueryNodeBuilder::new()),
        "SlopQueryNodeBuilder" => Arc::new(SlopQueryNodeBuilder::new()),
        other => harness_fail(format!("no builder {other} bound")),
    }
}

const BUILDER_NAMES: [&str; 9] = [
    "FieldQueryNodeBuilder",
    "BooleanQueryNodeBuilder",
    "GroupQueryNodeBuilder",
    "BoostQueryNodeBuilder",
    "ModifierQueryNodeBuilder",
    "MatchAllDocsQueryNodeBuilder",
    "MatchNoDocsQueryNodeBuilder",
    "FuzzyQueryNodeBuilder",
    "SlopQueryNodeBuilder",
];

type FnCtor = fn(&[Value]) -> Result<Arc<dyn IntervalFunction>>;

/// [`Registry::core`] with the classes of the flexible framework the specs
/// name.
fn registry() -> Registry {
    let mut r = Registry::core();

    // QueryOracle helpers
    r.static_fn(ORACLE, "nodeDump", 1, |a| {
        Ok(Value::Str(node_dump(&node(&a[0]))))
    });
    r.static_fn(ORACLE, "treeDump", 1, |a| {
        Ok(Value::Str(tree_dump(&node(&a[0]))))
    });
    r.static_fn(ORACLE, "withChildren", 2, |a| {
        let n = node(&a[0]);
        n.add_all(opt_nodes(&a[1]).ok_or_else(LuceneError::null_pointer)?)?;
        Ok(node_value(n))
    });
    r.static_fn(ORACLE, "intervalDump", 3, |a| {
        let f = func(&a[0]).expect("a function");
        let analyzer = opt_analyzer(&a[2]).expect("an analyzer");
        Ok(Value::Str(interval_dump(&f, a[1].as_str(), &*analyzer)))
    });
    r.static_fn(ORACLE, "processTrace", 2, |a| {
        let p = a[0].get::<Arc<dyn QueryNodeProcessor>>();
        Ok(Value::Str(process_trace(pipeline_of(&p), node(&a[1]))))
    });

    // core nodes
    let c = |n: &str| format!("{CORE}{n}");
    let s = |n: &str| format!("{STD}{n}");
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    r.new_fn(leak(c("FieldQueryNode")), 4, |a| {
        Ok(node_value(FieldQueryNode::new(
            opt_cs(&a[0]),
            opt_cs(&a[1]),
            a[2].as_int(),
            a[3].as_int(),
        )))
    });
    r.new_fn(leak(c("QuotedFieldQueryNode")), 4, |a| {
        Ok(node_value(QuotedFieldQueryNode::new(
            opt_cs(&a[0]),
            opt_cs(&a[1]),
            a[2].as_int(),
            a[3].as_int(),
        )))
    });
    r.new_fn(leak(c("FuzzyQueryNode")), 5, |a| {
        Ok(node_value(FuzzyQueryNode::new(
            opt_cs(&a[0]),
            opt_cs(&a[1]),
            a[2].as_float(),
            a[3].as_int(),
            a[4].as_int(),
        )))
    });
    r.new_fn(leak(c("BooleanQueryNode")), 1, |a| {
        Ok(node_value(BooleanQueryNode::new(opt_nodes(&a[0]))?))
    });
    r.new_fn(leak(c("AndQueryNode")), 1, |a| {
        Ok(node_value(AndQueryNode::new(opt_nodes(&a[0]))?))
    });
    r.new_fn(leak(c("OrQueryNode")), 1, |a| {
        Ok(node_value(OrQueryNode::new(opt_nodes(&a[0]))?))
    });
    r.new_fn(leak(c("AnyQueryNode")), 3, |a| {
        Ok(node_value(AnyQueryNode::new(
            opt_nodes(&a[0]),
            opt_cs(&a[1]),
            a[2].as_int(),
        )?))
    });
    r.new_fn(leak(c("BoostQueryNode")), 2, |a| {
        Ok(node_value(BoostQueryNode::new(
            opt_node(&a[0]),
            a[1].as_float(),
        )?))
    });
    r.new_fn(leak(c("GroupQueryNode")), 1, |a| {
        Ok(node_value(GroupQueryNode::new(opt_node(&a[0]))?))
    });
    r.new_fn(leak(c("ModifierQueryNode")), 2, |a| {
        Ok(node_value(ModifierQueryNode::new(
            opt_node(&a[0]),
            a[1].get::<Modifier>(),
        )?))
    });
    r.new_fn(leak(c("SlopQueryNode")), 2, |a| {
        Ok(node_value(SlopQueryNode::new(
            opt_node(&a[0]),
            a[1].as_int(),
        )?))
    });
    r.new_fn(leak(c("PhraseSlopQueryNode")), 2, |a| {
        Ok(node_value(PhraseSlopQueryNode::new(
            opt_node(&a[0]),
            a[1].as_int(),
        )?))
    });
    r.new_fn(leak(c("ProximityQueryNode")), 5, |a| {
        Ok(node_value(ProximityQueryNode::new(
            opt_nodes(&a[0]),
            opt_cs(&a[1]),
            a[2].get::<ProximityType>(),
            a[3].as_int(),
            a[4].as_bool(),
        )?))
    });
    r.new_fn(leak(c("ProximityQueryNode")), 4, |a| {
        Ok(node_value(ProximityQueryNode::without_distance(
            opt_nodes(&a[0]),
            opt_cs(&a[1]),
            a[2].get::<ProximityType>(),
            a[3].as_bool(),
        )?))
    });
    r.new_fn(leak(c("TokenizedPhraseQueryNode")), 0, |_| {
        Ok(node_value(TokenizedPhraseQueryNode::new()))
    });
    r.new_fn(leak(c("MatchAllDocsQueryNode")), 0, |_| {
        Ok(node_value(MatchAllDocsQueryNode::new()))
    });
    r.new_fn(leak(c("MatchNoDocsQueryNode")), 0, |_| {
        Ok(node_value(MatchNoDocsQueryNode::new()))
    });
    r.new_fn(leak(c("NoTokenFoundQueryNode")), 0, |_| {
        Ok(node_value(NoTokenFoundQueryNode::new()))
    });
    r.new_fn(leak(c("DeletedQueryNode")), 0, |_| {
        Ok(node_value(DeletedQueryNode::new()))
    });
    r.new_fn(leak(c("OpaqueQueryNode")), 2, |a| {
        Ok(node_value(OpaqueQueryNode::new(
            opt_cs(&a[0]),
            opt_cs(&a[1]),
        )))
    });
    r.new_fn(QUERY_TEXT, 3, |a| {
        Ok(Value::object(
            QUERY_TEXT,
            QueryText::new(opt_cs(&a[0]), a[1].as_int(), a[2].as_int()),
        ))
    });
    r.new_fn(leak(c("PathQueryNode")), 1, |a| {
        let elems = match &a[0] {
            Value::Null => None,
            v => Some(v.as_values().iter().map(|e| e.get::<QueryText>()).collect()),
        };
        Ok(node_value(PathQueryNode::new(elems)?))
    });
    for (name, m) in [
        ("MOD_NONE", Modifier::ModNone),
        ("MOD_NOT", Modifier::ModNot),
        ("MOD_REQ", Modifier::ModReq),
    ] {
        r.field_fn(MODIFIER, name, move || Value::object(MODIFIER, m));
    }
    for (name, t) in [
        ("PARAGRAPH", ProximityType::Paragraph),
        ("SENTENCE", ProximityType::Sentence),
        ("NUMBER", ProximityType::Number),
    ] {
        r.field_fn(PROXIMITY_TYPE, name, move || {
            Value::object(PROXIMITY_TYPE, t)
        });
    }

    // standard nodes
    r.new_fn(leak(s("BooleanModifierNode")), 2, |a| {
        Ok(node_value(BooleanModifierNode::new(
            opt_node(&a[0]),
            a[1].get::<Modifier>(),
        )?))
    });
    r.new_fn(leak(s("MinShouldMatchNode")), 2, |a| {
        let group = opt_node(&a[1]).map(|g| {
            let any: Arc<dyn std::any::Any + Send + Sync> = g;
            any.downcast::<GroupQueryNode>()
                .unwrap_or_else(|_| harness_fail("not a GroupQueryNode".to_string()))
        });
        Ok(node_value(MinShouldMatchNode::new(a[0].as_int(), group)?))
    });
    r.new_fn(leak(s("MultiPhraseQueryNode")), 0, |_| {
        Ok(node_value(MultiPhraseQueryNode::new()))
    });
    r.new_fn(leak(s("PointQueryNode")), 3, |a| {
        Ok(node_value(PointQueryNode::new(
            opt_cs(&a[0]),
            number(&a[1]),
            number_format(&a[2]),
        )))
    });
    r.new_fn(leak(s("PointRangeQueryNode")), 5, |a| {
        let point = |v: &Value| {
            opt_node(v).map(|g| {
                let any: Arc<dyn std::any::Any + Send + Sync> = g;
                any.downcast::<PointQueryNode>()
                    .unwrap_or_else(|_| harness_fail("not a PointQueryNode".to_string()))
            })
        };
        let config = match &a[4] {
            Value::Null => None,
            v => Some(v.get::<Arc<PointsConfig>>()),
        };
        Ok(node_value(PointRangeQueryNode::new(
            point(&a[0]),
            point(&a[1]),
            a[2].as_bool(),
            a[3].as_bool(),
            config,
        )?))
    });
    r.new_fn(leak(s("PrefixWildcardQueryNode")), 4, |a| {
        Ok(node_value(PrefixWildcardQueryNode::new(
            opt_cs(&a[0]),
            opt_cs(&a[1]),
            a[2].as_int(),
            a[3].as_int(),
        )))
    });
    r.new_fn(leak(s("PrefixWildcardQueryNode")), 1, |a| {
        let n = node(&a[0]);
        Ok(node_value(PrefixWildcardQueryNode::from_field_query_node(
            n.as_field_query_node().expect("a FieldQueryNode"),
        )))
    });
    r.new_fn(leak(s("WildcardQueryNode")), 4, |a| {
        Ok(node_value(WildcardQueryNode::new(
            opt_cs(&a[0]),
            opt_cs(&a[1]),
            a[2].as_int(),
            a[3].as_int(),
        )))
    });
    r.new_fn(leak(s("WildcardQueryNode")), 1, |a| {
        let n = node(&a[0]);
        Ok(node_value(WildcardQueryNode::from_field_query_node(
            n.as_field_query_node().expect("a FieldQueryNode"),
        )))
    });
    r.new_fn(leak(s("RegexpQueryNode")), 2, |a| {
        let text = opt_cs(&a[1]).expect("a text");
        Ok(node_value(RegexpQueryNode::with_text(
            opt_cs(&a[0]),
            &text,
        )?))
    });
    r.new_fn(leak(s("RegexpQueryNode")), 4, |a| {
        let text = opt_cs(&a[1]).expect("a text");
        Ok(node_value(RegexpQueryNode::new(
            opt_cs(&a[0]),
            &text,
            a[2].as_int() as usize,
            a[3].as_int() as usize,
        )?))
    });
    r.new_fn(leak(s("SynonymQueryNode")), 1, |a| {
        Ok(node_value(SynonymQueryNode::new(opt_nodes(&a[0]))?))
    });
    r.new_fn(leak(s("TermRangeQueryNode")), 4, |a| {
        Ok(node_value(TermRangeQueryNode::new(
            opt_node(&a[0]),
            opt_node(&a[1]),
            a[2].as_bool(),
            a[3].as_bool(),
        )?))
    });
    r.new_fn(leak(s("IntervalQueryNode")), 2, |a| {
        Ok(node_value(IntervalQueryNode::new(
            a[0].as_opt_str().map(str::to_string),
            func(&a[1]),
        )?))
    });

    // interval functions
    let fns: [(&str, usize, FnCtor); 22] = [
        ("After", 2, |a| {
            Ok(Arc::new(After::new(func(&a[0]), func(&a[1]))?))
        }),
        ("AnalyzedText", 1, |a| {
            Ok(Arc::new(AnalyzedText::new(a[0].as_str())))
        }),
        ("AtLeast", 2, |a| {
            Ok(Arc::new(AtLeast::new(a[0].as_int(), funcs(&a[1]))?))
        }),
        ("Before", 2, |a| {
            Ok(Arc::new(Before::new(func(&a[0]), func(&a[1]))?))
        }),
        ("ContainedBy", 2, |a| {
            Ok(Arc::new(ContainedBy::new(func(&a[0]), func(&a[1]))?))
        }),
        ("Containing", 2, |a| {
            Ok(Arc::new(Containing::new(func(&a[0]), func(&a[1]))?))
        }),
        ("Extend", 3, |a| {
            Ok(Arc::new(Extend::new(
                func(&a[0]),
                a[1].as_int(),
                a[2].as_int(),
            )?))
        }),
        ("FuzzyTerm", 3, |a| {
            Ok(Arc::new(FuzzyTerm::new(
                a[0].as_str(),
                opt_int(&a[1]),
                opt_int(&a[2]),
            )))
        }),
        ("MaxGaps", 2, |a| {
            Ok(Arc::new(MaxGaps::new(a[0].as_int(), func(&a[1]))?))
        }),
        ("MaxWidth", 2, |a| {
            Ok(Arc::new(MaxWidth::new(a[0].as_int(), func(&a[1]))?))
        }),
        ("NonOverlapping", 2, |a| {
            Ok(Arc::new(NonOverlapping::new(func(&a[0]), func(&a[1]))?))
        }),
        ("NotContainedBy", 2, |a| {
            Ok(Arc::new(NotContainedBy::new(func(&a[0]), func(&a[1]))?))
        }),
        ("NotContaining", 2, |a| {
            Ok(Arc::new(NotContaining::new(func(&a[0]), func(&a[1]))?))
        }),
        ("NotWithin", 3, |a| {
            Ok(Arc::new(NotWithin::new(
                func(&a[0]),
                a[1].as_int(),
                func(&a[2]),
            )?))
        }),
        ("Or", 1, |a| Ok(Arc::new(Or::new(funcs(&a[0]))?))),
        ("Ordered", 1, |a| Ok(Arc::new(Ordered::new(funcs(&a[0]))?))),
        ("Overlapping", 2, |a| {
            Ok(Arc::new(Overlapping::new(func(&a[0]), func(&a[1]))?))
        }),
        ("Phrase", 1, |a| Ok(Arc::new(Phrase::new(funcs(&a[0]))?))),
        ("Unordered", 1, |a| {
            Ok(Arc::new(Unordered::new(funcs(&a[0]))?))
        }),
        ("UnorderedNoOverlaps", 2, |a| {
            Ok(Arc::new(UnorderedNoOverlaps::new(
                func(&a[0]),
                func(&a[1]),
            )?))
        }),
        ("Wildcard", 2, |a| {
            Ok(Arc::new(Wildcard::new(a[0].as_str(), a[1].as_int())))
        }),
        ("Within", 3, |a| {
            Ok(Arc::new(Within::new(
                func(&a[0]),
                a[1].as_int(),
                func(&a[2]),
            )?))
        }),
    ];
    for (name, arity, ctor) in fns {
        let class: &'static str = leak(format!("{FN}{name}"));
        r.new_fn(class, arity, move |a| Ok(Value::object(class, ctor(a)?)));
    }

    // messages
    for key in MESSAGE_KEYS {
        r.field_fn(MESSAGES, key, move || {
            message_field(key).get().map_or(Value::Null, Value::Str)
        });
    }
    let args_of = |v: &Value| -> Option<Vec<JavaObject>> {
        match v {
            Value::Null => None,
            v => Some(v.as_values().iter().map(java_object).collect()),
        }
    };
    r.static_fn(NLS_CLASS, "getLocalizedMessage", 3, move |a| {
        let args = args_of(&a[2]);
        Ok(Value::Str(NLS::get_localized_message_with_locale_and_args(
            a[0].as_opt_str(),
            &a[1].get::<Locale>(),
            args.as_deref(),
        )?))
    });
    r.static_fn(NLS_CLASS, "getLocalizedMessage", 2, |a| {
        Ok(Value::Str(NLS::get_localized_message_with_locale(
            a[0].as_opt_str(),
            &a[1].get::<Locale>(),
        )?))
    });
    r.new_fn(MESSAGE_IMPL, 1, |a| {
        Ok(Value::object(
            MESSAGE_IMPL,
            Arc::new(MessageImpl::new(a[0].as_opt_str().map(str::to_string))),
        ))
    });
    r.new_fn(MESSAGE_IMPL, 2, move |a| {
        Ok(Value::object(
            MESSAGE_IMPL,
            Arc::new(MessageImpl::with_args(
                a[0].as_opt_str().map(str::to_string),
                args_of(&a[1]),
            )),
        ))
    });
    r.call_fn(MESSAGE_IMPL, "getLocalizedMessage", 1, |m, a| {
        let m = m.get::<Arc<MessageImpl>>();
        Ok(Value::Str(m.get_localized_message_with_locale(
            &a[0].get::<Locale>(),
        )?))
    });
    r.call_fn(MESSAGE_IMPL, "toString", 0, |m, _| {
        Ok(Value::Str(m.get::<Arc<MessageImpl>>().to_java_string()?))
    });

    // JDK
    r.field_fn(LOCALE, "ROOT", || Value::object(LOCALE, Locale::root()));
    r.static_fn(LOCALE, "forLanguageTag", 1, |a| {
        Ok(Value::object(
            LOCALE,
            Locale::builder_from_language_tag(a[0].as_str())?,
        ))
    });
    r.static_fn(TIME_ZONE, "getTimeZone", 1, |a| {
        Ok(Value::object(
            "sun.util.calendar.ZoneInfo",
            TimeZone::get_time_zone(a[0].as_str()),
        ))
    });
    r.static_fn(CLASS, "forName", 1, |a| {
        Ok(Value::object(CLASS, class_for_name(a[0].as_str())))
    });
    r.call_fn(CLASS, "toString", 0, |c, _| {
        Ok(Value::Str(c.get::<&'static JavaClass>().to_java_string()))
    });
    r.static_fn("java.text.NumberFormat", "getIntegerInstance", 1, |a| {
        let f: Arc<dyn NumberFormat> = Arc::new(get_integer_instance(&a[0].get::<Locale>())?);
        Ok(Value::object(DECIMAL_FORMAT, f))
    });
    r.static_fn("java.text.NumberFormat", "getNumberInstance", 1, |a| {
        let f: Arc<dyn NumberFormat> = Arc::new(get_number_instance(&a[0].get::<Locale>())?);
        Ok(Value::object(DECIMAL_FORMAT, f))
    });
    r.static_fn("java.text.DateFormat", "getDateInstance", 2, |a| {
        let f = date_format::get_date_instance(a[0].as_int(), &a[1].get::<Locale>())?;
        Ok(Value::object(SIMPLE_DATE_FORMAT, Arc::new(f)))
    });
    r.call_fn(SIMPLE_DATE_FORMAT, "setTimeZone", 1, |f, a| {
        f.get::<Arc<SimpleDateFormat>>()
            .set_time_zone(&a[0].get::<TimeZone>());
        Ok(Value::Null)
    });
    r.new_fn(NUMBER_DATE_FORMAT, 1, |a| {
        let df: Arc<SimpleDateFormat> = a[0].get::<Arc<SimpleDateFormat>>();
        let f: Arc<dyn NumberFormat> = Arc::new(NumberDateFormat::new(df));
        Ok(Value::object(NUMBER_DATE_FORMAT, f))
    });
    for class in [DECIMAL_FORMAT, NUMBER_DATE_FORMAT] {
        r.call_fn(class, "format", 1, |f, a| {
            let n = number(&a[0]).expect("a number");
            Ok(Value::Str(f.get::<Arc<dyn NumberFormat>>().format(n)?))
        });
        r.call_fn(class, "parse", 1, |f, a| {
            Ok(number_value(
                f.get::<Arc<dyn NumberFormat>>().parse(a[0].as_str())?,
            ))
        });
    }
    r.new_fn(POINTS_CONFIG, 2, |a| {
        let type_ = match &a[1] {
            Value::Null => None,
            v => Some(v.get::<&'static JavaClass>()),
        };
        Ok(Value::object(
            POINTS_CONFIG,
            Arc::new(PointsConfig::new(number_format(&a[0]), type_)?),
        ))
    });
    r.call_fn(POINTS_CONFIG, "equals", 1, |p, a| {
        let p = p.get::<Arc<PointsConfig>>();
        Ok(Value::Bool(match &a[0] {
            Value::Null => false,
            o => p.equals(&o.get::<Arc<PointsConfig>>()),
        }))
    });
    r.call_fn(POINTS_CONFIG, "getType", 0, |p, _| {
        Ok(Value::object(
            CLASS,
            p.get::<Arc<PointsConfig>>().get_type(),
        ))
    });

    // analyzers
    r.new_fn(CHAR_ARRAY_SET, 2, |a| {
        Ok(Value::object(
            CHAR_ARRAY_SET,
            CharArraySet::from_iter(a[0].as_strings(), a[1].as_bool()),
        ))
    });
    r.new_fn(STANDARD, 1, |a| {
        Ok(Value::object(
            STANDARD,
            Arc::new(StandardAnalyzer::with_stopwords(a[0].get::<CharArraySet>()))
                as Arc<dyn Analyzer>,
        ))
    });

    // builders
    r.new_fn(QTB, 0, |_| {
        Ok(Value::object(QTB, Arc::new(QueryTreeBuilder::new())))
    });
    r.call_fn(QTB, "setBuilder", 2, |q, a| {
        let qtb = q.get::<Arc<QueryTreeBuilder>>();
        let b = a[1].get::<Arc<dyn QueryBuilder>>();
        match &a[0] {
            Value::Str(field) => qtb.set_builder_for_field(&CharSequence::from(field.as_str()), b),
            v => qtb.set_builder_for_class(v.get::<&'static JavaClass>(), b),
        }
        Ok(Value::Null)
    });
    r.call_fn(QTB, "build", 1, |q, a| {
        let obj = q.get::<Arc<QueryTreeBuilder>>().build(&node(&a[0]))?;
        Ok(match obj {
            None => Value::Null,
            Some(obj) => Value::query(query_of(&obj)?),
        })
    });
    for name in BUILDER_NAMES {
        let class: &'static str = leak(format!("{BUILDERS}{name}"));
        r.new_fn(class, 0, move |_| Ok(Value::object(class, builder(name))));
    }
    r.new_fn(SQTB, 0, |_| {
        Ok(Value::object(
            SQTB,
            Arc::new(StandardQueryTreeBuilder::new()) as Arc<dyn QueryBuilder>,
        ))
    });
    r.call_fn(SQTB, "build", 1, |b, a| {
        let obj = b.get::<Arc<dyn QueryBuilder>>().build(&node(&a[0]))?;
        Ok(match obj {
            None => Value::Null,
            Some(obj) => Value::query(query_of(&obj)?),
        })
    });

    // syntax parser and processors
    r.new_fn(SSP, 0, |_| {
        Ok(Value::object(
            SSP,
            Arc::new(Mutex::new(StandardSyntaxParser::new())) as Arc<dyn SyntaxParser>,
        ))
    });
    r.call_fn(SSP, "parse", 2, |p, a| {
        let query = CharSequence::from(a[0].as_str());
        let field = opt_cs(&a[1]);
        let n = p
            .get::<Arc<dyn SyntaxParser>>()
            .parse(&query, field.as_ref())?;
        Ok(node_value(n))
    });
    for class in [STD_PIPELINE, PRECEDENCE_PIPELINE] {
        r.call_fn(class, "process", 1, |p, a| {
            let n = p
                .get::<Arc<dyn QueryNodeProcessor>>()
                .process(node(&a[0]))?;
            Ok(node_value(n))
        });
    }

    // parsers and configuration
    r.new_fn(SQP, 0, |_| {
        Ok(Value::object(SQP, StandardQueryParser::new()))
    });
    r.new_fn(SQP, 1, |a| {
        Ok(Value::object(
            SQP,
            StandardQueryParser::with_analyzer(opt_analyzer(&a[0])),
        ))
    });
    register_parser::<StandardQueryParser>(&mut r, SQP);
    r.new_fn(PQP, 0, |_| {
        Ok(Value::object(PQP, PrecedenceQueryParser::new()?))
    });
    r.new_fn(PQP, 1, |a| {
        Ok(Value::object(
            PQP,
            PrecedenceQueryParser::with_analyzer(opt_analyzer(&a[0]))?,
        ))
    });
    register_parser::<PrecedenceQueryParser>(&mut r, PQP);
    r.new_fn(SQCH, 0, |_| {
        Ok(Value::object(SQCH, StandardQueryConfigHandler::create()))
    });
    r.call_fn(SQCH, "getFieldConfig", 1, |h, a| {
        let fc = h
            .get::<Arc<QueryConfigHandler>>()
            .get_field_config(a[0].as_opt_str())?;
        Ok(fc.map_or(Value::Null, |fc| Value::object(FIELD_CONFIG, Arc::new(fc))))
    });
    r.call_fn(FIELD_CONFIG, "get", 1, |fc, a| {
        let fc = fc.get::<Arc<FieldConfig>>();
        Ok(match a[0].get::<Key>() {
            Key::Boost => fc
                .get(ConfigurationKeys::BOOST)
                .map_or(Value::Null, Value::Float),
            Key::DateResolution => fc
                .get(ConfigurationKeys::DATE_RESOLUTION)
                .map_or(Value::Null, |r| Value::object(RESOLUTION, r)),
            Key::PointsConfig => fc
                .get(ConfigurationKeys::POINTS_CONFIG)
                .map_or(Value::Null, |p| Value::object(POINTS_CONFIG, p)),
        })
    });
    for (name, k) in [
        ("BOOST", Key::Boost),
        ("DATE_RESOLUTION", Key::DateResolution),
        ("POINTS_CONFIG", Key::PointsConfig),
    ] {
        r.field_fn(KEYS, name, move || Value::object(KEYS, k));
    }
    r.field_fn(OPERATOR, "AND", || Value::object(OPERATOR, Operator::And));
    r.field_fn(OPERATOR, "OR", || Value::object(OPERATOR, Operator::Or));
    for (name, res) in [
        ("YEAR", Resolution::YEAR),
        ("MONTH", Resolution::MONTH),
        ("DAY", Resolution::DAY),
        ("HOUR", Resolution::HOUR),
        ("MINUTE", Resolution::MINUTE),
        ("SECOND", Resolution::SECOND),
        ("MILLISECOND", Resolution::MILLISECOND),
    ] {
        r.field_fn(RESOLUTION, name, move || Value::object(RESOLUTION, res));
    }
    r.call_fn(RESOLUTION, "toString", 0, |v, _| {
        Ok(Value::Str(
            format!("{:?}", v.get::<Resolution>()).to_lowercase(),
        ))
    });
    type RewriteFn = fn() -> Arc<dyn RewriteMethod>;
    let rewrites: [(&str, RewriteFn); 4] = [
        ("CONSTANT_SCORE_REWRITE", constant_score_rewrite),
        (
            "CONSTANT_SCORE_BLENDED_REWRITE",
            constant_score_blended_rewrite,
        ),
        (
            "CONSTANT_SCORE_BOOLEAN_REWRITE",
            constant_score_boolean_rewrite,
        ),
        ("SCORING_BOOLEAN_REWRITE", scoring_boolean_rewrite),
    ];
    for (name, f) in rewrites {
        r.field_fn(MTQ, name, move || Value::object(MTQ, f()));
    }
    r.new_fn(HASH_MAP, 0, |_| {
        Ok(Value::object(HASH_MAP, JMap::default()))
    });
    r.call_fn(HASH_MAP, "put", 2, |m, a| {
        let k = a[0].as_str().to_string();
        let v = a[1].clone();
        m.with_mut(|m: &mut JMap| {
            if let Value::Float(f) = v {
                m.boosts()
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(k, f);
            } else if v.is::<Resolution>() {
                m.dates()
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(CharSequence::from(k), v.get::<Resolution>());
            } else {
                m.points()
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(k, v.get::<Arc<PointsConfig>>());
            }
        });
        // `put` returns the previous value; the specs put each key once.
        Ok(Value::Null)
    });
    r
}

// ---------------------------------------------------------------------------
// Spec builders
// ---------------------------------------------------------------------------

fn call(method: &str, receiver: &str, args: &[&str]) -> String {
    let mut all = vec![method, receiver];
    all.extend_from_slice(args);
    expr("call", &all)
}

fn static_call(class: &str, method: &str, args: &[&str]) -> String {
    let mut all = vec![class, method];
    all.extend_from_slice(args);
    expr("static", &all)
}

fn new(class: &str, args: &[&str]) -> String {
    let mut all = vec![class];
    all.extend_from_slice(args);
    expr("new", &all)
}

fn field(class: &str, name: &str) -> String {
    expr("field", &[class, name])
}

fn list(items: &[String]) -> String {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    expr("list", &refs)
}

fn locale(tag: &str) -> String {
    static_call(LOCALE, "forLanguageTag", &[&lit(tag)])
}

fn time_zone(id: &str) -> String {
    static_call(TIME_ZONE, "getTimeZone", &[&lit(id)])
}

fn class_for(name: &str) -> String {
    static_call(CLASS, "forName", &[&lit(name)])
}

fn pick<'a>(random: &mut JdkRandom, items: &[&'a str]) -> &'a str {
    items[random.next_int_bound(items.len() as i32) as usize]
}

/// Both dumps compared, after a non-vacuity check: the Java dump holds
/// records and some of them are results, not exceptions. Diagnostic aid:
/// `RUCENE_ORACLE_DUMP_DIR=<dir>` keeps both dumps as `<what>.java` and
/// `<what>.rust`.
fn assert_parity(what: &str, oracle: &ParseOracle) {
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    let lines: Vec<&str> = java.lines().collect();
    let records = lines.iter().filter(|l| l.starts_with("parse\t")).count();
    let errors = lines
        .windows(2)
        .filter(|w| w[0].starts_with("parse\t") && w[1].starts_with("error\t"))
        .count();
    assert!(records > 0, "{what}: the oracle has records");
    assert!(errors < records, "{what}: some records must give a result");
    if let Ok(dir) = std::env::var("RUCENE_ORACLE_DUMP_DIR") {
        std::fs::write(format!("{dir}/{what}.java"), &java).expect("write the Java dump");
        std::fs::write(format!("{dir}/{what}.rust"), &rust).expect("write the Rust dump");
    }
    if let Err(diff) = compare(&java, &rust) {
        panic!("{what}: parity with Lucene 10.5.0 fails:\n{diff}");
    }
}

/// A parse record of a value: `expr` evaluated (no `$input`).
fn value_record(oracle: &mut ParseOracle, id: &str, e: &str) {
    oracle.parse(id, e, &[], None);
}

/// A parse record of a query string.
fn input_record(oracle: &mut ParseOracle, id: &str, e: &str, input: &str, field: Option<&str>) {
    let units: Vec<u16> = input.encode_utf16().collect();
    oracle.parse(id, e, &units, field);
}

// ---------------------------------------------------------------------------
// #486 messages
// ---------------------------------------------------------------------------

/// A seeded `Object[]` of 0 to 3 arguments.
fn message_args(random: &mut JdkRandom) -> String {
    let n = random.next_int_bound(4);
    let mut elems: Vec<String> = Vec::new();
    for _ in 0..n {
        elems.push(match random.next_int_bound(10) {
            0 => "null".to_string(),
            1 => format!("i:{}", random.next_int()),
            2 => format!("l:{}", random.next_long()),
            3 => format!("d:{}", random.next_double() * 1.0e6 - 5.0e5),
            4 => format!("f:{}", random.next_float() * 1000.0),
            5 => pick(
                random,
                &[
                    "d:NaN",
                    "d:Infinity",
                    "d:-0.0",
                    "f:-Infinity",
                    "l:-9223372036854775808",
                ],
            )
            .to_string(),
            6 => format!("c:{:04x}", 0x41 + random.next_int_bound(26)),
            7 => format!("z:{}", random.next_boolean()),
            _ => lit(pick(
                random,
                &[
                    "XXX", "foo bar", "{0}", "'quoted'", "", "çé", "a\\b", "1,234",
                ],
            )),
        });
    }
    let mut all: Vec<&str> = vec!["java.lang.Object"];
    all.extend(elems.iter().map(String::as_str));
    expr("array", &all)
}

fn messages_oracle(locale_expr: &str, seed: i64) -> ParseOracle {
    let mut oracle = ParseOracle::new(registry());
    oracle.bind("loc", locale_expr);
    let mut random = JdkRandom::new(seed);
    let mut keys: Vec<String> = MESSAGE_KEYS.iter().map(|k| field(MESSAGES, k)).collect();
    keys.push(lit("NO_SUCH_KEY"));
    for (k, key) in keys.iter().enumerate() {
        value_record(
            &mut oracle,
            &format!("k{k}"),
            &static_call(NLS_CLASS, "getLocalizedMessage", &[key, "$loc"]),
        );
        for j in 0..4 {
            let args = message_args(&mut random);
            value_record(
                &mut oracle,
                &format!("k{k}a{j}"),
                &static_call(NLS_CLASS, "getLocalizedMessage", &[key, "$loc", &args]),
            );
            let m = new(MESSAGE_IMPL, &[key, &args]);
            value_record(
                &mut oracle,
                &format!("k{k}m{j}"),
                &call("getLocalizedMessage", &m, &["$loc"]),
            );
            value_record(
                &mut oracle,
                &format!("k{k}t{j}"),
                &call("toString", &m, &[]),
            );
        }
        value_record(
            &mut oracle,
            &format!("k{k}m"),
            &call("getLocalizedMessage", &new(MESSAGE_IMPL, &[key]), &["$loc"]),
        );
    }
    oracle
}

#[test]
fn messages_root_matches_lucene() {
    assert_parity(
        "messages_root_matches_lucene",
        &messages_oracle(&field(LOCALE, "ROOT"), 0x0486_0001),
    );
}

#[test]
fn messages_ja_matches_lucene() {
    assert_parity(
        "messages_ja_matches_lucene",
        &messages_oracle(&locale("ja"), 0x0486_0002),
    );
}

/// The test bundles of `TestNLS` are Lucene's resources byte for byte.
#[test]
fn messages_test_bundle_resources_match_lucene() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/resources/org/apache/lucene/queryparser/flexible/messages/"
    );
    for (name, expected) in [
        (
            "MessagesTestBundle.properties",
            "cc645b3e480f3acd05da5be0eda4b7a676a7b8c7",
        ),
        (
            "MessagesTestBundle_ja.properties",
            "deae4415e79996d0e62974ea1ca91e2abfc9bc59",
        ),
    ] {
        let data = std::fs::read(format!("{dir}{name}")).expect("the resource");
        assert_eq!(sha1(&data), expected, "SHA-1 of {name}");
    }
}

// ---------------------------------------------------------------------------
// #487 / #488 node trees
// ---------------------------------------------------------------------------

const TEXTS: [&str; 16] = [
    "a", "foo", "Foo Bar", "a:b", "*", "wi?d*", "\\", "-x", "AND", "", "ç中", "(x)", "\"q\"",
    "a b c", "1.5", "x~2",
];

const FIELDS: [&str; 6] = ["f", "title", "_plain", "*", "a b", ""];

/// Seeded node trees.
struct NodeGen {
    random: JdkRandom,
    standard: bool,
}

impl NodeGen {
    fn text(&mut self) -> String {
        lit(pick(&mut self.random, &TEXTS))
    }

    fn field(&mut self) -> String {
        if self.random.next_int_bound(10) == 0 {
            "null".to_string()
        } else {
            lit(pick(&mut self.random, &FIELDS))
        }
    }

    fn int(&mut self, bound: i32) -> String {
        format!("i:{}", self.random.next_int_bound(bound))
    }

    fn fqn(&mut self) -> String {
        let f = self.field();
        self.fqn_of(&f)
    }

    /// A `FieldQueryNode` of field `f` (a spec atom).
    fn fqn_of(&mut self, f: &str) -> String {
        let (t, b, e) = (self.text(), self.int(5), self.int(9));
        new(&format!("{CORE}FieldQueryNode"), &[f, &t, &b, &e])
    }

    /// The field of the second bound of a range: mostly the first one's (a
    /// range of two fields is rejected), sometimes another.
    fn range_field(&mut self, first: &str) -> String {
        if self.random.next_int_bound(5) == 0 {
            self.field()
        } else {
            first.to_string()
        }
    }

    fn children(&mut self, depth: u32) -> String {
        let n = self.random.next_int_bound(4) as usize;
        let items: Vec<String> = (0..n).map(|_| self.node(depth + 1)).collect();
        list(&items)
    }

    fn leaf(&mut self) -> String {
        let k = if self.standard { 14 } else { 9 };
        match self.random.next_int_bound(k) {
            0 | 1 => self.fqn(),
            2 => {
                let (f, t, b, e) = (self.field(), self.text(), self.int(5), self.int(9));
                new(&format!("{CORE}QuotedFieldQueryNode"), &[&f, &t, &b, &e])
            }
            3 => {
                let (f, t) = (self.field(), self.text());
                let sim = format!("f:{}", self.random.next_int_bound(30) as f32 / 10.0);
                new(
                    &format!("{CORE}FuzzyQueryNode"),
                    &[&f, &t, &sim, "i:0", "i:3"],
                )
            }
            4 => new(&format!("{CORE}MatchAllDocsQueryNode"), &[]),
            5 => new(&format!("{CORE}MatchNoDocsQueryNode"), &[]),
            6 => new(
                &format!(
                    "{CORE}{}",
                    pick(
                        &mut self.random,
                        &["NoTokenFoundQueryNode", "DeletedQueryNode"]
                    )
                ),
                &[],
            ),
            7 => {
                let (a, b) = (self.text(), self.text());
                new(&format!("{CORE}OpaqueQueryNode"), &[&a, &b])
            }
            8 => {
                let n = 1 + self.random.next_int_bound(3) as usize;
                let elems: Vec<String> = (0..n)
                    .map(|i| {
                        let t = self.text();
                        new(
                            QUERY_TEXT,
                            &[&t, &format!("i:{i}"), &format!("i:{}", i + 1)],
                        )
                    })
                    .collect();
                new(&format!("{CORE}PathQueryNode"), &[&list(&elems)])
            }
            9 => {
                let class = pick(
                    &mut self.random,
                    &["PrefixWildcardQueryNode", "WildcardQueryNode"],
                );
                if self.random.next_boolean() {
                    let (f, t, b, e) = (self.field(), self.text(), self.int(5), self.int(9));
                    new(&format!("{STD}{class}"), &[&f, &t, &b, &e])
                } else {
                    let fqn = self.fqn();
                    new(&format!("{STD}{class}"), &[&fqn])
                }
            }
            10 => {
                let f = self.field();
                let t = pick(&mut self.random, &["ab.+", "a\\/b", "[A-Z]x", "boo"]);
                let len = t.encode_utf16().count() as i32;
                if self.random.next_boolean() {
                    new(&format!("{STD}RegexpQueryNode"), &[&f, &lit(t)])
                } else {
                    let b = self.random.next_int_bound(len + 1);
                    let e = b + self.random.next_int_bound(len - b + 1);
                    new(
                        &format!("{STD}RegexpQueryNode"),
                        &[&f, &lit(t), &format!("i:{b}"), &format!("i:{e}")],
                    )
                }
            }
            11 => {
                let f = self.field();
                let f2 = self.range_field(&f);
                let (lo, hi) = (self.fqn_of(&f), self.fqn_of(&f2));
                let (li, ui) = (self.random.next_boolean(), self.random.next_boolean());
                new(
                    &format!("{STD}TermRangeQueryNode"),
                    &[&lo, &hi, &format!("z:{li}"), &format!("z:{ui}")],
                )
            }
            12 => self.point_range(),
            _ => {
                let f = if self.random.next_boolean() {
                    lit("f")
                } else {
                    "null".to_string()
                };
                let func = self.interval_function(0);
                new(&format!("{STD}IntervalQueryNode"), &[&f, &func])
            }
        }
    }

    fn number_format(&mut self) -> String {
        let tag = pick(&mut self.random, &["en-US", "de-DE", "fr-FR", "und"]);
        let method = pick(
            &mut self.random,
            &["getIntegerInstance", "getNumberInstance"],
        );
        static_call("java.text.NumberFormat", method, &[&locale(tag)])
    }

    fn point(&mut self, f: &str, type_: &str) -> String {
        let value = match type_ {
            "java.lang.Integer" => format!("i:{}", self.random.next_int_bound(20000) - 10000),
            "java.lang.Long" => format!("l:{}", self.random.next_long()),
            "java.lang.Float" => format!("f:{}", self.random.next_float() * 1000.0),
            _ => format!("d:{}", self.random.next_double() * 1.0e7),
        };
        let nf = self.number_format();
        new(&format!("{STD}PointQueryNode"), &[f, &value, &nf])
    }

    fn point_range(&mut self) -> String {
        let type_ = pick(
            &mut self.random,
            &[
                "java.lang.Integer",
                "java.lang.Long",
                "java.lang.Float",
                "java.lang.Double",
            ],
        );
        let f = self.field();
        let f2 = self.range_field(&f);
        let (lo, hi) = (self.point(&f, type_), self.point(&f2, type_));
        let nf = self.number_format();
        let config = new(POINTS_CONFIG, &[&nf, &class_for(type_)]);
        let (li, ui) = (self.random.next_boolean(), self.random.next_boolean());
        new(
            &format!("{STD}PointRangeQueryNode"),
            &[&lo, &hi, &format!("z:{li}"), &format!("z:{ui}"), &config],
        )
    }

    fn word(&mut self) -> String {
        lit(pick(
            &mut self.random,
            &["foo", "bar", "Baz", "quick brown", "the", "fo*", "a-b"],
        ))
    }

    fn interval_function(&mut self, depth: u32) -> String {
        let fname = |n: &str| format!("{FN}{n}");
        if depth >= 2 || self.random.next_int_bound(3) == 0 {
            return match self.random.next_int_bound(4) {
                0 | 1 => {
                    let w = self.word();
                    new(&fname("AnalyzedText"), &[&w])
                }
                2 => {
                    let w = lit(pick(&mut self.random, &["fo*", "b?r", "*", "x"]));
                    let m = format!("i:{}", 1 + self.random.next_int_bound(200));
                    new(&fname("Wildcard"), &[&w, &m])
                }
                _ => {
                    let w = self.word();
                    let edits = pick(&mut self.random, &["null", "i:0", "i:1", "i:2"]);
                    let exp = pick(&mut self.random, &["null", "i:5", "i:128"]);
                    new(&fname("FuzzyTerm"), &[&w, edits, exp])
                }
            };
        }
        let sub = |s: &mut Self| s.interval_function(depth + 1);
        let pairs = [
            "After",
            "Before",
            "ContainedBy",
            "Containing",
            "NonOverlapping",
            "NotContainedBy",
            "NotContaining",
            "Overlapping",
            "UnorderedNoOverlaps",
        ];
        let lists = ["Or", "Ordered", "Phrase", "Unordered"];
        match self.random.next_int_bound(6) {
            0 => {
                let name = pick(&mut self.random, &pairs);
                let (a, b) = (sub(self), sub(self));
                new(&fname(name), &[&a, &b])
            }
            1 => {
                let name = pick(&mut self.random, &lists);
                let n = 1 + self.random.next_int_bound(3) as usize;
                let items: Vec<String> = (0..n).map(|_| sub(self)).collect();
                new(&fname(name), &[&list(&items)])
            }
            2 => {
                let name = pick(&mut self.random, &["Within", "NotWithin"]);
                let (a, b) = (sub(self), sub(self));
                let p = self.int(4);
                new(&fname(name), &[&a, &p, &b])
            }
            3 => {
                let name = pick(&mut self.random, &["MaxGaps", "MaxWidth"]);
                let n = self.int(5);
                let a = sub(self);
                new(&fname(name), &[&n, &a])
            }
            4 => {
                let n = 1 + self.random.next_int_bound(3) as usize;
                let items: Vec<String> = (0..n).map(|_| sub(self)).collect();
                let m = self.int(4);
                new(&fname("AtLeast"), &[&m, &list(&items)])
            }
            _ => {
                let a = sub(self);
                let (b, e) = (self.int(3), self.int(3));
                new(&fname("Extend"), &[&a, &b, &e])
            }
        }
    }

    fn node(&mut self, depth: u32) -> String {
        if depth >= 3 || self.random.next_int_bound(3) == 0 {
            return self.leaf();
        }
        let k = if self.standard { 15 } else { 11 };
        let c = |n: &str| format!("{CORE}{n}");
        match self.random.next_int_bound(k) {
            0 => {
                let class = pick(
                    &mut self.random,
                    &["BooleanQueryNode", "AndQueryNode", "OrQueryNode"],
                );
                let ch = self.children(depth);
                new(&c(class), &[&ch])
            }
            1 => {
                let (ch, f, m) = (self.children(depth), self.field(), self.int(3));
                new(&c("AnyQueryNode"), &[&ch, &f, &m])
            }
            2 => {
                let n = self.node(depth + 1);
                let b = format!("f:{}", self.random.next_int_bound(40) as f32 / 4.0);
                new(&c("BoostQueryNode"), &[&n, &b])
            }
            3 => {
                let n = self.node(depth + 1);
                new(&c("GroupQueryNode"), &[&n])
            }
            4 => {
                let n = self.node(depth + 1);
                let m = field(
                    MODIFIER,
                    pick(&mut self.random, &["MOD_NONE", "MOD_NOT", "MOD_REQ"]),
                );
                new(&c("ModifierQueryNode"), &[&n, &m])
            }
            5 => {
                let n = self.node(depth + 1);
                let v = self.int(5);
                let class = pick(&mut self.random, &["SlopQueryNode", "PhraseSlopQueryNode"]);
                new(&c(class), &[&n, &v])
            }
            6 => {
                let (ch, f) = (self.children(depth), self.field());
                let t = field(
                    PROXIMITY_TYPE,
                    pick(&mut self.random, &["PARAGRAPH", "SENTENCE", "NUMBER"]),
                );
                let inorder = format!("z:{}", self.random.next_boolean());
                if self.random.next_boolean() {
                    let d = self.int(5);
                    new(&c("ProximityQueryNode"), &[&ch, &f, &t, &d, &inorder])
                } else {
                    new(&c("ProximityQueryNode"), &[&ch, &f, &t, &inorder])
                }
            }
            7 => {
                let ch = self.children(depth);
                static_call(
                    ORACLE,
                    "withChildren",
                    &[&new(&c("TokenizedPhraseQueryNode"), &[]), &ch],
                )
            }
            8..=10 => {
                let ch = self.children(depth);
                new(&c("BooleanQueryNode"), &[&ch])
            }
            11 => {
                let n = self.node(depth + 1);
                let m = field(MODIFIER, pick(&mut self.random, &["MOD_NOT", "MOD_REQ"]));
                new(&format!("{STD}BooleanModifierNode"), &[&n, &m])
            }
            12 => {
                let n = self.node(depth + 1);
                let m = self.int(4);
                new(
                    &format!("{STD}MinShouldMatchNode"),
                    &[&m, &new(&c("GroupQueryNode"), &[&n])],
                )
            }
            13 => {
                let ch = self.children(depth);
                new(&format!("{STD}SynonymQueryNode"), &[&ch])
            }
            _ => {
                let ch = self.children(depth);
                static_call(
                    ORACLE,
                    "withChildren",
                    &[&new(&format!("{STD}MultiPhraseQueryNode"), &[]), &ch],
                )
            }
        }
    }
}

fn nodes_oracle(seed: i64, standard: bool, n: usize) -> ParseOracle {
    let mut oracle = ParseOracle::new(registry());
    let mut g = NodeGen {
        random: JdkRandom::new(seed),
        standard,
    };
    for i in 0..n {
        let tree = g.node(0);
        value_record(
            &mut oracle,
            &format!("n{i}"),
            &static_call(ORACLE, "nodeDump", &[&tree]),
        );
    }
    oracle
}

#[test]
fn core_nodes_matches_lucene_0() {
    assert_parity(
        "core_nodes_matches_lucene_0",
        &nodes_oracle(0x0487_0000, false, 150),
    );
}

#[test]
fn core_nodes_matches_lucene_1() {
    assert_parity(
        "core_nodes_matches_lucene_1",
        &nodes_oracle(0x0487_0001, false, 150),
    );
}

#[test]
fn standard_nodes_matches_lucene_0() {
    assert_parity(
        "standard_nodes_matches_lucene_0",
        &nodes_oracle(0x0488_0000, true, 150),
    );
}

#[test]
fn standard_nodes_matches_lucene_1() {
    assert_parity(
        "standard_nodes_matches_lucene_1",
        &nodes_oracle(0x0488_0001, true, 150),
    );
}

fn query_tree_builder_oracle(seed: i64) -> ParseOracle {
    let mut oracle = ParseOracle::new(registry());
    let qtb = new(QTB, &[]);
    oracle.bind("qtb", &qtb);
    let classes = [
        ("FieldQueryNode", "FieldQueryNodeBuilder"),
        ("BooleanQueryNode", "BooleanQueryNodeBuilder"),
        ("GroupQueryNode", "GroupQueryNodeBuilder"),
        ("BoostQueryNode", "BoostQueryNodeBuilder"),
        ("ModifierQueryNode", "ModifierQueryNodeBuilder"),
        ("MatchAllDocsQueryNode", "MatchAllDocsQueryNodeBuilder"),
        ("MatchNoDocsQueryNode", "MatchNoDocsQueryNodeBuilder"),
        ("FuzzyQueryNode", "FuzzyQueryNodeBuilder"),
        ("SlopQueryNode", "SlopQueryNodeBuilder"),
    ];
    for (class, b) in classes {
        oracle.run(&call(
            "setBuilder",
            "$qtb",
            &[
                &class_for(&format!("{CORE}{class}")),
                &new(&format!("{BUILDERS}{b}"), &[]),
            ],
        ));
    }
    // a field builder takes precedence over the class builders
    oracle.run(&call(
        "setBuilder",
        "$qtb",
        &[
            &lit("title"),
            &new(&format!("{BUILDERS}MatchAllDocsQueryNodeBuilder"), &[]),
        ],
    ));
    oracle.bind("sqtb", &new(SQTB, &[]));
    let mut g = NodeGen {
        random: JdkRandom::new(seed),
        standard: false,
    };
    for i in 0..120 {
        let tree = g.node(0);
        value_record(
            &mut oracle,
            &format!("q{i}"),
            &call("build", "$qtb", &[&tree]),
        );
        value_record(
            &mut oracle,
            &format!("s{i}"),
            &call("build", "$sqtb", &[&tree]),
        );
    }
    oracle
}

#[test]
fn query_tree_builder_matches_lucene() {
    assert_parity(
        "query_tree_builder_matches_lucene",
        &query_tree_builder_oracle(0x0487_0b01),
    );
}

fn interval_functions_oracle(seed: i64) -> ParseOracle {
    let mut oracle = ParseOracle::new(registry());
    oracle.bind("a", &new(STANDARD, &[]));
    oracle.bind(
        "stop",
        &new(
            STANDARD,
            &[&new(
                CHAR_ARRAY_SET,
                &[&list(&[lit("the"), lit("quick")]), "z:true"],
            )],
        ),
    );
    let mut g = NodeGen {
        random: JdkRandom::new(seed),
        standard: true,
    };
    for i in 0..200 {
        let f = g.interval_function(0);
        let analyzer = if i % 3 == 0 { "$stop" } else { "$a" };
        value_record(
            &mut oracle,
            &format!("i{i}"),
            &static_call(ORACLE, "intervalDump", &[&f, &lit("body"), analyzer]),
        );
    }
    oracle
}

#[test]
fn interval_functions_matches_lucene() {
    assert_parity(
        "interval_functions_matches_lucene",
        &interval_functions_oracle(0x0488_1f00),
    );
}

#[test]
fn number_date_format_matches_lucene() {
    let mut oracle = ParseOracle::new(registry());
    let random = JdkRandom::new(0x0488_0df0);
    let mut i = 0;
    for tag in ["en-US", "de-DE", "ja-JP", "pt-BR", "und"] {
        for style in [0, 1, 2, 3] {
            for zone in ["UTC", "America/New_York", "Asia/Kolkata"] {
                let df = format!("df{i}");
                oracle.bind(
                    &df,
                    &static_call(
                        "java.text.DateFormat",
                        "getDateInstance",
                        &[&format!("i:{style}"), &locale(tag)],
                    ),
                );
                oracle.run(&call("setTimeZone", &format!("${df}"), &[&time_zone(zone)]));
                let ndf = format!("ndf{i}");
                oracle.bind(&ndf, &new(NUMBER_DATE_FORMAT, &[&format!("${df}")]));
                let v = format!("${ndf}");
                for j in 0..4 {
                    let millis = random.next_long() % 4_000_000_000_000;
                    value_record(
                        &mut oracle,
                        &format!("f{i}_{j}"),
                        &call("format", &v, &[&format!("l:{millis}")]),
                    );
                    value_record(
                        &mut oracle,
                        &format!("g{i}_{j}"),
                        &call("format", &v, &[&format!("d:{}.75", millis / 7)]),
                    );
                    // the text the format gives, parsed back
                    value_record(
                        &mut oracle,
                        &format!("p{i}_{j}"),
                        &call(
                            "parse",
                            &v,
                            &[&call("format", &v, &[&format!("l:{millis}")])],
                        ),
                    );
                }
                for text in ["1/2/02", "02.01.2002", "2002/01/02", "Jan 2, 2002", "x", ""] {
                    value_record(
                        &mut oracle,
                        &format!("t{i}_{}", sha1(text.as_bytes())),
                        &call("parse", &v, &[&lit(text)]),
                    );
                }
                i += 1;
            }
        }
    }
    assert_parity("number_date_format_matches_lucene", &oracle);
}

#[test]
fn config_handlers_match_lucene() {
    let mut oracle = ParseOracle::new(registry());
    oracle.bind("qp", &new(SQP, &[]));
    oracle.bind("boosts", &new(HASH_MAP, &[]));
    oracle.run(&call("put", "$boosts", &[&lit("b"), "f:5.0"]));
    oracle.run(&call("setFieldsBoost", "$qp", &["$boosts"]));
    // a put after the set reaches the parser (the map is held by reference)
    oracle.run(&call("put", "$boosts", &[&lit("t"), "f:0.25"]));
    oracle.bind("dates", &new(HASH_MAP, &[]));
    oracle.run(&call(
        "put",
        "$dates",
        &[&lit("month"), &field(RESOLUTION, "MONTH")],
    ));
    oracle.run(&call("setDateResolutionMap", "$qp", &["$dates"]));
    oracle.run(&call(
        "put",
        "$dates",
        &[&lit("hour"), &field(RESOLUTION, "HOUR")],
    ));
    oracle.run(&call(
        "setDateResolution",
        "$qp",
        &[&field(RESOLUTION, "DAY")],
    ));
    oracle.bind("points", &new(HASH_MAP, &[]));
    let nf = static_call(
        "java.text.NumberFormat",
        "getIntegerInstance",
        &[&field(LOCALE, "ROOT")],
    );
    oracle.bind(
        "pc",
        &new(POINTS_CONFIG, &[&nf, &class_for("java.lang.Long")]),
    );
    oracle.run(&call("put", "$points", &[&lit("n"), "$pc"]));
    oracle.run(&call("setPointsConfigMap", "$qp", &["$points"]));
    oracle.bind("h", &call("getQueryConfigHandler", "$qp", &[]));
    for f in ["b", "t", "month", "hour", "n", "other", ""] {
        let fc = call("getFieldConfig", "$h", &[&lit(f)]);
        for key in ["BOOST", "DATE_RESOLUTION"] {
            value_record(
                &mut oracle,
                &format!("{f}.{key}"),
                &call("get", &fc, &[&field(KEYS, key)]),
            );
        }
        value_record(
            &mut oracle,
            &format!("{f}.POINTS"),
            &call(
                "getType",
                &call("get", &fc, &[&field(KEYS, "POINTS_CONFIG")]),
                &[],
            ),
        );
    }
    value_record(
        &mut oracle,
        "nullfield",
        &call("getFieldConfig", "$h", &["null"]),
    );
    // a fresh handler: no configuration but its defaults
    value_record(
        &mut oracle,
        "fresh",
        &call(
            "get",
            &call("getFieldConfig", &new(SQCH, &[]), &[&lit("b")]),
            &[&field(KEYS, "DATE_RESOLUTION")],
        ),
    );
    // PointsConfig: equality, and the types it accepts
    let nf2 = static_call(
        "java.text.NumberFormat",
        "getIntegerInstance",
        &[&field(LOCALE, "ROOT")],
    );
    let nf3 = static_call(
        "java.text.NumberFormat",
        "getNumberInstance",
        &[&locale("de-DE")],
    );
    for (id, a, b) in [
        (
            "eq",
            new(POINTS_CONFIG, &[&nf2, &class_for("java.lang.Long")]),
            "$pc",
        ),
        (
            "ne_type",
            new(POINTS_CONFIG, &[&nf2, &class_for("java.lang.Integer")]),
            "$pc",
        ),
        (
            "ne_format",
            new(POINTS_CONFIG, &[&nf3, &class_for("java.lang.Long")]),
            "$pc",
        ),
    ] {
        value_record(&mut oracle, id, &call("equals", &a, &[b]));
    }
    for t in ["java.lang.Short", "java.lang.Double"] {
        value_record(
            &mut oracle,
            &format!("type_{t}"),
            &call("getType", &new(POINTS_CONFIG, &[&nf2, &class_for(t)]), &[]),
        );
    }
    value_record(
        &mut oracle,
        "null_format",
        &new(POINTS_CONFIG, &["null", &class_for("java.lang.Long")]),
    );
    value_record(
        &mut oracle,
        "null_type",
        &new(POINTS_CONFIG, &[&nf2, "null"]),
    );
    assert_parity("config_handlers_match_lucene", &oracle);
}

// ---------------------------------------------------------------------------
// #489 syntax parser, #491 corpus
// ---------------------------------------------------------------------------

/// Query strings of the flexible syntax: interval functions, the
/// minimum-should-match operator, comparison ranges, regular expressions,
/// escapes and malformed input.
fn flexible_edge_cases() -> Vec<String> {
    [
        "fn:ordered(a b c)",
        "fn:atleast(2 a b c)",
        "fn:atleast(3 FOO BAR baz)",
        "fn:maxwidth(3 fn:or(a b))",
        "fn:maxgaps(1 fn:unordered(a b))",
        "fn:wildcard(fo*)",
        "fn:wildcard(fo* 10)",
        "fn:fuzzyTerm(abc)",
        "fn:fuzzyTerm(abc 1)",
        "fn:fuzzyTerm(abc 1 5)",
        "fn:phrase(a \"b c\" fn:or(d e))",
        "fn:before(a b)",
        "fn:after(a fn:ordered(b c))",
        "fn:containing(big small)",
        "fn:containedBy(small big)",
        "fn:notContaining(a b)",
        "fn:notContainedBy(a b)",
        "fn:within(a 2 b)",
        "fn:notWithin(a 2 fn:ordered(b c))",
        "fn:overlapping(a b)",
        "fn:nonOverlapping(a b)",
        "fn:unorderedNoOverlaps(a b)",
        "fn:extend(a 1 2)",
        "title:fn:ordered(a b)",
        "fn:atleast(2 \"foo\" \"BAR baz\")",
        "fn:unknown(a)",
        "fn:ordered()",
        "fn:atleast(a b)",
        "fn:maxwidth(x a)",
        "fn:ordered(a b",
        "(a b c)@2",
        "(a b)@1.5",
        "(foo)@1",
        "a@2",
        "title:(a b)@2",
        "(title:foo OR title:bar OR title:baz)@2",
        "field>a",
        "field>=a",
        "field<a",
        "field<=a",
        "f:>a",
        "field>\"a b\"",
        "/ab.+/",
        "f:/a[bc]/",
        "/a/^2",
        "/a/~2",
        "/unterminated",
        "a\\u0062c",
        "a\\u00",
        "a\\uZZZZ",
        "a\\",
        "\\\\",
        "a~1.5",
        "a~0.5",
        "a~2",
        "a~3",
        "\"a b\"~2",
        "\"a b\"~1.5",
        "(a b)^2^3",
        "a^",
        "a^x",
        "*:*",
        "*",
        "*:a",
        "a:*",
        "-*:*",
        "+a -b !c NOT d",
        "a AND OR b",
        "AND",
        "OR a",
        "a NOT",
        "\"",
        "()",
        "( )",
        "[a TO]",
        "[a TO b",
        "{* TO *}",
        "[\"a b\" TO c]",
        "[a TO b}^3",
        "a:b:c",
        "a::b",
        ":a",
        "a\u{3000}b",
        "ç ñ 中国",
        "",
        "   ",
        "intField:[1 TO 3]",
        "intField:1",
        "intField:{1 TO *]",
        "intField:abc",
        "intField:[1 TO 2.5]",
        "intField:\"7\"",
        "intField:>3",
        "intField:<=-3",
        "intField:1*",
        "longField:[-5 TO 5]",
        "floatField:1.5",
        "floatField:[1,5 TO 2,5]",
        "doubleField:[* TO 3.25]",
        "doubleField:[1,000.5 TO 2.000,5]",
        "date:[1/2/02 TO 3/4/02]",
        "date:[02/01/2002 TO 2002-12-31]",
        "date:{1.2.02 TO 4.5.06}",
        "date:[\"1/2/02\" TO \"3/4/02\"]",
        "month:[1/2/02 TO 3/4/02]",
        "hour:[1/2/02 TO 1/3/02]",
        "hour:{1/2/02 TO *}",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn syntax_corpus(seed: i64, n: usize) -> Vec<String> {
    flexible_edge_cases()
        .into_iter()
        .chain(corpus::classic_edge_cases())
        .chain(corpus::classic_random(seed, n))
        .collect()
}

fn shard<T: Clone>(items: &[T], shard: usize, shards: usize) -> Vec<T> {
    items
        .iter()
        .enumerate()
        .filter(|(i, _)| i % shards == shard)
        .map(|(_, x)| x.clone())
        .collect()
}

fn syntax_oracle(shard_index: usize) -> ParseOracle {
    let mut oracle = ParseOracle::new(registry());
    oracle.bind("p", &new(SSP, &[]));
    let inputs = shard(&syntax_corpus(0x0489_5e7d, 240), shard_index, 4);
    for (i, input) in inputs.iter().enumerate() {
        input_record(
            &mut oracle,
            &format!("s{i}"),
            &static_call(
                ORACLE,
                "nodeDump",
                &[&call("parse", "$p", &["$input", &lit("field")])],
            ),
            input,
            None,
        );
        if i % 5 == 0 {
            input_record(
                &mut oracle,
                &format!("n{i}"),
                &static_call(
                    ORACLE,
                    "treeDump",
                    &[&call("parse", "$p", &["$input", "null"])],
                ),
                input,
                None,
            );
        }
    }
    oracle
}

#[test]
fn syntax_parser_matches_lucene_0() {
    assert_parity("syntax_parser_matches_lucene_0", &syntax_oracle(0));
}

#[test]
fn syntax_parser_matches_lucene_1() {
    assert_parity("syntax_parser_matches_lucene_1", &syntax_oracle(1));
}

#[test]
fn syntax_parser_matches_lucene_2() {
    assert_parity("syntax_parser_matches_lucene_2", &syntax_oracle(2));
}

#[test]
fn syntax_parser_matches_lucene_3() {
    assert_parity("syntax_parser_matches_lucene_3", &syntax_oracle(3));
}

// ---------------------------------------------------------------------------
// Parser configurations (#490, #491)
// ---------------------------------------------------------------------------

/// Binds `$<name>` to a parser of class `class` and runs a seeded random
/// configuration of its setters. The locale and the time zone are always
/// set, so that no default of either JVM or emulation is read.
fn configure(oracle: &mut ParseOracle, random: &mut JdkRandom, name: &str, class: &str) {
    let analyzer = match random.next_int_bound(6) {
        0 => "null".to_string(),
        1 | 2 => new(
            STANDARD,
            &[&new(
                CHAR_ARRAY_SET,
                &[
                    &list(&[lit("the"), lit("of"), lit("and"), lit("stop")]),
                    "z:true",
                ],
            )],
        ),
        _ => new(STANDARD, &[]),
    };
    let var = format!("${name}");
    if random.next_boolean() {
        oracle.bind(name, &new(class, &[&analyzer]));
    } else {
        oracle.bind(name, &new(class, &[]));
        oracle.run(&call("setAnalyzer", &var, &[&analyzer]));
    }
    let tag = pick(
        random,
        &[
            "en-US", "de-DE", "fr-FR", "ja-JP", "pt-BR", "en-GB", "tr-TR", "und",
        ],
    );
    oracle.run(&call("setLocale", &var, &[&locale(tag)]));
    let zone = pick(
        random,
        &[
            "UTC",
            "America/New_York",
            "Europe/Lisbon",
            "Asia/Tokyo",
            "Australia/Lord_Howe",
        ],
    );
    oracle.run(&call("setTimeZone", &var, &[&time_zone(zone)]));
    if random.next_boolean() {
        oracle.run(&call(
            "setDefaultOperator",
            &var,
            &[&field(OPERATOR, pick(random, &["AND", "OR"]))],
        ));
    }
    if random.next_boolean() {
        oracle.run(&call(
            "setAllowLeadingWildcard",
            &var,
            &[&format!("z:{}", random.next_boolean())],
        ));
    }
    if random.next_boolean() {
        oracle.run(&call(
            "setEnablePositionIncrements",
            &var,
            &[&format!("z:{}", random.next_boolean())],
        ));
    }
    if random.next_boolean() {
        oracle.run(&call(
            "setPhraseSlop",
            &var,
            &[&format!("i:{}", random.next_int_bound(4))],
        ));
    }
    if random.next_boolean() {
        oracle.run(&call(
            "setFuzzyMinSim",
            &var,
            &[pick(random, &["f:0.5", "f:1.0", "f:2.0", "f:0.0"])],
        ));
    }
    if random.next_boolean() {
        oracle.run(&call(
            "setFuzzyPrefixLength",
            &var,
            &[&format!("i:{}", random.next_int_bound(3))],
        ));
    }
    if random.next_boolean() {
        oracle.run(&call(
            "setMultiTermRewriteMethod",
            &var,
            &[&field(
                MTQ,
                pick(
                    random,
                    &[
                        "CONSTANT_SCORE_REWRITE",
                        "CONSTANT_SCORE_BLENDED_REWRITE",
                        "CONSTANT_SCORE_BOOLEAN_REWRITE",
                        "SCORING_BOOLEAN_REWRITE",
                    ],
                ),
            )],
        ));
    }
    if random.next_boolean() {
        oracle.run(&call(
            "setDateResolution",
            &var,
            &[&field(
                RESOLUTION,
                pick(random, &["YEAR", "MONTH", "DAY", "HOUR", "MILLISECOND"]),
            )],
        ));
    }
    if random.next_boolean() {
        let m = format!("{name}dates");
        oracle.bind(&m, &new(HASH_MAP, &[]));
        oracle.run(&call(
            "put",
            &format!("${m}"),
            &[&lit("month"), &field(RESOLUTION, "MONTH")],
        ));
        oracle.run(&call(
            "put",
            &format!("${m}"),
            &[&lit("hour"), &field(RESOLUTION, "HOUR")],
        ));
        oracle.run(&call("setDateResolutionMap", &var, &[&format!("${m}")]));
    }
    if random.next_int_bound(3) == 0 {
        let fields: Vec<String> = ["b", "t", "title"][..1 + random.next_int_bound(3) as usize]
            .iter()
            .map(|f| lit(f))
            .collect();
        let mut all: Vec<&str> = vec!["java.lang.CharSequence"];
        all.extend(fields.iter().map(String::as_str));
        oracle.run(&call("setMultiFields", &var, &[&expr("array", &all)]));
        if random.next_boolean() {
            let m = format!("{name}boosts");
            oracle.bind(&m, &new(HASH_MAP, &[]));
            oracle.run(&call("put", &format!("${m}"), &[&lit("b"), "f:5.0"]));
            oracle.run(&call("put", &format!("${m}"), &[&lit("title"), "f:0.5"]));
            oracle.run(&call("setFieldsBoost", &var, &[&format!("${m}")]));
        }
    }
    if random.next_boolean() {
        let m = format!("{name}points");
        oracle.bind(&m, &new(HASH_MAP, &[]));
        let ptag = pick(random, &["en-US", "de-DE", "und"]);
        for (f, method, type_) in [
            ("intField", "getIntegerInstance", "java.lang.Integer"),
            ("longField", "getIntegerInstance", "java.lang.Long"),
            ("floatField", "getNumberInstance", "java.lang.Float"),
            ("doubleField", "getNumberInstance", "java.lang.Double"),
        ] {
            let nf = static_call("java.text.NumberFormat", method, &[&locale(ptag)]);
            oracle.run(&call(
                "put",
                &format!("${m}"),
                &[&lit(f), &new(POINTS_CONFIG, &[&nf, &class_for(type_)])],
            ));
        }
        oracle.run(&call("setPointsConfigMap", &var, &[&format!("${m}")]));
    }
}

fn processing_oracle(seed: i64, class: &str) -> ParseOracle {
    let mut oracle = ParseOracle::new(registry());
    let mut random = JdkRandom::new(seed);
    configure(&mut oracle, &mut random, "qp", class);
    let inputs: Vec<String> = flexible_edge_cases()
        .into_iter()
        .chain(corpus::classic_random(seed, 40))
        .collect();
    for (i, input) in inputs.iter().enumerate() {
        let default_field = if i % 4 == 3 {
            "null".to_string()
        } else {
            lit("body")
        };
        let parsed = call(
            "parse",
            &call("getSyntaxParser", "$qp", &[]),
            &["$input", &default_field],
        );
        input_record(
            &mut oracle,
            &format!("t{i}"),
            &static_call(
                ORACLE,
                "processTrace",
                &[&call("getQueryNodeProcessor", "$qp", &[]), &parsed],
            ),
            input,
            None,
        );
        input_record(
            &mut oracle,
            &format!("b{i}"),
            &call(
                "build",
                &call("getQueryBuilder", "$qp", &[]),
                &[&call(
                    "process",
                    &call("getQueryNodeProcessor", "$qp", &[]),
                    &[&parsed],
                )],
            ),
            input,
            Some("body"),
        );
    }
    oracle
}

#[test]
fn processing_matches_lucene_0() {
    assert_parity(
        "processing_matches_lucene_0",
        &processing_oracle(0x0490_0000, SQP),
    );
}

#[test]
fn processing_matches_lucene_1() {
    assert_parity(
        "processing_matches_lucene_1",
        &processing_oracle(0x0490_0001, SQP),
    );
}

#[test]
fn processing_matches_lucene_2() {
    assert_parity(
        "processing_matches_lucene_2",
        &processing_oracle(0x0490_0002, SQP),
    );
}

#[test]
fn processing_precedence_matches_lucene() {
    assert_parity(
        "processing_precedence_matches_lucene",
        &processing_oracle(0x0490_0003, PQP),
    );
}

fn parser_oracle(seed: i64, class: &str) -> ParseOracle {
    let mut oracle = ParseOracle::new(registry());
    let mut random = JdkRandom::new(seed);
    configure(&mut oracle, &mut random, "qp", class);
    let inputs = syntax_corpus(seed, 150);
    for (i, input) in inputs.iter().enumerate() {
        let default_field = match i % 5 {
            4 => "null".to_string(),
            3 => lit("title"),
            _ => lit("body"),
        };
        input_record(
            &mut oracle,
            &format!("p{i}"),
            &call("parse", "$qp", &["$input", &default_field]),
            input,
            Some("body"),
        );
    }
    oracle
}

#[test]
fn standard_query_parser_matches_lucene_0() {
    assert_parity(
        "standard_query_parser_matches_lucene_0",
        &parser_oracle(0x0491_0000, SQP),
    );
}

#[test]
fn standard_query_parser_matches_lucene_1() {
    assert_parity(
        "standard_query_parser_matches_lucene_1",
        &parser_oracle(0x0491_0001, SQP),
    );
}

#[test]
fn standard_query_parser_matches_lucene_2() {
    assert_parity(
        "standard_query_parser_matches_lucene_2",
        &parser_oracle(0x0491_0002, SQP),
    );
}

#[test]
fn standard_query_parser_matches_lucene_3() {
    assert_parity(
        "standard_query_parser_matches_lucene_3",
        &parser_oracle(0x0491_0003, SQP),
    );
}

#[test]
fn standard_query_parser_matches_lucene_4() {
    assert_parity(
        "standard_query_parser_matches_lucene_4",
        &parser_oracle(0x0491_0004, SQP),
    );
}

#[test]
fn standard_query_parser_matches_lucene_5() {
    assert_parity(
        "standard_query_parser_matches_lucene_5",
        &parser_oracle(0x0491_0005, SQP),
    );
}

#[test]
fn precedence_query_parser_matches_lucene_0() {
    assert_parity(
        "precedence_query_parser_matches_lucene_0",
        &parser_oracle(0x0491_1000, PQP),
    );
}

#[test]
fn precedence_query_parser_matches_lucene_1() {
    assert_parity(
        "precedence_query_parser_matches_lucene_1",
        &parser_oracle(0x0491_1001, PQP),
    );
}

#[test]
fn precedence_query_parser_matches_lucene_2() {
    assert_parity(
        "precedence_query_parser_matches_lucene_2",
        &parser_oracle(0x0491_1002, PQP),
    );
}

/// Negative control: a tampered Rucene dump of a flexible oracle is
/// reported as a difference (the comparison is not vacuous).
#[test]
fn flexible_oracle_negative_control() {
    let mut oracle = ParseOracle::new(registry());
    oracle.bind("p", &new(SSP, &[]));
    input_record(
        &mut oracle,
        "s0",
        &static_call(
            ORACLE,
            "nodeDump",
            &[&call("parse", "$p", &["$input", &lit("field")])],
        ),
        "a AND (b OR c)^2",
        None,
    );
    let java = oracle.java_dump();
    let rust = oracle.rust_dump();
    assert!(compare(&java, &rust).is_ok(), "the untampered dumps agree");
    let tampered = rust.replacen("OrQueryNode", "AndQueryNode", 1);
    assert_ne!(tampered, rust, "the tamper changes the dump");
    let clean = compare(&java, &tampered).is_ok();
    assert!(!clean, "a tampered dump must not compare clean");
}
