/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *    http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! The types declared in `PolyshapeWriter.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::context::spatial_context::SpatialContext;
use crate::context::spatial_context_factory::SpatialContextFactoryOps;
use crate::io::legacy_shape_writer::LegacyShapeWriter;
use crate::io::shape_io::{self, ShapeIO};
use crate::io::shape_writer::{io_error, ShapeWriter};
use crate::jdk::java_round;
use crate::shape::r#impl::buffered_line::BufferedLine;
use crate::shape::r#impl::buffered_line_string::BufferedLineString;
use crate::shape::shape::Shape;

/// Equivalent to `org.locationtech.spatial4j.io.PolyshapeWriter` (`public
/// class PolyshapeWriter implements ShapeWriter`): writes shapes in the
/// "polyshape" format, an extension of Google's encoded polyline format.
#[derive(Debug)]
pub struct PolyshapeWriter;

impl PolyshapeWriter {
    /// `KEY_POINT`.
    pub const KEY_POINT: char = '0';
    /// `KEY_LINE`.
    pub const KEY_LINE: char = '1';
    /// `KEY_POLYGON`.
    pub const KEY_POLYGON: char = '2';
    /// `KEY_MULTIPOINT`.
    pub const KEY_MULTIPOINT: char = '3';
    /// `KEY_CIRCLE`.
    pub const KEY_CIRCLE: char = '4';
    /// `KEY_BOX`.
    pub const KEY_BOX: char = '5';
    /// `KEY_ARG_START`.
    pub const KEY_ARG_START: char = '(';
    /// `KEY_ARG_END`.
    pub const KEY_ARG_END: char = ')';
    /// `KEY_SEPERATOR`.
    pub const KEY_SEPERATOR: char = ' ';

    /// `PolyshapeWriter(SpatialContext ctx, SpatialContextFactory factory)`.
    pub fn new(_ctx: &Arc<SpatialContext>, _factory: &dyn SpatialContextFactoryOps) -> Self {
        Self
    }

    /// `write(Encoder enc, Shape shape)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the encoder's output.
    pub fn write_encoded(&self, enc: &mut Encoder<'_>, shape: &dyn Shape) -> Result<()> {
        if let Some(v) = shape.as_point() {
            enc.write_event(Self::KEY_POINT)?;
            enc.write(v.get_x(), v.get_y())?;
            return Ok(());
        }
        if let Some(v) = shape.as_rectangle() {
            enc.write_event(Self::KEY_BOX)?;
            enc.write(v.get_min_x(), v.get_min_y())?;
            enc.write(v.get_max_x(), v.get_max_y())?;
            return Ok(());
        }
        if let Some(v) = shape.as_any().downcast_ref::<BufferedLine>() {
            enc.write_event(Self::KEY_LINE)?;
            if v.get_buf() > 0.0 {
                enc.write_arg(v.get_buf())?;
            }
            enc.write(v.get_a().get_x(), v.get_a().get_y())?;
            enc.write(v.get_b().get_x(), v.get_b().get_y())?;
            return Ok(());
        }
        if let Some(v) = shape.as_any().downcast_ref::<BufferedLineString>() {
            enc.write_event(Self::KEY_LINE)?;
            if v.get_buf() > 0.0 {
                enc.write_arg(v.get_buf())?;
            }
            let mut last: Option<&Arc<BufferedLine>> = None;
            for seg in v.get_segment_lines() {
                enc.write(seg.get_a().get_x(), seg.get_a().get_y())?;
                last = Some(seg);
            }
            if let Some(last) = last {
                enc.write(last.get_b().get_x(), last.get_b().get_y())?;
            }
            return Ok(());
        }
        if let Some(v) = shape.as_circle() {
            // See: https://github.com/geoscript/geoscript-js/blob/master/src/main/resources/org/geoscript/js/lib/geoscript/geom/circle.js
            let center = v.get_center();
            let radius = v.get_radius();
            enc.write_event(Self::KEY_CIRCLE)?;
            enc.write_arg(radius)?;
            enc.write(center.get_x(), center.get_y())?;
            return Ok(());
        }
        if let Some(v) = shape.as_shape_collection() {
            let shapes = v.get_shapes();
            for (i, s) in shapes.iter().enumerate() {
                self.write_encoded(enc, &**s)?;
                if i + 1 < shapes.len() {
                    enc.seperator()?;
                }
            }
            return Ok(());
        }
        enc.writer
            .write_str(&format!(
                "{{unkwnwon {}}}",
                LegacyShapeWriter::write_shape(shape)
            ))
            .map_err(io_error)
    }
}

impl ShapeIO for PolyshapeWriter {
    fn get_format_name(&self) -> &'static str {
        shape_io::POLY
    }
}

impl ShapeWriter for PolyshapeWriter {
    fn write(&self, output: &mut dyn std::fmt::Write, shape: &dyn Shape) -> Result<()> {
        self.write_encoded(&mut Encoder::new(output), shape)
    }

    /// `toString(Shape)`: Java wraps an `IOException` of the in-memory writer
    /// in a `RuntimeException`, which cannot happen.
    fn to_string_of(&self, shape: &dyn Shape) -> String {
        let mut buffer = String::new();
        ShapeWriter::write(self, &mut buffer, shape)
            .unwrap_or_else(|e| panic!("java.lang.RuntimeException: {e}"));
        buffer
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Equivalent to `org.locationtech.spatial4j.io.PolyshapeWriter.Encoder`
/// (`public static class`): the delta encoder of coordinates.
pub struct Encoder<'a> {
    last_lat: i64,
    last_lng: i64,
    writer: &'a mut dyn std::fmt::Write,
}

impl<'a> Encoder<'a> {
    /// `Encoder(Writer writer)`.
    pub fn new(writer: &'a mut dyn std::fmt::Write) -> Self {
        Self {
            last_lat: 0,
            last_lng: 0,
            writer,
        }
    }

    /// `seperator()`.
    ///
    /// # Errors
    ///
    /// `IOException` from the output.
    pub fn seperator(&mut self) -> Result<()> {
        self.writer
            .write_char(PolyshapeWriter::KEY_SEPERATOR)
            .map_err(io_error)?;
        self.last_lat = 0;
        self.last_lng = 0;
        Ok(())
    }

    /// `startRing()`.
    ///
    /// # Errors
    ///
    /// `IOException` from the output.
    pub fn start_ring(&mut self) -> Result<()> {
        self.writer
            .write_char(PolyshapeWriter::KEY_ARG_START)
            .map_err(io_error)?;
        self.last_lat = 0;
        self.last_lng = 0;
        Ok(())
    }

    /// `write(char event)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the output.
    pub fn write_event(&mut self, event: char) -> Result<()> {
        self.writer.write_char(event).map_err(io_error)?;
        self.last_lat = 0;
        self.last_lng = 0;
        Ok(())
    }

    /// `writeArg(double value)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the output.
    pub fn write_arg(&mut self, value: f64) -> Result<()> {
        self.writer
            .write_char(PolyshapeWriter::KEY_ARG_START)
            .map_err(io_error)?;
        self.encode(java_round(value * 1e5))?;
        self.writer
            .write_char(PolyshapeWriter::KEY_ARG_END)
            .map_err(io_error)
    }

    /// `write(double latitude, double longitude)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the output.
    pub fn write(&mut self, latitude: f64, longitude: f64) -> Result<()> {
        let lat = java_round(latitude * 1e5);
        let lng = java_round(longitude * 1e5);
        let d_lat = lat.wrapping_sub(self.last_lat);
        let d_lng = lng.wrapping_sub(self.last_lng);
        self.encode(d_lat)?;
        self.encode(d_lng)?;
        self.last_lat = lat;
        self.last_lng = lng;
        Ok(())
    }

    fn encode(&mut self, v: i64) -> Result<()> {
        let mut v = if v < 0 {
            !(v.wrapping_shl(1))
        } else {
            v.wrapping_shl(1)
        };
        while v >= 0x20 {
            self.write_code_point(((0x20 | (v & 0x1f)) + 63) as i32)?;
            v >>= 5;
        }
        self.write_code_point((v + 63) as i32)
    }

    /// `writer.write(Character.toChars(cp))`.
    fn write_code_point(&mut self, cp: i32) -> Result<()> {
        let c = u32::try_from(cp)
            .ok()
            .and_then(char::from_u32)
            .ok_or_else(|| {
                rucene::error::LuceneError::illegal_argument(format!(
                    "Not a valid Unicode code point: 0x{cp:X}"
                ))
            })?;
        self.writer.write_char(c).map_err(io_error)
    }
}
