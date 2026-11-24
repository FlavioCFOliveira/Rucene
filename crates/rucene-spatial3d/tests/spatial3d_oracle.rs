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

//! The Java-oracle portability tests of `lucene/spatial3d` (rmp #631, harness
//! of rmp #628): `Spatial3dOracle.java geom` and `Spatial3dOracle.java
//! queries` and Rucene, from one seed, give the same output.
//!
//! * `geom`: forty seeded shapes (circle, exact circle, bounding box,
//!   polygon, path) on seeded planet models through the public factories:
//!   each shape's `toString`, its `XYZBounds` and `LatLonBounds`, ten seeded
//!   points (`isWithin`, outside and inside arc distances), five seeded XYZ
//!   solids (`GeoArea.getRelationship`) and its relationship to the previous
//!   area shape.
//! * `queries`: three hundred seeded `Geo3DPoint`s indexed, then twenty seeded
//!   shapes as `Geo3DPoint.newShapeQuery`: each query's `toString`, total hits
//!   and the sorted ids matched.
//!
//! Doubles are compared as their bits.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use std::collections::BTreeSet;
use std::f64::consts::PI;
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene::document::{Document, NumericDocValuesField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_spatial3d::geo3_d_point::Geo3DPoint;
use rucene_spatial3d::geom::distance_style::ARC;
use rucene_spatial3d::geom::geo_area_factory::GeoAreaFactory;
use rucene_spatial3d::geom::geo_area_shape::GeoAreaShape;
use rucene_spatial3d::geom::geo_b_box_factory::GeoBBoxFactory;
use rucene_spatial3d::geom::geo_circle_factory::GeoCircleFactory;
use rucene_spatial3d::geom::geo_distance::GeoDistance;
use rucene_spatial3d::geom::geo_path_factory::GeoPathFactory;
use rucene_spatial3d::geom::geo_point::GeoPoint;
use rucene_spatial3d::geom::geo_polygon_factory::GeoPolygonFactory;
use rucene_spatial3d::geom::geo_shape::GeoShape;
use rucene_spatial3d::geom::lat_lon_bounds::LatLonBounds;
use rucene_spatial3d::geom::planet_model::{PlanetModel, CLARKE_1866, SPHERE, WGS84};
use rucene_spatial3d::geom::xyz_bounds::XYZBounds;
use sprint17_oracle::{bits64 as bits, escape, Module};

const SEED_GEOM: i64 = 0x0631_0001;
const SEED_QUERIES: i64 = 0x0631_0002;

/// `model(Random)`.
fn model(r: &JdkRandom) -> Arc<PlanetModel> {
    match r.next_int_bound(3) {
        0 => SPHERE.clone(),
        1 => WGS84.clone(),
        _ => CLARKE_1866.clone(),
    }
}

fn lat(r: &JdkRandom) -> f64 {
    (r.next_double() * 2.0 - 1.0) * PI / 2.0
}

fn lon(r: &JdkRandom) -> f64 {
    (r.next_double() * 2.0 - 1.0) * PI
}

/// `ring(Random, PlanetModel, GeoPoint, int, double)`.
fn ring(
    r: &JdkRandom,
    pm: &PlanetModel,
    center: &GeoPoint,
    count: i32,
    max_dist: f64,
) -> Result<Vec<GeoPoint>> {
    let mut points = Vec::new();
    for i in 0..count {
        let bearing = (f64::from(i) + r.next_double() * 0.8) * 2.0 * PI / f64::from(count);
        let dist = (0.2 + 0.8 * r.next_double()) * max_dist;
        points.push(pm.surface_point_on_bearing(center, dist, bearing)?);
    }
    Ok(points)
}

/// `Math.min(double, double)` / `Math.max(double, double)` over the
/// non-`NaN`, non-equal-zero values the scenarios draw.
fn min(a: f64, b: f64) -> f64 {
    if a <= b {
        a
    } else {
        b
    }
}

fn max(a: f64, b: f64) -> f64 {
    if a >= b {
        a
    } else {
        b
    }
}

/// A shape of `shape(Random, PlanetModel)`, with its inside distance when it
/// is a `GeoDistanceShape` (circles, paths, the degenerate point a bounding
/// box may be).
struct Shape {
    shape: Arc<dyn GeoShape>,
    distance: Option<Arc<dyn GeoDistance>>,
}

/// `shape(Random, PlanetModel)`: `None` for a degenerate polygon.
fn shape(r: &JdkRandom, pm: &Arc<PlanetModel>) -> Result<Option<Shape>> {
    let shape_type = r.next_int_bound(5);
    let lat = lat(r);
    let lon = lon(r);
    let with_circle_cast = |s: Arc<dyn GeoShape>| {
        let distance = s
            .clone()
            .into_geo_circle()
            .map(|c| c as Arc<dyn GeoDistance>);
        Some(Shape { shape: s, distance })
    };
    Ok(match shape_type {
        0 => with_circle_cast(GeoCircleFactory::make_geo_circle(
            pm,
            lat,
            lon,
            r.next_double() * 0.5,
        )?),
        1 => with_circle_cast(GeoCircleFactory::make_exact_geo_circle(
            pm,
            lat,
            lon,
            r.next_double() * 0.5,
            1e-6,
        )?),
        2 => {
            let lat2 = self::lat(r);
            let lon2 = self::lon(r);
            with_circle_cast(GeoBBoxFactory::make_geo_bbox(
                pm,
                max(lat, lat2),
                min(lat, lat2),
                min(lon, lon2),
                max(lon, lon2),
            )?)
        }
        3 => {
            let center = GeoPoint::new_lat_lon(pm, lat, lon)?;
            let count = 3 + r.next_int_bound(5);
            let max_dist = r.next_double() * 0.6;
            let points = ring(r, pm, &center, count, max_dist)?;
            GeoPolygonFactory::make_geo_polygon(pm, points)?.map(|p| Shape {
                shape: p,
                distance: None,
            })
        }
        _ => {
            let start = GeoPoint::new_lat_lon(pm, lat, lon)?;
            let n = 1 + r.next_int_bound(3);
            let mut points = vec![start];
            for i in 1..n as usize {
                let dist = r.next_double() * 0.3;
                let bearing = r.next_double() * 2.0 * PI;
                points.push(pm.surface_point_on_bearing(&points[i - 1], dist, bearing)?);
            }
            let path = GeoPathFactory::make_geo_path(pm, r.next_double() * 0.2, &points)?;
            Some(Shape {
                shape: path.clone(),
                distance: Some(path),
            })
        }
    })
}

fn obits(d: Option<f64>) -> String {
    d.map_or_else(|| "null".to_string(), bits)
}

/// `geom(Random, PrintStream)`.
fn rust_geom(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let mut previous: Option<Arc<dyn GeoAreaShape>> = None;
    for _i in 0..40 {
        let pm = model(&r);
        out.push_str(&format!("model {pm}\n"));
        let s = match shape(&r, &pm) {
            Ok(Some(s)) => s,
            Ok(None) => {
                out.push_str("shape null\n");
                continue;
            }
            Err(e) => {
                out.push_str(&format!("shape {}\n", e.kind().java_simple_name()));
                continue;
            }
        };
        let shape = &s.shape;
        out.push_str(&format!("shape {}\n", escape(&shape.to_string())));
        let mut xyz = XYZBounds::new();
        shape.get_bounds(&mut xyz);
        out.push_str(&format!(
            "xyz {} {} {} {} {} {}\n",
            obits(xyz.get_minimum_x()),
            obits(xyz.get_maximum_x()),
            obits(xyz.get_minimum_y()),
            obits(xyz.get_maximum_y()),
            obits(xyz.get_minimum_z()),
            obits(xyz.get_maximum_z())
        ));
        let mut ll = LatLonBounds::new();
        shape.get_bounds(&mut ll);
        out.push_str(&format!(
            "latlon {} {} {} {} {} {} {}\n",
            ll.check_no_longitude_bound(),
            ll.check_no_top_latitude_bound(),
            ll.check_no_bottom_latitude_bound(),
            obits(ll.get_min_latitude()),
            obits(ll.get_max_latitude()),
            obits(ll.get_left_longitude()),
            obits(ll.get_right_longitude())
        ));
        let membership = shape.clone().into_geo_membership_shape();
        for _j in 0..10 {
            let p = GeoPoint::new_lat_lon(&pm, lat(&r), lon(&r))?;
            let mut line = format!("point {}", shape.is_within(&p));
            if let Some(m) = &membership {
                line.push_str(&format!(" {}", bits(m.compute_outside_distance(&ARC, &p)?)));
            }
            if let Some(d) = &s.distance {
                line.push_str(&format!(" {}", bits(d.compute_distance(&ARC, &p)?)));
            }
            out.push_str(&line);
            out.push('\n');
        }
        for _j in 0..5 {
            let mut v = [0.0f64; 6];
            for k in v.iter_mut() {
                *k = r.next_double() * 2.2 - 1.1;
            }
            let solid = GeoAreaFactory::make_geo_area_xyz(
                &pm,
                min(v[0], v[1]),
                max(v[0], v[1]),
                min(v[2], v[3]),
                max(v[2], v[3]),
                min(v[4], v[5]),
                max(v[4], v[5]),
            )?;
            out.push_str(&format!("solid {}\n", solid.get_relationship(&**shape)?));
        }
        if let Some(area) = shape.clone().into_geo_area_shape() {
            if let Some(prev) = &previous {
                if **prev.get_planet_model() == *pm {
                    out.push_str(&format!("previous {}\n", area.get_relationship(&**prev)?));
                }
            }
            previous = Some(area);
        }
    }
    Ok(out)
}

/// `queries(Random, PrintStream)`.
fn rust_queries(seed: i64) -> Result<String> {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    let pm = model(&r);
    out.push_str(&format!("model {pm}\n"));
    let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
    {
        let w = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new())?;
        for id in 0..300 {
            let mut doc = Document::new();
            doc.add(NumericDocValuesField::new("id", id));
            doc.add(Geo3DPoint::new_lat_lon_with_model(
                "point",
                &pm,
                lat(&r).to_degrees(),
                lon(&r).to_degrees(),
            )?);
            w.add_document(&doc)?;
        }
        w.close()?;
    }
    let reader = directory_reader::open(Arc::clone(&dir))?;
    let r_index: Arc<dyn IndexReader> = Arc::clone(&reader) as Arc<dyn IndexReader>;
    let searcher = IndexSearcher::new(Arc::clone(&r_index))?;
    for _i in 0..20 {
        let s = match shape(&r, &pm) {
            Ok(Some(s)) => s,
            Ok(None) => {
                out.push_str("shape null\n");
                continue;
            }
            Err(e) => {
                out.push_str(&format!("shape {}\n", e.kind().java_simple_name()));
                continue;
            }
        };
        let q = Geo3DPoint::new_shape_query("point", s.shape);
        out.push_str(&format!("query {}\n", escape(&q.to_query_string(""))));
        let top = searcher.search(Arc::clone(&q), 300)?;
        let mut docs: Vec<i32> = top.score_docs.iter().map(|sd| sd.doc).collect();
        docs.sort_unstable();
        let mut ids: BTreeSet<i64> = BTreeSet::new();
        let mut dv = MultiDocValues::get_numeric_values(&r_index, "id")?.expect("id doc values");
        for doc in docs {
            assert!(dv.advance_exact(doc)?);
            ids.insert(dv.long_value()?);
        }
        let ids: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
        out.push_str(&format!(
            "hits {} [{}]\n",
            top.total_hits.value(),
            ids.join(", ")
        ));
    }
    drop(searcher);
    reader.close()?;
    Ok(out)
}

#[test]
fn spatial3d_geom_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Spatial3d, "geom", SEED_GEOM, &[]);
    // Not vacuous: forty models, and shapes with points, solids and relationships.
    assert_eq!(java.lines().filter(|l| l.starts_with("model ")).count(), 40);
    assert!(java.lines().filter(|l| l.starts_with("point ")).count() >= 200);
    assert!(java.lines().any(|l| l.starts_with("previous ")));
    sprint17_oracle::assert_same(&java, &rust_geom(SEED_GEOM).unwrap());
}

#[test]
fn spatial3d_queries_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Spatial3d, "queries", SEED_QUERIES, &[]);
    // Not vacuous: queries that match some, but not all, of the 300 points.
    assert!(java.lines().any(|l| l.starts_with("hits ")
        && !l.starts_with("hits 0 ")
        && !l.starts_with("hits 300 ")));
    sprint17_oracle::assert_same(&java, &rust_queries(SEED_QUERIES).unwrap());
}
