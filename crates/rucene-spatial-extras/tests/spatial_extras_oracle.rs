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

//! Java-oracle portability tests of `lucene/spatial-extras` (rmp #632):
//! `SpatialExtrasOracle.java <scenario>` and Rucene, from one seed, give the
//! same dump.
//!
//! * `terms`: the terms RecursivePrefixTreeStrategy and
//!   TermQueryPrefixTreeStrategy index over quad, geohash, packed-quad and S2
//!   trees (index compatibility of the cell bytes).
//! * `nr`: NumberRangePrefixTreeStrategy over DateRangePrefixTree (both
//!   template calendars): shapes, their parse round trip and indexed terms.
//! * `queries`: RPT (quad, geohash, S2), TermQuery, BBox, PointVector,
//!   SerializedDV and Composite strategies: every operation's matches, and
//!   the distance values.
//! * `heatmap`: HeatmapFacetCounter counts.
//! * `wkt`: WKTWriter / WKTReader round trips.
//! * `decimalFormat`: the WKT number format with HALF_EVEN, FLOOR and
//!   CEILING rounding (negatives, underflow such as 0.0009 at 2 digits).

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::analysis::tokenattributes::TermToBytesRefAttribute;
use rucene::analysis::{BytesTermAttributeImpl, TokenStream};
use rucene::document::{Document, Store, StringField};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::IndexReader;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{IndexSearcher, Query};
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene::util::java_fmt::RoundingMode;
use rucene::util::{java_math, BytesRef};
use rucene_queryparser::jdk::calendar::{self as calendar, Calendar};
use rucene_spatial_extras::bbox::b_box_strategy::BBoxStrategy;
use rucene_spatial_extras::composite::composite_spatial_strategy::CompositeSpatialStrategy;
use rucene_spatial_extras::prefix::heatmap_facet_counter;
use rucene_spatial_extras::prefix::number_range_prefix_tree_strategy::NumberRangePrefixTreeStrategy;
use rucene_spatial_extras::prefix::prefix_tree_strategy::PrefixTreeStrategy;
use rucene_spatial_extras::prefix::recursive_prefix_tree_strategy::{
    RecursivePrefixTreeStrategy, RecursivePrefixTreeStrategyOps,
};
use rucene_spatial_extras::prefix::term_query_prefix_tree_strategy::TermQueryPrefixTreeStrategy;
use rucene_spatial_extras::prefix::tree::date_range_prefix_tree::{
    DateRangePrefixTree, DEFAULT_CAL, JAVA_UTIL_TIME_COMPAT_CAL,
};
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::number_range_prefix_tree::{
    NRShape, NumberRangePrefixTreeImpl,
};
use rucene_spatial_extras::prefix::tree::packed_quad_prefix_tree::PackedQuadPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::s2_prefix_tree::S2PrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use rucene_spatial_extras::query::spatial_args::SpatialArgs;
use rucene_spatial_extras::query::spatial_operation::SpatialOperation;
use rucene_spatial_extras::serialized::serialized_dv_strategy::SerializedDVStrategy;
use rucene_spatial_extras::spatial4j::geo3d_spatial_context_factory::Geo3dSpatialContextFactory;
use rucene_spatial_extras::spatial_strategy::SpatialStrategy;
use rucene_spatial_extras::vector::point_vector_strategy::PointVectorStrategy;
use rucene_test_framework::util::java_exceptions::{java_class_name, java_throwable};
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::SpatialContextFactoryOps;
use spatial4j::io::legacy_shape_writer::LegacyShapeWriter;
use spatial4j::io::shape_reader::ShapeReader;
use spatial4j::io::shape_writer::ShapeWriter;
use spatial4j::shape::shape::Shape;
use sprint17_oracle::{bits64 as bits, escape, Module};

const SEED_TERMS: i64 = 0x0632_0001;
const SEED_NR: i64 = 0x0632_0002;
const SEED_QUERIES: i64 = 0x0632_0003;
const SEED_HEATMAP: i64 = 0x0632_0004;
const SEED_WKT: i64 = 0x0632_0005;
const SEED_DECIMAL_FORMAT: i64 = 0x0632_0006;

/// `hex(BytesRef)`.
fn hex(b: &BytesRef) -> String {
    b.bytes[b.offset..b.offset + b.length]
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// `indexedTerms(PrefixTreeStrategy, Shape)`.
fn indexed_terms(strategy: &dyn PrefixTreeStrategy, shape: &Arc<dyn Shape>) -> Result<String> {
    let mut ts = strategy.token_stream();
    ts.set_shape(shape)?;
    let mut sb = String::new();
    ts.reset()?;
    let mut n = 0;
    while ts.increment_token()? {
        let bytes = ts
            .attribute_source()
            .get_attribute::<BytesTermAttributeImpl>()
            .expect("the BytesTermAttribute")
            .get_bytes_ref();
        sb.push(' ');
        sb.push_str(&hex(&bytes));
        n += 1;
    }
    ts.end()?;
    ts.close()?;
    Ok(format!("{n}{sb}"))
}

/// `randomShape(Random, SpatialContext, StringBuilder)`.
fn random_shape(r: &dyn Random, ctx: &SpatialContext, desc: &mut String) -> Result<Arc<dyn Shape>> {
    let sf = ctx.get_shape_factory();
    let kind = r.next_int_bound(3);
    let x = r.next_double() * 360.0 - 180.0;
    let y = r.next_double() * 180.0 - 90.0;
    match kind {
        0 => {
            desc.push_str(&format!("point {} {}", bits(x), bits(y)));
            Ok(sf.point_xy(x, y)?)
        }
        1 => {
            let w = r.next_double() * 40.0;
            let h = r.next_double() * 20.0;
            let max_x = if x + w > 180.0 { x + w - 360.0 } else { x + w };
            let max_y = f64::min(90.0, y + h);
            desc.push_str(&format!(
                "rect {} {} {} {}",
                bits(x),
                bits(max_x),
                bits(y),
                bits(max_y)
            ));
            Ok(sf.rect(x, max_x, y, max_y)?)
        }
        _ => {
            let d = r.next_double() * 10.0;
            desc.push_str(&format!("circle {} {} {}", bits(x), bits(y), bits(d)));
            Ok(sf.circle(x, y, d)?)
        }
    }
}

/// `terms`.
fn rust_terms(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let geo = SpatialContext::geo();
    let geo3d = Geo3dSpatialContextFactory::new().new_spatial_context()?;
    for t in 0..6 {
        let (ctx, grid, name): (Arc<SpatialContext>, Arc<dyn SpatialPrefixTree>, String) = match t {
            0 => (
                geo.clone(),
                QuadPrefixTree::new(geo.clone(), 8),
                "quad8".into(),
            ),
            1 => (
                geo.clone(),
                GeohashPrefixTree::new(geo.clone(), 4)?,
                "geohash4".into(),
            ),
            2 => {
                let pq = PackedQuadPrefixTree::new(geo.clone(), 10)?;
                pq.set_prune_leafy_branches(r.next_boolean());
                (geo.clone(), pq, "packedquad10".into())
            }
            _ => {
                let arity = t - 2;
                (
                    geo3d.clone(),
                    S2PrefixTree::with_arity(geo3d.clone(), 5 - arity, arity)?,
                    format!("s2-{arity}"),
                )
            }
        };
        for _ in 0..8 {
            let mut desc = String::new();
            let shape = random_shape(&r, &ctx, &mut desc)?;
            let dist_err_pct = if r.next_boolean() { 0.025 } else { 0.15 };
            let prune = r.next_boolean();
            let rpt = RecursivePrefixTreeStrategy::new(Arc::clone(&grid), "rpt")?;
            rpt.set_dist_err_pct(dist_err_pct);
            rpt.set_prune_leafy_branches(prune);
            if shape.as_point().is_some() && r.next_boolean() {
                rpt.set_points_only(true);
            }
            let tq = TermQueryPrefixTreeStrategy::new(Arc::clone(&grid), "tq")?;
            tq.set_dist_err_pct(dist_err_pct);
            out.push_str(&format!("{name} {desc} {} {prune}\n", bits(dist_err_pct)));
            out.push_str(&format!("rpt {}\n", indexed_terms(&*rpt, &shape)?));
            out.push_str(&format!("tq {}\n", indexed_terms(&*tq, &shape)?));
        }
    }
    Ok(out)
}

/// `Calendar.compareTo`.
fn cal_compare(a: &Calendar, b: &Calendar) -> Result<i32> {
    let millis = |c: &Calendar| -> Result<i64> {
        let mut c = c.clone();
        c.set_lenient(true);
        c.get_time_in_millis()
    };
    Ok(match millis(a)?.cmp(&millis(b)?) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    })
}

/// `randomCal(Random, DateRangePrefixTree)`.
fn random_cal(r: &dyn Random, tree: &DateRangePrefixTree) -> Calendar {
    let mut cal = tree.new_cal();
    cal.set_time_in_millis(r.next_long() % 315_569_520_000_000);
    let fields = [
        calendar::YEAR,
        calendar::MONTH,
        calendar::DAY_OF_MONTH,
        calendar::HOUR_OF_DAY,
        calendar::MINUTE,
        calendar::SECOND,
        calendar::MILLISECOND,
    ];
    tree.clear_fields_after(
        &mut cal,
        fields[r.next_int_bound(fields.len() as i32) as usize] as i32,
    );
    cal
}

/// `nr`.
fn rust_nr(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    for template in [&*DEFAULT_CAL, &*JAVA_UTIL_TIME_COMPAT_CAL] {
        let tree = DateRangePrefixTree::new(template)?;
        let strategy = NumberRangePrefixTreeStrategy::new(tree.clone(), "dateRange")?;
        for _ in 0..25 {
            let mut c1 = random_cal(&r, &tree);
            let s1 = tree.to_shape(&mut c1)?;
            let mut shape: Arc<dyn NRShape> = s1.clone();
            if r.next_boolean() {
                let mut c2 = random_cal(&r, &tree);
                let s2 = tree.to_shape(&mut c2)?;
                let range = if cal_compare(&c1, &c2)? < 0 {
                    tree.to_range_shape(s1.clone(), s2)
                } else {
                    tree.to_range_shape(s2, s1.clone())
                };
                match range {
                    Ok(range) => shape = range,
                    Err(e) if e.is_illegal_argument_exception() => {
                        out.push_str(&format!(
                            "iae {}\n",
                            escape(java_throwable(&e).message().unwrap_or("null"))
                        ));
                    }
                    Err(e) => return Err(e),
                }
            }
            let str = shape.to_string();
            out.push_str(&format!(
                "shape {} cal {}\n",
                escape(&str),
                tree.to_string_calendar(&mut c1)?
            ));
            out.push_str(&format!(
                "parsed {}\n",
                escape(&tree.parse_shape(&str)?.to_string())
            ));
            strategy.set_points_only(false);
            let shape: Arc<dyn Shape> = shape;
            out.push_str(&format!("terms {}\n", indexed_terms(&*strategy, &shape)?));
        }
    }
    Ok(out)
}

/// `matches(IndexSearcher, Query)`.
fn matches(searcher: &IndexSearcher, query: Arc<dyn Query>) -> Result<String> {
    let docs = searcher.search(query, 1000)?;
    let mut ids = Vec::new();
    let stored = searcher.stored_fields()?;
    for sd in &docs.score_docs {
        ids.push(stored.document(sd.doc)?.get("id").unwrap_or_default());
    }
    ids.sort();
    Ok(format!("{} [{}]", docs.total_hits.value(), ids.join(", ")))
}

/// `queries`.
fn rust_queries(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let geo = SpatialContext::geo();
    for s in 0..8 {
        let mut ctx = geo.clone();
        let mut points_only = false;
        let strategy: Arc<dyn SpatialStrategy> = match s {
            0 => RecursivePrefixTreeStrategy::new(QuadPrefixTree::new(ctx.clone(), 6), "rpt_quad")?,
            1 => RecursivePrefixTreeStrategy::new(
                GeohashPrefixTree::new(ctx.clone(), 3)?,
                "rpt_geohash",
            )?,
            2 => TermQueryPrefixTreeStrategy::new(
                GeohashPrefixTree::new(ctx.clone(), 3)?,
                "tq_geohash",
            )?,
            3 => BBoxStrategy::new_instance(ctx.clone(), "bbox")?,
            4 => {
                points_only = true;
                PointVectorStrategy::new_instance(ctx.clone(), "pv")?
            }
            5 => Arc::new(SerializedDVStrategy::new(ctx.clone(), "sdv")?),
            6 => {
                let rpt = RecursivePrefixTreeStrategy::new(
                    GeohashPrefixTree::new(ctx.clone(), 3)?,
                    "comp_rpt",
                )?;
                Arc::new(CompositeSpatialStrategy::new(
                    "comp",
                    rpt,
                    Arc::new(SerializedDVStrategy::new(ctx.clone(), "comp_sdv")?),
                )?)
            }
            _ => {
                ctx = Geo3dSpatialContextFactory::new().new_spatial_context()?;
                RecursivePrefixTreeStrategy::new(
                    S2PrefixTree::with_arity(ctx.clone(), 4, 1)?,
                    "rpt_s2",
                )?
            }
        };
        out.push_str(&format!("strategy {}\n", strategy.get_field_name()));
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let w = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new())?;
        for d in 0..12 {
            let mut desc = String::new();
            let shape: Arc<dyn Shape> = if points_only {
                let x = r.next_double() * 360.0 - 180.0;
                let y = r.next_double() * 180.0 - 90.0;
                desc.push_str(&format!("point {} {}", bits(x), bits(y)));
                ctx.get_shape_factory().point_xy(x, y)?
            } else {
                random_shape(&r, &ctx, &mut desc)?
            };
            let mut doc = Document::new();
            doc.add(StringField::new("id", format!("{d:02}"), Store::YES)?);
            for f in strategy.create_indexable_fields(&shape)? {
                doc.add_shared(f);
            }
            w.add_document(&doc)?;
            out.push_str(&format!("doc {d} {desc}\n"));
        }
        w.close()?;
        let reader = directory_reader::open(Arc::clone(&dir))?;
        let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
        for op in SpatialOperation::values() {
            for _ in 0..3 {
                let mut desc = String::new();
                let query_shape = random_shape(&r, &ctx, &mut desc)?;
                let result = match strategy
                    .make_query(&SpatialArgs::new(op, query_shape))
                    .and_then(|q| matches(&searcher, q))
                {
                    Ok(m) => m,
                    Err(e) if e.is_unsupported_operation_exception() => {
                        format!("unsupported {}", java_class_name(&e))
                    }
                    Err(e) => return Err(e),
                };
                out.push_str(&format!("{op} {desc} -> {result}\n"));
            }
        }
        let px = r.next_double() * 360.0 - 180.0;
        let py = r.next_double() * 180.0 - 90.0;
        let mut dist = format!("distance {} {}", bits(px), bits(py));
        let values = || -> Result<String> {
            let vs: Arc<dyn DoubleValuesSource> = strategy
                .make_distance_value_source_default(&ctx.get_shape_factory().point_xy(px, py)?)?;
            let mut s = String::new();
            for leaf in Arc::clone(searcher.get_top_reader_context()).leaves() {
                let mut v = Arc::clone(&vs).get_values(&leaf, None)?;
                for doc in 0..leaf.leaf_reader().max_doc() {
                    s.push(' ');
                    if v.advance_exact(doc)? {
                        s.push_str(&bits(v.double_value()?));
                    } else {
                        s.push_str("none");
                    }
                }
            }
            Ok(s)
        };
        match values() {
            Ok(s) => dist.push_str(&s),
            Err(e) if e.is_unsupported_operation_exception() => dist.push_str(" unsupported"),
            Err(e) => return Err(e),
        }
        out.push_str(&dist);
        out.push('\n');
        reader.close()?;
    }
    Ok(out)
}

/// `heatmap`.
fn rust_heatmap(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let ctx = SpatialContext::geo();
    let strategy = RecursivePrefixTreeStrategy::new(QuadPrefixTree::new(ctx.clone(), 6), "heat")?;
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    let w = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new())?;
    for _ in 0..30 {
        let mut desc = String::new();
        let shape = random_shape(&r, &ctx, &mut desc)?;
        let mut doc = Document::new();
        for f in strategy.create_indexable_fields(&shape)? {
            doc.add_shared(f);
        }
        w.add_document(&doc)?;
        out.push_str(&format!("doc {desc}\n"));
    }
    w.close()?;
    let reader = directory_reader::open(Arc::clone(&dir))?;
    let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
    for i in 0..6 {
        let region: Option<Arc<dyn Shape>> = if i < 3 {
            None
        } else {
            let x = r.next_double() * 360.0 - 180.0;
            let y = r.next_double() * 160.0 - 80.0;
            let w2 = r.next_double() * 100.0;
            let max_x = if x + w2 > 180.0 {
                x + w2 - 360.0
            } else {
                x + w2
            };
            let max_y = f64::min(90.0, y + r.next_double() * 30.0);
            Some(ctx.get_shape_factory().rect(x, max_x, y, max_y)?)
        };
        let level = 1 + (i % 3) + if i < 3 { 0 } else { 1 };
        let h = heatmap_facet_counter::calc_facets(
            &*strategy,
            searcher.get_top_reader_context(),
            None,
            region.clone(),
            level,
            100_000,
        )?;
        let counts: Vec<String> = h.counts.iter().map(|c| c.to_string()).collect();
        out.push_str(&format!(
            "heatmap {level} {} -> {}x{} {} [{}]\n",
            region.map_or_else(|| "world".to_string(), |s| s.to_string()),
            h.columns,
            h.rows,
            h.region,
            counts.join(", ")
        ));
    }
    reader.close()?;
    Ok(out)
}

/// `roundTripWkt`.
fn round_trip_wkt(writer: &dyn ShapeWriter, reader: &dyn ShapeReader, wkt: &str, out: &mut String) {
    match reader.read(wkt) {
        Ok(Some(s)) => out.push_str(&format!(
            "read {wkt} -> {s} -> {}\n",
            writer.to_string_of(&*s)
        )),
        Ok(None) => out.push_str(&format!("read {wkt} -> null\n")),
        Err(e) => {
            let t = java_throwable(&e);
            out.push_str(&format!(
                "read {wkt} -> {}: {}\n",
                t.class_name(),
                escape(t.message().unwrap_or("null"))
            ));
        }
    }
}

/// `wkt`.
fn rust_wkt(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let ctx = SpatialContext::geo();
    let writer = ctx.get_formats().get_wkt_writer().expect("a WKT writer");
    let reader = ctx.get_formats().get_wkt_reader().expect("a WKT reader");
    let fixed = [
        "POINT(-0.0009 0.0009)",
        "ENVELOPE(-10.123456789, 10.987654321, 20.5, -20.5)",
        "BUFFER(POINT(-80 33), 1.0000005)",
        "LINESTRING(1 2, 3.25 -4.125, -5 6)",
        "MULTIPOINT((10 40), (40 30), (-20 -20))",
        "ENVELOPE(170.0000001, -170.0000001, 0.0000004, -0.0000004)",
        "POLYGON((0 0, 10 0, 10 10, 0 10, 0 0))",
        "GEOMETRYCOLLECTION(POINT(4 6),LINESTRING(4 6,7 10))",
    ];
    for wkt in fixed {
        round_trip_wkt(&*writer, &*reader, wkt, &mut out);
    }
    for _ in 0..20 {
        let mut desc = String::new();
        let shape = random_shape(&r, &ctx, &mut desc)?;
        let wkt = writer.to_string_of(&*shape);
        out.push_str(&format!("shape {desc} -> {wkt} | {shape}\n"));
        round_trip_wkt(&*writer, &*reader, &wkt, &mut out);
    }
    Ok(out)
}

/// `decimalFormat`.
fn rust_decimal_format(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let mut values: Vec<f64> = vec![
        0.0009,
        -0.0009,
        0.0,
        -0.0,
        1.005,
        -1.005,
        2.5,
        -2.5,
        0.125,
        -0.125,
        123.456789,
        -123.456789,
        1e-7,
        -1e-7,
        179.9999999,
        -179.9999999,
        0.995,
        -0.995,
        1e15,
        -1e15,
        0.5,
        -0.5,
    ];
    for _ in 0..30 {
        let a = r.next_double() * 2.0 - 1.0;
        let e = r.next_int_bound(8) - 4;
        values.push(a * java_math::pow(10.0, f64::from(e)));
    }
    for digits in [0usize, 1, 2, 6] {
        let def = LegacyShapeWriter::make_number_format(digits);
        let mut floor = LegacyShapeWriter::make_number_format(digits);
        floor.set_rounding_mode(RoundingMode::FLOOR);
        let mut ceiling = LegacyShapeWriter::make_number_format(digits);
        ceiling.set_rounding_mode(RoundingMode::CEILING);
        for &v in &values {
            out.push_str(&format!(
                "{digits} {} {} {} {}\n",
                bits(v),
                def.format(v),
                floor.format(v),
                ceiling.format(v)
            ));
        }
    }
    out
}

/// Runs `scenario` on both sides and asserts equal dumps.
fn check(scenario: &str, seed: i64, rust: impl FnOnce(i64) -> Result<String>) {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::SpatialExtras, scenario, seed, &[]);
    let rust = rust(seed).unwrap_or_else(|e: LuceneError| panic!("{scenario}: {e:?}"));
    sprint17_oracle::assert_same(&java, &rust);
}

#[test]
fn spatial_extras_terms_oracle() {
    check("terms", SEED_TERMS, rust_terms);
}

#[test]
fn spatial_extras_nr_oracle() {
    check("nr", SEED_NR, rust_nr);
}

#[test]
fn spatial_extras_queries_oracle() {
    check("queries", SEED_QUERIES, rust_queries);
}

#[test]
fn spatial_extras_heatmap_oracle() {
    check("heatmap", SEED_HEATMAP, rust_heatmap);
}

#[test]
fn spatial_extras_wkt_oracle() {
    check("wkt", SEED_WKT, rust_wkt);
}

#[test]
fn spatial_extras_decimal_format_oracle() {
    check("decimalFormat", SEED_DECIMAL_FORMAT, |s| {
        Ok(rust_decimal_format(s))
    });
}
