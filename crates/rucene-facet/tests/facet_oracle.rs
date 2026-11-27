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

//! The Java-oracle portability tests of `lucene/facet` (rmp #630): Lucene
//! 10.5.0 (`tests/fixtures/sprint17-oracle/FacetOracle.java`) and Rucene
//! index one seeded corpus and print the same dumps.
//!
//! * `taxonomy`: `FastTaxonomyFacetCounts` (default and custom index fields,
//!   counting hits and counting all documents), `TaxonomyFacetIntAssociations`
//!   and `TaxonomyFacetFloatAssociations` (`SUM`, `MAX`): `getAllDims`,
//!   `getTopChildren`, `getAllChildren`, `getSpecificValue` (with its
//!   `IllegalArgumentException`), `getTopDims`, `FacetResult.toString`.
//! * `sortedset`: `SortedSetDocValuesFacetCounts` and
//!   `ConcurrentSortedSetDocValuesFacetCounts` over flat, hierarchical and
//!   multi-valued dimensions of a `DefaultSortedSetDocValuesReaderState`.
//! * `range`: `LongRangeFacetCounts` (single- and multi-valued),
//!   `DoubleRangeFacetCounts`, `LongRangeOnRangeFacetCounts` and
//!   `DoubleRangeOnRangeFacetCounts` (every `QueryType`),
//!   `MatchingFacetSetsCounts` (exact and range matchers).
//! * `drill`: `DrillDownQuery` (`toString`, hits) and `DrillSideways`
//!   (taxonomy and sorted-set: hits and sideways counts).
//! * `taxo-write` / `taxo-read`: a taxonomy and facet-bearing main index
//!   written by Rucene is read by Lucene, and one written by Lucene is read by
//!   Rucene; both read dumps (file names, every ordinal's parent, child,
//!   sibling, label and reverse lookup, commit data, counts of every facet
//!   kind) are equal. File bytes are not compared: every segment carries a
//!   random 16-byte id (`StringHelper.randomId()`) and the `.si` diagnostics
//!   carry a timestamp and the JVM's identity, so no two writes are
//!   byte-identical, even two Lucene writes of one corpus.
//!
//! Every comparison checks the Java dump is non-trivial ([`non_trivial`]);
//! [`facet_oracle_positive_control`] proves a one-label perturbation of the
//! corpus is detected.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::document::range_doc_values_fields::{
    DoubleRangeDocValuesField, LongRangeDocValuesField,
};
use rucene::document::range_fields::{
    DoubleRange as DoubleRangeField, LongRange as LongRangeField, RangeQueryType,
};
use rucene::document::{
    Document, DoubleDocValuesField, LongPoint, NumericDocValuesField, SortedNumericDocValuesField,
    Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_commit::TwoPhaseCommit;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, Term};
use rucene::search::{
    query_to_string, Executor, IndexSearcher, MatchAllDocsQuery, Query, TermQuery, TopDocs,
};
use rucene::store::{ByteBuffersDirectory, Directory, FSDirectory};
use rucene::util::{BytesRef, NamedThreadFactory};
use rucene_facet::drill_down_query::DrillDownQuery;
use rucene_facet::drill_sideways::{DrillSideways, DrillSidewaysResult};
use rucene_facet::facet_field::FacetField;
use rucene_facet::facet_result::FacetResult;
use rucene_facet::facets::Facets;
use rucene_facet::facets_collector::FacetsCollector;
use rucene_facet::facets_collector_manager::{FacetsCollectorManager, FacetsTopDocs};
use rucene_facet::facets_config::{DrillDownTermsIndexing, FacetsConfig, DEFAULT_INDEX_FIELD_NAME};
use rucene_facet::facetset::dim_range::DimRange;
use rucene_facet::facetset::exact_facet_set_matcher::ExactFacetSetMatcher;
use rucene_facet::facetset::facet_set::FacetSet;
use rucene_facet::facetset::facet_set_decoder::{decode_longs, FacetSetDecoder};
use rucene_facet::facetset::facet_set_matcher::FacetSetMatcher;
use rucene_facet::facetset::facet_sets_field::FacetSetsField;
use rucene_facet::facetset::long_facet_set::LongFacetSet;
use rucene_facet::facetset::matching_facet_sets_counts::MatchingFacetSetsCounts;
use rucene_facet::facetset::range_facet_set_matcher::RangeFacetSetMatcher;
use rucene_facet::range::double_range::DoubleRange;
use rucene_facet::range::double_range_facet_counts::DoubleRangeFacetCounts;
use rucene_facet::range::long_range::LongRange;
use rucene_facet::range::long_range_facet_counts::LongRangeFacetCounts;
use rucene_facet::rangeonrange::double_range::DoubleRange as RorDoubleRange;
use rucene_facet::rangeonrange::double_range_on_range_facet_counts::DoubleRangeOnRangeFacetCounts;
use rucene_facet::rangeonrange::long_range::LongRange as RorLongRange;
use rucene_facet::rangeonrange::long_range_on_range_facet_counts::LongRangeOnRangeFacetCounts;
use rucene_facet::sortedset::concurrent_sorted_set_doc_values_facet_counts::ConcurrentSortedSetDocValuesFacetCounts;
use rucene_facet::sortedset::default_sorted_set_doc_values_reader_state::DefaultSortedSetDocValuesReaderState;
use rucene_facet::sortedset::sorted_set_doc_values_facet_counts::SortedSetDocValuesFacetCounts;
use rucene_facet::sortedset::sorted_set_doc_values_facet_field::SortedSetDocValuesFacetField;
use rucene_facet::sortedset::sorted_set_doc_values_reader_state::SortedSetDocValuesReaderState;
use rucene_facet::taxonomy::association_aggregation_function::{
    AssociationAggregationFunction, MaxAggregation, SumAggregation,
};
use rucene_facet::taxonomy::directory::directory_taxonomy_reader::DirectoryTaxonomyReader;
use rucene_facet::taxonomy::directory::directory_taxonomy_writer::DirectoryTaxonomyWriter;
use rucene_facet::taxonomy::fast_taxonomy_facet_counts::FastTaxonomyFacetCounts;
use rucene_facet::taxonomy::float_association_facet_field::FloatAssociationFacetField;
use rucene_facet::taxonomy::int_association_facet_field::IntAssociationFacetField;
use rucene_facet::taxonomy::taxonomy_facet_float_associations::TaxonomyFacetFloatAssociations;
use rucene_facet::taxonomy::taxonomy_facet_int_associations::TaxonomyFacetIntAssociations;
use rucene_facet::taxonomy::taxonomy_reader::TaxonomyReader;
use rucene_facet::taxonomy::taxonomy_writer::TaxonomyWriter;
use rucene_test_framework::jdk::executor_service::ExecutorService;
use rucene_test_framework::jdk::thread_pool_executor::ThreadPoolExecutor;
use rucene_test_framework::util::java_exceptions::java_throwable;
use sprint17_oracle::{bits32, bits64, corpus, Module, WorkDir};

/// The seed of every scenario.
const SEED: i64 = 630;

/// `FacetOracle.NUM_DOCS`.
const NUM_DOCS: i32 = 40;

macro_rules! outln {
    ($out:expr, $($arg:tt)*) => {{
        $out.push_str(&format!($($arg)*));
        $out.push('\n');
    }};
}

/// `FacetOracle.Doc`.
#[derive(Default)]
struct Doc {
    id: String,
    body: String,
    author: String,
    date: [String; 3],
    tags: Vec<String>,
    int_labels: Vec<String>,
    int_values: Vec<i32>,
    float_labels: Vec<String>,
    float_values: Vec<f32>,
    price: i64,
    weight: f64,
    multi: Vec<i64>,
    lr_min: i64,
    lr_max: i64,
    dr_min: f64,
    dr_max: f64,
    sets: Vec<[i64; 2]>,
}

/// Java's `List.toString()` of already-rendered elements.
fn list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(", "))
}

impl Doc {
    /// `Doc.describe()`.
    fn describe(&self) -> String {
        let ints = (0..self.int_labels.len())
            .map(|i| format!("{}:{}", self.int_labels[i], self.int_values[i]))
            .collect::<Vec<_>>()
            .join(", ");
        let floats = (0..self.float_labels.len())
            .map(|i| format!("{}:{}", self.float_labels[i], bits32(self.float_values[i])))
            .collect::<Vec<_>>()
            .join(", ");
        let sets = self
            .sets
            .iter()
            .map(|s| format!("({},{})", s[0], s[1]))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "doc {} body={} author={} date={} tags={} int=[{ints}] float=[{floats}] price={} \
             weight={} multi={} lr={}..{} dr={}..{} fs=[{sets}]",
            self.id,
            self.body,
            self.author,
            self.date.join("/"),
            list(self.tags.iter().cloned()),
            self.price,
            bits64(self.weight),
            list(self.multi.iter().map(i64::to_string)),
            self.lr_min,
            self.lr_max,
            bits64(self.dr_min),
            bits64(self.dr_max),
        )
    }
}

/// `vocab(i)`.
fn vocab(i: i32) -> String {
    corpus::VOCABULARY[i as usize].to_string()
}

/// `gen(Random, int)`.
fn gen(r: &JdkRandom, i: i32) -> Doc {
    let mut d = Doc {
        id: format!("d{i:02}"),
        body: format!("b{}", r.next_int_bound(3)),
        author: vocab(4 + r.next_int_bound(6)),
        ..Doc::default()
    };
    let year = (2010 + r.next_int_bound(4)).to_string();
    let month = (1 + r.next_int_bound(12)).to_string();
    let day = (1 + r.next_int_bound(28)).to_string();
    d.date = [year, month, day];
    let n_tags = r.next_int_bound(4);
    for _ in 0..n_tags {
        let tag = vocab(10 + r.next_int_bound(8));
        if !d.tags.contains(&tag) {
            d.tags.push(tag);
        }
    }
    let n_int = 1 + r.next_int_bound(2);
    for _ in 0..n_int {
        let label = vocab(18 + r.next_int_bound(4));
        let value = r.next_int_bound(100);
        if !d.int_labels.contains(&label) {
            d.int_labels.push(label);
            d.int_values.push(value);
        }
    }
    let n_float = 1 + r.next_int_bound(2);
    for _ in 0..n_float {
        let label = vocab(22 + r.next_int_bound(4));
        let value = r.next_float() * 10.0;
        if !d.float_labels.contains(&label) {
            d.float_labels.push(label);
            d.float_values.push(value);
        }
    }
    d.price = i64::from(r.next_int_bound(1000) - 100);
    d.weight = r.next_double() * 100.0;
    let n_multi = r.next_int_bound(3);
    for _ in 0..n_multi {
        d.multi.push(i64::from(r.next_int_bound(50)));
    }
    d.lr_min = i64::from(r.next_int_bound(100));
    d.lr_max = d.lr_min + i64::from(r.next_int_bound(50));
    d.dr_min = r.next_double() * 100.0;
    d.dr_max = d.dr_min + r.next_double() * 30.0;
    let n_sets = 1 + r.next_int_bound(2);
    for _ in 0..n_sets {
        let a = i64::from(r.next_int_bound(5));
        let b = i64::from(r.next_int_bound(5));
        d.sets.push([a, b]);
    }
    d
}

/// `config()`.
fn config() -> Arc<FacetsConfig> {
    let c = FacetsConfig::new();
    c.set_hierarchical("Publish Date", true);
    c.set_drill_down_terms_indexing("Publish Date", DrillDownTermsIndexing::ALL_PATHS_NO_DIM);
    c.set_multi_valued("Tags", true);
    c.set_require_dim_count("Tags", true);
    c.set_index_field_name("Tags", "$tags");
    c.set_multi_valued("Int", true);
    c.set_index_field_name("Int", "$int");
    c.set_multi_valued("Float", true);
    c.set_index_field_name("Float", "$float");
    c.set_index_field_name("SAuthor", "$ssdv");
    c.set_hierarchical("SDate", true);
    c.set_index_field_name("SDate", "$ssdv");
    c.set_multi_valued("STags", true);
    c.set_index_field_name("STags", "$ssdv");
    Arc::new(c)
}

/// `document(Doc)`.
fn document(d: &Doc) -> Result<Document> {
    let date: Vec<&str> = d.date.iter().map(String::as_str).collect();
    let mut doc = Document::new();
    doc.add(StringField::new("id", d.id.clone(), Store::YES)?);
    doc.add(StringField::new("body", d.body.clone(), Store::NO)?);
    doc.add(FacetField::new("Author", &[&d.author])?);
    doc.add(FacetField::new("Publish Date", &date)?);
    for t in &d.tags {
        doc.add(FacetField::new("Tags", &[t])?);
    }
    for (label, value) in d.int_labels.iter().zip(&d.int_values) {
        doc.add(IntAssociationFacetField::new(*value, "Int", &[label])?);
    }
    for (label, value) in d.float_labels.iter().zip(&d.float_values) {
        doc.add(FloatAssociationFacetField::new(*value, "Float", &[label])?);
    }
    doc.add(SortedSetDocValuesFacetField::new("SAuthor", &[&d.author])?);
    doc.add(SortedSetDocValuesFacetField::new("SDate", &date)?);
    for t in &d.tags {
        doc.add(SortedSetDocValuesFacetField::new("STags", &[t])?);
    }
    doc.add(LongPoint::new("price", &[d.price])?);
    doc.add(NumericDocValuesField::new("price", d.price));
    doc.add(DoubleDocValuesField::new("weight", d.weight));
    for m in &d.multi {
        doc.add(SortedNumericDocValuesField::new("multi", *m));
    }
    doc.add(LongRangeField::new("lr", &[d.lr_min], &[d.lr_max])?);
    doc.add(DoubleRangeField::new("dr", &[d.dr_min], &[d.dr_max])?);
    doc.add(
        LongRangeDocValuesField::new("lr", &[d.lr_min], &[d.lr_max])?
            .as_binary_range_doc_values_field()
            .clone(),
    );
    doc.add(
        DoubleRangeDocValuesField::new("dr", &[d.dr_min], &[d.dr_max])?
            .as_binary_range_doc_values_field()
            .clone(),
    );
    let sets: Vec<LongFacetSet> = d
        .sets
        .iter()
        .map(|s| LongFacetSet::new(s.to_vec()))
        .collect::<Result<_>>()?;
    let refs: Vec<&dyn FacetSet> = sets.iter().map(|s| s as &dyn FacetSet).collect();
    doc.add(FacetSetsField::create("fs", &refs)?);
    Ok(doc)
}

/// `build(Random, Directory, Directory, PrintStream)`; `perturb` (Rucene
/// only, for the positive control) renames the author of the first document
/// after its draws.
fn build(
    r: &JdkRandom,
    index_dir: &Arc<dyn Directory>,
    taxo_dir: &Arc<dyn Directory>,
    out: &mut String,
    perturb: bool,
) -> Result<Arc<FacetsConfig>> {
    let config = config();
    let w = IndexWriter::new(Arc::clone(index_dir), IndexWriterConfig::new())?;
    let tw = DirectoryTaxonomyWriter::new(Arc::clone(taxo_dir))?;
    for i in 0..NUM_DOCS {
        let mut d = gen(r, i);
        if perturb && i == 0 {
            d.author = "zulu".to_string();
        }
        outln!(out, "{}", d.describe());
        w.add_document(&config.build_with_taxonomy(Some(&tw), &document(&d)?)?)?;
        if i == NUM_DOCS / 2 - 1 {
            w.commit()?;
        }
    }
    TwoPhaseCommit::commit(&tw)?;
    w.commit()?;
    TaxonomyWriter::close(&tw)?;
    w.close()?;
    Ok(config)
}

/// `print(PrintStream, String, FacetResult)`.
fn print(out: &mut String, what: &str, result: Option<FacetResult>) {
    match result {
        Some(r) => out.push_str(&format!("{what} {r}")),
        None => outln!(out, "{what} null"),
    }
}

/// `printAll(PrintStream, String, List<FacetResult>)`.
fn print_all(out: &mut String, what: &str, results: Vec<FacetResult>) {
    outln!(out, "{what} {}", results.len());
    for fr in results {
        out.push_str(&fr.to_string());
    }
}

/// `value(PrintStream, String, Facets, String, String...)`.
fn value(
    out: &mut String,
    what: &str,
    facets: &dyn Facets,
    dim: &str,
    path: &[&str],
) -> Result<()> {
    let v = match facets.get_specific_value(dim, path) {
        Ok(v) => v.to_string(),
        Err(e) if e.is_illegal_argument_exception() || e.is_unsupported_operation_exception() => {
            java_throwable(&e).to_string()
        }
        Err(e) => return Err(e),
    };
    outln!(out, "{what} {v}");
    Ok(())
}

/// `hits(IndexSearcher, TopDocs)` over the hit doc ids.
fn hits_of(
    searcher: &IndexSearcher,
    total: i64,
    docs: impl Iterator<Item = i32>,
) -> Result<String> {
    let stored = searcher.stored_fields()?;
    let mut ids = Vec::new();
    for doc in docs {
        ids.push(stored.document(doc)?.get("id").unwrap_or_default());
    }
    ids.sort();
    Ok(format!("{total} {}", list(ids)))
}

fn hits(searcher: &IndexSearcher, td: &TopDocs) -> Result<String> {
    hits_of(
        searcher,
        td.total_hits.value(),
        td.score_docs.iter().map(|sd| sd.doc),
    )
}

fn facets_hits(searcher: &IndexSearcher, td: &FacetsTopDocs) -> Result<String> {
    match td {
        FacetsTopDocs::TopDocs(td) => hits(searcher, td),
        FacetsTopDocs::TopFieldDocs(td) => hits_of(
            searcher,
            td.total_hits.value(),
            td.score_docs.iter().map(|fd| fd.score_doc.doc),
        ),
    }
}

/// `new Term(field, text)`.
fn term(field: &str, text: &str) -> Term {
    Term::new(field.to_string(), BytesRef::from_char_sequence(text))
}

/// `queries()`.
fn queries() -> Vec<Arc<dyn Query>> {
    vec![
        Arc::new(MatchAllDocsQuery),
        Arc::new(TermQuery::new(term("body", "b0"))),
    ]
}

/// `FacetsCollectorManager.search(searcher, q, 10, new FacetsCollectorManager())`.
fn collect(searcher: &IndexSearcher, q: &Arc<dyn Query>) -> Result<(FacetsCollector, i64)> {
    let res = FacetsCollectorManager::search(
        searcher,
        Arc::clone(q),
        10,
        &FacetsCollectorManager::new(),
    )?;
    let total = res.top_docs().total_hits().value();
    Ok((res.facets_collector, total))
}

fn new_dirs() -> (Arc<dyn Directory>, Arc<dyn Directory>) {
    (
        Arc::new(ByteBuffersDirectory::new()),
        Arc::new(ByteBuffersDirectory::new()),
    )
}

fn aggregations() -> [(&'static str, Arc<dyn AssociationAggregationFunction>); 2] {
    [
        ("sum", Arc::new(SumAggregation)),
        ("max", Arc::new(MaxAggregation)),
    ]
}

/// `taxonomy(Random, PrintStream)`.
fn rust_taxonomy(seed: i64, perturb: bool) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let (index_dir, taxo_dir) = new_dirs();
    let config = build(&r, &index_dir, &taxo_dir, &mut out, perturb)?;
    let reader = directory_reader::open(Arc::clone(&index_dir))?;
    let taxo: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::open(taxo_dir)?);
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    outln!(out, "taxonomy size {}", taxo.get_size()?);
    for q in queries() {
        let (fc, total) = collect(&searcher, &q)?;
        outln!(out, "query {} hits {total}", query_to_string(&*q));
        let counts = FastTaxonomyFacetCounts::new(Arc::clone(&taxo), Arc::clone(&config), &fc)?;
        print_all(&mut out, "allDims", counts.get_all_dims(10)?);
        print(
            &mut out,
            "top3 date",
            counts.get_top_children(3, "Publish Date", &[])?,
        );
        print(
            &mut out,
            "top5 date/2011",
            counts.get_top_children(5, "Publish Date", &["2011"])?,
        );
        print(
            &mut out,
            "top5 date/2099",
            counts.get_top_children(5, "Publish Date", &["2099"])?,
        );
        print(
            &mut out,
            "all author",
            counts.get_all_children("Author", &[])?,
        );
        value(
            &mut out,
            "value author/alpha",
            &counts,
            "Author",
            &["alpha"],
        )?;
        value(&mut out, "value author", &counts, "Author", &[])?;
        value(
            &mut out,
            "value date/2012",
            &counts,
            "Publish Date",
            &["2012"],
        )?;
        print_all(&mut out, "topDims 2 2", counts.get_top_dims(2, 2)?);
        let tags = FastTaxonomyFacetCounts::with_index_field_name(
            "$tags",
            Arc::clone(&taxo),
            Arc::clone(&config),
            &fc,
        )?;
        print_all(&mut out, "tags allDims", tags.get_all_dims(10)?);
        print(
            &mut out,
            "tags top2",
            tags.get_top_children(2, "Tags", &[])?,
        );
        value(&mut out, "value tags", &tags, "Tags", &[])?;
        value(&mut out, "value tags/golf", &tags, "Tags", &["golf"])?;
        for (name, f) in aggregations() {
            let ints = TaxonomyFacetIntAssociations::with_index_field_name(
                "$int",
                Arc::clone(&taxo),
                Arc::clone(&config),
                &fc,
                Arc::clone(&f),
            )?;
            print_all(
                &mut out,
                &format!("int {name} allDims"),
                ints.get_all_dims(10)?,
            );
            print(
                &mut out,
                &format!("int {name} top2"),
                ints.get_top_children(2, "Int", &[])?,
            );
            value(
                &mut out,
                &format!("int {name} value/papa"),
                &ints,
                "Int",
                &["papa"],
            )?;
            let floats = TaxonomyFacetFloatAssociations::with_index_field_name(
                "$float",
                Arc::clone(&taxo),
                Arc::clone(&config),
                &fc,
                Arc::clone(&f),
            )?;
            print_all(
                &mut out,
                &format!("float {name} allDims"),
                floats.get_all_dims(10)?,
            );
            print(
                &mut out,
                &format!("float {name} top2"),
                floats.get_top_children(2, "Float", &[])?,
            );
            value(
                &mut out,
                &format!("float {name} value/tango"),
                &floats,
                "Float",
                &["tango"],
            )?;
        }
    }
    let all = FastTaxonomyFacetCounts::count_all_docs(
        DEFAULT_INDEX_FIELD_NAME,
        &(Arc::clone(&reader) as Arc<dyn IndexReader>),
        Arc::clone(&taxo),
        Arc::clone(&config),
    )?;
    print_all(&mut out, "countAll allDims", all.get_all_dims(10)?);
    taxo.close()?;
    reader.close()?;
    Ok(out)
}

/// `Executors.newFixedThreadPool(2)`.
fn fixed_thread_pool() -> Arc<ThreadPoolExecutor> {
    Arc::new(ThreadPoolExecutor::new(
        2,
        2,
        Duration::ZERO,
        NamedThreadFactory::new("facet-oracle"),
    ))
}

/// `sortedSet(Random, PrintStream)`.
fn rust_sorted_set(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let (index_dir, taxo_dir) = new_dirs();
    let config = build(&r, &index_dir, &taxo_dir, &mut out, false)?;
    let pool = fixed_thread_pool();
    let exec: Arc<dyn Executor> = Arc::clone(&pool) as Arc<dyn Executor>;
    let result = (|| -> Result<()> {
        let reader = directory_reader::open(Arc::clone(&index_dir))?;
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        let state: Arc<dyn SortedSetDocValuesReaderState> =
            Arc::new(DefaultSortedSetDocValuesReaderState::with_field(
                Arc::clone(&reader) as Arc<dyn IndexReader>,
                "$ssdv",
                Arc::clone(&config),
            )?);
        outln!(
            out,
            "state size {} dims {}",
            state.get_size(),
            list(state.get_dims())
        );
        for q in queries() {
            let (fc, _) = collect(&searcher, &q)?;
            outln!(out, "query {}", query_to_string(&*q));
            let ssdv = SortedSetDocValuesFacetCounts::with_hits(Arc::clone(&state), Some(&fc))?;
            print_all(&mut out, "allDims", ssdv.get_all_dims(10)?);
            print(
                &mut out,
                "top3 date",
                ssdv.get_top_children(3, "SDate", &[])?,
            );
            print(
                &mut out,
                "top10 date/2011",
                ssdv.get_top_children(10, "SDate", &["2011"])?,
            );
            print(&mut out, "all tags", ssdv.get_all_children("STags", &[])?);
            print(
                &mut out,
                "all date/2012/5",
                ssdv.get_all_children("SDate", &["2012", "5"])?,
            );
            value(&mut out, "value author/alpha", &ssdv, "SAuthor", &["alpha"])?;
            value(&mut out, "value date/2013", &ssdv, "SDate", &["2013"])?;
            print_all(&mut out, "topDims 2 3", ssdv.get_top_dims(2, 3)?);
            let conc = ConcurrentSortedSetDocValuesFacetCounts::with_hits(
                Arc::clone(&state),
                Some(&fc),
                Arc::clone(&exec),
            )?;
            print_all(&mut out, "concurrent allDims", conc.get_all_dims(10)?);
            print(
                &mut out,
                "concurrent top3 date",
                conc.get_top_children(3, "SDate", &[])?,
            );
        }
        print_all(
            &mut out,
            "countAll",
            SortedSetDocValuesFacetCounts::new(Arc::clone(&state))?.get_all_dims(10)?,
        );
        print_all(
            &mut out,
            "concurrent countAll",
            ConcurrentSortedSetDocValuesFacetCounts::new(Arc::clone(&state), Arc::clone(&exec))?
                .get_all_dims(10)?,
        );
        reader.close()
    })();
    pool.shutdown();
    result.map(|()| out)
}

/// The Java name of a `RangeFieldQuery.QueryType`.
fn query_type_name(t: RangeQueryType) -> &'static str {
    match t {
        RangeQueryType::Intersects => "INTERSECTS",
        RangeQueryType::Within => "WITHIN",
        RangeQueryType::Contains => "CONTAINS",
        RangeQueryType::Crosses => "CROSSES",
    }
}

/// `range(Random, PrintStream)`.
fn rust_range(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let (index_dir, taxo_dir) = new_dirs();
    build(&r, &index_dir, &taxo_dir, &mut out, false)?;
    let reader = directory_reader::open(Arc::clone(&index_dir))?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    let mut price = Vec::new();
    for i in 0..5 {
        let min = i64::from(r.next_int_bound(1000) - 150);
        let max = min + 2 + i64::from(r.next_int_bound(400));
        let min_inc = r.next_boolean();
        let max_inc = r.next_boolean();
        let range = LongRange::new(&format!("p{i}"), min, min_inc, max, max_inc)?;
        outln!(out, "price range {range}");
        price.push(range);
    }
    price.push(LongRange::new("all", i64::MIN, true, i64::MAX, true)?);
    let mut multi = Vec::new();
    for i in 0..4 {
        let min = i64::from(r.next_int_bound(50));
        let max = min + 1 + i64::from(r.next_int_bound(20));
        let range = LongRange::new(&format!("m{i}"), min, true, max, r.next_boolean())?;
        outln!(out, "multi range {range}");
        multi.push(range);
    }
    let mut weight = Vec::new();
    for i in 0..5 {
        let min = r.next_double() * 80.0;
        let max = min + r.next_double() * 40.0;
        let min_inc = r.next_boolean();
        let max_inc = r.next_boolean();
        let range = DoubleRange::new(&format!("w{i}"), min, min_inc, max, max_inc)?;
        outln!(out, "weight range {range}");
        weight.push(range);
    }
    let mut lr = Vec::new();
    for i in 0..4 {
        let min = i64::from(r.next_int_bound(120));
        let max = min + 2 + i64::from(r.next_int_bound(60));
        let min_inc = r.next_boolean();
        let max_inc = r.next_boolean();
        let range = RorLongRange::new(&format!("lr{i}"), min, min_inc, max, max_inc)?;
        outln!(out, "lr range {range}");
        lr.push(range);
    }
    let mut dr = Vec::new();
    for i in 0..4 {
        let min = r.next_double() * 100.0;
        let max = min + r.next_double() * 50.0;
        let min_inc = r.next_boolean();
        let max_inc = r.next_boolean();
        let range = RorDoubleRange::new(&format!("dr{i}"), min, min_inc, max, max_inc)?;
        outln!(out, "dr range {range}");
        dr.push(range);
    }
    let mut matchers: Vec<Arc<dyn FacetSetMatcher>> = Vec::new();
    for i in 0..3 {
        let a = i64::from(r.next_int_bound(5));
        let b = i64::from(r.next_int_bound(5));
        matchers.push(Arc::new(ExactFacetSetMatcher::new(
            &format!("e{i}({a},{b})"),
            &LongFacetSet::new(vec![a, b])?,
        )?));
    }
    for i in 0..2 {
        let a = i64::from(r.next_int_bound(4));
        let b = i64::from(r.next_int_bound(4));
        matchers.push(Arc::new(RangeFacetSetMatcher::new(
            &format!("r{i}({a}..{},{b}..{})", a + 1, b + 2),
            &[
                DimRange::from_longs(a, true, a + 1, true)?,
                DimRange::from_longs(b, true, b + 2, false)?,
            ],
        )?));
    }
    let decoder: Arc<dyn FacetSetDecoder> = Arc::new(decode_longs);
    for q in queries() {
        let (fc, _) = collect(&searcher, &q)?;
        outln!(out, "query {}", query_to_string(&*q));
        let p = LongRangeFacetCounts::new("price", &fc, price.clone())?;
        print(&mut out, "price top3", p.get_top_children(3, "price", &[])?);
        print(&mut out, "price all", p.get_all_children("price", &[])?);
        print_all(&mut out, "price allDims", p.get_all_dims(10)?);
        let m = LongRangeFacetCounts::new("multi", &fc, multi.clone())?;
        print(&mut out, "multi all", m.get_all_children("multi", &[])?);
        let w = DoubleRangeFacetCounts::new("weight", &fc, weight.clone())?;
        print(
            &mut out,
            "weight top10",
            w.get_top_children(10, "weight", &[])?,
        );
        print(&mut out, "weight all", w.get_all_children("weight", &[])?);
        for t in [
            RangeQueryType::Intersects,
            RangeQueryType::Within,
            RangeQueryType::Contains,
            RangeQueryType::Crosses,
        ] {
            let name = query_type_name(t);
            let l = LongRangeOnRangeFacetCounts::new("lr", &fc, t, &lr)?;
            print(
                &mut out,
                &format!("lr {name} all"),
                l.get_all_children("lr", &[])?,
            );
            let d = DoubleRangeOnRangeFacetCounts::new("dr", &fc, t, &dr)?;
            print(
                &mut out,
                &format!("dr {name} all"),
                d.get_all_children("dr", &[])?,
            );
        }
        let fs = MatchingFacetSetsCounts::new("fs", &fc, Arc::clone(&decoder), matchers.clone())?;
        print(&mut out, "fs top10", fs.get_top_children(10, "fs", &[])?);
        print(&mut out, "fs all", fs.get_all_children("fs", &[])?);
        print_all(&mut out, "fs allDims", fs.get_all_dims(10)?);
    }
    reader.close()?;
    Ok(out)
}

/// `sideways(PrintStream, String, IndexSearcher, DrillSidewaysResult, String...)`.
fn sideways(
    out: &mut String,
    what: &str,
    searcher: &IndexSearcher,
    res: &DrillSidewaysResult,
    dims: &[&str],
) -> Result<()> {
    let hits = res
        .hits
        .as_ref()
        .expect("DrillSideways.search returns hits");
    outln!(out, "{what} hits {}", facets_hits(searcher, hits)?);
    let facets = res
        .facets
        .as_ref()
        .expect("DrillSideways.search returns facets");
    for dim in dims {
        print(
            out,
            &format!("{what} {dim}"),
            facets.get_top_children(10, dim, &[])?,
        );
    }
    Ok(())
}

/// `drill(Random, PrintStream)`.
fn rust_drill(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let (index_dir, taxo_dir) = new_dirs();
    let config = build(&r, &index_dir, &taxo_dir, &mut out, false)?;
    let reader = directory_reader::open(Arc::clone(&index_dir))?;
    let taxo: Arc<dyn TaxonomyReader> = Arc::new(DirectoryTaxonomyReader::open(taxo_dir)?);
    let searcher = Arc::new(IndexSearcher::new(
        Arc::clone(&reader) as Arc<dyn IndexReader>
    )?);
    let state: Arc<dyn SortedSetDocValuesReaderState> =
        Arc::new(DefaultSortedSetDocValuesReaderState::with_field(
            Arc::clone(&reader) as Arc<dyn IndexReader>,
            "$ssdv",
            Arc::clone(&config),
        )?);
    for round in 0..4 {
        let a1 = vocab(4 + r.next_int_bound(6));
        let a2 = vocab(4 + r.next_int_bound(6));
        let year = (2010 + r.next_int_bound(4)).to_string();
        let tag = vocab(10 + r.next_int_bound(8));
        let with_base = r.next_boolean();
        let base: Option<Arc<dyn Query>> = if with_base {
            let body = format!("b{}", r.next_int_bound(3));
            Some(Arc::new(TermQuery::new(term("body", &body))))
        } else {
            None
        };
        let base_str = base
            .as_ref()
            .map_or_else(|| "null".to_string(), |b| query_to_string(&**b));
        outln!(out, "round {round} {a1} {a2} {year} {tag} {base_str}");

        let mut ddq = DrillDownQuery::with_base_query(Arc::clone(&config), base.clone());
        ddq.add("Author", &[&a1])?;
        ddq.add("Author", &[&a2])?;
        ddq.add("Publish Date", &[&year])?;
        outln!(out, "ddq {}", query_to_string(&ddq));
        let ddq_hits = searcher.search(Arc::new(ddq.clone_query()), 100)?;
        outln!(out, "ddq hits {}", hits(&searcher, &ddq_hits)?);
        let mut ddq_tags = DrillDownQuery::with_base_query(Arc::clone(&config), base.clone());
        ddq_tags.add("Tags", &[&tag])?;
        let tag_hits = searcher.search(Arc::new(ddq_tags), 100)?;
        outln!(out, "ddq tags hits {}", hits(&searcher, &tag_hits)?);

        let ds = DrillSideways::new(
            Arc::clone(&searcher),
            Arc::clone(&config),
            Arc::clone(&taxo),
        );
        let res = ds.search(ddq, 100)?;
        sideways(&mut out, "ds", &searcher, &res, &["Author", "Publish Date"])?;

        let mut sddq = DrillDownQuery::with_base_query(Arc::clone(&config), base.clone());
        sddq.add("SAuthor", &[&a1])?;
        sddq.add("STags", &[&tag])?;
        let sds = DrillSideways::with_state(
            Arc::clone(&searcher),
            Arc::clone(&config),
            Arc::clone(&state),
        );
        let res = sds.search(sddq, 100)?;
        sideways(
            &mut out,
            "sds",
            &searcher,
            &res,
            &["SAuthor", "STags", "SDate"],
        )?;
    }
    taxo.close()?;
    reader.close()?;
    Ok(out)
}

/// `files(Directory)`.
fn files(dir: &dyn Directory) -> Result<String> {
    let mut names: Vec<String> = dir
        .list_all()?
        .into_iter()
        .filter(|n| n != "write.lock")
        .collect();
    names.sort();
    Ok(list(names))
}

fn fs_dir(path: &Path) -> Result<Arc<dyn Directory>> {
    Ok(Arc::new(FSDirectory::open(path)?))
}

/// `taxoWrite(Random, Path, PrintStream)`.
fn rust_taxo_write(seed: i64, dir: &Path) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    {
        let index_dir = fs_dir(&dir.join("index"))?;
        let taxo_dir = fs_dir(&dir.join("taxo"))?;
        build(&r, &index_dir, &taxo_dir, &mut out, false)?;
        index_dir.close()?;
        taxo_dir.close()?;
    }
    out.push_str(&rust_taxo_read(dir)?);
    Ok(out)
}

/// `taxoRead(Path, PrintStream)`.
fn rust_taxo_read(dir: &Path) -> Result<String> {
    let mut out = String::new();
    let config = config();
    let index_dir = fs_dir(&dir.join("index"))?;
    let taxo_dir = fs_dir(&dir.join("taxo"))?;
    let reader = directory_reader::open(Arc::clone(&index_dir))?;
    let taxo: Arc<dyn TaxonomyReader> =
        Arc::new(DirectoryTaxonomyReader::open(Arc::clone(&taxo_dir))?);
    outln!(out, "taxo files {}", files(&*taxo_dir)?);
    outln!(out, "index files {}", files(&*index_dir)?);
    let commit: BTreeMap<String, String> = taxo.get_commit_user_data()?.into_iter().collect();
    let commit = commit
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ");
    outln!(out, "taxo commit {{{commit}}}");
    let size = taxo.get_size()?;
    outln!(out, "taxo size {size}");
    let arrays = taxo.get_parallel_taxonomy_arrays()?;
    for ord in 0..size {
        let label = taxo
            .get_path(ord)?
            .expect("every ordinal below getSize() has a path");
        outln!(
            out,
            "ord {ord} parent {} child {} sibling {} {label} -> {}",
            arrays.parents().get(ord),
            arrays.children().get(ord),
            arrays.siblings().get(ord),
            taxo.get_ordinal(&label)?
        );
    }
    outln!(
        out,
        "index maxDoc {} numDocs {}",
        reader.max_doc(),
        reader.num_docs()
    );
    let index_reader = Arc::clone(&reader) as Arc<dyn IndexReader>;
    for leaf in Arc::clone(&index_reader).leaves()? {
        outln!(
            out,
            "leaf {} maxDoc {}",
            leaf.ord(),
            leaf.leaf_reader().max_doc()
        );
    }
    let searcher = IndexSearcher::new(Arc::clone(&index_reader))?;
    let all = searcher.search(Arc::new(MatchAllDocsQuery), 100)?;
    outln!(out, "ids {}", hits(&searcher, &all)?);
    print_all(
        &mut out,
        "counts",
        FastTaxonomyFacetCounts::count_all_docs(
            DEFAULT_INDEX_FIELD_NAME,
            &index_reader,
            Arc::clone(&taxo),
            Arc::clone(&config),
        )?
        .get_all_dims(10)?,
    );
    print_all(
        &mut out,
        "tags counts",
        FastTaxonomyFacetCounts::count_all_docs(
            "$tags",
            &index_reader,
            Arc::clone(&taxo),
            Arc::clone(&config),
        )?
        .get_all_dims(10)?,
    );
    let match_all: Arc<dyn Query> = Arc::new(MatchAllDocsQuery);
    let (fc, _) = collect(&searcher, &match_all)?;
    print_all(
        &mut out,
        "int sum",
        TaxonomyFacetIntAssociations::with_index_field_name(
            "$int",
            Arc::clone(&taxo),
            Arc::clone(&config),
            &fc,
            Arc::new(SumAggregation),
        )?
        .get_all_dims(10)?,
    );
    print_all(
        &mut out,
        "float sum",
        TaxonomyFacetFloatAssociations::with_index_field_name(
            "$float",
            Arc::clone(&taxo),
            Arc::clone(&config),
            &fc,
            Arc::new(SumAggregation),
        )?
        .get_all_dims(10)?,
    );
    let state: Arc<dyn SortedSetDocValuesReaderState> =
        Arc::new(DefaultSortedSetDocValuesReaderState::with_field(
            Arc::clone(&index_reader),
            "$ssdv",
            Arc::clone(&config),
        )?);
    print_all(
        &mut out,
        "ssdv counts",
        SortedSetDocValuesFacetCounts::new(state)?.get_all_dims(10)?,
    );
    taxo.close()?;
    reader.close()?;
    index_dir.close()?;
    taxo_dir.close()?;
    Ok(out)
}

/// Asserts the Java dump of `scenario` is non-trivial: at least
/// `min_lines` lines, and every marker present.
fn non_trivial(scenario: &str, java: &str, min_lines: usize, markers: &[&str]) {
    let lines = java.lines().count();
    assert!(
        lines >= min_lines,
        "{scenario}: the Java dump has {lines} lines, fewer than {min_lines}"
    );
    for marker in markers {
        assert!(
            java.contains(marker),
            "{scenario}: the Java dump lacks {marker:?}"
        );
    }
}

/// Runs `scenario` on both sides and asserts the dumps equal and the Java one
/// non-trivial.
fn check(
    scenario: &str,
    rust: impl FnOnce(i64) -> Result<String>,
    min_lines: usize,
    markers: &[&str],
) {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Facet, scenario, SEED, &[]);
    non_trivial(scenario, &java, min_lines, markers);
    let rust = rust(SEED).unwrap_or_else(|e| panic!("{scenario}: Rucene failed: {e}"));
    sprint17_oracle::assert_same(&java, &rust);
}

#[test]
fn facet_oracle_taxonomy() {
    check(
        "taxonomy",
        |seed| rust_taxonomy(seed, false),
        200,
        &[
            "dim=Author path=[] value=40 childCount=",
            "dim=Tags path=[] value=",
            "int sum allDims 1",
            "float max allDims 1",
            "query body:b0 hits ",
            "IllegalArgumentException",
            "countAll allDims 2",
        ],
    );
}

#[test]
fn facet_oracle_sorted_set() {
    check(
        "sortedset",
        rust_sorted_set,
        200,
        &[
            "state size 85 dims [S",
            "dim=SDate path=[2011] value=",
            "concurrent countAll 3",
            "dim=STags path=[] value=",
        ],
    );
}

#[test]
fn facet_oracle_range() {
    check(
        "range",
        rust_range,
        200,
        &[
            "price top3 dim=price path=[] value=",
            "weight all dim=weight",
            "lr CROSSES all dim=lr",
            "dr WITHIN all dim=dr",
            "fs all dim=fs",
        ],
    );
}

#[test]
fn facet_oracle_drill() {
    check(
        "drill",
        rust_drill,
        100,
        &[
            "ddq ",
            "ds hits ",
            "sds hits ",
            "ds Author dim=Author",
            "sds STags dim=STags",
        ],
    );
}

/// A taxonomy and main index written by Rucene, read by Lucene; one written
/// by Lucene, read by Rucene; and both writes read alike.
#[test]
fn facet_oracle_taxonomy_index_compatibility() {
    sprint17_oracle::pin_tests_seed();
    let work = WorkDir::new("facet-taxo-compat");
    // Rucene writes, Lucene reads.
    let rucene_dir = work.path().join("rucene");
    let rust_written = rust_taxo_write(SEED, &rucene_dir).expect("Rucene writes the indexes");
    let rucene_dir_arg = rucene_dir.display().to_string();
    let java_read = sprint17_oracle::run(Module::Facet, "taxo-read", SEED, &[&rucene_dir_arg]);
    non_trivial(
        "taxo-read",
        &java_read,
        100,
        &[
            "taxo size ",
            "ord 1 parent 0",
            "index maxDoc 40 numDocs 40",
            "ssdv counts 3",
        ],
    );
    let rust_read = rust_taxo_read(&rucene_dir).expect("Rucene reads its own indexes");
    sprint17_oracle::assert_same(&java_read, &rust_read);
    // Lucene writes, Rucene reads.
    let lucene_dir = work.path().join("lucene");
    let lucene_dir_arg = lucene_dir.display().to_string();
    let java_written = sprint17_oracle::run(Module::Facet, "taxo-write", SEED, &[&lucene_dir_arg]);
    non_trivial(
        "taxo-write",
        &java_written,
        140,
        &["doc d39 ", "taxo size "],
    );
    let rust_read_java = rust_taxo_write_dump_of(&java_written, &lucene_dir);
    sprint17_oracle::assert_same(&java_written, &rust_read_java);
    // Both writes of one corpus hold the same taxonomy and index.
    sprint17_oracle::assert_same(&java_written, &rust_written);
}

/// Rucene's dump of the Lucene-written indexes: the corpus lines the Java
/// write printed (Rucene regenerates them from the seed), then Rucene's read.
fn rust_taxo_write_dump_of(java_written: &str, dir: &Path) -> String {
    let r = JdkRandom::new(SEED);
    let mut out = String::new();
    for i in 0..NUM_DOCS {
        outln!(out, "{}", gen(&r, i).describe());
    }
    assert!(
        java_written.starts_with(&out),
        "the Java write printed another corpus"
    );
    out.push_str(&rust_taxo_read(dir).expect("Rucene reads the Lucene-written indexes"));
    out
}

/// Positive control: a corpus whose first document names another author
/// gives a dump the comparison rejects.
#[test]
fn facet_oracle_positive_control() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Facet, "taxonomy", SEED, &[]);
    let perturbed = rust_taxonomy(SEED, true).expect("Rucene runs the perturbed corpus");
    let err = sprint17_oracle::compare(&java, &perturbed)
        .expect_err("the perturbed corpus must be detected");
    assert!(err.contains("author=zulu"), "{err}");
    // The facet results themselves (the dump without its corpus lines)
    // differ too: the perturbation is not caught by the corpus echo alone.
    let results = |dump: &str| {
        dump.lines()
            .filter(|l| !l.starts_with("doc "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let err = sprint17_oracle::compare(&results(&java), &results(&perturbed))
        .expect_err("the perturbed facet counts must be detected");
    assert!(err.contains("Java   taxonomy size"), "{err}");
}
