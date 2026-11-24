/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `LegacyShapeWriter.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_fmt::DecimalFormat;

use crate::context::spatial_context::SpatialContext;
use crate::context::spatial_context_factory::SpatialContextFactoryOps;
use crate::io::shape_io::{self, ShapeIO};
use crate::io::shape_writer::{io_error, ShapeWriter};
use crate::jdk::make_number_format;
use crate::shape::shape::Shape;

/// Equivalent to `org.locationtech.spatial4j.io.LegacyShapeWriter`
/// (`@Deprecated public class LegacyShapeWriter implements ShapeWriter`):
/// writes the legacy spatial4j shape format.
#[derive(Debug)]
pub struct LegacyShapeWriter;

impl LegacyShapeWriter {
    /// `LegacyShapeWriter(SpatialContext ctx, SpatialContextFactory
    /// factory)`.
    pub fn new(_ctx: &Arc<SpatialContext>, _factory: &dyn SpatialContextFactoryOps) -> Self {
        Self
    }

    /// `writeShape(Shape shape)`: with 6 fraction digits.
    pub fn write_shape(shape: &dyn Shape) -> String {
        Self::write_shape_with(shape, &Self::make_number_format(6))
    }

    /// `writeShape(Shape shape, NumberFormat nf)`.
    pub fn write_shape_with(shape: &dyn Shape, nf: &DecimalFormat) -> String {
        if let Some(point) = shape.as_point() {
            return format!("{} {}", nf.format(point.get_x()), nf.format(point.get_y()));
        } else if let Some(rect) = shape.as_rectangle() {
            return format!(
                "{} {} {} {}",
                nf.format(rect.get_min_x()),
                nf.format(rect.get_min_y()),
                nf.format(rect.get_max_x()),
                nf.format(rect.get_max_y())
            );
        } else if let Some(c) = shape.as_circle() {
            return format!(
                "Circle({} {} d={})",
                nf.format(c.get_center().get_x()),
                nf.format(c.get_center().get_y()),
                nf.format(c.get_radius())
            );
        }
        shape.to_string()
    }

    /// `makeNumberFormat(int fractionDigits)`: `NumberFormat.getInstance(
    /// Locale.ROOT)` without grouping and with at most `fractionDigits`
    /// fraction digits.
    pub fn make_number_format(fraction_digits: usize) -> DecimalFormat {
        make_number_format(fraction_digits)
    }
}

impl ShapeIO for LegacyShapeWriter {
    fn get_format_name(&self) -> &'static str {
        shape_io::LEGACY
    }
}

impl ShapeWriter for LegacyShapeWriter {
    fn write(&self, output: &mut dyn std::fmt::Write, shape: &dyn Shape) -> Result<()> {
        output
            .write_str(&Self::write_shape(shape))
            .map_err(io_error)
    }

    /// Equivalent to `LegacyShapeWriter.toString(Shape)`: `writeShape(shape)`.
    fn to_string_of(&self, shape: &dyn Shape) -> String {
        Self::write_shape(shape)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
