/*******************************************************************************
 * Copyright (c) 2016 David Smiley
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `OnePointsBuilder.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::sync::Arc;

use rucene::error::Result;

use crate::shape::point::Point;
use crate::shape::shape_factory::{PointsBuilder, ShapeFactory};

/// Equivalent to `org.locationtech.spatial4j.io.OnePointsBuilder` (`public
/// class OnePointsBuilder implements
/// ShapeFactory.PointsBuilder<OnePointsBuilder>`): a points builder that
/// accepts exactly one point.
pub struct OnePointsBuilder {
    shape_factory: Arc<dyn ShapeFactory>,
    point: Option<Arc<dyn Point>>,
}

impl OnePointsBuilder {
    /// `OnePointsBuilder(ShapeFactory shapeFactory)`.
    pub fn new(shape_factory: Arc<dyn ShapeFactory>) -> Self {
        Self { shape_factory, point: None }
    }

    /// `getPoint()`; `None` (Java `null`) before a point was given.
    pub fn get_point(&self) -> Option<Arc<dyn Point>> {
        self.point.clone()
    }
}

impl PointsBuilder for OnePointsBuilder {
    fn point_xy(&mut self, x: f64, y: f64) -> Result<()> {
        debug_assert!(self.point.is_none());
        self.point = Some(self.shape_factory.point_xy(x, y)?);
        Ok(())
    }

    fn point_xyz(&mut self, x: f64, y: f64, z: f64) -> Result<()> {
        debug_assert!(self.point.is_none());
        self.point = Some(self.shape_factory.point_xyz(x, y, z)?);
        Ok(())
    }

    fn point_lat_lon(&mut self, latitude: f64, longitude: f64) -> Result<()> {
        debug_assert!(self.point.is_none());
        self.point = Some(self.shape_factory.point_lat_lon(latitude, longitude)?);
        Ok(())
    }
}
