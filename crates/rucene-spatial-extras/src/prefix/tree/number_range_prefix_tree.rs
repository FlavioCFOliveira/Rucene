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

//! The types declared in `NumberRangePrefixTree.java`, of the Java package
//! `org.apache.lucene.spatial.prefix.tree`.

use std::any::Any;
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use rucene::error::{LuceneError, Result};
use rucene::util::extra::ParseException;
use rucene::util::BytesRef;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::context::spatial_context_factory::SpatialContextFactory;
use spatial4j::shape::point::Point;
use spatial4j::shape::r#impl::rectangle_impl::RectangleImpl;
use spatial4j::shape::rectangle::Rectangle;
use spatial4j::shape::shape::Shape;
use spatial4j::shape::spatial_relation::SpatialRelation;

use crate::jdk::{addr, panic_unsupported, unsupported};
use crate::prefix::tree::cell::{Cell, CellRef};
use crate::prefix::tree::cell_iterator::CellIterator;
use crate::prefix::tree::spatial_prefix_tree::{SpatialPrefixTree, SpatialPrefixTreeBase};

/// `DUMMY_CTX`: a dummy `SpatialContext` (not geo, world bounds
/// `(-Infinity, Infinity, 0, 0)`).
static DUMMY_CTX: LazyLock<Arc<SpatialContext>> = LazyLock::new(|| {
    let mut factory = SpatialContextFactory::new();
    factory.geo = false;
    let world: Arc<dyn Rectangle> =
        RectangleImpl::new(f64::NEG_INFINITY, f64::INFINITY, 0.0, 0.0, None);
    factory.world_bounds = Some(world);
    SpatialContext::new(&mut factory).expect("the dummy SpatialContext builds")
});

/// `NumberRangePrefixTree.DUMMY_CTX`.
pub fn dummy_ctx() -> Arc<SpatialContext> {
    Arc::clone(&DUMMY_CTX)
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.NumberRangePrefixTree.NRShape`
/// (`public interface NRShape extends Shape, Cloneable`): base interface for
/// [`Shape`]s this prefix tree supports. It extends `Shape` (spatial4j) for
/// compatibility with the spatial API even though it doesn't intermix with
/// conventional 2D shapes.
pub trait NRShape: Shape {
    /// `roundToLevel(int targetLevel)`: this shape rounded to the target
    /// level. If we are already more course (lower level) then the shape is
    /// returned. Otherwise effectively the cell at the target level is
    /// returned.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` of `toRangeShape` for a range.
    fn round_to_level(&self, target_level: i32) -> Result<Arc<dyn NRShape>>;
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.NumberRangePrefixTree.UnitNRShape`
/// (`public interface UnitNRShape extends NRShape, Comparable<UnitNRShape>`):
/// a unit value shape compatible with this tree. It's basically a fixed
/// grid cell (not range relative to a grid cell). Formerly known as
/// `LevelledValue`.
pub trait UnitNRShape: NRShape {
    /// `getLevel()`.
    fn get_unit_level(&self) -> i32;

    /// `getValAtLevel(int level)`.
    fn get_val_at_level(&self, level: i32) -> i32;

    /// `getShapeAtLevel(int level)`.
    fn get_shape_at_level(&self, level: i32) -> Arc<dyn UnitNRShape>;

    /// `roundToLevel(int targetLevel)` (the covariant override).
    fn round_to_unit_level(&self, target_level: i32) -> Arc<dyn UnitNRShape>;

    /// `clone()`: deep clone with no cells in common with the source.
    fn clone_unit(&self) -> Arc<dyn UnitNRShape>;

    /// `compareTo(UnitNRShape o)`.
    fn compare_to(&self, o: &dyn UnitNRShape) -> i32;
}

/// `shape instanceof UnitNRShape ? (UnitNRShape) shape : null`.
pub fn as_unit_nr_shape(shape: &dyn Shape) -> Option<Arc<dyn UnitNRShape>> {
    shape
        .as_any()
        .downcast_ref::<NRCell>()
        .map(|c| Arc::new(c.clone()) as Arc<dyn UnitNRShape>)
}

/// `shape instanceof SpanUnitsNRShape ? (SpanUnitsNRShape) shape : null`.
pub fn as_span_units_nr_shape(shape: &dyn Shape) -> Option<&SpanUnitsNRShape> {
    shape.as_any().downcast_ref::<SpanUnitsNRShape>()
}

/// Java's `a == b` between two shapes. An [`NRCell`] is a handle on one
/// level of a shared cell stack, so two handles are the same Java object
/// when they name the same level of the same stack.
fn same_shape(a: &dyn Shape, b: &dyn Shape) -> bool {
    match (
        a.as_any().downcast_ref::<NRCell>(),
        b.as_any().downcast_ref::<NRCell>(),
    ) {
        (Some(a), Some(b)) => a.same(b),
        _ => addr(a) == addr(b),
    }
}

/// `NumberRangePrefixTree.comparePrefix(UnitNRShape a, UnitNRShape b)`:
/// compares a to b, returning less than 0, 0, or greater than 0, if a is
/// less than, equal to, or greater than b, respectively, up to their common
/// prefix (i.e. only min(a.levels,b.levels) are compared).
pub fn compare_prefix(a: &dyn UnitNRShape, b: &dyn UnitNRShape) -> i32 {
    let min_level = a.get_unit_level().min(b.get_unit_level());
    for level in 1..=min_level {
        let diff = a.get_val_at_level(level) - b.get_val_at_level(level);
        if diff != 0 {
            return diff;
        }
    }
    0
}

/// `NumberRangePrefixTree.toStringUnitRaw(UnitNRShape lv)`.
pub fn to_string_unit_raw(lv: &dyn UnitNRShape) -> String {
    let mut buf = String::with_capacity(100);
    buf.push('[');
    for level in 1..=lv.get_unit_level() {
        buf.push_str(&lv.get_val_at_level(level).to_string());
        buf.push(',');
    }
    buf.pop(); // chop off ','
    buf.push(']');
    buf
}

/// The fields of `NumberRangePrefixTree`.
#[derive(Debug)]
pub struct NumberRangePrefixTree {
    /// The `SpatialPrefixTree` fields.
    pub base: SpatialPrefixTreeBase,
    /// `maxSubCellsByLevel`.
    pub max_sub_cells_by_level: Vec<i32>,
    /// `termLenByLevel`.
    pub term_len_by_level: Vec<i32>,
    /// `levelByTermLen`.
    pub level_by_term_len: Vec<i32>,
    /// `maxTermLen`: how long could cell.getToken... (that is a leaf)
    /// possibly be?
    pub max_term_len: i32,
}

impl NumberRangePrefixTree {
    /// `NumberRangePrefixTree(int[] maxSubCellsByLevel)`.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when a level has 1 or fewer, or `1 << 15`
    /// or more, states.
    pub fn new(max_sub_cells_by_level: Vec<i32>) -> Result<Self> {
        let max_levels = max_sub_cells_by_level.len() as i32;
        let base = SpatialPrefixTreeBase::new(dummy_ctx(), max_levels);
        // Fill termLenByLevel
        let mut term_len_by_level = vec![0i32; (max_levels + 1) as usize];
        const MAX_STATES: i32 = 1 << 15; // 1 bit less than 2 bytes
        for level in 1..=max_levels as usize {
            let states = max_sub_cells_by_level[level - 1];
            if states >= MAX_STATES || states <= 1 {
                return Err(LuceneError::illegal_argument(format!(
                    "Max states is {MAX_STATES}, given {states} at level {level}"
                )));
            }
            let two_bytes = states >= 256;
            term_len_by_level[level] = term_len_by_level[level - 1] + if two_bytes { 2 } else { 1 };
        }
        let max_term_len = term_len_by_level[max_levels as usize] + 1; // + 1 for leaf byte
                                                                       // Fill levelByTermLen
        let mut level_by_term_len = vec![0i32; max_term_len as usize];
        for level in 1..term_len_by_level.len() {
            let term_len = term_len_by_level[level] as usize;
            let prev_term_len = term_len_by_level[level - 1] as usize;
            if term_len - prev_term_len == 2 {
                // 2 byte delta
                // if the term doesn't completely cover this cell then it must be a leaf of the prior.
                level_by_term_len[term_len - 1] = -1; // won't be used; otherwise erroneous
                level_by_term_len[term_len] = level as i32;
            } else {
                // 1 byte delta
                debug_assert_eq!(term_len - prev_term_len, 1);
                level_by_term_len[term_len] = level as i32;
            }
        }
        Ok(Self {
            base,
            max_sub_cells_by_level,
            term_len_by_level,
            level_by_term_len,
            max_term_len,
        })
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.NumberRangePrefixTree`
/// (`public abstract class NumberRangePrefixTree extends SpatialPrefixTree`):
/// a `SpatialPrefixTree` for single-dimensional numbers and number ranges of
/// fixed precision values (not floating point). Despite its name, the
/// indexed values (and queries) need not actually be ranges, they can be
/// unit instance/values.
///
/// Why might you use this instead of Lucene's built-in integer/long
/// support? Here are some reasons with features based on code in this
/// class, *or are possible based on this class but require a subclass to
/// fully realize it*: Index ranges, not just unit instances; parent nodes
/// can be indexed for faceting; it's flexible on the varying number of
/// subdivisions at each level.
///
/// The fields are [`nr`](Self::nr); the abstract methods are the required
/// methods of this trait, and the methods Java implements in the class are
/// its provided methods.
pub trait NumberRangePrefixTreeImpl: SpatialPrefixTree {
    /// The `NumberRangePrefixTree` fields.
    fn nr(&self) -> &NumberRangePrefixTree;

    /// This tree as a shared reference, for the shapes and cells that refer
    /// back to it (Java's `NumberRangePrefixTree.this`).
    fn me_nr(&self) -> Arc<dyn NumberRangePrefixTreeImpl>;

    /// `toUnitShape(Object value)`: converts the value to a unit shape.
    /// Doesn't parse strings; see [`parse_shape`](Self::parse_shape) for
    /// that. This is the reverse of [`to_object`](Self::to_object).
    ///
    /// Java may change the state of a mutable `value` (a `Calendar` whose
    /// fields `toShape` computes), so it is taken mutably.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a value of an unsupported type.
    fn to_unit_shape(&self, value: &mut dyn Any) -> Result<Arc<dyn UnitNRShape>>;

    /// `toObject(UnitNRShape shape)`: returns the shape as the number type
    /// of this tree (a `Calendar` for the date tree). The reverse of
    /// [`to_unit_shape`](Self::to_unit_shape).
    ///
    /// # Errors
    ///
    /// The errors of the conversion.
    fn to_object(&self, shape: &dyn UnitNRShape) -> Result<Box<dyn Any>>;

    /// `toString(UnitNRShape lv)`: a string representation of the unit
    /// shape, parsable by `parseUnitShape`.
    fn to_string_unit(&self, lv: &dyn UnitNRShape) -> String;

    /// `parseUnitShape(String str)`: parse a string to a [`UnitNRShape`].
    ///
    /// # Errors
    ///
    /// `ParseException` for a malformed string.
    fn parse_unit_shape(&self, str: &str) -> Result<Arc<dyn UnitNRShape>>;

    /// `getNumSubCells(UnitNRShape lv)`: the number of sub-cells of `lv`.
    fn get_num_sub_cells(&self, lv: &dyn UnitNRShape) -> i32 {
        self.nr().max_sub_cells_by_level[lv.get_unit_level() as usize]
    }

    /// `toRangeShape(UnitNRShape startUnit, UnitNRShape endUnit)`: returns a
    /// shape that represents the continuous range between `start` and
    /// `end`. It will be normalized, and so sometimes a [`UnitNRShape`] will
    /// be returned, other times a [`SpanUnitsNRShape`] will be.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Wrong order: ...")` when the arguments
    /// are not in order.
    fn to_range_shape(
        &self,
        start_unit: Arc<dyn UnitNRShape>,
        end_unit: Arc<dyn UnitNRShape>,
    ) -> Result<Arc<dyn NRShape>> {
        // note: this normalization/optimization process is actually REQUIRED based on assumptions
        // elsewhere.
        // Normalize start & end
        let start_unit = start_unit.get_shape_at_level(self.truncate_start_vals(&*start_unit, 0)); // chops off trailing min-vals (zeroes)
        let end_unit = end_unit.get_shape_at_level(self.truncate_end_vals(&*end_unit, 0)); // chops off trailing max-vals
                                                                                           // Optimize to just start or end if it's equivalent, e.g. April to April 1st is April 1st.
        let cmp = compare_prefix(&*start_unit, &*end_unit);
        if cmp > 0 {
            return Err(LuceneError::illegal_argument(format!(
                "Wrong order: {start_unit} TO {end_unit}"
            )));
        }
        if cmp == 0 {
            // one is a prefix of the other
            if start_unit.get_unit_level() == end_unit.get_unit_level() {
                // same
                return Ok(start_unit);
            } else if end_unit.get_unit_level() > start_unit.get_unit_level() {
                // e.g. April to April 1st
                if self.truncate_start_vals(&*end_unit, start_unit.get_unit_level())
                    == start_unit.get_unit_level()
                {
                    return Ok(end_unit);
                }
            } else {
                // minLV level > maxLV level
                // e.g. April 30 to April
                if self.truncate_end_vals(&*start_unit, end_unit.get_unit_level())
                    == end_unit.get_unit_level()
                {
                    return Ok(start_unit);
                }
            }
        }
        Ok(Arc::new(SpanUnitsNRShape::new(
            self.me_nr(),
            start_unit,
            end_unit,
        )))
    }

    /// `truncateStartVals(UnitNRShape lv, int endLevel)`: returns the
    /// level of the last non-zero value of `lv` above `end_level`.
    fn truncate_start_vals(&self, lv: &dyn UnitNRShape, end_level: i32) -> i32 {
        let mut level = lv.get_unit_level();
        while level > end_level {
            if lv.get_val_at_level(level) != 0 {
                return level;
            }
            level -= 1;
        }
        end_level
    }

    /// `truncateEndVals(UnitNRShape lv, int endLevel)`.
    fn truncate_end_vals(&self, lv: &dyn UnitNRShape, end_level: i32) -> i32 {
        let mut level = lv.get_unit_level();
        while level > end_level {
            let max = self.get_num_sub_cells(&*lv.get_shape_at_level(level - 1)) - 1;
            if lv.get_val_at_level(level) != max {
                return level;
            }
            level -= 1;
        }
        end_level
    }

    /// `parseShape(String str)`: detects a range pattern and parses it,
    /// otherwise it's parsed as one shape via `parseUnitShape(String)`. The
    /// range pattern looks like this BNF: `'[' + parseShapeLV + ' TO ' +
    /// parseShapeLV + ']'`. It's the same thing as the `toString()` of the
    /// range shape, notwithstanding range optimization.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` for a null or blank string;
    /// `ParseException` for a malformed one.
    fn parse_shape(&self, str: &str) -> Result<Arc<dyn NRShape>> {
        if str.is_empty() {
            return Err(LuceneError::illegal_argument("str is null or blank"));
        }
        if str.starts_with('[') {
            if !str.ends_with(']') {
                return Err(ParseException::error(
                    format!("If starts with [ must end with ]; got {str}"),
                    str.encode_utf16().count() as i32 - 1,
                ));
            }
            let Some(middle) = str.find(" TO ") else {
                return Err(ParseException::error(
                    format!("If starts with [ must contain ' TO '; got {str}"),
                    -1,
                ));
            };
            let left_str = &str[1..middle];
            let right_str = &str[middle + " TO ".len()..str.len() - 1];
            self.to_range_shape(
                self.parse_unit_shape(left_str)?,
                self.parse_unit_shape(right_str)?,
            )
        } else if str.starts_with('{') {
            Err(ParseException::error(
                format!("Exclusive ranges not supported; got {str}"),
                0,
            ))
        } else {
            Ok(self.parse_unit_shape(str)?)
        }
    }

    /// `toShape(int[] valStack, int len)`.
    fn to_shape_stack(&self, val_stack: &[i32], len: usize) -> Arc<dyn UnitNRShape> {
        let stack = NRCell::new_cell_stack(self.me_nr(), len);
        for (i, &val) in val_stack[..len].iter().enumerate() {
            stack.cell(i + 1).reset_cell_with_cell_num(val);
        }
        Arc::new(stack.cell(len))
    }

    /// `getWorldCell()`.
    fn nr_get_world_cell(&self) -> CellRef {
        Arc::new(NRCell::new_cell_stack(self.me_nr(), self.nr().base.max_levels as usize).cell(0))
    }

    /// `readCell(BytesRef term, Cell scratch)`.
    fn nr_read_cell(&self, term: &BytesRef, scratch: Option<CellRef>) -> CellRef {
        let scratch = scratch.unwrap_or_else(|| self.nr_get_world_cell());
        // We decode level #, leaf boolean, and populate bytes by reference. We don't decode the stack.
        // reverse lookup term length to the level and hence the cell
        let cells_by_level = Arc::clone(&nr_cell(&*scratch).stack);
        let is_leaf = term.bytes[term.offset + term.length - 1] == 0;
        let len_no_leaf = if is_leaf {
            term.length - 1
        } else {
            term.length
        };
        let level = self.nr().level_by_term_len[len_no_leaf];
        let result = NRCell::handle(
            &cells_by_level,
            usize::try_from(level).unwrap_or_else(|_| {
                panic!("java.lang.ArrayIndexOutOfBoundsException: Index {level} out of bounds")
            }),
        );
        {
            let mut st = cells_by_level.lock();
            if st.term_buf.is_none() {
                // a kluge; see cell.ensureOwnTermBytes()
                st.term_buf = Some(std::mem::take(&mut st.term.bytes));
            }
            st.term.bytes = term.bytes.clone();
            st.term.offset = term.offset;
            st.term.length = len_no_leaf; // technically this isn't used but may help debugging
            let cs = &mut st.cells[result.level];
            cs.cell_is_leaf = false;
            cs.cell_shape_rel = None;
            if is_leaf {
                cs.cell_is_leaf = true;
            }
            cs.cell_number = -1; // lazy decode flag
        }
        Arc::new(result)
    }

    /// `toString()`: `getClass().getSimpleName()`.
    fn nr_to_string(&self) -> String;
}

/// `(NRCell) c`.
fn nr_cell(c: &dyn Cell) -> &NRCell {
    c.as_any()
        .downcast_ref::<NRCell>()
        .unwrap_or_else(|| panic!("java.lang.ClassCastException: {c:?} cannot be cast to NRCell"))
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.NumberRangePrefixTree.SpanUnitsNRShape`
/// (`public class SpanUnitsNRShape implements NRShape`): a range of two
/// [`UnitNRShape`]s, one at the start and one at the end of the range.
pub struct SpanUnitsNRShape {
    tree: Arc<dyn NumberRangePrefixTreeImpl>,
    min_lv: Arc<dyn UnitNRShape>,
    max_lv: Arc<dyn UnitNRShape>,
    last_level_in_common: i32, // computed; not part of identity
}

impl SpanUnitsNRShape {
    /// `SpanUnitsNRShape(UnitNRShape minLV, UnitNRShape maxLV)`: don't call
    /// directly; see `toRangeShape`.
    fn new(
        tree: Arc<dyn NumberRangePrefixTreeImpl>,
        min_lv: Arc<dyn UnitNRShape>,
        max_lv: Arc<dyn UnitNRShape>,
    ) -> Self {
        // calc lastLevelInCommon
        let mut level = 1;
        while level <= min_lv.get_unit_level() && level <= max_lv.get_unit_level() {
            if min_lv.get_val_at_level(level) != max_lv.get_val_at_level(level) {
                break;
            }
            level += 1;
        }
        Self {
            tree,
            min_lv,
            max_lv,
            last_level_in_common: level - 1,
        }
    }

    /// `getMinUnit()`.
    pub fn get_min_unit(&self) -> &Arc<dyn UnitNRShape> {
        &self.min_lv
    }

    /// `getMaxUnit()`.
    pub fn get_max_unit(&self) -> &Arc<dyn UnitNRShape> {
        &self.max_lv
    }

    fn get_levels_in_common(&self) -> i32 {
        self.last_level_in_common
    }

    /// `relate(SpanUnitsNRShape ext)`.
    pub fn relate_span(&self, ext: &SpanUnitsNRShape) -> SpatialRelation {
        // This logic somewhat mirrors RectangleImpl.relate_range()
        let ext_min_int_max = compare_prefix(&**ext.get_min_unit(), &**self.get_max_unit());
        if ext_min_int_max > 0 {
            return SpatialRelation::Disjoint;
        }
        let ext_max_int_min = compare_prefix(&**ext.get_max_unit(), &**self.get_min_unit());
        if ext_max_int_min < 0 {
            return SpatialRelation::Disjoint;
        }
        let ext_min_int_min = compare_prefix(&**ext.get_min_unit(), &**self.get_min_unit());
        let ext_max_int_max = compare_prefix(&**ext.get_max_unit(), &**self.get_max_unit());
        if (ext_min_int_min > 0
            || ext_min_int_min == 0
                && ext.get_min_unit().get_unit_level() >= self.get_min_unit().get_unit_level())
            && (ext_max_int_max < 0
                || ext_max_int_max == 0
                    && ext.get_max_unit().get_unit_level() >= self.get_max_unit().get_unit_level())
        {
            return SpatialRelation::Contains;
        }
        if (ext_min_int_min < 0
            || ext_min_int_min == 0
                && ext.get_min_unit().get_unit_level() <= self.get_min_unit().get_unit_level())
            && (ext_max_int_max > 0
                || ext_max_int_max == 0
                    && ext.get_max_unit().get_unit_level() <= self.get_max_unit().get_unit_level())
        {
            return SpatialRelation::Within;
        }
        SpatialRelation::Intersects
    }

    /// `clone()`.
    pub fn clone_span(&self) -> SpanUnitsNRShape {
        SpanUnitsNRShape::new(
            Arc::clone(&self.tree),
            self.min_lv.clone_unit(),
            self.max_lv.clone_unit(),
        )
    }
}

impl fmt::Debug for SpanUnitsNRShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for SpanUnitsNRShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{} TO {}]",
            self.tree.to_string_unit(&*self.min_lv),
            self.tree.to_string_unit(&*self.max_lv)
        )
    }
}

impl NRShape for SpanUnitsNRShape {
    fn round_to_level(&self, target_level: i32) -> Result<Arc<dyn NRShape>> {
        self.tree.to_range_shape(
            self.min_lv.round_to_unit_level(target_level),
            self.max_lv.round_to_unit_level(target_level),
        )
    }
}

impl Shape for SpanUnitsNRShape {
    spatial4j::spatial4j_shape_boilerplate!(
        "org.apache.lucene.spatial.prefix.tree.NumberRangePrefixTree$SpanUnitsNRShape"
    );

    fn relate(&self, shape: &dyn Shape) -> SpatialRelation {
        //      if (shape instanceof UnitNRShape)
        //        return relate((UnitNRShape)shape);
        if let Some(span) = as_span_units_nr_shape(shape) {
            return self.relate_span(span);
        }
        shape.relate(self).transpose() // probably a UnitNRShape
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        panic_unsupported()
    }

    fn has_area(&self) -> bool {
        true
    }

    fn get_area(&self, _ctx: Option<&SpatialContext>) -> f64 {
        panic_unsupported()
    }

    fn get_center(&self) -> Arc<dyn Point> {
        panic_unsupported()
    }

    fn get_buffered(&self, _distance: f64, _ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        Err(unsupported())
    }

    fn is_empty(&self) -> bool {
        false
    }

    fn equals(&self, o: &dyn Shape) -> bool {
        if same_shape(self, o) {
            return true;
        }
        let Some(span_shape) = as_span_units_nr_shape(o) else {
            return false;
        };
        if !self.max_lv.equals(span_shape.max_lv.as_shape()) {
            return false;
        }
        if !self.min_lv.equals(span_shape.min_lv.as_shape()) {
            return false;
        }
        true
    }

    fn hash_code(&self) -> i32 {
        let mut result = self.min_lv.hash_code();
        result = result
            .wrapping_mul(31)
            .wrapping_add(self.max_lv.hash_code());
        result
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        Some(dummy_ctx())
    }
}

// ---------------------------------------------------------------- NRCell

/// A `BytesRef` field of the cell stack: the shared term of all its cells.
#[derive(Debug, Clone, Default)]
struct TermRef {
    bytes: Vec<u8>,
    offset: usize,
    length: usize,
}

/// The per-cell state of an `NRCell` (its "Cell state" and "CellIterator
/// state" fields, and the `nextCell`/`thisCell` fields it inherits from
/// `CellIterator`, which only ever hold the cell itself).
#[derive(Debug, Clone, Default)]
struct NRCellState {
    cell_number: i32, // relative to parent cell. It's unused for level 0. Starts at 0.
    cell_shape_rel: Option<SpatialRelation>,
    cell_is_leaf: bool,
    iter_filter: Option<Arc<dyn Shape>>, // UnitNRShape or NRShape
    iter_first_is_intersects: bool,
    iter_last_is_intersects: bool,
    iter_first_cell_number: i32,
    iter_last_cell_number: i32,
    next_cell: bool,
    this_cell: bool,
}

/// The state shared by the cells of one stack (`cellsByLevel`, `term`, and
/// cell 0's `termBuf`).
#[derive(Debug)]
struct NRStackState {
    term: TermRef,             // AKA the token
    term_buf: Option<Vec<u8>>, // see ensureOwnTermBytes(), only for cell0
    cells: Vec<NRCellState>,
}

/// One `cellsByLevel` array of `NRCell`s: the cells of one stack share
/// their term bytes and know each other.
struct NRStack {
    tree: Arc<dyn NumberRangePrefixTreeImpl>,
    state: Mutex<NRStackState>,
}

impl fmt::Debug for NRStack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NRStack")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl NRStack {
    fn lock(&self) -> MutexGuard<'_, NRStackState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cell(self: &Arc<Self>, level: usize) -> NRCell {
        NRCell::handle(self, level)
    }
}

/// Equivalent to `org.apache.lucene.spatial.prefix.tree.NumberRangePrefixTree.NRCell`
/// (`protected class NRCell extends CellIterator implements Cell,
/// UnitNRShape`): most of the guts of this prefix tree are actually in this
/// class. One struct but serves many purposes: a [`Cell`], a
/// [`CellIterator`], and a [`UnitNRShape`]. Even though a cell is a
/// `UnitNRShape`, it is never handed out as one directly: Java's
/// `getShape()` returns the same object (after decoding).
///
/// Java creates the cells of one stack (`cellsByLevel`) together; they
/// share their term bytes and refer to each other. Here the stack is shared
/// state behind a lock and an `NRCell` is a handle on one of its levels:
/// two handles on the same level of the same stack are the same Java object
/// (for `==`, which Java uses on cells and on the iterator's filter shape).
#[derive(Clone)]
pub struct NRCell {
    stack: Arc<NRStack>,
    level: usize, // cellLevel; assert levelStack[cellLevel] == this
}

impl fmt::Debug for NRCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NRCell(level={})", self.level)
    }
}

impl NRCell {
    /// `newCellStack(int levels)`: the cells of a new stack, of levels
    /// `0..=levels`.
    fn new_cell_stack(tree: Arc<dyn NumberRangePrefixTreeImpl>, levels: usize) -> Arc<NRStack> {
        let max_term_len = tree.nr().max_term_len as usize;
        let cells = (0..=levels)
            .map(|level| NRCellState {
                cell_number: if level == 0 { 0 } else { -1 },
                cell_is_leaf: false,
                ..NRCellState::default()
            })
            .collect();
        Arc::new(NRStack {
            tree,
            state: Mutex::new(NRStackState {
                term: TermRef {
                    bytes: vec![0u8; max_term_len],
                    offset: 0,
                    length: 0,
                },
                term_buf: None,
                cells,
            }),
        })
    }

    fn handle(stack: &Arc<NRStack>, level: usize) -> NRCell {
        NRCell {
            stack: Arc::clone(stack),
            level,
        }
    }

    /// Java's `this == other`.
    fn same(&self, other: &NRCell) -> bool {
        Arc::ptr_eq(&self.stack, &other.stack) && self.level == other.level
    }

    fn tree(&self) -> &Arc<dyn NumberRangePrefixTreeImpl> {
        &self.stack.tree
    }

    fn term_len_by_level(&self, level: usize) -> usize {
        self.tree().nr().term_len_by_level[level] as usize
    }

    /// `ensureOwnTermBytes()`.
    fn ensure_own_term_bytes(st: &mut NRStackState) {
        let Some(mut buf) = st.term_buf.take() else {
            return; // we already own the bytes
        };
        let (off, len) = (st.term.offset, st.term.length);
        buf[..len].copy_from_slice(&st.term.bytes[off..off + len]);
        st.term.bytes = buf;
        st.term.offset = 0;
    }

    /// `resetCellWithCellNum(int cellNumber)`.
    fn reset_cell_with_cell_num(&self, cell_number: i32) {
        let term_len = self.term_len_by_level(self.level);
        let two_bytes = self.level > 0 && (term_len - self.term_len_by_level(self.level - 1)) > 1;
        let mut st = self.stack.lock();
        Self::reset_with(&mut st, self.level, cell_number, term_len, two_bytes);
    }

    fn reset_with(
        st: &mut NRStackState,
        level: usize,
        cell_number: i32,
        term_len: usize,
        two_bytes: bool,
    ) {
        // reset()
        st.cells[level].cell_is_leaf = false;
        st.cells[level].cell_shape_rel = None;
        // update bytes
        //  note: see lazyInitCellNumsFromBytes() for the reverse
        if cell_number >= 0 {
            // valid
            Self::ensure_own_term_bytes(st);
            if two_bytes {
                // right 7 bits, plus 1 (may overflow to 8th bit which is okay)
                st.term.bytes[term_len - 2] = (cell_number >> 7) as u8;
                st.term.bytes[term_len - 1] = ((cell_number & 0x7F) + 1) as u8;
            } else {
                st.term.bytes[term_len - 1] = (cell_number + 1) as u8;
            }
            debug_assert_ne!(st.term.bytes[term_len - 1], 0);
            st.term.length = term_len;
        }
        st.cells[level].cell_number = cell_number;
    }

    /// `ensureDecoded()`.
    fn ensure_decoded(&self) {
        let nr = self.tree().nr();
        let mut st = self.stack.lock();
        if st.cells[self.level].cell_number >= 0 {
            return;
        }
        // Decode cell numbers from bytes. This is the inverse of resetCellWithCellNum().
        for level in 1..=self.level {
            let term_len = nr.term_len_by_level[level] as usize;
            let two_bytes = (term_len - nr.term_len_by_level[level - 1] as usize) > 1;
            let off = st.term.offset;
            let cell_number = if two_bytes {
                let byte_h = i32::from(st.term.bytes[off + term_len - 2]);
                let byte_l = i32::from(st.term.bytes[off + term_len - 1]);
                debug_assert!(byte_l - 1 < (1 << 7));
                (byte_h << 7) + (byte_l - 1)
            } else {
                i32::from(st.term.bytes[off + term_len - 1]) - 1
            };
            st.cells[level].cell_number = cell_number;
            debug_assert!(
                cell_number >= 0,
                "Illegal state; ensureDecoded() wasn't called"
            );
        }
    }

    fn assert_decoded(&self) {
        debug_assert!(
            self.stack.lock().cells[self.level].cell_number >= 0,
            "Illegal state; ensureDecoded() wasn't called"
        );
    }

    /// The term bytes up to `termLenByLevel[level]`.
    fn prefix_bytes(&self) -> Vec<u8> {
        let len = self.term_len_by_level(self.level);
        let st = self.stack.lock();
        st.term.bytes[st.term.offset..st.term.offset + len].to_vec()
    }

    /// `initIter(Shape filter)`.
    fn init_iter(&self, filter: Option<Arc<dyn Shape>>) {
        self.stack.lock().cells[self.level].cell_number = -1;
        let mut filter = filter;
        if let Some(f) = &filter {
            if let Some(unit) = as_unit_nr_shape(&**f) {
                if unit.get_unit_level() == 0 {
                    filter = None; // world means everything -- no filter
                }
            }
        }
        self.stack.lock().cells[self.level].iter_filter = filter.clone();
        let parent = self.get_shape_at_level_cell(self.level as i32 - 1);
        let (p_filter, p_first_is, p_last_is, p_first, p_last, p_cell_number) = {
            let st = self.stack.lock();
            let p = &st.cells[parent.level];
            (
                p.iter_filter.clone(),
                p.iter_first_is_intersects,
                p.iter_last_is_intersects,
                p.iter_first_cell_number,
                p.iter_last_cell_number,
                p.cell_number,
            )
        };
        let my_level = self.level as i32;
        let set = |first: i32, first_is: bool, last: i32, last_is: bool| {
            let mut st = self.stack.lock();
            let c = &mut st.cells[self.level];
            c.iter_first_cell_number = first;
            c.iter_first_is_intersects = first_is;
            c.iter_last_cell_number = last;
            c.iter_last_is_intersects = last_is;
        };
        // Initialize iter* members.
        // no filter means all subcells
        let Some(filter) = filter else {
            set(0, false, self.tree().get_num_sub_cells(&parent) - 1, false);
            return;
        };
        let min_lv: Arc<dyn UnitNRShape>;
        let max_lv: Arc<dyn UnitNRShape>;
        let last_level_in_common: i32; // between minLV & maxLV
        if let Some(span_shape) = as_span_units_nr_shape(&*filter) {
            min_lv = Arc::clone(span_shape.get_min_unit());
            max_lv = Arc::clone(span_shape.get_max_unit());
            last_level_in_common = span_shape.get_levels_in_common();
        } else {
            min_lv = as_unit_nr_shape(&*filter).unwrap_or_else(|| {
                panic!("java.lang.ClassCastException: {filter} cannot be cast to UnitNRShape")
            });
            max_lv = Arc::clone(&min_lv);
            last_level_in_common = min_lv.get_unit_level();
        }
        let first_last = |mut first: i32, mut first_is: bool, mut last: i32, mut last_is: bool| {
            if first == last {
                if last_is {
                    first_is = true;
                } else if first_is {
                    last_is = true;
                }
            }
            let _ = (&mut first, &mut last);
            set(first, first_is, last, last_is);
        };
        // fast path optimization that is usually true, but never first level
        if p_filter
            .as_ref()
            .is_some_and(|pf| same_shape(&*filter, &**pf))
            && (my_level <= last_level_in_common || p_first != p_last)
        {
            // TODO benchmark if this optimization pays off. We avoid two comparePrefixLV calls.
            let (first, first_is) =
                if p_first_is && p_cell_number == p_first && min_lv.get_unit_level() >= my_level {
                    (
                        min_lv.get_val_at_level(my_level),
                        min_lv.get_unit_level() > my_level,
                    )
                } else {
                    (0, false)
                };
            let (last, last_is) =
                if p_last_is && p_cell_number == p_last && max_lv.get_unit_level() >= my_level {
                    (
                        max_lv.get_val_at_level(my_level),
                        max_lv.get_unit_level() > my_level,
                    )
                } else {
                    (self.tree().get_num_sub_cells(&parent) - 1, false)
                };
            first_last(first, first_is, last, last_is);
            return;
        }
        // not common to get here, except for level 1 which always happens
        let start_cmp = compare_prefix(&*min_lv, &parent);
        if start_cmp > 0 {
            // start comes after this cell
            set(0, false, -1, false); // so ends early (no cells)
            return;
        }
        let end_cmp = compare_prefix(&*max_lv, &parent); // compare to end cell
        if end_cmp < 0 {
            // end comes before this cell
            set(0, false, -1, false); // so ends early (no cells)
            return;
        }
        let (first, first_is) = if start_cmp < 0 || min_lv.get_unit_level() < my_level {
            // start comes before...
            (0, false)
        } else {
            (
                min_lv.get_val_at_level(my_level),
                min_lv.get_unit_level() > my_level,
            )
        };
        let (last, last_is) = if end_cmp > 0 || max_lv.get_unit_level() < my_level {
            // end comes after...
            (self.tree().get_num_sub_cells(&parent) - 1, false)
        } else {
            (
                max_lv.get_val_at_level(my_level),
                max_lv.get_unit_level() > my_level,
            )
        };
        first_last(first, first_is, last, last_is);
    }

    /// `getShapeAtLevel(int level)` as the cell.
    fn get_shape_at_level_cell(&self, level: i32) -> NRCell {
        debug_assert!(level as usize <= self.level);
        NRCell::handle(
            &self.stack,
            usize::try_from(level).unwrap_or_else(|_| {
                panic!("java.lang.ArrayIndexOutOfBoundsException: Index {level} out of bounds")
            }),
        )
    }

    /// `relate(UnitNRShape lv)`.
    pub fn relate_unit(&self, lv: &dyn UnitNRShape) -> SpatialRelation {
        self.assert_decoded();
        let cmp = compare_prefix(self, lv);
        if cmp != 0 {
            return SpatialRelation::Disjoint;
        }
        if self.get_unit_level() > lv.get_unit_level() {
            return SpatialRelation::Within;
        }
        SpatialRelation::Contains // or equals
                                  // no INTERSECTS; that won't happen.
    }

    /// `relate(SpanUnitsNRShape spanShape)`.
    pub fn relate_span(&self, span_shape: &SpanUnitsNRShape) -> SpatialRelation {
        self.assert_decoded();
        let start_cmp = compare_prefix(&**span_shape.get_min_unit(), self);
        if start_cmp > 0 {
            // start comes after this cell
            return SpatialRelation::Disjoint;
        }
        let end_cmp = compare_prefix(&**span_shape.get_max_unit(), self);
        if end_cmp < 0 {
            // end comes before this cell
            return SpatialRelation::Disjoint;
        }
        let mut nr_min_level = span_shape.get_min_unit().get_unit_level();
        let mut nr_max_level = span_shape.get_max_unit().get_unit_level();
        let level = self.get_unit_level();
        if (start_cmp < 0 || start_cmp == 0 && nr_min_level <= level)
            && (end_cmp > 0 || end_cmp == 0 && nr_max_level <= level)
        {
            return SpatialRelation::Within; // or equals
        }
        // At this point it's Contains or Within.
        if start_cmp != 0 || end_cmp != 0 {
            return SpatialRelation::Intersects;
        }
        // if min or max Level is less, it might be on the equivalent edge.
        while nr_min_level < level {
            if self.get_val_at_level(nr_min_level + 1) != 0 {
                return SpatialRelation::Intersects;
            }
            nr_min_level += 1;
        }
        while nr_max_level < level {
            if self.get_val_at_level(nr_max_level + 1)
                != self
                    .tree()
                    .get_num_sub_cells(&self.get_shape_at_level_cell(nr_max_level))
                    - 1
            {
                return SpatialRelation::Intersects;
            }
            nr_max_level += 1;
        }
        SpatialRelation::Contains
    }

    /// `toStringDebug()`.
    pub fn to_string_debug(&self) -> String {
        let pretty = self.to_string();
        if self.level == 0 {
            return pretty;
        }
        format!(
            "{}{} {pretty}",
            to_string_unit_raw(self),
            if self.is_leaf() { "\u{2022}" } else { "" }
        )
    }

    /// `term.compareTo(other)` over the two prefixes: `Arrays.compareUnsigned`.
    fn compare_unsigned(a: &[u8], b: &[u8]) -> i32 {
        for (x, y) in a.iter().zip(b.iter()) {
            if x != y {
                return i32::from(*x) - i32::from(*y);
            }
        }
        a.len() as i32 - b.len() as i32
    }
}

impl fmt::Display for NRCell {
    /// `toString()`: `NumberRangePrefixTree.this.toString(getShape())`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.ensure_decoded();
        f.write_str(&self.tree().to_string_unit(self))
    }
}

impl Cell for NRCell {
    fn get_shape_rel(&self) -> Option<SpatialRelation> {
        self.stack.lock().cells[self.level].cell_shape_rel
    }

    fn set_shape_rel(&self, rel: Option<SpatialRelation>) {
        self.stack.lock().cells[self.level].cell_shape_rel = rel;
    }

    fn is_leaf(&self) -> bool {
        self.stack.lock().cells[self.level].cell_is_leaf
    }

    fn set_leaf(&self) {
        self.stack.lock().cells[self.level].cell_is_leaf = true;
    }

    fn get_token_bytes_with_leaf(&self, result: Option<BytesRef>) -> BytesRef {
        Self::ensure_own_term_bytes(&mut self.stack.lock()); // normally shouldn't do anything
        let mut result = self.get_token_bytes_no_leaf(result);
        if self.is_leaf() {
            let at = result.length;
            // Java writes the leaf byte into the shared term array.
            {
                let mut st = self.stack.lock();
                if st.term.bytes.len() > at {
                    st.term.bytes[at] = 0;
                }
            }
            if result.bytes.len() <= at {
                result.bytes.resize(at + 1, 0);
            }
            result.bytes[at] = 0;
            result.length += 1;
        }
        result
    }

    fn get_token_bytes_no_leaf(&self, result: Option<BytesRef>) -> BytesRef {
        let len = self.term_len_by_level(self.level);
        let st = self.stack.lock();
        let mut result = result.unwrap_or_else(|| BytesRef::new(Vec::new()));
        result.bytes.clear();
        result.bytes.extend_from_slice(&st.term.bytes);
        result.offset = st.term.offset;
        result.length = len;
        debug_assert!(result.length <= st.term.length);
        result
    }

    fn get_level(&self) -> i32 {
        self.level as i32
    }

    fn get_next_level_cells(&self, shape_filter: Option<Arc<dyn Shape>>) -> Box<dyn CellIterator> {
        self.ensure_decoded();
        let sub_cell = NRCell::handle(&self.stack, self.level + 1);
        sub_cell.init_iter(shape_filter);
        Box::new(sub_cell)
    }

    fn get_shape(&self) -> Arc<dyn Shape> {
        self.ensure_decoded();
        Arc::new(self.clone())
    }

    fn is_prefix_of(&self, c: &dyn Cell) -> bool {
        let other_cell = nr_cell(c);
        // trick to re-use bytesref; provided that we re-instate it
        let mine = self.prefix_bytes();
        let other = other_cell.prefix_bytes();
        other.starts_with(&mine)
    }

    fn compare_to_no_leaf(&self, from_cell: &dyn Cell) -> i32 {
        let nr_cell = nr_cell(from_cell);
        // trick to re-use bytesref; provided that we re-instate it
        NRCell::compare_unsigned(&self.prefix_bytes(), &nr_cell.prefix_bytes())
    }

    fn cell_equals(&self, other: &dyn Cell) -> bool {
        match other.as_any().downcast_ref::<NRCell>() {
            Some(o) => Shape::equals(self, o),
            None => false,
        }
    }

    fn cell_hash_code(&self) -> i32 {
        Shape::hash_code(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl CellIterator for NRCell {
    fn has_next(&mut self) -> bool {
        let term_len = self.term_len_by_level(self.level);
        let two_bytes = self.level > 0 && (term_len - self.term_len_by_level(self.level - 1)) > 1;
        let mut st = self.stack.lock();
        let level = self.level;
        st.cells[level].this_cell = false;
        if st.cells[level].next_cell {
            // calling hasNext twice in a row
            return true;
        }
        let c = st.cells[level].clone();
        if c.cell_number >= c.iter_last_cell_number {
            return false;
        }
        let next_num = if c.cell_number < c.iter_first_cell_number {
            c.iter_first_cell_number
        } else {
            c.cell_number + 1
        };
        Self::reset_with(&mut st, level, next_num, term_len, two_bytes);
        let cell_number = st.cells[level].cell_number;
        let has_children = (cell_number == c.iter_first_cell_number && c.iter_first_is_intersects)
            || (cell_number == c.iter_last_cell_number && c.iter_last_is_intersects);
        let cs = &mut st.cells[level];
        if !has_children {
            cs.cell_is_leaf = true;
            cs.cell_shape_rel = Some(SpatialRelation::Within);
        } else if c.iter_first_cell_number == c.iter_last_cell_number {
            cs.cell_shape_rel = Some(SpatialRelation::Contains);
        } else {
            cs.cell_shape_rel = Some(SpatialRelation::Intersects);
        }
        cs.next_cell = true;
        true
    }

    fn next(&mut self) -> CellRef {
        let next_cell = self.stack.lock().cells[self.level].next_cell;
        let has_next = next_cell || self.has_next();
        if !has_next {
            panic!("java.util.NoSuchElementException");
        }
        let mut st = self.stack.lock();
        st.cells[self.level].this_cell = true;
        st.cells[self.level].next_cell = false;
        Arc::new(self.clone())
    }

    fn this_cell(&self) -> CellRef {
        assert!(
            self.stack.lock().cells[self.level].this_cell,
            "Only call thisCell() after next(), not hasNext()"
        );
        Arc::new(self.clone())
    }

    // TODO override nextFrom to be more efficient
}

impl NRShape for NRCell {
    fn round_to_level(&self, target_level: i32) -> Result<Arc<dyn NRShape>> {
        Ok(self.round_to_unit_level(target_level))
    }
}

impl UnitNRShape for NRCell {
    fn get_unit_level(&self) -> i32 {
        self.level as i32
    }

    fn get_val_at_level(&self, level: i32) -> i32 {
        let result = self.stack.lock().cells[level as usize].cell_number;
        debug_assert!(result >= 0); // initialized (decoded)
        result
    }

    fn get_shape_at_level(&self, level: i32) -> Arc<dyn UnitNRShape> {
        Arc::new(self.get_shape_at_level_cell(level))
    }

    fn round_to_unit_level(&self, target_level: i32) -> Arc<dyn UnitNRShape> {
        if self.get_unit_level() <= target_level {
            Arc::new(self.clone())
        } else {
            self.get_shape_at_level(target_level)
        }
    }

    fn clone_unit(&self) -> Arc<dyn UnitNRShape> {
        // no leaf distinction; this is purely based on UnitNRShape
        let cell = self
            .tree()
            .nr_read_cell(&self.get_token_bytes_no_leaf(None), None);
        let cell = nr_cell(&*cell);
        NRCell::ensure_own_term_bytes(&mut cell.stack.lock());
        cell.ensure_decoded();
        Arc::new(cell.clone())
    }

    fn compare_to(&self, o: &dyn UnitNRShape) -> i32 {
        self.assert_decoded();
        // no leaf distinction; this is purely based on UnitNRShape
        let cmp = compare_prefix(self, o);
        if cmp != 0 {
            cmp
        } else {
            self.get_unit_level() - o.get_unit_level()
        }
    }
}

impl Shape for NRCell {
    spatial4j::spatial4j_shape_boilerplate!(
        "org.apache.lucene.spatial.prefix.tree.NumberRangePrefixTree$NRCell"
    );

    fn relate(&self, shape: &dyn Shape) -> SpatialRelation {
        self.assert_decoded();
        let (filter, rel) = {
            let st = self.stack.lock();
            let c = &st.cells[self.level];
            (c.iter_filter.clone(), c.cell_shape_rel)
        };
        if let (Some(filter), Some(rel)) = (&filter, rel) {
            if same_shape(shape, &**filter) {
                return rel;
            }
        }
        if let Some(lv) = shape.as_any().downcast_ref::<NRCell>() {
            return self.relate_unit(lv);
        }
        if let Some(span) = as_span_units_nr_shape(shape) {
            return self.relate_span(span);
        }
        shape.relate(self).transpose()
    }

    fn get_bounding_box(&self) -> Arc<dyn Rectangle> {
        panic_unsupported()
    }

    fn has_area(&self) -> bool {
        true
    }

    fn get_area(&self, _ctx: Option<&SpatialContext>) -> f64 {
        panic_unsupported()
    }

    fn get_center(&self) -> Arc<dyn Point> {
        panic_unsupported()
    }

    fn get_buffered(&self, _distance: f64, _ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        Err(unsupported())
    }

    fn is_empty(&self) -> bool {
        false
    }

    fn equals(&self, obj: &dyn Shape) -> bool {
        let Some(nr_cell) = obj.as_any().downcast_ref::<NRCell>() else {
            return false;
        };
        if self.same(nr_cell) {
            return true;
        }
        if self.level != nr_cell.level {
            return false;
        }
        // trick to re-use bytesref; provided that we re-instate it
        self.prefix_bytes() == nr_cell.prefix_bytes()
    }

    fn hash_code(&self) -> i32 {
        // trick to re-use bytesref; provided that we re-instate it
        BytesRef::new(self.prefix_bytes()).hash_code()
    }

    fn get_context(&self) -> Option<Arc<SpatialContext>> {
        Some(dummy_ctx())
    }
}
