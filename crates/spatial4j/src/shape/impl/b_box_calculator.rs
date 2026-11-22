/*******************************************************************************
 * Copyright (c) 2015 David Smiley
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `BBoxCalculator.java`, of the Java package
//! `org.locationtech.spatial4j.shape.impl`.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_double_compare;

use crate::context::spatial_context::SpatialContext;
use crate::shape::rectangle::Rectangle;

/// A `Double` key ordered as `Double.compareTo` orders it.
#[derive(Clone, Copy, Debug)]
struct DoubleKey(f64);

impl PartialEq for DoubleKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for DoubleKey {}

impl PartialOrd for DoubleKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DoubleKey {
    fn cmp(&self, other: &Self) -> Ordering {
        java_double_compare(self.0, other.0)
    }
}

/// Equivalent to `org.locationtech.spatial4j.shape.impl.BBoxCalculator`
/// (`public class`): computes the bounding box of a set of rectangles,
/// finding the smallest longitude range on a geo context.
///
/// Java keeps the longitude ranges in a `TreeMap<Double, Double>` (`maxX ->
/// minX`) and walks it with an entry iterator that it removes through; a
/// `BTreeMap` keyed by `Double.compareTo` order holds the same map, and the
/// iterator is reproduced as a cursor on the current key, whose `next` is the
/// smallest key after it.
#[derive(Debug)]
pub struct BBoxCalculator {
    ctx: Arc<SpatialContext>,
    min_y: f64,
    max_y: f64,
    min_x: f64,
    max_x: f64,
    /// `ranges`: `maxX -> minX`; `None` stands for `null`.
    ranges: Option<BTreeMap<DoubleKey, f64>>,
}

impl BBoxCalculator {
    /// `BBoxCalculator(SpatialContext ctx)`.
    pub fn new(ctx: Arc<SpatialContext>) -> Self {
        Self {
            ctx,
            min_y: f64::INFINITY,
            max_y: f64::NEG_INFINITY,
            min_x: f64::INFINITY,
            max_x: f64::NEG_INFINITY,
            ranges: None,
        }
    }

    /// `expandRange(Rectangle rect)`.
    pub fn expand_range(&mut self, rect: &dyn Rectangle) {
        self.expand_range_xy(rect.get_min_x(), rect.get_max_x(), rect.get_min_y(), rect.get_max_y());
    }

    /// `expandRange(double minX, double maxX, double minY, double maxY)`.
    pub fn expand_range_xy(&mut self, min_x: f64, max_x: f64, min_y: f64, max_y: f64) {
        self.min_y = self.min_y.min(min_y);
        self.max_y = self.max_y.max(max_y);
        self.expand_x_range(min_x, max_x);
    }

    /// `expandXRange(double minX, double maxX)`.
    pub fn expand_x_range(&mut self, min_x: f64, max_x: f64) {
        if !self.ctx.is_geo() {
            self.min_x = self.min_x.min(min_x);
            self.max_x = self.max_x.max(max_x);
            return;
        }
        if self.does_x_world_wrap() {
            return;
        }
        let Some(ranges) = self.ranges.as_mut() else {
            let mut ranges = BTreeMap::new();
            ranges.insert(DoubleKey(max_x), min_x);
            self.ranges = Some(ranges);
            return;
        };
        debug_assert!(!ranges.is_empty());
        // `ranges.tailMap(minX, true).entrySet().iterator()`, or the whole
        // map's iterator when that is empty (wrapped across the dateline).
        let first_key = ranges
            .range((Bound::Included(DoubleKey(min_x)), Bound::Unbounded))
            .next()
            .or_else(|| ranges.iter().next())
            .map(|(k, _)| *k)
            .expect("ranges is not empty");
        let mut cursor = first_key;
        let mut entry_min = ranges[&cursor];
        let mut entry_max = cursor.0;
        if Self::range_contains(entry_min, entry_max, max_x) {
            if Self::range_contains(entry_min, entry_max, min_x) {
                if (min_x != entry_min || max_x != entry_max) //ranges not equal
                    && Self::range_contains(min_x, max_x, entry_min)
                    && Self::range_contains(min_x, max_x, entry_max)
                {
                    self.min_x = -180.0;
                    self.max_x = 180.0;
                    self.ranges = None;
                }
            } else {
                ranges.remove(&cursor);
                ranges.insert(DoubleKey(entry_max), min_x);
            }
        } else {
            //entry does NOT contain maxX:
            let new_min_x = if Self::range_contains(entry_min, entry_max, min_x) { entry_min } else { min_x };
            let mut new_max_x = max_x;
            while Self::range_contains(new_min_x, new_max_x, entry_min) {
                ranges.remove(&cursor); //remove entry!
                if !Self::range_contains(min_x, max_x, entry_max) {
                    new_max_x = entry_max; //adjust newMaxX and stop.
                    break;
                }
                // `entryIter.hasNext()`: a key after the removed one.
                let next = ranges
                    .range((Bound::Excluded(cursor), Bound::Unbounded))
                    .next()
                    .map(|(k, _)| *k);
                cursor = match next {
                    Some(k) => k,
                    None => {
                        if ranges.is_empty() {
                            break;
                        }
                        *ranges.keys().next().expect("ranges is not empty")
                    }
                };
                entry_min = ranges[&cursor];
                entry_max = cursor.0;
            }
            ranges.insert(DoubleKey(new_max_x), new_min_x);
        }
    }

    fn process_ranges(&mut self) {
        let Some(ranges) = self.ranges.take() else {
            return;
        };
        if ranges.len() == 1 {
            // an optimization
            let (k, v) = ranges.iter().next().expect("one entry");
            self.min_x = *v;
            self.max_x = k.0;
        } else {
            let (mut prev_key, _) = ranges.iter().next_back().map(|(k, v)| (k.0, *v)).expect("entries");
            let mut biggest_gap = 0.0;
            let mut possible_remaining_gap = 360.0; //calculating this enables us to exit early; often on the first lap!
            for (k, v) in &ranges {
                let mut width_plus_gap = k.0 - prev_key; // this max - last max
                if width_plus_gap < 0.0 {
                    width_plus_gap += 360.0;
                }
                let mut gap = *v - prev_key; // this min - last max
                if gap < 0.0 {
                    gap += 360.0;
                }
                possible_remaining_gap -= width_plus_gap;
                if gap > biggest_gap {
                    biggest_gap = gap;
                    self.min_x = *v;
                    self.max_x = prev_key;
                    if possible_remaining_gap <= biggest_gap {
                        break; // no point in continuing
                    }
                }
                prev_key = k.0;
            }
        }
    }

    fn range_contains(min_x: f64, max_x: f64, x: f64) -> bool {
        if min_x <= max_x {
            x >= min_x && x <= max_x
        } else {
            x >= min_x || x <= max_x
        }
    }

    /// `doesXWorldWrap()`.
    pub fn does_x_world_wrap(&self) -> bool {
        debug_assert!(self.ctx.is_geo());
        self.min_x == -180.0 && self.max_x == 180.0
    }

    /// `getBoundary()`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of `ctx.makeRectangle`.
    pub fn get_boundary(&mut self) -> Result<Arc<dyn Rectangle>> {
        let (min_x, max_x) = (self.get_min_x(), self.get_max_x());
        self.ctx.make_rectangle(min_x, max_x, self.get_min_y(), self.get_max_y())
    }

    /// `getMinX()`.
    pub fn get_min_x(&mut self) -> f64 {
        if self.ranges.is_some() {
            self.process_ranges();
        }
        self.min_x
    }

    /// `getMaxX()`.
    pub fn get_max_x(&mut self) -> f64 {
        if self.ranges.is_some() {
            self.process_ranges();
        }
        self.max_x
    }

    /// `getMinY()`.
    pub fn get_min_y(&self) -> f64 {
        self.min_y
    }

    /// `getMaxY()`.
    pub fn get_max_y(&self) -> f64 {
        self.max_y
    }
}
