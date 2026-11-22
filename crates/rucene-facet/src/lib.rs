//! This crate ports Apache Lucene 10.5.0 `lucene/facet` to Rust: faceted search
//! over taxonomy and sorted-set doc values, numeric ranges, range-on-range and
//! facet sets, with drill-down and drill-sideways queries.
//!
//! Every type of the module is declared here as a skeleton: a declaration that
//! stands for one Java type, without fields, methods or behaviour (an enum lists
//! its Java constants as variants), whose doc names the sprint-17 rmp task that
//! will port it. A type stays a skeleton until that task replaces it with its
//! port.
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.facet`: the types of that package sit at the crate root
//! and `org.apache.lucene.facet.taxonomy.writercache` is the module
//! [`taxonomy::writercache`]. Each Java source file is one module, named in
//! snake_case (`FacetsConfig.java` is [`facets_config`]); a Java package name
//! that is not snake_case becomes a snake_case module, and one that is a Rust
//! keyword a raw identifier. A nested Java type is declared in the module of its
//! source file under its own name; the name of its enclosing type is prefixed
//! only when its own name is a name of the Rust prelude or the name of another
//! item of that module, and the Java kind is appended when the prefixed name is
//! itself taken (`DrillSideways.Result` is `DrillSidewaysResultRecord`).

// Package-private and private Java types are `pub(crate)` skeletons that nothing
// uses yet. This allowance goes away when the module is fully ported.
#![allow(dead_code)]

pub mod drill_down_query;
pub mod drill_sideways;
pub mod drill_sideways_query;
pub mod drill_sideways_scorer;
pub mod facet_counts_with_filter_query;
pub mod facet_field;
pub mod facet_query;
pub mod facet_result;
pub mod facet_utils;
pub mod facets;
pub mod facets_collector;
pub mod facets_collector_manager;
pub mod facets_config;
pub mod facetset;
pub mod label_and_value;
pub mod long_value_facet_counts;
pub mod multi_double_values;
pub mod multi_double_values_source;
pub mod multi_facet_query;
pub mod multi_facets;
pub mod multi_long_values;
pub mod multi_long_values_source;
pub mod random_sampling_facets_collector;
pub mod range;
pub mod rangeonrange;
pub mod sortedset;
pub mod string_doc_values_reader_state;
pub mod string_value_facet_counts;
pub mod taxonomy;
pub mod top_ord_and_float_queue;
pub mod top_ord_and_int_queue;
pub mod top_ord_and_number_queue;
