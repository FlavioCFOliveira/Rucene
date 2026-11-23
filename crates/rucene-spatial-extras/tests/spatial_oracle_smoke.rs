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

//! The smoke oracles of `lucene/spatial3d` and `lucene/spatial-extras`
//! (rmp #628): `Spatial3dOracle.java smoke` and `SpatialExtrasOracle.java
//! smoke` and Rucene, from one seed, give the same circle memberships and
//! distances, and the same prefix-tree cells. The `spatial3d` smoke lives here
//! because this crate depends on `rucene-spatial3d`, whose `tests/` belong to
//! another lane of the sprint.

#[path = "../../../tests/sprint17_oracle/mod.rs"]
mod sprint17_oracle;

use std::f64::consts::PI;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use rucene_spatial3d::geom::distance_style::ARC;
use rucene_spatial3d::geom::geo_circle_factory::GeoCircleFactory;
use rucene_spatial3d::geom::geo_point::GeoPoint;
use rucene_spatial3d::geom::planet_model::SPHERE;
use rucene_spatial_extras::prefix::tree::geohash_prefix_tree::GeohashPrefixTree;
use rucene_spatial_extras::prefix::tree::quad_prefix_tree::QuadPrefixTree;
use rucene_spatial_extras::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use spatial4j::context::spatial_context::SpatialContext;
use sprint17_oracle::{bits64 as bits, escape, Module};

const SEED_3D: i64 = 0x0628_0004;
const SEED_EXTRAS: i64 = 0x0628_0005;

/// `Spatial3dOracle.smoke`.
fn rust_smoke_3d(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let mut out = String::new();
    for _ in 0..20 {
        let lat = (r.next_double() * 2.0 - 1.0) * PI / 2.0;
        let lon = (r.next_double() * 2.0 - 1.0) * PI;
        let radius = r.next_double() * 0.5;
        let circle = GeoCircleFactory::make_geo_circle(&SPHERE, lat, lon, radius).unwrap();
        out.push_str(&format!(
            "circle {} {} {} {}\n",
            bits(lat),
            bits(lon),
            bits(radius),
            bits(circle.get_radius())
        ));
        for _ in 0..5 {
            let plat = (r.next_double() * 2.0 - 1.0) * PI / 2.0;
            let plon = (r.next_double() * 2.0 - 1.0) * PI;
            let p = GeoPoint::new_lat_lon(&SPHERE, plat, plon).unwrap();
            out.push_str(&format!(
                "point {} {} {} {} {}\n",
                bits(p.x),
                bits(p.y),
                bits(p.z),
                circle.is_within(&p),
                bits(circle.compute_outside_distance(&ARC, &p).unwrap())
            ));
        }
    }
    out
}

/// `SpatialExtrasOracle.smoke`.
fn rust_smoke_extras(seed: i64) -> String {
    let r = JdkRandom::new(seed);
    let ctx = SpatialContext::geo();
    let trees: Vec<std::sync::Arc<dyn SpatialPrefixTree>> = vec![
        GeohashPrefixTree::new(ctx.clone(), 6).unwrap(),
        QuadPrefixTree::new(ctx.clone(), 8),
    ];
    let mut out = String::new();
    for _ in 0..10 {
        let x = r.next_double() * 360.0 - 180.0;
        let y = r.next_double() * 180.0 - 90.0;
        let p = ctx.make_point(x, y).unwrap();
        out.push_str(&format!("point {} {}\n", bits(x), bits(y)));
        for tree in &trees {
            let mut it = tree
                .get_tree_cell_iterator(p.clone(), tree.get_max_levels())
                .unwrap();
            let mut line = format!("tree {}", tree.get_max_levels());
            while it.has_next() {
                let cell = it.next();
                line.push_str(&format!(
                    " {}:{}",
                    cell.get_level(),
                    escape(&cell.get_token_bytes_no_leaf(None).utf8_to_string().unwrap())
                ));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn spatial3d_smoke_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::Spatial3d, "smoke", SEED_3D, &[]);
    sprint17_oracle::assert_same(&java, &rust_smoke_3d(SEED_3D));
}

#[test]
fn spatial_extras_smoke_oracle() {
    sprint17_oracle::pin_tests_seed();
    let java = sprint17_oracle::run(Module::SpatialExtras, "smoke", SEED_EXTRAS, &[]);
    sprint17_oracle::assert_same(&java, &rust_smoke_extras(SEED_EXTRAS));
}
