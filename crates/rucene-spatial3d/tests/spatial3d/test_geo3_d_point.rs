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

//! Port of `TestGeo3DPoint.java` (package `org.apache.lucene.spatial3d`).
//!
//! * The anonymous `FilterCodec`/`PointsFormat` of `getCodec()` is
//!   [`Lucene84PointsFormat`] in a [`FilterCodec`]; as in Java, it is used
//!   only when the default codec is named `Lucene84`.
//! * The anonymous `IntSupplier` `Cell.nextCellID` is the counter
//!   [`NEXT_CELL_ID`].
//! * `ExplainingVisitor.compare` mutates the visitor; the port's
//!   `IntersectVisitor::compare` takes `&self`, so the mutable state sits in
//!   [`RefCell`]s.
//! * The `StringWriter`/`PrintWriter` log of `testGeo3DRelations` is a
//!   `String` with `println` appending a line.

use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};

use randomizedtesting::generators::{RandomNumbers, RandomPicks};
use randomizedtesting::jdk::system;
use rucene::codecs::lucene90::points::{
    Lucene90PointsReader, Lucene90PointsWriter, VERSION_CURRENT,
};
use rucene::codecs::points::{PointsFormat, PointsReader, PointsWriter};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::{Codec, FilterCodec};
use rucene::document::{Document, NumericDocValuesField, Store, StringField};
use rucene::geo::{Polygon, Rectangle};
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::point_values::{IntersectVisitor, PointValues, Relation};
use rucene::index::terms::Term;
use rucene::index::{sub_index_from_leaves, IndexReader};
use rucene::search::query::Query;
use rucene::util::doc_id_set::DocIdSetBuilder;
use rucene::util::{java_math, BytesRef, IOUtils, NumericUtils};
use rucene_spatial3d::geo3_d_point::Geo3DPoint;
use rucene_spatial3d::geo3_d_util::Geo3DUtil;
use rucene_spatial3d::point_in_geo3_d_shape_query::PointInGeo3DShapeQuery;
use rucene_spatial3d::point_in_shape_intersect_visitor::PointInShapeIntersectVisitor;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::search::FixedBitSetCollector;
use rucene_test_framework::util::lucene_test_case::{Nightly, TestDirectory};
use rucene_test_framework::util::TestUtil;

use crate::spatial3d_support::prelude::{
    class, d2s, java_next_down, java_next_up, npe, random, test, unbox, verbose, Arc, Assert,
    Class, GeoAreaFactory, GeoBBoxFactory, GeoCircleFactory, GeoPathFactory, GeoPoint, GeoPolygon,
    GeoPolygonFactory, GeoShape, GeoTestUtil, LuceneError, LuceneTestCase, Plane, PlanetModel,
    Random, Result, SidedPlane, XYZBounds, XYZSolidFactory, CLARKE_1866, CONTAINS, DISJOINT,
    OVERLAPS, PI, SPHERE, WGS84, WITHIN,
};

/// `TestGeo3DPoint`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.spatial3d.TestGeo3DPoint",
        vec![
            test("testBasic", test_basic),
            test("testGeo3DRelations", test_geo3_d_relations),
            test("testRandomTiny", test_random_tiny),
            test("testRandomMedium", test_random_medium).annotate(Nightly),
            test("testRandomBig", test_random_big).annotate(Nightly),
            test("testPolygonOrdering", test_polygon_ordering),
            test("testToString", test_to_string),
            test("testShapeQueryToString", test_shape_query_to_string),
            test("testEquals", test_equals),
            test("testComplexPolygons", test_complex_polygons),
            test("testEncodeDecodeCeil", test_encode_decode_ceil),
            test("testEncodeDecodeRoundsDown", test_encode_decode_rounds_down),
            test("testMinValueQuantization", test_min_value_quantization),
            test("testMaxValueQuantization", test_max_value_quantization),
            test("testQuantization", test_quantization),
            test("testEncodeDecodeIsStable", test_encode_decode_is_stable),
            test("testDistanceQuery", test_distance_query),
        ],
    )
}

fn random_planet_model() -> Arc<PlanetModel> {
    RandomPicks::random_from(
        &*random(),
        &[WGS84.clone(), CLARKE_1866.clone(), SPHERE.clone()],
    )
    .clone()
}

/// The anonymous `PointsFormat` of `getCodec()`.
#[derive(Debug)]
struct Lucene84PointsFormat {
    max_points_in_leaf_node: i32,
    max_mb_sort_in_heap: f64,
}

impl PointsFormat for Lucene84PointsFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn fields_writer(&self, write_state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(Lucene90PointsWriter::with_config(
            write_state,
            self.max_points_in_leaf_node,
            self.max_mb_sort_in_heap,
            VERSION_CURRENT,
        )?))
    }

    fn fields_reader(&self, read_state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(Lucene90PointsReader::new(read_state)?))
    }
}

fn get_codec() -> Arc<dyn Codec> {
    let default = npe(rucene::codecs::get_default())
        .unwrap_or_else(|e| rucene_test_framework::util::java_exceptions::rethrow_java(e));
    if default.name() == "Lucene84" {
        let max_points_in_leaf_node = TestUtil::next_int(&*random(), 16, 2048);
        let max_mb_sort_in_heap = 3.0 + (3.0 * random().next_double());
        if verbose() {
            system::out_println(&format!(
                "TEST: using Lucene60PointsFormat with maxPointsInLeafNode={} and maxMBSortInHeap={}",
                max_points_in_leaf_node,
                d2s(max_mb_sort_in_heap)
            ));
        }
        Arc::new(
            FilterCodec::new("Lucene84".to_string(), Arc::clone(&default)).with_points_format(
                Lucene84PointsFormat {
                    max_points_in_leaf_node,
                    max_mb_sort_in_heap,
                },
            ),
        )
    } else {
        default
    }
}

fn test_basic() -> Result<()> {
    let dir = get_directory();
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_codec(get_codec())?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    let planet_model = random_planet_model();
    doc.add(Geo3DPoint::new_lat_lon_with_model(
        "field",
        &planet_model,
        50.7345267,
        -97.5303555,
    )?);
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    // We can't wrap with "exotic" readers because the query must see the BKD3DDVFormat:
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    Assert::assert_equals(
        1i64,
        s.search(
            Geo3DPoint::new_shape_query(
                "field",
                GeoCircleFactory::make_geo_circle(
                    &planet_model,
                    to_radians(50.0),
                    to_radians(-97.0),
                    PI / 180.0,
                )?,
            ),
            1,
        )?
        .total_hits
        .value(),
    );
    w.close()?;
    drop(s);
    r.close()?;
    dir.close()
}

fn to_radians(degrees: f64) -> f64 {
    degrees * Geo3DUtil::RADIANS_PER_DEGREE
}

/// `Cell.nextCellID`: the anonymous `IntSupplier` counting from 0.
static NEXT_CELL_ID: AtomicI32 = AtomicI32::new(0);

/// `TestGeo3DPoint.Cell`.
struct Cell {
    parent_cell_id: Option<i32>,
    cell_id: i32,
    x_min_enc: i32,
    x_max_enc: i32,
    y_min_enc: i32,
    y_max_enc: i32,
    z_min_enc: i32,
    z_max_enc: i32,
    split_count: i32,
    planet_model: Arc<PlanetModel>,
}

impl Cell {
    #[allow(clippy::too_many_arguments)]
    fn new(
        parent: Option<&Cell>,
        x_min_enc: i32,
        x_max_enc: i32,
        y_min_enc: i32,
        y_max_enc: i32,
        z_min_enc: i32,
        z_max_enc: i32,
        planet_model: &Arc<PlanetModel>,
        split_count: i32,
    ) -> Self {
        Self {
            parent_cell_id: parent.map(|p| p.cell_id),
            x_min_enc,
            x_max_enc,
            y_min_enc,
            y_max_enc,
            z_min_enc,
            z_max_enc,
            cell_id: NEXT_CELL_ID.fetch_add(1, Ordering::Relaxed),
            split_count,
            planet_model: Arc::clone(planet_model),
        }
    }

    /// Returns true if the quantized point lies within this cell, inclusive
    /// on all bounds.
    fn contains(&self, point: &GeoPoint) -> Result<bool> {
        let doc_x = self.planet_model.encode_value(point.x)?;
        let doc_y = self.planet_model.encode_value(point.y)?;
        let doc_z = self.planet_model.encode_value(point.z)?;

        Ok(doc_x >= self.x_min_enc
            && doc_x <= self.x_max_enc
            && doc_y >= self.y_min_enc
            && doc_y <= self.y_max_enc
            && doc_z >= self.z_min_enc
            && doc_z <= self.z_max_enc)
    }
}

impl std::fmt::Display for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cell={}{} x: {} TO {}, y: {} TO {}, z: {} TO {}, splits: {}",
            self.cell_id,
            match self.parent_cell_id {
                None => String::new(),
                Some(id) => format!(" parentCellID={id}"),
            },
            self.x_min_enc,
            self.x_max_enc,
            self.y_min_enc,
            self.y_max_enc,
            self.z_min_enc,
            self.z_max_enc,
            self.split_count
        )
    }
}

fn quantize_value(xyz_value: f64, planet_model: &PlanetModel) -> Result<f64> {
    Ok(planet_model.decode_value(planet_model.encode_value(xyz_value)?))
}

fn quantize(point: &GeoPoint, planet_model: &PlanetModel) -> Result<GeoPoint> {
    Ok(GeoPoint::new(
        quantize_value(point.x, planet_model)?,
        quantize_value(point.y, planet_model)?,
        quantize_value(point.z, planet_model)?,
    ))
}

/// `PrintWriter.println` on the `StringWriter` log.
fn log_println(log: &mut String, line: &str) {
    log.push_str(line);
    log.push('\n');
}

/// Tests consistency of GeoArea.getRelationship vs GeoShape.isWithin
fn test_geo3_d_relations() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(200);
    if verbose() {
        system::out_println(&format!("TEST: {num_docs} docs"));
    }

    let mut docs: Vec<GeoPoint> = Vec::with_capacity(num_docs as usize);
    let mut unquantized_docs: Vec<GeoPoint> = Vec::with_capacity(num_docs as usize);
    let planet_model: Arc<PlanetModel> = CLARKE_1866.clone();
    for doc_id in 0..num_docs as usize {
        unquantized_docs.push(GeoPoint::new_lat_lon(
            &planet_model,
            to_radians(GeoTestUtil::next_latitude()),
            to_radians(GeoTestUtil::next_longitude()),
        )?);
        docs.push(quantize(&unquantized_docs[doc_id], &planet_model)?);
        if verbose() {
            system::out_println(&format!(
                "  doc={}: {}; unquantized: {}",
                doc_id, docs[doc_id], unquantized_docs[doc_id]
            ));
        }
    }

    let iters = LuceneTestCase::at_least(10);

    let recurse_depth = RandomNumbers::random_int_between(&*random(), 5, 15);

    for iter in 0..iters {
        let shape = random_shape(&planet_model)?;

        let mut log = String::new();

        if verbose() {
            log_println(&mut log, &format!("TEST: iter={iter} shape={shape}"));
        }

        let mut bounds = XYZBounds::new();
        shape.get_bounds(&mut bounds);

        // Start with the root cell that fully contains the shape:
        let root = Cell::new(
            None,
            encode_value_lenient(unbox(bounds.get_minimum_x()), &planet_model)?,
            encode_value_lenient(unbox(bounds.get_maximum_x()), &planet_model)?,
            encode_value_lenient(unbox(bounds.get_minimum_y()), &planet_model)?,
            encode_value_lenient(unbox(bounds.get_maximum_y()), &planet_model)?,
            encode_value_lenient(unbox(bounds.get_minimum_z()), &planet_model)?,
            encode_value_lenient(unbox(bounds.get_maximum_z()), &planet_model)?,
            &planet_model,
            0,
        );

        if verbose() {
            log_println(&mut log, &format!("  root cell: {root}"));
        }

        // make sure the root cell (XYZBounds) does in fact contain all points that the shape contains
        {
            let mut fail = false;
            for doc_id in 0..num_docs as usize {
                if !root.contains(&docs[doc_id])? {
                    let expected = shape.is_within(&unquantized_docs[doc_id]);
                    if expected {
                        log_println(
                            &mut log,
                            &format!(
                                "    doc={doc_id} is contained by shape but is outside the returned XYZBounds"
                            ),
                        );
                        log_println(
                            &mut log,
                            &format!("      unquantized={}", unquantized_docs[doc_id]),
                        );
                        log_println(&mut log, &format!("      quantized={}", docs[doc_id]));
                        fail = true;
                    }
                }
            }

            if fail {
                log_println(&mut log, &format!("  shape={shape}"));
                log_println(&mut log, &format!("  bounds={bounds}"));
                system::out_print(&log);
                Assert::fail(&format!("invalid bounds for shape={shape}"));
            }
        }

        let mut queue: Vec<Cell> = vec![root];
        let mut hits: HashSet<i32> = HashSet::new();

        // `Cell cell = queue.get(queue.size() - 1); queue.remove(queue.size() - 1);`
        while let Some(cell) = queue.pop() {
            if verbose() {
                log_println(
                    &mut log,
                    &format!("  cycle: {} queue.size()={}", cell, queue.len()),
                );
            }

            if random().next_int_bound(10) == 7 || cell.split_count > recurse_depth {
                if verbose() {
                    log_println(&mut log, "    leaf");
                }
                // Leaf cell: brute force check all docs that fall within this cell:
                for doc_id in 0..num_docs as usize {
                    let point = &docs[doc_id];
                    let mapped_point = &unquantized_docs[doc_id];
                    let point_within_shape = shape.is_within(point);
                    let mapped_point_within_shape = shape.is_within(mapped_point);
                    if cell.contains(point)? {
                        if mapped_point_within_shape {
                            if verbose() {
                                log_println(
                                    &mut log,
                                    &format!(
                                        "    check doc={doc_id}: match!  Actual quantized point within: {point_within_shape}"
                                    ),
                                );
                            }
                            hits.insert(doc_id as i32);
                        } else if verbose() {
                            log_println(
                                &mut log,
                                &format!(
                                    "    check doc={doc_id}: no match.  Quantized point within: {point_within_shape}"
                                ),
                            );
                        }
                    }
                }
            } else {
                let xyz_solid = GeoAreaFactory::make_geo_area_xyz(
                    &planet_model,
                    Geo3DUtil::decode_value_floor(cell.x_min_enc, &planet_model),
                    Geo3DUtil::decode_value_ceil(cell.x_max_enc, &planet_model),
                    Geo3DUtil::decode_value_floor(cell.y_min_enc, &planet_model),
                    Geo3DUtil::decode_value_ceil(cell.y_max_enc, &planet_model),
                    Geo3DUtil::decode_value_floor(cell.z_min_enc, &planet_model),
                    Geo3DUtil::decode_value_ceil(cell.z_max_enc, &planet_model),
                )?;

                if verbose() {
                    log_println(
                        &mut log,
                        &format!(
                            "    minx={} maxx={} miny={} maxy={} minz={} maxz={}",
                            d2s(Geo3DUtil::decode_value_floor(cell.x_min_enc, &planet_model)),
                            d2s(Geo3DUtil::decode_value_ceil(cell.x_max_enc, &planet_model)),
                            d2s(Geo3DUtil::decode_value_floor(cell.y_min_enc, &planet_model)),
                            d2s(Geo3DUtil::decode_value_ceil(cell.y_max_enc, &planet_model)),
                            d2s(Geo3DUtil::decode_value_floor(cell.z_min_enc, &planet_model)),
                            d2s(Geo3DUtil::decode_value_ceil(cell.z_max_enc, &planet_model))
                        ),
                    );
                }

                match xyz_solid.get_relationship(&*shape)? {
                    CONTAINS => {
                        // Shape fully contains the cell: blindly add all docs in this cell:
                        if verbose() {
                            log_println(&mut log, "    GeoArea.CONTAINS: now addAll");
                        }
                        for doc_id in 0..num_docs as usize {
                            if cell.contains(&docs[doc_id])? {
                                if verbose() {
                                    log_println(&mut log, &format!("    addAll doc={doc_id}"));
                                }
                                hits.insert(doc_id as i32);
                            }
                        }
                        continue;
                    }
                    OVERLAPS => {
                        if verbose() {
                            log_println(&mut log, "    GeoArea.OVERLAPS: keep splitting");
                        }
                        // They do overlap but neither contains the other:
                        // log.println("    crosses1");
                    }
                    WITHIN => {
                        if verbose() {
                            log_println(&mut log, "    GeoArea.WITHIN: keep splitting");
                        }
                        // Cell fully contains the shape:
                        // log.println("    crosses2");
                    }
                    DISJOINT => {
                        // They do not overlap at all: don't recurse on this cell
                        // log.println("    outside");
                        if verbose() {
                            log_println(&mut log, "    GeoArea.DISJOINT: drop this cell");
                            for doc_id in 0..num_docs as usize {
                                if cell.contains(&docs[doc_id])? {
                                    log_println(&mut log, &format!("    skip doc={doc_id}"));
                                }
                            }
                        }
                        continue;
                    }
                    _ => {
                        if cfg!(debug_assertions) {
                            return Err(LuceneError::from(
                                rucene::error::LuceneErrorKind::AssertionError(String::new()),
                            ));
                        }
                    }
                }

                // Randomly split:
                let split = random().next_int_bound(3);
                let (axis, min, max) = match split {
                    0 => ("x", cell.x_min_enc, cell.x_max_enc),
                    1 => ("y", cell.y_min_enc, cell.y_max_enc),
                    _ => ("z", cell.z_min_enc, cell.z_max_enc),
                };
                if (0..3).contains(&split) {
                    let split_value = RandomNumbers::random_int_between(&*random(), min, max);
                    if verbose() {
                        log_println(&mut log, &format!("    now split on {axis}={split_value}"));
                    }
                    let (cell1, cell2) = match split {
                        // Split on X:
                        0 => (
                            Cell::new(
                                Some(&cell),
                                cell.x_min_enc,
                                split_value,
                                cell.y_min_enc,
                                cell.y_max_enc,
                                cell.z_min_enc,
                                cell.z_max_enc,
                                &planet_model,
                                cell.split_count + 1,
                            ),
                            Cell::new(
                                Some(&cell),
                                split_value,
                                cell.x_max_enc,
                                cell.y_min_enc,
                                cell.y_max_enc,
                                cell.z_min_enc,
                                cell.z_max_enc,
                                &planet_model,
                                cell.split_count + 1,
                            ),
                        ),
                        // Split on Y:
                        1 => (
                            Cell::new(
                                Some(&cell),
                                cell.x_min_enc,
                                cell.x_max_enc,
                                cell.y_min_enc,
                                split_value,
                                cell.z_min_enc,
                                cell.z_max_enc,
                                &planet_model,
                                cell.split_count + 1,
                            ),
                            Cell::new(
                                Some(&cell),
                                cell.x_min_enc,
                                cell.x_max_enc,
                                split_value,
                                cell.y_max_enc,
                                cell.z_min_enc,
                                cell.z_max_enc,
                                &planet_model,
                                cell.split_count + 1,
                            ),
                        ),
                        // Split on Z:
                        _ => (
                            Cell::new(
                                Some(&cell),
                                cell.x_min_enc,
                                cell.x_max_enc,
                                cell.y_min_enc,
                                cell.y_max_enc,
                                cell.z_min_enc,
                                split_value,
                                &planet_model,
                                cell.split_count + 1,
                            ),
                            Cell::new(
                                Some(&cell),
                                cell.x_min_enc,
                                cell.x_max_enc,
                                cell.y_min_enc,
                                cell.y_max_enc,
                                split_value,
                                cell.z_max_enc,
                                &planet_model,
                                cell.split_count + 1,
                            ),
                        ),
                    };
                    if verbose() {
                        log_println(&mut log, &format!("    split cell1: {cell1}"));
                        log_println(&mut log, &format!("    split cell2: {cell2}"));
                    }
                    queue.push(cell1);
                    queue.push(cell2);
                }
            }
        }

        if verbose() {
            log_println(&mut log, &format!("  {} hits", hits.len()));
        }

        // Done matching, now verify:
        let mut fail = false;
        for doc_id in 0..num_docs as usize {
            let point = &docs[doc_id];
            let mapped_point = &unquantized_docs[doc_id];
            let expected = shape.is_within(mapped_point);
            let actual = hits.contains(&(doc_id as i32));
            if actual != expected {
                if actual {
                    log_println(
                        &mut log,
                        &format!("doc={doc_id} should not have matched but did"),
                    );
                } else {
                    log_println(&mut log, &format!("doc={doc_id} should match but did not"));
                }
                log_println(&mut log, &format!("  point={point}"));
                log_println(&mut log, &format!("  mappedPoint={mapped_point}"));
                fail = true;
            }
        }

        if fail {
            system::out_print(&log);
            Assert::fail(&format!("invalid hits for shape={shape}"));
        }
    }
    Ok(())
}

fn test_random_tiny() -> Result<()> {
    // Make sure single-leaf-node case is OK:
    do_test_random(10)
}

// TODO: incredibly slow
fn test_random_medium() -> Result<()> {
    do_test_random(1000)
}

fn test_random_big() -> Result<()> {
    do_test_random(50000)
}

fn do_test_random(count: i32) -> Result<()> {
    let num_points = LuceneTestCase::at_least(count) as usize;

    if verbose() {
        system::err_println(&format!("TEST: numPoints={num_points}"));
    }

    let mut lats = vec![0.0f64; num_points];
    let mut lons = vec![0.0f64; num_points];

    let mut have_real_doc = false;

    let random = random();
    for doc_id in 0..num_points {
        let x = random.next_int_bound(20);
        if x == 17 {
            // Some docs don't have a point:
            lats[doc_id] = f64::NAN;
            if verbose() {
                system::err_println(&format!("  doc={doc_id} is missing"));
            }
            continue;
        }

        if doc_id > 0 && x < 3 && have_real_doc {
            let mut old_doc_id;
            loop {
                old_doc_id = random.next_int_bound(doc_id as i32) as usize;
                if !lats[old_doc_id].is_nan() {
                    break;
                }
            }

            if x == 0 {
                // Identical lat to old point
                lats[doc_id] = lats[old_doc_id];
                lons[doc_id] = GeoTestUtil::next_longitude();
                if verbose() {
                    system::err_println(&format!(
                        "  doc={} lat={} lon={} (same lat as doc={})",
                        doc_id,
                        d2s(lats[doc_id]),
                        d2s(lons[doc_id]),
                        old_doc_id
                    ));
                }
            } else if x == 1 {
                // Identical lon to old point
                lats[doc_id] = GeoTestUtil::next_latitude();
                lons[doc_id] = lons[old_doc_id];
                if verbose() {
                    system::err_println(&format!(
                        "  doc={} lat={} lon={} (same lon as doc={})",
                        doc_id,
                        d2s(lats[doc_id]),
                        d2s(lons[doc_id]),
                        old_doc_id
                    ));
                }
            } else {
                if cfg!(debug_assertions) && x != 2 {
                    return Err(LuceneError::from(
                        rucene::error::LuceneErrorKind::AssertionError(String::new()),
                    ));
                }
                // Fully identical point:
                lats[doc_id] = lats[old_doc_id];
                lons[doc_id] = lons[old_doc_id];
                if verbose() {
                    system::err_println(&format!(
                        "  doc={} lat={} lon={} (same lat/lon as doc={})",
                        doc_id,
                        d2s(lats[doc_id]),
                        d2s(lons[doc_id]),
                        old_doc_id
                    ));
                }
            }
        } else {
            lats[doc_id] = GeoTestUtil::next_latitude();
            lons[doc_id] = GeoTestUtil::next_longitude();
            have_real_doc = true;
            if verbose() {
                system::err_println(&format!(
                    "  doc={} lat={} lon={}",
                    doc_id,
                    d2s(lats[doc_id]),
                    d2s(lons[doc_id])
                ));
            }
        }
    }

    verify(&lats, &lons, &random_planet_model())
}

fn test_polygon_ordering() -> Result<()> {
    let lats = vec![
        51.204382859999996,
        50.89947531437482,
        50.8093624806861,
        50.8093624806861,
        50.89947531437482,
        51.204382859999996,
        51.51015366140113,
        51.59953838204167,
        51.59953838204167,
        51.51015366140113,
        51.204382859999996,
    ];
    let lons = vec![
        0.8747711978759765,
        0.6509219832137298,
        0.35960265165247807,
        0.10290284834752167,
        -0.18841648321373008,
        -0.41226569787597667,
        -0.18960465285650027,
        0.10285893781346236,
        0.35964656218653757,
        0.6521101528565002,
        0.8747711978759765,
    ];
    let q = Geo3DPoint::new_polygon_query(
        "point",
        &random_planet_model(),
        &[Polygon::new(lats, lons, Vec::new())?],
    )?;
    // System.out.println(q);
    Assert::assert_true(!q.to_query_string("").contains("GeoConcavePolygon"));
    Ok(())
}

/// A `try { return <query>; } catch (IllegalArgumentException e) { continue; }`
/// arm: `Some(query)`, or `None` to continue.
fn query_or_continue(q: Result<Arc<dyn Query>>) -> Result<Option<Arc<dyn Query>>> {
    match q {
        Ok(q) => Ok(Some(q)),
        Err(e) if e.is_illegal_argument_exception() => Ok(None),
        Err(e) => Err(e),
    }
}

fn random3_d_query(field: &str, planet_model: &Arc<PlanetModel>) -> Result<Arc<dyn Query>> {
    loop {
        let shape_type = random().next_int_bound(5);
        let q =
            match shape_type {
                4 => {
                    // Large polygons
                    let is_clockwise = random().next_double() < 0.5;
                    let pole = GeoPoint::new_lat_lon(
                        planet_model,
                        to_radians(GeoTestUtil::next_latitude()),
                        to_radians(GeoTestUtil::next_longitude()),
                    )?;
                    query_or_continue(make_poly(planet_model, &pole, is_clockwise, true).and_then(
                        |p| Geo3DPoint::new_large_polygon_query(field, planet_model, &[p]),
                    ))?
                    // System.err.println("Generated: "+q);
                    // assertTrue(false);
                }
                0 => {
                    // Polygons
                    let is_clockwise = random().next_double() < 0.5;
                    let pole = GeoPoint::new_lat_lon(
                        planet_model,
                        to_radians(GeoTestUtil::next_latitude()),
                        to_radians(GeoTestUtil::next_longitude()),
                    )?;
                    query_or_continue(
                        make_poly(planet_model, &pole, is_clockwise, true)
                            .and_then(|p| Geo3DPoint::new_polygon_query(field, planet_model, &[p])),
                    )?
                    // System.err.println("Generated: "+q);
                    // assertTrue(false);
                }
                1 => {
                    // Circles
                    let width_meters = random().next_double() * PI * planet_model.get_mean_radius();
                    query_or_continue(Geo3DPoint::new_distance_query(
                        field,
                        planet_model,
                        GeoTestUtil::next_latitude(),
                        GeoTestUtil::next_longitude(),
                        width_meters,
                    ))?
                }
                2 => {
                    // Rectangles
                    let r: Rectangle = GeoTestUtil::next_box();
                    query_or_continue(Geo3DPoint::new_box_query(
                        field,
                        planet_model,
                        r.min_lat(),
                        r.max_lat(),
                        r.min_lon(),
                        r.max_lon(),
                    ))?
                }
                3 => {
                    // Paths
                    // TBD: Need to rework generation to be realistic
                    let point_count = (random().next_int_bound(5) + 1) as usize;
                    let width = random().next_double() * PI * 0.5 * planet_model.get_mean_radius();
                    let mut latitudes = vec![0.0f64; point_count];
                    let mut longitudes = vec![0.0f64; point_count];
                    for i in 0..point_count {
                        latitudes[i] = GeoTestUtil::next_latitude();
                        longitudes[i] = GeoTestUtil::next_longitude();
                    }
                    // An IllegalArgumentException is what happens when we create a shape that is
                    // invalid.  Although it is conceivable that there are cases where the exception
                    // is thrown incorrectly, we aren't going to be able to do that in this random
                    // test.
                    query_or_continue(Geo3DPoint::new_path_query(
                        field,
                        &latitudes,
                        &longitudes,
                        width,
                        planet_model,
                    ))?
                }
                _ => {
                    return Err(LuceneError::illegal_state("Unexpected shape type"));
                }
            };
        if let Some(q) = q {
            return Ok(q);
        }
    }
}

/// A `try { ... } catch (IllegalArgumentException e) { continue; }` arm
/// over a shape: `Some(shape)`, or `None` to continue.
fn shape_or_continue<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.is_illegal_argument_exception() => Ok(None),
        Err(e) => Err(e),
    }
}

// Poached from Geo3dRptTest.randomShape:
fn random_shape(planet_model: &Arc<PlanetModel>) -> Result<Arc<dyn GeoShape>> {
    loop {
        let shape_type = random().next_int_bound(4);
        match shape_type {
            0 => {
                // Polygons
                let vertex_count = (random().next_int_bound(3) + 3) as usize;
                let mut geo_points: Vec<GeoPoint> = Vec::new();
                while geo_points.len() < vertex_count {
                    let g_pt = GeoPoint::new_lat_lon(
                        planet_model,
                        to_radians(GeoTestUtil::next_latitude()),
                        to_radians(GeoTestUtil::next_longitude()),
                    )?;
                    geo_points.push(g_pt);
                }
                // An IllegalArgumentException is what happens when we create a shape that is
                // invalid.  Although it is conceivable that there are cases where the exception
                // is thrown incorrectly, we aren't going to be able to do that in this random
                // test.
                match shape_or_continue(GeoPolygonFactory::make_geo_polygon(
                    planet_model,
                    geo_points,
                ))? {
                    // Degenerate polygon
                    None | Some(None) => continue,
                    Some(Some(rval)) => return Ok(rval),
                }
            }
            1 => {
                // Circles

                let lat = to_radians(GeoTestUtil::next_latitude());
                let lon = to_radians(GeoTestUtil::next_longitude());

                let angle = random().next_double() * PI / 2.0;

                // An IllegalArgumentException means the angle is too small; try again:
                if let Some(c) = shape_or_continue(GeoCircleFactory::make_geo_circle(
                    planet_model,
                    lat,
                    lon,
                    angle,
                ))? {
                    return Ok(c);
                }
            }
            2 => {
                // Rectangles
                let mut lat0 = to_radians(GeoTestUtil::next_latitude());
                let mut lat1 = to_radians(GeoTestUtil::next_latitude());
                if lat1 < lat0 {
                    std::mem::swap(&mut lat0, &mut lat1);
                }
                let mut lon0 = to_radians(GeoTestUtil::next_longitude());
                let mut lon1 = to_radians(GeoTestUtil::next_longitude());
                if lon1 < lon0 {
                    std::mem::swap(&mut lon0, &mut lon1);
                }

                return Ok(GeoBBoxFactory::make_geo_bbox(
                    planet_model,
                    lat1,
                    lat0,
                    lon0,
                    lon1,
                )?);
            }
            3 => {
                // Paths
                let point_count = (random().next_int_bound(5) + 1) as usize;
                let width = to_radians(f64::from(random().next_int_bound(89) + 1));
                let mut points: Vec<GeoPoint> = Vec::with_capacity(point_count);
                for _i in 0..point_count {
                    points.push(GeoPoint::new_lat_lon(
                        planet_model,
                        to_radians(GeoTestUtil::next_latitude()),
                        to_radians(GeoTestUtil::next_longitude()),
                    )?);
                }
                // An IllegalArgumentException is what happens when we create a shape that is
                // invalid (see above).
                if let Some(p) =
                    shape_or_continue(GeoPathFactory::make_geo_path(planet_model, width, &points))?
                {
                    return Ok(p);
                }
            }
            _ => {
                return Err(LuceneError::illegal_state("Unexpected shape type"));
            }
        }
    }
}

fn verify(lats: &[f64], lons: &[f64], planet_model: &Arc<PlanetModel>) -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();

    let mut points: Vec<Option<GeoPoint>> = vec![None; lats.len()];
    let mut unquantized_points: Vec<Option<GeoPoint>> = vec![None; lats.len()];

    // Pre-quantize all lat/lons:
    for i in 0..lats.len() {
        if !lats[i].is_nan() {
            // System.out.println("lats[" + i + "] = " + lats[i]);
            let p = GeoPoint::new_lat_lon(planet_model, to_radians(lats[i]), to_radians(lons[i]))?;
            points[i] = Some(quantize(&p, planet_model)?);
            unquantized_points[i] = Some(p);
        }
    }

    // Else we can get O(N^2) merging:
    let mbd = iwc.max_buffered_docs();
    if mbd != -1 && mbd < (points.len() / 100) as i32 {
        iwc.set_max_buffered_docs((points.len() / 100) as i32)?;
    }
    iwc.set_codec(get_codec())?;
    let dir: TestDirectory = if points.len() > 100000 {
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("TestBKDTree"))
    } else {
        get_directory()
    };
    let mut deleted: HashSet<i32> = HashSet::new();
    // RandomIndexWriter is too slow here:
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    for id in 0..points.len() {
        let mut doc = Document::new();
        doc.add(StringField::new("id", id.to_string(), Store::NO)?);
        doc.add(NumericDocValuesField::new("id", id as i64));
        if let Some(point) = &points[id] {
            doc.add(Geo3DPoint::new_xyz_with_model(
                "point",
                planet_model,
                point.x,
                point.y,
                point.z,
            )?);
        }
        w.add_document(&doc)?;
        if id > 0 && random().next_int_bound(100) == 42 {
            let id_to_delete = random().next_int_bound(id as i32);
            w.delete_documents_by_terms(vec![Term::new(
                "id",
                BytesRef::from_char_sequence(&id_to_delete.to_string()),
            )])?;
            deleted.insert(id_to_delete);
            if verbose() {
                system::err_println(&format!("  delete id={id_to_delete}"));
            }
        }
    }
    if random().next_boolean() {
        w.force_merge(1)?;
    }
    let dr = directory_reader::open_with_writer(&*w)?;
    let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;
    if verbose() {
        system::out_println(&format!("TEST: using reader {r}"));
    }
    w.close()?;
    drop(w);

    // We can't wrap with "exotic" readers because the geo3d query must see the Geo3DDVFormat:
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&dr), false);

    let iters = LuceneTestCase::at_least(100);

    for _iter in 0..iters {
        /*
        GeoShape shape = randomShape();

        if (VERBOSE) {
          System.err.println("\nTEST: iter=" + iter + " shape="+shape);
        }
        */

        let query = random3_d_query("point", planet_model)?; // Geo3DPoint.newShapeQuery("point", shape);

        if verbose() {
            system::err_println(&format!("  using query: {}", query.to_query_string("")));
        }

        let hits = s.search_with_manager(
            Arc::clone(&query),
            &FixedBitSetCollector::create_manager(r.max_doc()),
        )?;

        if verbose() {
            system::err_println(&format!("  hitCount: {}", hits.cardinality()));
        }

        let mut doc_id_to_id = npe(MultiDocValues::get_numeric_values(&r, "id")?)?;

        for doc_id in 0..r.max_doc() {
            Assert::assert_equals(doc_id, doc_id_to_id.next_doc()?);
            let id = doc_id_to_id.long_value()? as i32 as usize;
            let point = &points[id];
            let unquantized_point = &unquantized_points[id];
            if let (Some(point), Some(unquantized_point)) = (point, unquantized_point) {
                let shape = query
                    .as_any()
                    .downcast_ref::<PointInGeo3DShapeQuery>()
                    .ok_or_else(|| {
                        LuceneError::unchecked(
                            "java.lang.ClassCastException",
                            format!(
                                "class {} cannot be cast to class org.apache.lucene.spatial3d.PointInGeo3DShapeQuery",
                                query.java_class_name()
                            ),
                        )
                    })?
                    .get_shape()
                    .clone();
                let mut bounds = XYZBounds::new();
                shape.get_bounds(&mut bounds);
                let solid = XYZSolidFactory::make_xyz_solid(
                    planet_model,
                    unbox(bounds.get_minimum_x()),
                    unbox(bounds.get_maximum_x()),
                    unbox(bounds.get_minimum_y()),
                    unbox(bounds.get_maximum_y()),
                    unbox(bounds.get_minimum_z()),
                    unbox(bounds.get_maximum_z()),
                )?;

                let expected = !deleted.contains(&(id as i32)) && shape.is_within(point);
                if hits.get(doc_id as usize) != expected {
                    let mut b = String::new();
                    if expected {
                        b.push_str(&format!("FAIL: id={id} should have matched but did not\n"));
                    } else {
                        b.push_str(&format!("FAIL: id={id} should not have matched but did\n"));
                    }
                    b.push_str(&format!("  shape={shape}\n"));
                    b.push_str(&format!("  bounds={bounds}\n"));
                    b.push_str(&format!(
                        "  world bounds=( minX={} maxX={} minY={} maxY={} minZ={} maxZ={}\n",
                        d2s(planet_model.get_minimum_x_value()),
                        d2s(planet_model.get_maximum_x_value()),
                        d2s(planet_model.get_minimum_y_value()),
                        d2s(planet_model.get_maximum_y_value()),
                        d2s(planet_model.get_minimum_z_value()),
                        d2s(planet_model.get_maximum_z_value())
                    ));
                    b.push_str(&format!(
                        "  quantized point={} within shape? {} within bounds? {}\n",
                        point,
                        shape.is_within(point),
                        solid.is_within(point)
                    ));
                    b.push_str(&format!(
                        "  unquantized point={} within shape? {} within bounds? {}\n",
                        unquantized_point,
                        shape.is_within(unquantized_point),
                        solid.is_within(unquantized_point)
                    ));
                    b.push_str(&format!(
                        "  docID={} deleted?={}\n",
                        doc_id,
                        deleted.contains(&(id as i32))
                    ));
                    b.push_str(&format!("  query={}\n", query.to_query_string("")));
                    b.push_str(&format!(
                        "  explanation:\n    {}",
                        explain("point", &shape, point, unquantized_point, &r, doc_id)?
                            .replace('\n', "\n  ")
                    ));
                    Assert::fail(&b);
                }
            } else {
                Assert::assert_false(hits.get(doc_id as usize));
            }
        }
    }

    drop(s);
    drop(dr);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

fn test_to_string() -> Result<()> {
    // Don't compare entire strings because Java 9 and Java 8 have slightly different values
    let point =
        Geo3DPoint::new_lat_lon_with_model("point", &random_planet_model(), 44.244272, 7.769736)?;
    let string_to_compare = "Geo3DPoint <point: x=";
    Assert::assert_equals(
        string_to_compare,
        java_substring(&point.to_string(), string_to_compare.len())?,
    );
    Ok(())
}

/// `s.substring(0, end)` over an ASCII prefix: Java's
/// `StringIndexOutOfBoundsException` when `s` is shorter.
fn java_substring(s: &str, end: usize) -> Result<&str> {
    s.get(..end).ok_or_else(|| {
        LuceneError::unchecked(
            "java.lang.StringIndexOutOfBoundsException",
            format!("begin 0, end {}, length {}", end, s.encode_utf16().count()),
        )
    })
}

fn test_shape_query_to_string() -> Result<()> {
    // Don't compare entire strings because Java 9 and Java 8 have slightly different values
    let string_to_compare = "PointInGeo3DShapeQuery: field=point: Shape: GeoStandardCircle: {planetmodel=PlanetModel.WGS84, center=[lat=0.7";
    Assert::assert_equals(
        string_to_compare,
        java_substring(
            &Geo3DPoint::new_shape_query(
                "point",
                GeoCircleFactory::make_geo_circle(
                    &WGS84,
                    to_radians(44.244272),
                    to_radians(7.769736),
                    0.1,
                )?,
            )
            .to_query_string(""),
            string_to_compare.len(),
        )?,
    );
    Ok(())
}

fn get_directory() -> TestDirectory {
    LuceneTestCase::new_directory()
}

fn test_equals() -> Result<()> {
    let planet_model = random_planet_model();
    let shape = random_shape(&planet_model)?;
    let q = Geo3DPoint::new_shape_query("point", Arc::clone(&shape));
    Assert::assert_true(q.query_eq(&*Geo3DPoint::new_shape_query("point", Arc::clone(&shape))));
    Assert::assert_false(q.query_eq(&*Geo3DPoint::new_shape_query("point2", Arc::clone(&shape))));

    // make a different random shape:
    let mut shape2: Arc<dyn GeoShape>;
    loop {
        shape2 = random_shape(&planet_model)?;
        if !shape.java_equals(shape2.as_any()) {
            break;
        }
    }

    Assert::assert_false(q.query_eq(&*Geo3DPoint::new_shape_query("point", shape2)));
    Ok(())
}

fn test_complex_polygons() -> Result<()> {
    let pm = random_planet_model();
    // Pick a random pole
    let random_pole = GeoPoint::new_lat_lon(
        &pm,
        to_radians(GeoTestUtil::next_latitude()),
        to_radians(GeoTestUtil::next_longitude()),
    )?;
    let mut iters = LuceneTestCase::at_least(100);
    for _i in 0..iters {
        // Create a polygon that's less than 180 degrees
        make_poly(&pm, &random_pole, true, true)?;
    }
    iters = LuceneTestCase::at_least(100);
    for _i in 0..iters {
        // Create a polygon that's greater than 180 degrees
        make_poly(&pm, &random_pole, false, true)?;
    }
    Ok(())
}

fn minimum_edge_angle() -> f64 {
    to_radians(5.0)
}

fn minimum_arc_angle() -> f64 {
    to_radians(1.0)
}

/// Cook up a random Polygon that makes sense, with possible nested polygon
/// within. This is part of testing more complex polygons with nested holes.
/// Picking random points doesn't do it because it's almost impossible to come
/// up with nested ones of the proper clockwise/counterclockwise rotation that
/// way.
fn make_poly(
    pm: &Arc<PlanetModel>,
    pole: &GeoPoint,
    clockwise_desired: bool,
    _create_holes: bool,
) -> Result<Polygon> {
    // Polygon edges will be arranged around the provided pole, and holes will each have a pole
    // selected within the parent
    // polygon.
    let point_count = TestUtil::next_int(&*random(), 3, 10) as usize;
    // The point angles we pick next.  The only requirement is that they are not all on one side of
    // the pole.
    // We arrange that by picking the next point within what's left of the remaining angle, but
    // never more than 180 degrees,
    // and never less than what is needed to insure that the remaining point choices are less than
    // 180 degrees always.
    // These are all picked in the context of the pole,
    let mut angles = vec![0.0f64; point_count];
    let mut arc_distance = vec![0.0f64; point_count];
    // Pick a set of points
    #[allow(clippy::never_loop)]
    loop {
        let mut accumulated_angle = 0.0;
        for i in 0..point_count {
            let remaining_edge_count = point_count - i;
            let remaining_angle = 2.0 * PI - accumulated_angle;
            if remaining_edge_count == 1 {
                angles[i] = remaining_angle;
            } else {
                // The maximum angle is 180 degrees, or what's left when you give a minimal amount to each
                // edge.
                let mut maximum_angle =
                    remaining_angle - (remaining_edge_count - 1) as f64 * minimum_edge_angle();
                if maximum_angle > PI {
                    maximum_angle = PI;
                }
                // The minimum angle is MINIMUM_EDGE_ANGLE, or enough to be sure nobody afterwards needs
                // more than 180 degrees.  And since we have three points to start with, we already know
                // that.
                let minimum_angle = minimum_edge_angle();
                // Pick the angle
                let angle =
                    random().next_double() * (maximum_angle - minimum_angle) + minimum_angle;
                angles[i] = angle;
                accumulated_angle += angle;
            }
            // Pick the arc distance randomly; not quite the full range though
            arc_distance[i] =
                random().next_double() * (PI * 0.5 - minimum_arc_angle()) + minimum_arc_angle();
        }
        if clockwise_desired {
            // Reverse the signs
            for i in 0..point_count {
                angles[i] = -angles[i];
            }
        }

        // Now, use the pole's information plus angles and arcs to create GeoPoints in the right
        // order.
        let poly_points = convert_to_points(pm, pole, &angles, &arc_distance);

        // Next, do some holes.  No more than 2 of these.  The poles for holes must always be within
        // the polygon, so we're going to use Geo3D to help us select those given the points we just
        // made.

        let hole_list: Vec<Polygon> = Vec::new();

        /* Hole logic is broken and needs rethinking (see the Java source). */

        let holes = hole_list;

        // Finally, build the polygon and return it
        let mut lats = vec![0.0f64; poly_points.len() + 1];
        let mut lons = vec![0.0f64; poly_points.len() + 1];

        for i in 0..poly_points.len() {
            lats[i] = poly_points[i].get_latitude() * 180.0 / PI;
            lons[i] = poly_points[i].get_longitude() * 180.0 / PI;
        }
        lats[poly_points.len()] = lats[0];
        lons[poly_points.len()] = lons[0];
        return Polygon::new(lats, lons, holes);
    }
}

fn convert_to_points(
    pm: &PlanetModel,
    pole: &GeoPoint,
    angles: &[f64],
    arc_distances: &[f64],
) -> Vec<GeoPoint> {
    // To do the point rotations, we need the sine and cosine of the pole latitude and longitude.
    // Get it here for performance.
    let sin_latitude = java_math::sin(pole.get_latitude());
    let cos_latitude = java_math::cos(pole.get_latitude());
    let sin_longitude = java_math::sin(pole.get_longitude());
    let cos_longitude = java_math::cos(pole.get_longitude());
    let mut rval: Vec<GeoPoint> = Vec::new();
    for i in 0..angles.len() {
        rval.push(create_point(
            pm,
            angles[i],
            arc_distances[i],
            sin_latitude,
            cos_latitude,
            sin_longitude,
            cos_longitude,
        ));
    }
    rval
}

fn create_point(
    pm: &PlanetModel,
    angle: f64,
    arc_distance: f64,
    sin_latitude: f64,
    cos_latitude: f64,
    sin_longitude: f64,
    cos_longitude: f64,
) -> GeoPoint {
    // From the angle and arc distance, convert to (x,y,z) in unit space.
    // We want the perspective to be looking down the x axis.  The "angle" measurement is thus in
    // the Y-Z plane. The arcdistance is in X.
    let x = java_math::cos(arc_distance);
    let yz_scale = java_math::sin(arc_distance);
    let y = java_math::cos(angle) * yz_scale;
    let z = java_math::sin(angle) * yz_scale;
    // Now, rotate coordinates so that we shift everything from pole = x-axis to actual coordinates.
    // This transformation should take the point (1,0,0) and transform it to the pole's actual
    // (x,y,z) coordinates (see the derivation in the Java source).
    let x1 = x * cos_latitude - z * sin_latitude;
    let y1 = y;
    let z1 = x * sin_latitude + z * cos_latitude;
    let x2 = x1 * cos_longitude - y1 * sin_longitude;
    let y2 = x1 * sin_longitude + y1 * cos_longitude;
    let z2 = z1;
    // Scale to put the point on the surface
    pm.create_surface_point_xyz(x2, y2, z2)
}

/// `verifyPolygon(PlanetModel, Polygon, GeoPolygon)`: referenced only by the
/// commented-out hole logic of `makePoly`, as in Java.
#[allow(dead_code)]
fn verify_polygon(
    pm: &Arc<PlanetModel>,
    polygon: &Polygon,
    outside_polygon: &Arc<dyn GeoPolygon>,
) -> Result<bool> {
    // Each point in the new poly should be inside the outside poly, and each edge should not
    // intersect the outside poly edge
    let lats = polygon.get_polylats();
    let lons = polygon.get_polylons();
    let mut poly_points: Vec<GeoPoint> = Vec::with_capacity(lats.len() - 1);
    for i in 0..lats.len() - 1 {
        let new_point = GeoPoint::new_lat_lon(pm, to_radians(lats[i]), to_radians(lons[i]))?;
        if !outside_polygon.is_within(&new_point) {
            return Ok(false);
        }
        poly_points.push(new_point);
    }
    // We don't need to construct the world to find intersections -- just the bordering planes.
    let size = poly_points.len() as i32;
    for plane_index in 0..size {
        let start_point = &poly_points[plane_index as usize];
        let end_point = &poly_points[legal_index(plane_index + 1, size) as usize];
        let before_start_point = &poly_points[legal_index(plane_index - 1, size) as usize];
        let after_end_point = &poly_points[legal_index(plane_index + 2, size) as usize];
        let before_plane = SidedPlane::new_cross(end_point, before_start_point, start_point)?;
        let after_plane = SidedPlane::new_cross(start_point, end_point, after_end_point)?;
        let plane = Plane::new_cross(start_point, end_point)?;

        // Check for intersections!!
        if outside_polygon.intersects(&plane, &[], &[&before_plane, &after_plane]) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn legal_index(mut index: i32, size: i32) -> i32 {
    if index >= size {
        index -= size;
    }
    if index < 0 {
        index += size;
    }
    index
}

fn test_encode_decode_ceil() -> Result<()> {
    let planet_model = random_planet_model();
    // just for testing quantization error
    let encoding_tolerance = planet_model.decode;

    let iters = LuceneTestCase::at_least(10000);
    for _iter in 0..iters {
        let point = GeoPoint::new_lat_lon(
            &planet_model,
            to_radians(GeoTestUtil::next_latitude()),
            to_radians(GeoTestUtil::next_longitude()),
        )?;
        let x_enc = planet_model.decode_value(planet_model.encode_value(point.x)?);
        Assert::assert_equals_double_msg(
            Some(&format!(
                "x={} xEnc={} diff={}",
                d2s(point.x),
                d2s(x_enc),
                d2s(point.x - x_enc)
            )),
            point.x,
            x_enc,
            encoding_tolerance,
        );

        let y_enc = planet_model.decode_value(planet_model.encode_value(point.y)?);
        Assert::assert_equals_double_msg(
            Some(&format!(
                "y={} yEnc={} diff={}",
                d2s(point.y),
                d2s(y_enc),
                d2s(point.y - y_enc)
            )),
            point.y,
            y_enc,
            encoding_tolerance,
        );

        let z_enc = planet_model.decode_value(planet_model.encode_value(point.z)?);
        Assert::assert_equals_double_msg(
            Some(&format!(
                "z={} zEnc={} diff={}",
                d2s(point.z),
                d2s(z_enc),
                d2s(point.z - z_enc)
            )),
            point.z,
            z_enc,
            encoding_tolerance,
        );
    }

    // check edge/interesting cases explicitly
    let planet_max = planet_model.get_maximum_magnitude();
    for value in [0.0, -planet_max, planet_max] {
        Assert::assert_equals_double_msg(
            None,
            value,
            planet_model.decode_value(planet_model.encode_value(value)?),
            encoding_tolerance,
        );
    }
    Ok(())
}

/// make sure values always go down: this is important for edge case
/// consistency
fn test_encode_decode_rounds_down() -> Result<()> {
    let planet_model = random_planet_model();
    let iters = LuceneTestCase::at_least(1000);
    for _iter in 0..iters {
        let lat_base = GeoTestUtil::next_latitude();
        let lon_base = GeoTestUtil::next_longitude();

        // test above the value
        let mut lat = lat_base;
        let mut lon = lon_base;
        for _i in 0..1000 {
            lat = java_min(90.0, java_next_up(lat));
            lon = java_min(180.0, java_next_up(lon));
            check_rounds_down(&planet_model, lat, lon)?;
        }

        // test below the value
        lat = lat_base;
        lon = lon_base;
        for _i in 0..1000 {
            lat = java_max(-90.0, java_next_down(lat));
            lon = java_max(-180.0, java_next_down(lon));
            check_rounds_down(&planet_model, lat, lon)?;
        }
    }
    Ok(())
}

/// The loop body both halves of `testEncodeDecodeRoundsDown` share.
fn check_rounds_down(planet_model: &Arc<PlanetModel>, lat: f64, lon: f64) -> Result<()> {
    let point = GeoPoint::new_lat_lon(planet_model, to_radians(lat), to_radians(lon))?;
    let point_enc = GeoPoint::new(
        Geo3DUtil::decode_value_floor(planet_model.encode_value(point.x)?, planet_model),
        Geo3DUtil::decode_value_floor(planet_model.encode_value(point.y)?, planet_model),
        Geo3DUtil::decode_value_floor(planet_model.encode_value(point.z)?, planet_model),
    );
    Assert::assert_true(point_enc.x <= point.x);
    Assert::assert_true(point_enc.y <= point.y);
    Assert::assert_true(point_enc.z <= point.z);
    Ok(())
}

/// `Math.min(double, double)`.
fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() {
            a
        } else {
            b
        }
    } else if a <= b {
        a
    } else {
        b
    }
}

/// `Math.max(double, double)`.
fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_positive() {
            a
        } else {
            b
        }
    } else if a >= b {
        a
    } else {
        b
    }
}

fn test_min_value_quantization() -> Result<()> {
    let planet_model = random_planet_model();
    let encoded = planet_model.min_encoded_value;
    let min_value = -planet_model.get_maximum_magnitude();
    // Normal encoding
    let decoded = planet_model.decode_value(encoded);
    Assert::assert_equals_double_msg(None, min_value, decoded, 0.0);
    Assert::assert_equals(encoded, planet_model.encode_value(decoded)?);
    // Encoding floor
    let decoded_floor = Geo3DUtil::decode_value_floor(encoded, &planet_model);
    Assert::assert_equals_double_msg(None, min_value, decoded_floor, 0.0);
    Assert::assert_equals(encoded, planet_model.encode_value(decoded_floor)?);
    // Encoding ceiling
    let decoded_ceiling = Geo3DUtil::decode_value_ceil(encoded, &planet_model);
    Assert::assert_true(decoded_ceiling > min_value);
    Assert::assert_equals(encoded, planet_model.encode_value(decoded_ceiling)?);
    Ok(())
}

fn test_max_value_quantization() -> Result<()> {
    let planet_model = random_planet_model();
    let encoded = planet_model.max_encoded_value;
    let max_value = planet_model.get_maximum_magnitude();
    // Normal encoding
    let decoded = planet_model.decode_value(encoded);
    Assert::assert_equals_double_msg(None, max_value, decoded, 0.0);
    Assert::assert_equals(encoded, planet_model.encode_value(decoded)?);
    // Encoding floor
    let decoded_floor = Geo3DUtil::decode_value_floor(encoded, &planet_model);
    Assert::assert_true(decoded_floor < max_value);
    Assert::assert_equals(encoded, planet_model.encode_value(decoded_floor)?);
    // Encoding ceiling
    let decoded_ceiling = Geo3DUtil::decode_value_ceil(encoded, &planet_model);
    Assert::assert_equals_double_msg(None, max_value, decoded_ceiling, 0.0);
    Assert::assert_equals(encoded, planet_model.encode_value(decoded_ceiling)?);
    Ok(())
}

// poached from TestGeoEncodingUtils.testLatitudeQuantization:

/// step through some integers, ensuring they decode to their expected double
/// values. double values start at -planetMax and increase by
/// Geo3DUtil.DECODE for each integer. check edge cases within the double
/// range and random doubles within the range too.
fn test_quantization() -> Result<()> {
    let random = random();
    let planet_model = random_planet_model();
    for _i in 0..10000 {
        let encoded = random.next_int();
        if encoded <= planet_model.min_encoded_value {
            continue;
        }
        if encoded >= planet_model.max_encoded_value {
            continue;
        }
        let min = f64::from(encoded) * planet_model.decode;
        let decoded = Geo3DUtil::decode_value_floor(encoded, &planet_model);
        // should exactly equal expected value
        Assert::assert_equals_double_msg(None, min, decoded, 0.0);
        // should round-trip
        Assert::assert_equals(encoded, planet_model.encode_value(decoded)?);
        // test within the range
        if encoded != i32::MAX {
            // this is the next representable value
            // all double values between [min .. max) should encode to the current integer
            let max = min + planet_model.decode;
            Assert::assert_equals_double_msg(
                None,
                max,
                Geo3DUtil::decode_value_floor(encoded + 1, &planet_model),
                0.0,
            );
            Assert::assert_equals(encoded + 1, planet_model.encode_value(max)?);

            // first and last doubles in range that will be quantized
            let min_edge = java_next_up(min);
            let max_edge = java_next_down(max);
            Assert::assert_equals(encoded, planet_model.encode_value(min_edge)?);
            Assert::assert_equals(encoded, planet_model.encode_value(max_edge)?);

            // check random values within the double range
            let min_bits = NumericUtils::double_to_sortable_long(min_edge);
            let max_bits = NumericUtils::double_to_sortable_long(max_edge);
            for _j in 0..100 {
                let value = NumericUtils::sortable_long_to_double(TestUtil::next_long(
                    &*random, min_bits, max_bits,
                ));
                // round down
                Assert::assert_equals(encoded, planet_model.encode_value(value)?);
            }
        }
    }
    Ok(())
}

fn test_encode_decode_is_stable() -> Result<()> {
    let planet_model = random_planet_model();
    let iters = LuceneTestCase::at_least(1000);
    for _iter in 0..iters {
        let lat = GeoTestUtil::next_latitude();
        let lon = GeoTestUtil::next_longitude();

        let point = GeoPoint::new_lat_lon(&planet_model, to_radians(lat), to_radians(lon))?;

        // encode point
        let point_enc = GeoPoint::new(
            planet_model.decode_value(planet_model.encode_value(point.x)?),
            planet_model.decode_value(planet_model.encode_value(point.y)?),
            planet_model.decode_value(planet_model.encode_value(point.z)?),
        );

        // encode it again (double encode)
        let point_enc2 = GeoPoint::new(
            planet_model.decode_value(planet_model.encode_value(point_enc.x)?),
            planet_model.decode_value(planet_model.encode_value(point_enc.y)?),
            planet_model.decode_value(planet_model.encode_value(point_enc.z)?),
        );
        // System.out.println("TEST " + iter + ":\n  point    =" + point + "\n  pointEnc =" + pointEnc
        // + "\n  pointEnc2=" + pointEnc2);

        Assert::assert_equals_double_msg(None, point_enc.x, point_enc2.x, 0.0);
        Assert::assert_equals_double_msg(None, point_enc.y, point_enc2.y, 0.0);
        Assert::assert_equals_double_msg(None, point_enc.z, point_enc2.z, 0.0);
    }
    Ok(())
}

fn test_distance_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let planet_model = random_planet_model();

    // index two points:
    let mut doc = Document::new();
    doc.add(Geo3DPoint::new_lat_lon_with_model(
        "field",
        &planet_model,
        0.1,
        0.1,
    )?);
    w.add_document(&doc)?;
    doc = Document::new();
    doc.add(Geo3DPoint::new_lat_lon_with_model(
        "field",
        &planet_model,
        90.0,
        1.0,
    )?);
    w.add_document(&doc)?;

    ///// search //////
    let reader = w.get_reader()?;
    w.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // simple query, in any planet model one point should be inside and the other outside
    let q = Geo3DPoint::new_distance_query(
        "field",
        &planet_model,
        0.0,
        0.0,
        planet_model.get_mean_radius() * 0.01,
    )?;
    Assert::assert_equals(1, searcher.count(q)?);

    drop(searcher);
    IOUtils::close([
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Clips the incoming value to the allowed min/max range before encoding,
/// instead of throwing an exception.
fn encode_value_lenient(mut x: f64, planet_model: &PlanetModel) -> Result<i32> {
    let planet_max = planet_model.get_maximum_magnitude();
    if x > planet_max {
        x = planet_max;
    } else if x < -planet_max {
        x = -planet_max;
    }
    planet_model.encode_value(x)
}

/// `Relation.toString()`: the Java enum constant's name.
fn relation_name(r: Relation) -> &'static str {
    match r {
        Relation::CellInsideQuery => "CELL_INSIDE_QUERY",
        Relation::CellOutsideQuery => "CELL_OUTSIDE_QUERY",
        Relation::CellCrossesQuery => "CELL_CROSSES_QUERY",
    }
}

/// `ExplainingVisitor.Cell`.
#[derive(Clone)]
struct PackedCell {
    min_packed_value: Vec<u8>,
    max_packed_value: Vec<u8>,
}

/// `TestGeo3DPoint.ExplainingVisitor`.
struct ExplainingVisitor<'a> {
    shape: &'a Arc<dyn GeoShape>,
    target_doc_point: &'a GeoPoint,
    scaled_doc_point: &'a GeoPoint,
    r#in: PointInShapeIntersectVisitor<'a>,
    stack: RefCell<Vec<PackedCell>>,
    stack_to_target_doc: RefCell<Option<Vec<PackedCell>>>,
    target_doc_id: i32,
    num_dims: usize,
    bytes_per_dim: usize,
    target_stack_upto: StdCell<usize>,
    b: RefCell<String>,
    // In the first phase, we always return CROSSES to do a full scan of the BKD tree to see which
    // leaf block the document lives in
    first_phase: StdCell<bool>,
}

impl<'a> ExplainingVisitor<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        shape: &'a Arc<dyn GeoShape>,
        target_doc_point: &'a GeoPoint,
        scaled_doc_point: &'a GeoPoint,
        r#in: PointInShapeIntersectVisitor<'a>,
        target_doc_id: i32,
        num_dims: usize,
        bytes_per_dim: usize,
        b: String,
    ) -> Self {
        Self {
            shape,
            target_doc_point,
            scaled_doc_point,
            r#in,
            stack: RefCell::new(Vec::new()),
            stack_to_target_doc: RefCell::new(None),
            target_doc_id,
            num_dims,
            bytes_per_dim,
            target_stack_upto: StdCell::new(0),
            b: RefCell::new(b),
            first_phase: StdCell::new(true),
        }
    }

    fn start_second_phase(&self) -> Result<()> {
        if !self.first_phase.get() {
            return Err(LuceneError::illegal_state("already started second phase"));
        }
        if self.stack_to_target_doc.borrow().is_none() {
            self.b.borrow_mut().push_str(&format!(
                "target docID={} was never seen in points!\n",
                self.target_doc_id
            ));
        }
        self.first_phase.set(false);
        self.stack.borrow_mut().clear();
        Ok(())
    }

    /// Returns true if `this` cell fully contains the other one.
    fn cell_contains(&self, this: &PackedCell, other: &PackedCell) -> bool {
        for dim in 0..self.num_dims {
            let offset = self.bytes_per_dim * dim;
            let range = offset..offset + self.bytes_per_dim;
            // other.min < this.min?
            if other.min_packed_value[range.clone()] < this.min_packed_value[range.clone()] {
                return false;
            }
            // other.max < this.max?
            if other.max_packed_value[range.clone()] > this.max_packed_value[range] {
                return false;
            }
        }

        true
    }

    /// `Cell.toString()`.
    fn cell_to_string(&self, cell: &PackedCell) -> Result<String> {
        let pm = self.shape.get_planet_model();
        let x_min = Geo3DUtil::decode_value_floor(
            NumericUtils::sortable_bytes_to_int(&cell.min_packed_value, 0),
            pm,
        );
        let x_max = Geo3DUtil::decode_value_ceil(
            NumericUtils::sortable_bytes_to_int(&cell.max_packed_value, 0),
            pm,
        );
        let y_min = Geo3DUtil::decode_value_floor(
            NumericUtils::sortable_bytes_to_int(&cell.min_packed_value, 4),
            pm,
        );
        let y_max = Geo3DUtil::decode_value_ceil(
            NumericUtils::sortable_bytes_to_int(&cell.max_packed_value, 4),
            pm,
        );
        let z_min = Geo3DUtil::decode_value_floor(
            NumericUtils::sortable_bytes_to_int(&cell.min_packed_value, 2 * 4),
            pm,
        );
        let z_max = Geo3DUtil::decode_value_ceil(
            NumericUtils::sortable_bytes_to_int(&cell.max_packed_value, 2 * 4),
            pm,
        );
        let xyz_solid =
            XYZSolidFactory::make_xyz_solid(pm, x_min, x_max, y_min, y_max, z_min, z_max)?;
        let relationship = xyz_solid.get_relationship(&**self.shape)?;
        let point_within_cell = xyz_solid.is_within(self.target_doc_point);
        let scaled_within_cell = xyz_solid.is_within(self.scaled_doc_point);

        let relationship_string = match relationship {
            CONTAINS => "CONTAINS",
            WITHIN => "WITHIN",
            OVERLAPS => "OVERLAPS",
            DISJOINT => "DISJOINT",
            _ => "UNKNOWN",
        };
        Ok(format!(
            "Cell(x={} TO {} y={} TO {} z={} TO {}); Shape relationship = {}; Quantized point within cell = {}; Unquantized point within cell = {}",
            d2s(x_min),
            d2s(x_max),
            d2s(y_min),
            d2s(y_max),
            d2s(z_min),
            d2s(z_max),
            relationship_string,
            point_within_cell,
            scaled_within_cell
        ))
    }
}

/// `Cell.equals`: equal packed values.
fn cell_equals(a: &PackedCell, b: &PackedCell) -> bool {
    a.min_packed_value == b.min_packed_value && a.max_packed_value == b.max_packed_value
}

impl IntersectVisitor for ExplainingVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        if cfg!(debug_assertions) && self.first_phase.get() {
            return Err(LuceneError::from(
                rucene::error::LuceneErrorKind::AssertionError(String::new()),
            ));
        }
        if doc_id == self.target_doc_id {
            self.b
                .borrow_mut()
                .push_str(&format!("leaf visit docID={doc_id}\n"));
        }
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if self.first_phase.get() {
            if doc_id == self.target_doc_id {
                if cfg!(debug_assertions) && self.stack_to_target_doc.borrow().is_some() {
                    return Err(LuceneError::from(
                        rucene::error::LuceneErrorKind::AssertionError(String::new()),
                    ));
                }
                let stack = self.stack.borrow().clone();
                let mut text = String::from("  full BKD path to target doc:\n");
                for cell in &stack {
                    text.push_str(&format!("    {}\n", self.cell_to_string(cell)?));
                }
                self.b.borrow_mut().push_str(&text);
                *self.stack_to_target_doc.borrow_mut() = Some(stack);
            }
        } else if doc_id == self.target_doc_id {
            let pm = self.shape.get_planet_model();
            let x = Geo3DPoint::decode_dimension(packed_value, 0, pm);
            let y = Geo3DPoint::decode_dimension(packed_value, 4, pm);
            let z = Geo3DPoint::decode_dimension(packed_value, 2 * 4, pm);
            self.b.borrow_mut().push_str(&format!(
                "leaf visit docID={} x={} y={} z={}\n",
                doc_id,
                d2s(x),
                d2s(y),
                d2s(z)
            ));
            self.r#in.visit_with_value(doc_id, packed_value)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> Result<Relation> {
        let cell = PackedCell {
            min_packed_value: min_packed_value.to_vec(),
            max_packed_value: max_packed_value.to_vec(),
        };
        // System.out.println("compare: " + cell);

        // TODO: this is a bit hacky, having to reverse-engineer where we are in the BKD tree's
        // recursion ... but it's the lesser evil vs e.g. polluting this visitor API, or
        // implementing this "under the hood" in BKDReader instead?
        if self.first_phase.get() {
            let mut stack = self.stack.borrow_mut();
            // Pop stack:
            while !stack.is_empty() && !self.cell_contains(&stack[stack.len() - 1], &cell) {
                stack.pop();
                // System.out.println("  pop");
            }

            // Push stack:
            stack.push(cell);
            // System.out.println("  push");

            Ok(Relation::CellCrossesQuery)
        } else {
            let result = self.r#in.compare(min_packed_value, max_packed_value)?;
            let target = self.stack_to_target_doc.borrow();
            let path = target.as_ref().ok_or_else(LuceneError::null_pointer)?;
            let upto = self.target_stack_upto.get();
            if upto < path.len() && cell_equals(&cell, &path[upto]) {
                let text = format!(
                    "  on cell {}, wrapped visitor returned {}\n",
                    self.cell_to_string(&path[upto])?,
                    relation_name(result)
                );
                self.b.borrow_mut().push_str(&text);
                self.target_stack_upto.set(upto + 1);
            }
            Ok(result)
        }
    }
}

fn explain(
    field_name: &str,
    shape: &Arc<dyn GeoShape>,
    target_doc_point: &GeoPoint,
    scaled_doc_point: &GeoPoint,
    reader: &Arc<dyn IndexReader>,
    doc_id: i32,
) -> Result<String> {
    let mut bounds = XYZBounds::new();
    shape.get_bounds(&mut bounds);

    // First find the leaf reader that owns this doc:
    let leaves = Arc::clone(reader).leaves()?;
    let sub_index = sub_index_from_leaves(doc_id, &leaves) as usize;
    let leaf_reader = leaves[sub_index].leaf_reader();

    let b = format!("target is in leaf {leaf_reader} of full reader {reader}\n");

    let mut hits = DocIdSetBuilder::new(leaf_reader.max_doc());
    let mut visitor = ExplainingVisitor::new(
        shape,
        target_doc_point,
        scaled_doc_point,
        PointInShapeIntersectVisitor::new(&mut hits, &**shape, &bounds),
        doc_id - leaves[sub_index].doc_base(),
        3,
        4,
        b,
    );

    // Do first phase, where we just figure out the "path" that leads to the target docID:
    let point_values: Box<dyn PointValues> = npe(leaf_reader.get_point_values(field_name)?)?;
    point_values.intersect(&mut visitor)?;

    // Do second phase, where we we see how the wrapped visitor responded along that path:
    visitor.start_second_phase()?;
    let point_values: Box<dyn PointValues> = npe(leaf_reader.get_point_values(field_name)?)?;
    point_values.intersect(&mut visitor)?;

    Ok(visitor.b.into_inner())
}
