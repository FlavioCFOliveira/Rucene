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

//! The types declared in `BBoxOverlapRatioValueSource.java`, of the Java
//! package `org.apache.lucene.spatial.bbox`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::index::LeafReaderContext;
use rucene::search::double_values::DoubleValues;
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::similarities::Explanation;
use rucene::search::{IndexSearcher, SegmentCacheable};
use spatial4j::shape::rectangle::Rectangle;

use crate::bbox::b_box_similarity_value_source::{
    similarity_eq, similarity_explain, similarity_get_values, similarity_hash, similarity_to_string,
    BBoxSimilarityValueSource,
};
use crate::jdk::{d2s, double_compare, hash_double};
use crate::shape_values_source::ShapeValuesSource;

/// Equivalent to `org.apache.lucene.spatial.bbox.BBoxOverlapRatioValueSource`
/// (`public class BBoxOverlapRatioValueSource extends
/// BBoxSimilarityValueSource`): the algorithm is implemented as envelope on
/// envelope (rect on rect) overlays rather than complex polygon on complex
/// polygon overlays.
///
/// Spatial relevance scoring algorithm: `queryArea` = the area of the input
/// query envelope; `targetArea` = the area of the target envelope (per
/// Lucene document); `intersectionArea` = the area of the intersection
/// between the query and target envelopes; `queryTargetProportion` = a 0-1
/// factor that divides the score proportion between query and target;
/// `queryRatio = intersectionArea / queryArea`; `targetRatio =
/// intersectionArea / targetArea`; `queryFactor = queryRatio *
/// queryTargetProportion`; `targetFactor = targetRatio * (1 -
/// queryTargetProportion)`; `score = queryFactor + targetFactor`.
///
/// Additionally, note that an optional minimum side length `minSideLength`
/// may be used whenever an area is calculated (queryArea, targetArea,
/// intersectionArea). This allows for points or horizontal/vertical lines
/// to be used as the query shape and in such case the descending order
/// should have smallest boxes up front.
#[derive(Debug)]
pub struct BBoxOverlapRatioValueSource {
    bbox_value_source: Arc<dyn ShapeValuesSource>,
    is_geo: bool, // -180/+180 degrees  (not part of identity; attached to parent strategy/field)
    query_extent: Arc<dyn Rectangle>,
    query_area: f64, // not part of identity
    min_side_length: f64,
    query_target_proportion: f64,
}

impl BBoxOverlapRatioValueSource {
    /// `BBoxOverlapRatioValueSource(ShapeValuesSource rectValueSource,
    /// boolean isGeo, Rectangle queryExtent, double queryTargetProportion,
    /// double minSideLength)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("queryTargetProportion must be >= 0 and
    /// <= 1")`.
    pub fn new(
        rect_value_source: Arc<dyn ShapeValuesSource>,
        is_geo: bool,
        query_extent: Arc<dyn Rectangle>,
        query_target_proportion: f64,
        min_side_length: f64,
    ) -> Result<Self> {
        let query_area = Self::calc_area_with(min_side_length, query_extent.get_width(), query_extent.get_height());
        debug_assert!(query_area >= 0.0);
        if !(0.0..=1.0).contains(&query_target_proportion) {
            return Err(LuceneError::illegal_argument("queryTargetProportion must be >= 0 and <= 1"));
        }
        Ok(Self {
            bbox_value_source: rect_value_source,
            is_geo,
            query_extent,
            query_area,
            min_side_length,
            query_target_proportion,
        })
    }

    /// `BBoxOverlapRatioValueSource(ShapeValuesSource rectValueSource,
    /// Rectangle queryExtent)`: construct with 75% weighting towards target
    /// (roughly GeoPortal's default), geo degrees assumed, no minimum side
    /// length.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn with_defaults(rect_value_source: Arc<dyn ShapeValuesSource>, query_extent: Arc<dyn Rectangle>) -> Result<Self> {
        Self::new(rect_value_source, true, query_extent, 0.25, 0.0)
    }

    /// `calcArea(double width, double height)`.
    fn calc_area(&self, width: f64, height: f64) -> f64 {
        Self::calc_area_with(self.min_side_length, width, height)
    }

    fn calc_area_with(min_side_length: f64, width: f64, height: f64) -> f64 {
        min_side_length.max(width) * min_side_length.max(height)
    }
}

impl BBoxSimilarityValueSource for BBoxOverlapRatioValueSource {
    fn bbox_value_source(&self) -> &Arc<dyn ShapeValuesSource> {
        &self.bbox_value_source
    }

    fn simple_name(&self) -> &'static str {
        "BBoxOverlapRatioValueSource"
    }

    fn similarity_description(&self) -> String {
        format!("{},{}", self.query_extent, d2s(self.query_target_proportion))
    }

    fn score(&self, target: &dyn Rectangle, exp: Option<&mut Option<Explanation>>) -> f64 {
        let no_intersection = |exp: Option<&mut Option<Explanation>>| {
            if let Some(exp) = exp {
                *exp = Some(Explanation::no_match("No intersection", Vec::new()));
            }
            0.0 // no intersection
        };
        // calculate "height": the intersection height between two boxes.
        let top = self.query_extent.get_max_y().min(target.get_max_y());
        let bottom = self.query_extent.get_min_y().max(target.get_min_y());
        let height = top - bottom;
        if height < 0.0 {
            return no_intersection(exp);
        }

        // calculate "width": the intersection width between two boxes.
        let mut width = 0.0;
        {
            let mut a: &dyn Rectangle = &*self.query_extent;
            let mut b: &dyn Rectangle = target;
            if a.get_crosses_date_line() == b.get_crosses_date_line() {
                // both either cross or don't
                let left = a.get_min_x().max(b.get_min_x());
                let right = a.get_max_x().min(b.get_max_x());
                if !a.get_crosses_date_line() {
                    // both don't
                    if left <= right {
                        width = right - left;
                    } else if self.is_geo
                        && (a.get_min_x().abs() == 180.0 || a.get_max_x().abs() == 180.0)
                        && (b.get_min_x().abs() == 180.0 || b.get_max_x().abs() == 180.0)
                    {
                        width = 0.0; // both adjacent to dateline
                    } else {
                        return no_intersection(exp);
                    }
                } else {
                    // both cross
                    width = right - left + 360.0;
                }
            } else {
                if !a.get_crosses_date_line() {
                    // then flip
                    a = target;
                    b = &*self.query_extent;
                }
                // a crosses, b doesn't
                let qry_west_left = a.get_min_x().max(b.get_min_x());
                let qry_west_right = b.get_max_x();
                if qry_west_left < qry_west_right {
                    width += qry_west_right - qry_west_left;
                }

                let qry_east_left = b.get_min_x();
                let qry_east_right = a.get_max_x().min(b.get_max_x());
                if qry_east_left < qry_east_right {
                    width += qry_east_right - qry_east_left;
                }

                if qry_west_left > qry_west_right && qry_east_left > qry_east_right {
                    return no_intersection(exp);
                }
            }
        }

        // calculate queryRatio and targetRatio
        let intersection_area = self.calc_area(width, height);
        let query_ratio = if self.query_area > 0.0 {
            intersection_area / self.query_area
        } else if self.query_extent.get_height() > 0.0 {
            // vert line
            height / self.query_extent.get_height()
        } else if self.query_extent.get_width() > 0.0 {
            // horiz line
            width / self.query_extent.get_width()
        } else if self.query_extent.relate(target.as_shape()).intersects() {
            1.0 // could be optimized
        } else {
            0.0
        };

        let target_area = self.calc_area(target.get_width(), target.get_height());
        debug_assert!(target_area >= 0.0);
        let target_ratio = if target_area > 0.0 {
            intersection_area / target_area
        } else if target.get_height() > 0.0 {
            // vert line
            height / target.get_height()
        } else if target.get_width() > 0.0 {
            // horiz line
            width / target.get_width()
        } else if target.relate(self.query_extent.as_shape()).intersects() {
            1.0 // could be optimized
        } else {
            0.0
        };
        debug_assert!((0.0..=1.0).contains(&query_ratio), "{query_ratio}");
        debug_assert!((0.0..=1.0).contains(&target_ratio), "{target_ratio}");

        // combine ratios into a score

        let query_factor = query_ratio * self.query_target_proportion;
        let target_factor = target_ratio * (1.0 - self.query_target_proportion);
        let score = query_factor + target_factor;

        if let Some(exp) = exp {
            let min_side_desc = if self.min_side_length > 0.0 {
                format!(" (minSide={})", d2s(self.min_side_length))
            } else {
                String::new()
            };
            *exp = Some(Explanation::matched(
                score as f32,
                format!("{}: queryFactor + targetFactor", self.simple_name()),
                vec![
                    Explanation::matched(
                        intersection_area as f32,
                        format!("IntersectionArea{min_side_desc}"),
                        vec![
                            Explanation::matched(width as f32, "width", Vec::new()),
                            Explanation::matched(height as f32, "height", Vec::new()),
                            Explanation::matched(self.query_target_proportion as f32, "queryTargetProportion", Vec::new()),
                        ],
                    ),
                    Explanation::matched(
                        query_factor as f32,
                        "queryFactor",
                        vec![
                            Explanation::matched(target_ratio as f32, "ratio", Vec::new()),
                            Explanation::matched(
                                self.query_area as f32,
                                format!("area of {}{min_side_desc}", self.query_extent),
                                Vec::new(),
                            ),
                        ],
                    ),
                    Explanation::matched(
                        target_factor as f32,
                        "targetFactor",
                        vec![
                            Explanation::matched(target_ratio as f32, "ratio", Vec::new()),
                            Explanation::matched(
                                target_area as f32,
                                format!("area of {}{min_side_desc}", target.as_shape()),
                                Vec::new(),
                            ),
                        ],
                    ),
                ],
            ));
        }

        score
    }
}

impl SegmentCacheable for BBoxOverlapRatioValueSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.bbox_value_source.is_cacheable(ctx)
    }
}

impl DoubleValuesSource for BBoxOverlapRatioValueSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        similarity_get_values(self, ctx)
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        if !similarity_eq(self, other) {
            return false;
        }
        let that = other.as_any().downcast_ref::<BBoxOverlapRatioValueSource>().expect("same class");
        if double_compare(that.min_side_length, self.min_side_length) != 0 {
            return false;
        }
        if double_compare(that.query_target_proportion, self.query_target_proportion) != 0 {
            return false;
        }
        self.query_extent.equals(that.query_extent.as_shape())
    }

    fn source_hash(&self) -> i32 {
        let mut result = similarity_hash(self);
        result = result.wrapping_mul(31).wrapping_add(self.query_extent.hash_code());
        result = result.wrapping_mul(31).wrapping_add(hash_double(self.min_side_length));
        result = result.wrapping_mul(31).wrapping_add(hash_double(self.query_target_proportion));
        result
    }

    fn to_source_string(&self) -> String {
        similarity_to_string(self)
    }

    fn explain(self: Arc<Self>, ctx: &LeafReaderContext, doc_id: i32, score_explanation: &Explanation) -> Result<Explanation> {
        similarity_explain(self, ctx, doc_id, score_explanation)
    }
}
