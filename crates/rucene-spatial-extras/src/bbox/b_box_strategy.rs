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

//! The types declared in `BBoxStrategy.java`, of the Java package
//! `org.apache.lucene.spatial.bbox`.

use std::fmt;
use std::sync::{Arc, LazyLock, Weak};

use rucene::document::{
    DoubleDocValuesField, DoublePoint, Field, FieldType, NumericValue, StoredField, StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::{DocValuesType, IndexOptions, IndexableField, IndexableFieldType, Term};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::search::{
    BooleanQuery, BooleanQueryBuilder, ConstantScoreQuery, Occur, Query, TermQuery,
};
use rucene::util::BytesRef;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::point::Point;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;

use crate::bbox::b_box_overlap_ratio_value_source::BBoxOverlapRatioValueSource;
use crate::bbox::b_box_value_source::BBoxValueSource;
use crate::query::spatial_args::SpatialArgs;
use crate::query::spatial_operation::SpatialOperation;
use crate::query::unsupported_spatial_operation::UnsupportedSpatialOperation;
use crate::shape_values_source::ShapeValuesSource;
use crate::spatial_strategy::{SpatialStrategy, SpatialStrategyBase};
use crate::util::distance_to_shape_value_source::DistanceToShapeValueSource;

/// `DEFAULT_FIELDTYPE`: pointValues + docValues. We use a FieldType to
/// articulate the options we want on the field. We don't use it as-is with
/// a Field, we create more than one Field.
pub static DEFAULT_FIELDTYPE: LazyLock<FieldType> = LazyLock::new(|| {
    // Default: pointValues + docValues
    let field_type = FieldType::new();
    field_type.set_dimensions(1, 8).expect("unfrozen"); // pointValues (assume Double)
    field_type
        .set_doc_values_type(DocValuesType::NUMERIC)
        .expect("unfrozen"); // docValues
    field_type.set_stored(false).expect("unfrozen");
    field_type.freeze();
    field_type
});

/// Equivalent to `org.apache.lucene.spatial.bbox.BBoxStrategy` (`public
/// class BBoxStrategy extends SpatialStrategy`): a SpatialStrategy for
/// indexing and searching Rectangles by storing its coordinates in numeric
/// fields. It supports all `SpatialOperation`s and has a custom overlap
/// relevancy. It is based on GeoPortal's SpatialClauseAdapter.
///
/// Characteristics: only indexes Rectangles; just one per field value;
/// can query only by a Rectangle; supports all `SpatialOperation`s; uses
/// the DocValues API for any sorting / relevancy.
///
/// Implementation: this uses 4 double fields for minX, maxX, minY, maxY and
/// a boolean to mark a dateline cross. Depending on the particular
/// `SpatialOperation`s, there are a variety of range queries on
/// `DoublePoint`s to be done. The `makeOverlapRatioValueSource` works by
/// calculating the query bbox overlap percentage against the indexed shape
/// overlap percentage. The indexed shape's coordinates are retrieved from
/// `LeafReader.getNumericDocValues`.
///
/// The strategy is shared as an `Arc` (its shape value source refers to
/// it).
#[derive(Debug)]
pub struct BBoxStrategy {
    base: SpatialStrategyBase,
    pub(crate) field_bbox: String,
    pub(crate) field_min_x: String,
    pub(crate) field_min_y: String,
    pub(crate) field_max_x: String,
    pub(crate) field_max_y: String,
    pub(crate) field_xdl: String, // crosses dateline
    // from constructor; aggregate field type used to express all options
    options_field_type: FieldType,
    fields_len: usize,
    has_stored: bool,
    has_doc_vals: bool,
    has_point_vals: bool,
    xdl_field_type: Option<Arc<FieldType>>,
    me: Weak<BBoxStrategy>,
}

impl BBoxStrategy {
    /// `SUFFIX_MINX`.
    pub const SUFFIX_MINX: &'static str = "__minX";
    /// `SUFFIX_MAXX`.
    pub const SUFFIX_MAXX: &'static str = "__maxX";
    /// `SUFFIX_MINY`.
    pub const SUFFIX_MINY: &'static str = "__minY";
    /// `SUFFIX_MAXY`.
    pub const SUFFIX_MAXY: &'static str = "__maxY";
    /// `SUFFIX_XDL`.
    pub const SUFFIX_XDL: &'static str = "__xdl";

    /// `newInstance(SpatialContext ctx, String fieldNamePrefix)`: creates a
    /// new `BBoxStrategy` instance that uses `DoublePoint` and
    /// `DoublePoint.newRangeQuery`.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn new_instance(
        ctx: Arc<SpatialContext>,
        field_name_prefix: &str,
    ) -> Result<Arc<BBoxStrategy>> {
        Self::new(ctx, field_name_prefix, &DEFAULT_FIELDTYPE)
    }

    /// `BBoxStrategy(SpatialContext ctx, String fieldNamePrefix, FieldType
    /// fieldType)`: creates this strategy. `field_type` is used to customize
    /// the indexing options of the 4 number fields, and to a lesser degree
    /// the XDL field too. Search requires pointValues (or legacy numerics),
    /// and relevancy requires docValues. If these features aren't needed
    /// then disable them. The field type is frozen.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty field name.
    pub fn new(
        ctx: Arc<SpatialContext>,
        field_name_prefix: &str,
        field_type: &FieldType,
    ) -> Result<Arc<BBoxStrategy>> {
        let base = SpatialStrategyBase::new(ctx, field_name_prefix)?;
        field_type.freeze();
        let options_field_type = field_type.clone();
        let mut num_quads = 0;
        let has_stored = field_type.stored();
        if has_stored {
            num_quads += 1;
        }
        let has_doc_vals = field_type.doc_values_type() != DocValuesType::NONE;
        if has_doc_vals {
            num_quads += 1;
        }
        let has_point_vals = field_type.point_dimension_count() > 0;
        if has_point_vals {
            num_quads += 1;
        }
        let xdl_field_type = if has_point_vals {
            // if we have an index...
            let t = FieldType::new_from(StringField::type_not_stored());
            t.set_index_options(IndexOptions::DOCS)?;
            t.freeze();
            Some(Arc::new(t))
        } else {
            None
        };
        let fields_len = num_quads * 4 + if xdl_field_type.is_some() { 1 } else { 0 };
        Ok(Arc::new_cyclic(|me| BBoxStrategy {
            base,
            field_bbox: field_name_prefix.to_string(),
            field_min_x: format!("{field_name_prefix}{}", Self::SUFFIX_MINX),
            field_max_x: format!("{field_name_prefix}{}", Self::SUFFIX_MAXX),
            field_min_y: format!("{field_name_prefix}{}", Self::SUFFIX_MINY),
            field_max_y: format!("{field_name_prefix}{}", Self::SUFFIX_MAXY),
            field_xdl: format!("{field_name_prefix}{}", Self::SUFFIX_XDL),
            options_field_type,
            fields_len,
            has_stored,
            has_doc_vals,
            has_point_vals,
            xdl_field_type,
            me: me.clone(),
        }))
    }

    fn me(&self) -> Arc<BBoxStrategy> {
        self.me
            .upgrade()
            .expect("a BBoxStrategy is shared as an Arc")
    }

    /// `getFieldType()`.
    pub fn get_field_type(&self) -> &FieldType {
        &self.options_field_type
    }

    // ---------------------------------
    // Indexing
    // ---------------------------------

    /// `createIndexableFields(Rectangle bbox)`.
    fn create_indexable_fields_rect(
        &self,
        bbox: &dyn Rectangle,
    ) -> Result<Vec<Arc<dyn IndexableField>>> {
        let mut fields: Vec<Arc<dyn IndexableField>> = Vec::with_capacity(self.fields_len);
        if self.has_stored {
            fields.push(Arc::new(StoredField::new_number(
                self.field_min_x.clone(),
                NumericValue::Double(bbox.get_min_x()),
            )?));
            fields.push(Arc::new(StoredField::new_number(
                self.field_min_y.clone(),
                NumericValue::Double(bbox.get_min_y()),
            )?));
            fields.push(Arc::new(StoredField::new_number(
                self.field_max_x.clone(),
                NumericValue::Double(bbox.get_max_x()),
            )?));
            fields.push(Arc::new(StoredField::new_number(
                self.field_max_y.clone(),
                NumericValue::Double(bbox.get_max_y()),
            )?));
        }
        if self.has_doc_vals {
            fields.push(Arc::new(DoubleDocValuesField::new(
                self.field_min_x.clone(),
                bbox.get_min_x(),
            )));
            fields.push(Arc::new(DoubleDocValuesField::new(
                self.field_min_y.clone(),
                bbox.get_min_y(),
            )));
            fields.push(Arc::new(DoubleDocValuesField::new(
                self.field_max_x.clone(),
                bbox.get_max_x(),
            )));
            fields.push(Arc::new(DoubleDocValuesField::new(
                self.field_max_y.clone(),
                bbox.get_max_y(),
            )));
        }
        if self.has_point_vals {
            fields.push(Arc::new(DoublePoint::new(
                self.field_min_x.clone(),
                &[bbox.get_min_x()],
            )?));
            fields.push(Arc::new(DoublePoint::new(
                self.field_min_y.clone(),
                &[bbox.get_min_y()],
            )?));
            fields.push(Arc::new(DoublePoint::new(
                self.field_max_x.clone(),
                &[bbox.get_max_x()],
            )?));
            fields.push(Arc::new(DoublePoint::new(
                self.field_max_y.clone(),
                &[bbox.get_max_y()],
            )?));
        }
        if let Some(xdl_field_type) = &self.xdl_field_type {
            let value = if bbox.get_crosses_date_line() {
                "T"
            } else {
                "F"
            };
            fields.push(Arc::new(Field::new(
                self.field_xdl.clone(),
                value.to_string(),
                Arc::clone(xdl_field_type),
            )?));
        }
        debug_assert_eq!(fields.len(), self.fields_len);
        Ok(fields)
    }

    // ---------------------------------
    // Value Source / Relevancy
    // ---------------------------------

    /// `makeShapeValueSource()`: provides access to each rectangle per
    /// document as a `ShapeValuesSource`.
    pub fn make_shape_value_source(&self) -> Arc<dyn ShapeValuesSource> {
        // TODO raise to SpatialStrategy
        Arc::new(BBoxValueSource::new(self.me()))
    }

    /// `makeOverlapRatioValueSource(Rectangle queryBox, double
    /// queryTargetProportion)`: returns a similarity based on
    /// [`BBoxOverlapRatioValueSource`]. This is just a convenience method.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of an invalid proportion.
    pub fn make_overlap_ratio_value_source(
        &self,
        query_box: Arc<dyn Rectangle>,
        query_target_proportion: f64,
    ) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(Arc::new(BBoxOverlapRatioValueSource::new(
            self.make_shape_value_source(),
            self.base.ctx.is_geo(),
            query_box,
            query_target_proportion,
            0.0,
        )?))
    }

    // ---------------------------------
    // Query Building
    // ---------------------------------

    /// `makeContains(Rectangle bbox)`: constructs a query to retrieve
    /// documents that fully contain the input envelope.
    pub(crate) fn make_contains(&self, bbox: &dyn Rectangle) -> Result<Arc<dyn Query>> {
        // general case
        // docMinX <= queryExtent.getMinX() AND docMinY <= queryExtent.getMinY() AND docMaxX >=
        // queryExtent.getMaxX() AND docMaxY >= queryExtent.getMaxY()

        // Y conditions
        // docMinY <= queryExtent.getMinY() AND docMaxY >= queryExtent.getMaxY()
        let q_min_y = self.make_numeric_range_query(
            &self.field_min_y,
            None,
            Some(bbox.get_min_y()),
            false,
            true,
        )?;
        let q_max_y = self.make_numeric_range_query(
            &self.field_max_y,
            Some(bbox.get_max_y()),
            None,
            true,
            false,
        )?;
        let y_conditions = self.make_query(Occur::MUST, &[Some(q_min_y), Some(q_max_y)])?;

        // X conditions
        let x_conditions: Arc<dyn Query>;

        // queries that do not cross the date line
        if !bbox.get_crosses_date_line() {
            // X Conditions for documents that do not cross the date line,
            // documents that contain the min X and max X of the query envelope,
            // docMinX <= queryExtent.getMinX() AND docMaxX >= queryExtent.getMaxX()
            let q_min_x = self.make_numeric_range_query(
                &self.field_min_x,
                None,
                Some(bbox.get_min_x()),
                false,
                true,
            )?;
            let q_max_x = self.make_numeric_range_query(
                &self.field_max_x,
                Some(bbox.get_max_x()),
                None,
                true,
                false,
            )?;
            let q_min_max = self.make_query(Occur::MUST, &[Some(q_min_x), Some(q_max_x)])?;
            let q_non_xdl = self.make_xdl_with(false, q_min_max)?;

            if !self.base.ctx.is_geo() {
                x_conditions = q_non_xdl;
            } else {
                // X Conditions for documents that cross the date line,
                // the left portion of the document contains the min X of the query
                // OR the right portion of the document contains the max X of the query,
                // docMinXLeft <= queryExtent.getMinX() OR docMaxXRight >= queryExtent.getMaxX()
                let q_xdl_left = self.make_numeric_range_query(
                    &self.field_min_x,
                    None,
                    Some(bbox.get_min_x()),
                    false,
                    true,
                )?;
                let q_xdl_right = self.make_numeric_range_query(
                    &self.field_max_x,
                    Some(bbox.get_max_x()),
                    None,
                    true,
                    false,
                )?;
                let q_xdl_left_right =
                    self.make_query(Occur::SHOULD, &[Some(q_xdl_left), Some(q_xdl_right)])?;
                let q_xdl = self.make_xdl_with(true, q_xdl_left_right)?;

                let mut q_edge_dl: Option<Arc<dyn Query>> = None;
                if bbox.get_min_x() == bbox.get_max_x() && bbox.get_min_x().abs() == 180.0 {
                    let edge = bbox.get_min_x() * -1.0; // opposite dateline edge
                    q_edge_dl = Some(self.make_query(
                        Occur::SHOULD,
                        &[
                            Some(self.make_number_term_query(&self.field_min_x, edge)?),
                            Some(self.make_number_term_query(&self.field_max_x, edge)?),
                        ],
                    )?);
                }

                // apply the non-XDL and XDL conditions
                x_conditions =
                    self.make_query(Occur::SHOULD, &[Some(q_non_xdl), Some(q_xdl), q_edge_dl])?;
            }
        } else {
            // queries that cross the date line

            // No need to search for documents that do not cross the date line

            // X Conditions for documents that cross the date line,
            // the left portion of the document contains the min X of the query
            // AND the right portion of the document contains the max X of the query,
            // docMinXLeft <= queryExtent.getMinX() AND docMaxXRight >= queryExtent.getMaxX()
            let q_xdl_left = self.make_numeric_range_query(
                &self.field_min_x,
                None,
                Some(bbox.get_min_x()),
                false,
                true,
            )?;
            let q_xdl_right = self.make_numeric_range_query(
                &self.field_max_x,
                Some(bbox.get_max_x()),
                None,
                true,
                false,
            )?;
            let q_xdl_left_right = self.make_xdl_with(
                true,
                self.make_query(Occur::MUST, &[Some(q_xdl_left), Some(q_xdl_right)])?,
            )?;

            let q_world = self.make_query(
                Occur::MUST,
                &[
                    Some(self.make_number_term_query(&self.field_min_x, -180.0)?),
                    Some(self.make_number_term_query(&self.field_max_x, 180.0)?),
                ],
            )?;

            x_conditions =
                self.make_query(Occur::SHOULD, &[Some(q_xdl_left_right), Some(q_world)])?;
        }

        // both X and Y conditions must occur
        self.make_query(Occur::MUST, &[Some(x_conditions), Some(y_conditions)])
    }

    /// `makeDisjoint(Rectangle bbox)`: constructs a query to retrieve
    /// documents that are disjoint to the input envelope.
    pub(crate) fn make_disjoint(&self, bbox: &dyn Rectangle) -> Result<Arc<dyn Query>> {
        // general case
        // docMinX > queryExtent.getMaxX() OR docMaxX < queryExtent.getMinX() OR docMinY >
        // queryExtent.getMaxY() OR docMaxY < queryExtent.getMinY()

        // Y conditions
        // docMinY > queryExtent.getMaxY() OR docMaxY < queryExtent.getMinY()
        let q_min_y = self.make_numeric_range_query(
            &self.field_min_y,
            Some(bbox.get_max_y()),
            None,
            false,
            false,
        )?;
        let q_max_y = self.make_numeric_range_query(
            &self.field_max_y,
            None,
            Some(bbox.get_min_y()),
            false,
            false,
        )?;
        let y_conditions = self.make_query(Occur::SHOULD, &[Some(q_min_y), Some(q_max_y)])?;

        // X conditions
        let x_conditions: Arc<dyn Query>;

        // queries that do not cross the date line
        if !bbox.get_crosses_date_line() {
            // X Conditions for documents that do not cross the date line,
            // docMinX > queryExtent.getMaxX() OR docMaxX < queryExtent.getMinX()
            let mut q_min_x = self.make_numeric_range_query(
                &self.field_min_x,
                Some(bbox.get_max_x()),
                None,
                false,
                false,
            )?;
            if bbox.get_min_x() == -180.0 && self.base.ctx.is_geo() {
                // touches dateline; -180 == 180
                let mut bq = BooleanQueryBuilder::new();
                bq.add(q_min_x, Occur::MUST)?;
                bq.add(
                    self.make_number_term_query(&self.field_max_x, 180.0)?,
                    Occur::MUST_NOT,
                )?;
                q_min_x = Arc::new(bq.build());
            }
            let mut q_max_x = self.make_numeric_range_query(
                &self.field_max_x,
                None,
                Some(bbox.get_min_x()),
                false,
                false,
            )?;

            if bbox.get_max_x() == 180.0 && self.base.ctx.is_geo() {
                // touches dateline; -180 == 180
                let mut bq = BooleanQueryBuilder::new();
                bq.add(q_max_x, Occur::MUST)?;
                bq.add(
                    self.make_number_term_query(&self.field_min_x, -180.0)?,
                    Occur::MUST_NOT,
                )?;
                q_max_x = Arc::new(bq.build());
            }
            let q_min_max = self.make_query(Occur::SHOULD, &[Some(q_min_x), Some(q_max_x)])?;
            let q_non_xdl = self.make_xdl_with(false, q_min_max)?;

            if !self.base.ctx.is_geo() {
                x_conditions = q_non_xdl;
            } else {
                // X Conditions for documents that cross the date line,

                // both the left and right portions of the document must be disjoint to the query
                // (docMinXLeft > queryExtent.getMaxX() OR docMaxXLeft < queryExtent.getMinX()) AND
                // (docMinXRight > queryExtent.getMaxX() OR docMaxXRight < queryExtent.getMinX())
                // where: docMaxXLeft = 180.0, docMinXRight = -180.0
                // (docMaxXLeft  < queryExtent.getMinX()) equates to (180.0  < queryExtent.getMinX()) and is
                // ignored
                // (docMinXRight > queryExtent.getMaxX()) equates to (-180.0 > queryExtent.getMaxX()) and is
                // ignored
                let q_min_x_left = self.make_numeric_range_query(
                    &self.field_min_x,
                    Some(bbox.get_max_x()),
                    None,
                    false,
                    false,
                )?;
                let q_max_x_right = self.make_numeric_range_query(
                    &self.field_max_x,
                    None,
                    Some(bbox.get_min_x()),
                    false,
                    false,
                )?;
                let q_left_right =
                    self.make_query(Occur::MUST, &[Some(q_min_x_left), Some(q_max_x_right)])?;
                let q_xdl = self.make_xdl_with(true, q_left_right)?;

                // apply the non-XDL and XDL conditions
                x_conditions = self.make_query(Occur::SHOULD, &[Some(q_non_xdl), Some(q_xdl)])?;
            }
            // queries that cross the date line
        } else {
            // X Conditions for documents that do not cross the date line,
            // the document must be disjoint to both the left and right query portions
            // (docMinX > queryExtent.getMaxX()Left OR docMaxX < queryExtent.getMinX()) AND (docMinX >
            // queryExtent.getMaxX() OR docMaxX < queryExtent.getMinX()Left)
            // where: queryExtent.getMaxX()Left = 180.0, queryExtent.getMinX()Left = -180.0
            let q_min_x_left =
                self.make_numeric_range_query(&self.field_min_x, Some(180.0), None, false, false)?;
            let q_max_x_left = self.make_numeric_range_query(
                &self.field_max_x,
                None,
                Some(bbox.get_min_x()),
                false,
                false,
            )?;
            let q_min_x_right = self.make_numeric_range_query(
                &self.field_min_x,
                Some(bbox.get_max_x()),
                None,
                false,
                false,
            )?;
            let q_max_x_right =
                self.make_numeric_range_query(&self.field_max_x, None, Some(-180.0), false, false)?;
            let q_left =
                self.make_query(Occur::SHOULD, &[Some(q_min_x_left), Some(q_max_x_left)])?;
            let q_right =
                self.make_query(Occur::SHOULD, &[Some(q_min_x_right), Some(q_max_x_right)])?;
            let q_left_right = self.make_query(Occur::MUST, &[Some(q_left), Some(q_right)])?;

            // No need to search for documents that do not cross the date line

            x_conditions = self.make_xdl_with(false, q_left_right)?;
        }

        // either X or Y conditions should occur
        self.make_query(Occur::SHOULD, &[Some(x_conditions), Some(y_conditions)])
    }

    /// `makeEquals(Rectangle bbox)`: constructs a query to retrieve
    /// documents that equal the input envelope.
    pub(crate) fn make_equals(&self, bbox: &dyn Rectangle) -> Result<Arc<dyn Query>> {
        // docMinX = queryExtent.getMinX() AND docMinY = queryExtent.getMinY() AND docMaxX =
        // queryExtent.getMaxX() AND docMaxY = queryExtent.getMaxY()
        let q_min_x = self.make_number_term_query(&self.field_min_x, bbox.get_min_x())?;
        let q_min_y = self.make_number_term_query(&self.field_min_y, bbox.get_min_y())?;
        let q_max_x = self.make_number_term_query(&self.field_max_x, bbox.get_max_x())?;
        let q_max_y = self.make_number_term_query(&self.field_max_y, bbox.get_max_y())?;
        self.make_query(
            Occur::MUST,
            &[Some(q_min_x), Some(q_min_y), Some(q_max_x), Some(q_max_y)],
        )
    }

    /// `makeIntersects(Rectangle bbox)`: constructs a query to retrieve
    /// documents that intersect the input envelope.
    pub(crate) fn make_intersects(&self, bbox: &dyn Rectangle) -> Result<Arc<dyn Query>> {
        // the original intersects query does not work for envelopes that cross the date line,
        // switch to a NOT Disjoint query

        // MUST_NOT causes a problem when it's the only clause type within a BooleanQuery,
        // to get around it we add all documents as a SHOULD

        // there must be an envelope, it must not be disjoint
        let q_has_env: Arc<dyn Query> = if self.base.ctx.is_geo() {
            let q_is_non_xdl = self.make_xdl(false);
            let q_is_xdl = if self.base.ctx.is_geo() {
                Some(self.make_xdl(true))
            } else {
                None
            };
            self.make_query(Occur::SHOULD, &[Some(q_is_non_xdl), q_is_xdl])?
        } else {
            self.make_xdl(false)
        };

        let mut q_not_disjoint = BooleanQueryBuilder::new();
        q_not_disjoint.add(q_has_env, Occur::MUST)?;
        let q_disjoint = self.make_disjoint(bbox)?;
        q_not_disjoint.add(q_disjoint, Occur::MUST_NOT)?;

        // Query qDisjoint = makeDisjoint();
        // BooleanQuery qNotDisjoint = new BooleanQuery();
        // qNotDisjoint.add(MatchAllDocsQuery.INSTANCE,BooleanClause.Occur.SHOULD);
        // qNotDisjoint.add(qDisjoint,BooleanClause.Occur.MUST_NOT);
        Ok(Arc::new(q_not_disjoint.build()))
    }

    /// `makeQuery(BooleanClause.Occur occur, Query... queries)`: makes a
    /// boolean query based upon a collection of queries and a logical
    /// operator; `None` (Java `null`) queries are skipped.
    pub(crate) fn make_query(
        &self,
        occur: Occur,
        queries: &[Option<Arc<dyn Query>>],
    ) -> Result<Arc<dyn Query>> {
        let mut bq = BooleanQueryBuilder::new();
        for query in queries.iter().flatten() {
            bq.add(Arc::clone(query), occur)?;
        }
        let q: BooleanQuery = bq.build();
        Ok(Arc::new(q))
    }

    /// `makeWithin(Rectangle bbox)`: constructs a query to retrieve
    /// documents are fully within the input envelope.
    pub(crate) fn make_within(&self, bbox: &dyn Rectangle) -> Result<Arc<dyn Query>> {
        // general case
        // docMinX >= queryExtent.getMinX() AND docMinY >= queryExtent.getMinY() AND docMaxX <=
        // queryExtent.getMaxX() AND docMaxY <= queryExtent.getMaxY()

        // Y conditions
        // docMinY >= queryExtent.getMinY() AND docMaxY <= queryExtent.getMaxY()
        let q_min_y = self.make_numeric_range_query(
            &self.field_min_y,
            Some(bbox.get_min_y()),
            None,
            true,
            false,
        )?;
        let q_max_y = self.make_numeric_range_query(
            &self.field_max_y,
            None,
            Some(bbox.get_max_y()),
            false,
            true,
        )?;
        let y_conditions = self.make_query(Occur::MUST, &[Some(q_min_y), Some(q_max_y)])?;

        // X conditions
        let x_conditions: Arc<dyn Query>;

        if self.base.ctx.is_geo() && bbox.get_min_x() == -180.0 && bbox.get_max_x() == 180.0 {
            // if query world-wraps, only the y condition matters
            return Ok(y_conditions);
        } else if !bbox.get_crosses_date_line() {
            // queries that do not cross the date line

            // docMinX >= queryExtent.getMinX() AND docMaxX <= queryExtent.getMaxX()
            let q_min_x = self.make_numeric_range_query(
                &self.field_min_x,
                Some(bbox.get_min_x()),
                None,
                true,
                false,
            )?;
            let q_max_x = self.make_numeric_range_query(
                &self.field_max_x,
                None,
                Some(bbox.get_max_x()),
                false,
                true,
            )?;
            let mut q_min_max = self.make_query(Occur::MUST, &[Some(q_min_x), Some(q_max_x)])?;

            let mut edge = 0.0; // none, otherwise opposite dateline of query
            if bbox.get_min_x() == -180.0 {
                edge = 180.0;
            } else if bbox.get_max_x() == 180.0 {
                edge = -180.0;
            }
            if edge != 0.0 && self.base.ctx.is_geo() {
                let edge_q = self.make_query(
                    Occur::MUST,
                    &[
                        Some(self.make_number_term_query(&self.field_min_x, edge)?),
                        Some(self.make_number_term_query(&self.field_max_x, edge)?),
                    ],
                )?;
                q_min_max = self.make_query(Occur::SHOULD, &[Some(q_min_max), Some(edge_q)])?;
            }

            x_conditions = self.make_xdl_with(false, q_min_max)?;

            // queries that cross the date line
        } else {
            // X Conditions for documents that do not cross the date line

            // the document should be within the left portion of the query
            // docMinX >= queryExtent.getMinX() AND docMaxX <= 180.0
            let q_min_x_left = self.make_numeric_range_query(
                &self.field_min_x,
                Some(bbox.get_min_x()),
                None,
                true,
                false,
            )?;
            let q_max_x_left =
                self.make_numeric_range_query(&self.field_max_x, None, Some(180.0), false, true)?;
            let q_left = self.make_query(Occur::MUST, &[Some(q_min_x_left), Some(q_max_x_left)])?;

            // the document should be within the right portion of the query
            // docMinX >= -180.0 AND docMaxX <= queryExtent.getMaxX()
            let q_min_x_right =
                self.make_numeric_range_query(&self.field_min_x, Some(-180.0), None, true, false)?;
            let q_max_x_right = self.make_numeric_range_query(
                &self.field_max_x,
                None,
                Some(bbox.get_max_x()),
                false,
                true,
            )?;
            let q_right =
                self.make_query(Occur::MUST, &[Some(q_min_x_right), Some(q_max_x_right)])?;

            // either left or right conditions should occur,
            // apply the left and right conditions to documents that do not cross the date line
            let q_left_right = self.make_query(Occur::SHOULD, &[Some(q_left), Some(q_right)])?;
            let q_non_xdl = self.make_xdl_with(false, q_left_right)?;

            // X Conditions for documents that cross the date line,
            // the left portion of the document must be within the left portion of the query,
            // AND the right portion of the document must be within the right portion of the query
            // docMinXLeft >= queryExtent.getMinX() AND docMaxXLeft <= 180.0
            // AND docMinXRight >= -180.0 AND docMaxXRight <= queryExtent.getMaxX()
            let q_xdl_left = self.make_numeric_range_query(
                &self.field_min_x,
                Some(bbox.get_min_x()),
                None,
                true,
                false,
            )?;
            let q_xdl_right = self.make_numeric_range_query(
                &self.field_max_x,
                None,
                Some(bbox.get_max_x()),
                false,
                true,
            )?;
            let q_xdl_left_right =
                self.make_query(Occur::MUST, &[Some(q_xdl_left), Some(q_xdl_right)])?;
            let q_xdl = self.make_xdl_with(true, q_xdl_left_right)?;

            // apply the non-XDL and XDL conditions
            x_conditions = self.make_query(Occur::SHOULD, &[Some(q_non_xdl), Some(q_xdl)])?;
        }

        // both X and Y conditions must occur
        self.make_query(Occur::MUST, &[Some(x_conditions), Some(y_conditions)])
    }

    /// `makeXDL(boolean crossedDateLine)`: constructs a query to retrieve
    /// documents that do or do not cross the date line.
    fn make_xdl(&self, crossed_date_line: bool) -> Arc<dyn Query> {
        // The 'T' and 'F' values match solr fields
        Arc::new(TermQuery::new(Term::new(
            self.field_xdl.clone(),
            BytesRef::from_char_sequence(if crossed_date_line { "T" } else { "F" }),
        )))
    }

    /// `makeXDL(boolean crossedDateLine, Query query)`: constructs a query
    /// to retrieve documents that do or do not cross the date line and
    /// match the supplied spatial query.
    fn make_xdl_with(
        &self,
        crossed_date_line: bool,
        query: Arc<dyn Query>,
    ) -> Result<Arc<dyn Query>> {
        if !self.base.ctx.is_geo() {
            debug_assert!(!crossed_date_line);
            return Ok(query);
        }
        let mut bq = BooleanQueryBuilder::new();
        bq.add(self.make_xdl(crossed_date_line), Occur::MUST)?;
        bq.add(query, Occur::MUST)?;
        Ok(Arc::new(bq.build()))
    }

    /// `makeNumberTermQuery(String field, double number)`.
    fn make_number_term_query(&self, field: &str, number: f64) -> Result<Arc<dyn Query>> {
        if self.has_point_vals {
            return DoublePoint::new_exact_query(field, number);
        }
        Err(LuceneError::unsupported_operation(
            "An index is required for this operation.",
        ))
    }

    /// `makeNumericRangeQuery(String fieldname, Double min, Double max,
    /// boolean minInclusive, boolean maxInclusive)`: returns a numeric range
    /// query based on FieldType; `DoublePoint` is used if field type
    /// indexes point values.
    fn make_numeric_range_query(
        &self,
        fieldname: &str,
        min: Option<f64>,
        max: Option<f64>,
        min_inclusive: bool,
        max_inclusive: bool,
    ) -> Result<Arc<dyn Query>> {
        if self.has_point_vals {
            let mut min = min.unwrap_or(f64::NEG_INFINITY);
            let mut max = max.unwrap_or(f64::INFINITY);
            if !min_inclusive {
                min = java_next_up(min);
            }
            if !max_inclusive {
                max = java_next_down(max);
            }
            return DoublePoint::new_range_query(fieldname, &[min], &[max]);
        }
        Err(LuceneError::unsupported_operation(
            "An index is required for this operation.",
        ))
    }
}

/// `Math.nextUp(double)`.
fn java_next_up(d: f64) -> f64 {
    d.next_up()
}

/// `Math.nextDown(double)`.
fn java_next_down(d: f64) -> f64 {
    d.next_down()
}

impl fmt::Display for BBoxStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.base.to_string_named("BBoxStrategy"))
    }
}

impl SpatialStrategy for BBoxStrategy {
    fn strategy_base(&self) -> &SpatialStrategyBase {
        &self.base
    }

    fn create_indexable_fields(
        &self,
        shape: &Arc<dyn Shape>,
    ) -> Result<Vec<Arc<dyn IndexableField>>> {
        self.create_indexable_fields_rect(&*shape.get_bounding_box())
    }

    fn make_distance_value_source(
        &self,
        query_point: &Arc<dyn Point>,
        multiplier: f64,
    ) -> Result<Arc<dyn DoubleValuesSource>> {
        // TODO if makeShapeValueSource gets lifted to the top; this could become a generic impl.
        Ok(Arc::new(DistanceToShapeValueSource::new(
            self.make_shape_value_source(),
            Arc::clone(query_point),
            multiplier,
            &self.base.ctx,
        )))
    }

    fn make_query(&self, args: &SpatialArgs) -> Result<Arc<dyn Query>> {
        let shape = &args.get_shape();
        let Some(bbox) = shape.as_rectangle() else {
            return Err(LuceneError::unsupported_operation(format!(
                "Can only query by Rectangle, not {shape}"
            )));
        };

        // Useful for understanding Relations:
        // http://edndoc.esri.com/arcsde/9.1/general_topics/understand_spatial_relations.htm
        let op = args.get_operation();
        let spatial = if op == SpatialOperation::BBoxIntersects {
            self.make_intersects(bbox)?
        } else if op == SpatialOperation::BBoxWithin {
            self.make_within(bbox)?
        } else if op == SpatialOperation::Contains {
            self.make_contains(bbox)?
        } else if op == SpatialOperation::Intersects {
            self.make_intersects(bbox)?
        } else if op == SpatialOperation::IsEqualTo {
            self.make_equals(bbox)?
        } else if op == SpatialOperation::IsDisjointTo {
            self.make_disjoint(bbox)?
        } else if op == SpatialOperation::IsWithin {
            self.make_within(bbox)?
        } else {
            // no Overlaps support yet
            return Err(UnsupportedSpatialOperation::new(op));
        };
        Ok(Arc::new(ConstantScoreQuery::new(spatial)))
    }
}
