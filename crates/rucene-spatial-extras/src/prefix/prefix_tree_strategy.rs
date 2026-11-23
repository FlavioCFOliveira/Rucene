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

//! The types declared in `PrefixTreeStrategy.java`, of the Java package
//! `org.apache.lucene.spatial.prefix`.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use rucene::analysis::TokenStream;
use rucene::document::{Field, FieldType};
use rucene::error::{LuceneError, Result};
use rucene::index::{IndexOptions, IndexReaderContext, IndexableField};
use rucene::search::double_values_source::DoubleValuesSource;
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::attribute::AttributeSource;
use rucene::util::Bits;
use spatial4j::shape::point::Point;
use spatial4j::shape::shape::Shape;

use crate::jdk::AtomicF64;
use crate::prefix::bytes_ref_iterator_token_stream::BytesRefIteratorTokenStream;
use crate::prefix::cell_to_bytes_ref_iterator::CellToBytesRefIterator;
use crate::prefix::heatmap_facet_counter::{self, Heatmap};
use crate::prefix::point_prefix_tree_field_cache_provider::PointPrefixTreeFieldCacheProvider;
use crate::prefix::tree::cell_iterator::CellIterator;
use crate::prefix::tree::spatial_prefix_tree::SpatialPrefixTree;
use crate::query::spatial_args::SpatialArgs;
use crate::spatial_strategy::{SpatialStrategy, SpatialStrategyBase};
use crate::util::shape_field_cache_distance_value_source::ShapeFieldCacheDistanceValueSource;

/// `FIELD_TYPE`: tokenized, omits norms, indexes docs only, frozen.
pub static FIELD_TYPE: LazyLock<Arc<FieldType>> = LazyLock::new(|| {
    let field_type = FieldType::new();
    field_type.set_tokenized(true).expect("unfrozen");
    field_type.set_omit_norms(true).expect("unfrozen");
    field_type
        .set_index_options(IndexOptions::DOCS)
        .expect("unfrozen");
    field_type.freeze();
    Arc::new(field_type)
});

/// The fields of `PrefixTreeStrategy`. The configuration fields have
/// setters a shared strategy is reconfigured through, so they are atomics.
#[derive(Debug)]
pub struct PrefixTreeStrategyBase {
    /// The `SpatialStrategy` fields.
    pub strategy: SpatialStrategyBase,
    /// `grid`.
    pub grid: Arc<dyn SpatialPrefixTree>,
    provider: Mutex<HashMap<String, Arc<PointPrefixTreeFieldCacheProvider>>>,
    default_field_values_array_len: AtomicI32,
    dist_err_pct: AtomicF64, // [ 0 TO 0.5 ]
    points_only: AtomicBool, // if true, there are no leaves
}

impl PrefixTreeStrategyBase {
    /// `PrefixTreeStrategy(SpatialPrefixTree grid, String fieldName)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for an empty field name.
    pub fn new(grid: Arc<dyn SpatialPrefixTree>, field_name: &str) -> Result<Self> {
        Ok(Self {
            strategy: SpatialStrategyBase::new(grid.get_spatial_context(), field_name)?,
            grid,
            provider: Mutex::new(HashMap::new()),
            default_field_values_array_len: AtomicI32::new(2),
            dist_err_pct: AtomicF64::new(SpatialArgs::DEFAULT_DISTERRPCT),
            points_only: AtomicBool::new(false),
        })
    }
}

impl fmt::Debug for dyn PrefixTreeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.PrefixTreeStrategy`
/// (`public abstract class PrefixTreeStrategy extends SpatialStrategy`): an
/// abstract SpatialStrategy based on [`SpatialPrefixTree`]. The two
/// subclasses are `RecursivePrefixTreeStrategy` and
/// `TermQueryPrefixTreeStrategy`. This strategy is most effective as a
/// fast approximate spatial search filter.
///
/// Characteristics: can index any shape; however only
/// `RecursivePrefixTreeStrategy` can effectively search non-point shapes;
/// can index a variable number of shapes per field value; can sort by
/// distance by putting the shape's center point into a
/// `ShapeFieldCacheDistanceValueSource`.
///
/// Implementation: the [`SpatialPrefixTree`] does most of the work, for
/// example returning a list of terms representing grids of various sizes
/// for a supplied shape. An important configuration item is
/// [`set_dist_err_pct`](Self::set_dist_err_pct) which balances shape
/// precision against scalability.
///
/// The overridable methods are this trait's provided methods; the
/// strategies are shared as `Arc`s ([`me_prefix`](Self::me_prefix)).
pub trait PrefixTreeStrategy: SpatialStrategy + Any {
    /// The `PrefixTreeStrategy` fields.
    fn prefix_base(&self) -> &PrefixTreeStrategyBase;

    /// This strategy as a shared reference (for `ShapeTokenStream`, Java's
    /// inner class).
    fn me_prefix(&self) -> Arc<dyn PrefixTreeStrategy>;

    /// This strategy as `Any`.
    fn as_any(&self) -> &dyn Any;

    /// `getGrid()`.
    fn get_grid(&self) -> Arc<dyn SpatialPrefixTree> {
        Arc::clone(&self.prefix_base().grid)
    }

    /// `setDefaultFieldValuesArrayLen(int defaultFieldValuesArrayLen)`: a
    /// memory hint used by `makeDistanceValueSource` for how big the
    /// initial size of each Document's array should be. The default is 2.
    /// Set this to slightly more than the default expected number of points
    /// per document.
    fn set_default_field_values_array_len(&self, default_field_values_array_len: i32) {
        self.prefix_base()
            .default_field_values_array_len
            .store(default_field_values_array_len, Ordering::Relaxed);
    }

    /// `getDistErrPct()`.
    fn get_dist_err_pct(&self) -> f64 {
        self.prefix_base().dist_err_pct.get()
    }

    /// `setDistErrPct(double distErrPct)`: the default measure of shape
    /// precision affecting shapes at index and query times. Points don't
    /// use this as they are always indexed at the configured maximum
    /// precision (`SpatialPrefixTree.getMaxLevels()`); this applies to all
    /// other shapes. Specific shapes at index and query time can use
    /// something different than this default value. If you don't set a
    /// default then the default is `SpatialArgs.DEFAULT_DISTERRPCT` --
    /// 2.5%.
    fn set_dist_err_pct(&self, dist_err_pct: f64) {
        self.prefix_base().dist_err_pct.set(dist_err_pct);
    }

    /// `isPointsOnly()`.
    fn is_points_only(&self) -> bool {
        self.prefix_base().points_only.load(Ordering::Relaxed)
    }

    /// `setPointsOnly(boolean pointsOnly)`: if the data is only points
    /// (and not shapes), set this to true for improved performance.
    fn set_points_only(&self, points_only: bool) {
        self.prefix_base()
            .points_only
            .store(points_only, Ordering::Relaxed);
    }

    /// `createIndexableFields(Shape shape, double distErr)`.
    ///
    /// # Errors
    ///
    /// As [`create_indexable_fields_at_level`](Self::create_indexable_fields_at_level).
    fn create_indexable_fields_with_dist_err(
        &self,
        shape: &Arc<dyn Shape>,
        dist_err: f64,
    ) -> Result<Vec<Arc<dyn IndexableField>>> {
        let detail_level = self.prefix_base().grid.get_level_for_distance(dist_err);
        self.create_indexable_fields_at_level(shape, detail_level)
    }

    /// `createIndexableFields(Shape shape, int detailLevel)`.
    ///
    /// # Errors
    ///
    /// The errors of `createCellIteratorToIndex` and of the field.
    fn create_indexable_fields_at_level(
        &self,
        shape: &Arc<dyn Shape>,
        detail_level: i32,
    ) -> Result<Vec<Arc<dyn IndexableField>>> {
        // TODO re-use TokenStream LUCENE-5776: Subclass Field, put cell iterator there, override
        // tokenStream()
        let cells = self.create_cell_iterator_to_index(shape, detail_level)?;
        let mut cell_to_bytes_ref_iterator = self.new_cell_to_bytes_ref_iterator();
        cell_to_bytes_ref_iterator.reset(cells);
        let mut token_stream = BytesRefIteratorTokenStream::new();
        token_stream.set_bytes_ref_iterator(Box::new(cell_to_bytes_ref_iterator));
        let token_stream: Arc<AtomicRefCell<dyn TokenStream>> =
            Arc::new(AtomicRefCell::new(token_stream));
        let field = Field::new_with_token_stream(
            self.get_field_name().to_string(),
            token_stream,
            Arc::clone(&FIELD_TYPE),
        )?;
        Ok(vec![Arc::new(field)])
    }

    /// `tokenStream()`: returns a TokenStream of the terms for any shape
    /// given to its [`set_shape`](ShapeTokenStream::set_shape).
    fn token_stream(&self) -> ShapeTokenStream {
        ShapeTokenStream {
            inner: BytesRefIteratorTokenStream::new(),
            strategy: self.me_prefix(),
        }
    }

    /// `newCellToBytesRefIterator()`: subclasses could return one that
    /// never emits leaves, or does both, or who knows.
    fn new_cell_to_bytes_ref_iterator(&self) -> CellToBytesRefIterator {
        CellToBytesRefIterator::new()
    }

    /// `createCellIteratorToIndex(Shape shape, int detailLevel,
    /// Iterator<Cell> reuse)` (the unused `reuse` dropped).
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a non-point shape when `pointsOnly`;
    /// the errors of `getTreeCellIterator`.
    fn create_cell_iterator_to_index(
        &self,
        shape: &Arc<dyn Shape>,
        detail_level: i32,
    ) -> Result<Box<dyn CellIterator>> {
        prefix_create_cell_iterator_to_index(self, shape, detail_level)
    }

    /// `isPointShape(Shape shape)`.
    fn is_point_shape(&self, shape: &dyn Shape) -> bool {
        shape.as_point().is_some()
    }

    /// `calcFacets(IndexReaderContext context, Bits topAcceptDocs, Shape
    /// inputShape, int facetLevel, int maxCells)`: computes spatial facets
    /// in two dimensions as a grid of numbers. The data is often visualized
    /// as a so-called "heatmap". See `HeatmapFacetCounter.calcFacets`.
    ///
    /// # Errors
    ///
    /// The errors of the facet computation.
    fn calc_facets(
        &self,
        context: &Arc<dyn IndexReaderContext>,
        top_accept_docs: Option<&dyn Bits>,
        input_shape: Option<Arc<dyn Shape>>,
        facet_level: i32,
        max_cells: i32,
    ) -> Result<Heatmap> {
        heatmap_facet_counter::calc_facets(
            &*self.me_prefix(),
            context,
            top_accept_docs,
            input_shape,
            facet_level,
            max_cells,
        )
    }
}

/// The body of `PrefixTreeStrategy.createCellIteratorToIndex` (the one a
/// subclass reaches with `super`).
///
/// # Errors
///
/// As [`PrefixTreeStrategy::create_cell_iterator_to_index`].
pub fn prefix_create_cell_iterator_to_index<S: PrefixTreeStrategy + ?Sized>(
    s: &S,
    shape: &Arc<dyn Shape>,
    detail_level: i32,
) -> Result<Box<dyn CellIterator>> {
    if s.is_points_only() && !s.is_point_shape(&**shape) {
        return Err(LuceneError::illegal_argument(format!(
            "pointsOnly is true yet a class {} is given for indexing",
            shape.java_class_name()
        )));
    }
    s.prefix_base()
        .grid
        .get_tree_cell_iterator(Arc::clone(shape), detail_level) // TODO should take a re-use iterator
}

/// The body of `PrefixTreeStrategy.createIndexableFields(Shape shape)`.
///
/// # Errors
///
/// The errors of `SpatialArgs.calcDistanceFromErrPct` and of the fields.
pub fn prefix_create_indexable_fields<S: PrefixTreeStrategy + ?Sized>(
    s: &S,
    shape: &Arc<dyn Shape>,
) -> Result<Vec<Arc<dyn IndexableField>>> {
    let dist_err = SpatialArgs::calc_distance_from_err_pct(
        &**shape,
        s.get_dist_err_pct(),
        &s.prefix_base().strategy.ctx,
    )?;
    s.create_indexable_fields_with_dist_err(shape, dist_err)
}

/// The body of `PrefixTreeStrategy.makeDistanceValueSource(Point
/// queryPoint, double multiplier)`.
pub fn prefix_make_distance_value_source<S: PrefixTreeStrategy + ?Sized>(
    s: &S,
    query_point: &Arc<dyn Point>,
    multiplier: f64,
) -> Arc<dyn DoubleValuesSource> {
    let base = s.prefix_base();
    let field_name = s.get_field_name().to_string();
    let p = {
        // double checked locking idiom is okay since provider is threadsafe
        let mut provider = base.provider.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(provider.entry(field_name.clone()).or_insert_with(|| {
            Arc::new(PointPrefixTreeFieldCacheProvider::new(
                Arc::clone(&base.grid),
                &field_name,
                base.default_field_values_array_len.load(Ordering::Relaxed),
            ))
        }))
    };
    Arc::new(ShapeFieldCacheDistanceValueSource::new(
        Arc::clone(&base.strategy.ctx),
        p,
        Arc::clone(query_point),
        multiplier,
    ))
}

/// Equivalent to `org.apache.lucene.spatial.prefix.PrefixTreeStrategy.ShapeTokenStream`
/// (`public class ShapeTokenStream extends BytesRefIteratorTokenStream`): a
/// token stream of the cells of the shape given to
/// [`set_shape`](Self::set_shape).
pub struct ShapeTokenStream {
    inner: BytesRefIteratorTokenStream,
    strategy: Arc<dyn PrefixTreeStrategy>,
}

impl fmt::Debug for ShapeTokenStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShapeTokenStream")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl ShapeTokenStream {
    /// `setShape(Shape shape)`.
    ///
    /// # Errors
    ///
    /// The errors of computing the cells.
    pub fn set_shape(&mut self, shape: &Arc<dyn Shape>) -> Result<()> {
        let s = &self.strategy;
        let dist_err = SpatialArgs::calc_distance_from_err_pct(
            &**shape,
            s.get_dist_err_pct(),
            &s.prefix_base().strategy.ctx,
        )?;
        let detail_level = s.prefix_base().grid.get_level_for_distance(dist_err);
        let cells = s.create_cell_iterator_to_index(shape, detail_level)?;
        let mut cell_to_bytes_ref_iterator = s.new_cell_to_bytes_ref_iterator();
        cell_to_bytes_ref_iterator.reset(cells);
        self.inner
            .set_bytes_ref_iterator(Box::new(cell_to_bytes_ref_iterator));
        Ok(())
    }
}

impl TokenStream for ShapeTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        self.inner.increment_token()
    }

    fn reset(&mut self) -> Result<()> {
        self.inner.reset()
    }

    fn end(&mut self) -> Result<()> {
        self.inner.end()
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn attribute_source(&self) -> &AttributeSource {
        self.inner.attribute_source()
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        self.inner.attribute_source_mut()
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
