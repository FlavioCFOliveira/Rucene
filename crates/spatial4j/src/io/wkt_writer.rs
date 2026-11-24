/*******************************************************************************
 * Copyright (c) 2015 VoyagerSearch
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `WKTWriter.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;

use rucene::error::Result;
use rucene::util::java_fmt::{DecimalFormat, RoundingMode};

use crate::io::legacy_shape_writer::LegacyShapeWriter;
use crate::io::shape_io::{self, ShapeIO};
use crate::io::shape_writer::{io_error, ShapeWriter};
use crate::shape::point::Point;
use crate::shape::r#impl::buffered_line_string::BufferedLineString;
use crate::shape::shape::Shape;

/// Equivalent to `org.locationtech.spatial4j.io.WKTWriter` (`public class
/// WKTWriter implements ShapeWriter`): writes shapes as WKT.
#[derive(Debug, Default)]
pub struct WKTWriter;

impl WKTWriter {
    /// `WKTWriter()`.
    pub fn new() -> Self {
        Self
    }

    /// `append(StringBuilder buffer, Point p, NumberFormat nf)`.
    pub fn append<'a>(
        &self,
        buffer: &'a mut String,
        p: &dyn Point,
        nf: &DecimalFormat,
    ) -> &'a mut String {
        buffer.push_str(&nf.format(p.get_x()));
        buffer.push(' ');
        buffer.push_str(&nf.format(p.get_y()));
        buffer
    }

    /// `getNumberFormat()`.
    pub fn get_number_format(&self) -> DecimalFormat {
        LegacyShapeWriter::make_number_format(6)
    }
}

impl ShapeIO for WKTWriter {
    fn get_format_name(&self) -> &'static str {
        shape_io::WKT
    }
}

impl ShapeWriter for WKTWriter {
    /// Equivalent to `WKTWriter.toString(Shape)`.
    fn to_string_of(&self, shape: &dyn Shape) -> String {
        let nf = self.get_number_format();
        if let Some(point) = shape.as_point() {
            if point.is_empty() {
                return "POINT EMPTY".to_string();
            }
            let mut buffer = String::from("POINT (");
            self.append(&mut buffer, point, &nf).push(')');
            return buffer;
        }
        if let Some(rect) = shape.as_rectangle() {
            let mut nf_min = nf;
            let mut nf_max = LegacyShapeWriter::make_number_format(6);

            nf_min.set_rounding_mode(RoundingMode::FLOOR);
            nf_max.set_rounding_mode(RoundingMode::CEILING);

            return format!(
                "ENVELOPE ({}, {}, {}, {})",
                // '(' x1 ',' x2 ',' y2 ',' y1 ')'
                nf_min.format(rect.get_min_x()),
                nf_max.format(rect.get_max_x()),
                nf_max.format(rect.get_max_y()),
                nf_min.format(rect.get_min_y())
            );
        }
        if let Some(c) = shape.as_circle() {
            return format!(
                "BUFFER (POINT ({} {}), {})",
                nf.format(c.get_center().get_x()),
                nf.format(c.get_center().get_y()),
                nf.format(c.get_radius())
            );
        }
        if let Some(line) = shape.as_any().downcast_ref::<BufferedLineString>() {
            let mut str = String::new();
            let buf = line.get_buf();
            if buf > 0.0 {
                str.push_str("BUFFER (");
            }
            str.push_str("LINESTRING (");
            let segs = line.get_segment_lines();
            for (i, seg) in segs.iter().enumerate() {
                self.append(&mut str, &**seg.get_a(), &nf).push_str(", ");
                if i + 1 == segs.len() {
                    self.append(&mut str, &**seg.get_b(), &nf);
                }
            }
            str.push(')');
            if buf > 0.0 {
                str.push_str(", ");
                str.push_str(&nf.format(buf));
                str.push(')');
            }
            return str;
        }
        if let Some(collection) = shape.as_shape_collection() {
            if collection.is_empty_list() {
                return "GEOMETRYCOLLECTION EMPTY".to_string();
            }
            let mut buffer = String::from("GEOMETRYCOLLECTION (");
            for (i, sub) in collection.get_shapes().iter().enumerate() {
                if i > 0 {
                    buffer.push(',');
                }
                buffer.push_str(&self.to_string_of(&**sub));
            }
            buffer.push(')');
            return buffer;
        }
        LegacyShapeWriter::write_shape_with(shape, &nf)
    }

    fn write(&self, output: &mut dyn std::fmt::Write, shape: &dyn Shape) -> Result<()> {
        output
            .write_str(&self.to_string_of(shape))
            .map_err(io_error)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
