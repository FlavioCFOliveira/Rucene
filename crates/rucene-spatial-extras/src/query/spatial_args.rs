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

//! The types declared in `SpatialArgs.java`, of the Java package
//! `org.apache.lucene.spatial.query`.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use rucene::error::{LuceneError, Result};
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::shape::Shape;

use crate::query::spatial_args_parser::SpatialArgsParser;
use crate::query::spatial_operation::SpatialOperation;

/// Equivalent to `org.apache.lucene.spatial.query.SpatialArgs` (`public
/// class`): principally holds the query `Shape` and the
/// [`SpatialOperation`]. It's used as an argument to some methods on
/// `SpatialStrategy`.
///
/// Java's arguments are a mutable object the strategies may change (as
/// `CompositeSpatialStrategy.makeQuery` does), so the fields are behind a
/// lock and the setters take `&self`.
#[derive(Debug)]
pub struct SpatialArgs {
    state: Mutex<SpatialArgsState>,
}

#[derive(Debug, Clone)]
struct SpatialArgsState {
    operation: SpatialOperation,
    shape: Arc<dyn Shape>,
    dist_err_pct: Option<f64>,
    dist_err: Option<f64>,
}

impl Clone for SpatialArgs {
    fn clone(&self) -> Self {
        Self {
            state: Mutex::new(self.lock().clone()),
        }
    }
}

impl SpatialArgs {
    /// `DEFAULT_DISTERRPCT`.
    pub const DEFAULT_DISTERRPCT: f64 = 0.025;

    /// `SpatialArgs(SpatialOperation operation, Shape shape)` (both are
    /// required, which Rust's types enforce).
    pub fn new(operation: SpatialOperation, shape: Arc<dyn Shape>) -> Self {
        Self {
            state: Mutex::new(SpatialArgsState {
                operation,
                shape,
                dist_err_pct: None,
                dist_err: None,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, SpatialArgsState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `calcDistanceFromErrPct(Shape shape, double distErrPct,
    /// SpatialContext ctx)`: computes the distance given a shape and the
    /// `distErrPct`. The algorithm is the fraction of the distance from the
    /// center of the query shape to its closest bounding box corner.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when `dist_err_pct` is out of `[0, 0.5]`.
    pub fn calc_distance_from_err_pct(shape: &dyn Shape, dist_err_pct: f64, ctx: &SpatialContext) -> Result<f64> {
        if !(0.0..=0.5).contains(&dist_err_pct) {
            return Err(LuceneError::illegal_argument(format!(
                "distErrPct {} must be between [0 to 0.5]",
                crate::jdk::d2s(dist_err_pct)
            )));
        }
        if dist_err_pct == 0.0 || shape.as_point().is_some() {
            return Ok(0.0);
        }
        let bbox = shape.get_bounding_box();
        // Compute the distance from the center to a corner.  Because the distance
        // to a bottom corner vs a top corner can vary in a geospatial scenario,
        // take the closest one (greater precision).
        let ctr = bbox.get_center();
        let y = if ctr.get_y() >= 0.0 { bbox.get_max_y() } else { bbox.get_min_y() };
        let diagonal_dist = ctx.get_dist_calc().distance_xy(&*ctr, bbox.get_max_x(), y);
        Ok(diagonal_dist * dist_err_pct)
    }

    /// `resolveDistErr(SpatialContext ctx, double defaultDistErrPct)`: gets
    /// the error distance that specifies how precise the query shape is.
    /// This looks at `getDistErr()`, `getDistErrPct()`, and
    /// `default_dist_err_pct`.
    ///
    /// # Errors
    ///
    /// As [`calc_distance_from_err_pct`](Self::calc_distance_from_err_pct).
    pub fn resolve_dist_err(&self, ctx: &SpatialContext, default_dist_err_pct: f64) -> Result<f64> {
        if let Some(dist_err) = self.get_dist_err() {
            return Ok(dist_err);
        }
        let dist_err_pct = self.get_dist_err_pct().unwrap_or(default_dist_err_pct);
        Self::calc_distance_from_err_pct(&*self.get_shape(), dist_err_pct, ctx)
    }

    /// `validate()`: check if the arguments make sense.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException("Only distErr or distErrPct can be
    /// specified.")`.
    pub fn validate(&self) -> Result<()> {
        if self.get_dist_err().is_some() && self.get_dist_err_pct().is_some() {
            return Err(LuceneError::illegal_argument("Only distErr or distErrPct can be specified."));
        }
        Ok(())
    }

    /// `getOperation()`.
    pub fn get_operation(&self) -> SpatialOperation {
        self.lock().operation
    }

    /// `setOperation(SpatialOperation operation)`.
    pub fn set_operation(&self, operation: SpatialOperation) {
        self.lock().operation = operation;
    }

    /// `getShape()`.
    pub fn get_shape(&self) -> Arc<dyn Shape> {
        Arc::clone(&self.lock().shape)
    }

    /// `setShape(Shape shape)`.
    pub fn set_shape(&self, shape: Arc<dyn Shape>) {
        self.lock().shape = shape;
    }

    /// `getDistErrPct()`: a measure of acceptable error of the shape as a
    /// fraction. This effectively inflates the size of the shape but should
    /// not shrink it.
    pub fn get_dist_err_pct(&self) -> Option<f64> {
        self.lock().dist_err_pct
    }

    /// `setDistErrPct(Double distErrPct)`: a `None` (`null`) argument is
    /// ignored.
    pub fn set_dist_err_pct(&self, dist_err_pct: Option<f64>) {
        if dist_err_pct.is_some() {
            self.lock().dist_err_pct = dist_err_pct;
        }
    }

    /// `getDistErr()`: an alternative to `distErrPct` in which the error is
    /// specified as a distance.
    pub fn get_dist_err(&self) -> Option<f64> {
        self.lock().dist_err
    }

    /// `setDistErr(Double distErr)`.
    pub fn set_dist_err(&self, dist_err: Option<f64>) {
        self.lock().dist_err = dist_err;
    }
}

impl fmt::Display for SpatialArgs {
    /// `toString()`: `SpatialArgsParser.writeSpatialArgs(this)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&SpatialArgsParser::write_spatial_args(self))
    }
}
