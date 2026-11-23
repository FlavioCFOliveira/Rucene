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

//! The types declared in `PolyshapeReader.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::util::extra::ParseException;

use crate::context::spatial_context::SpatialContext;
use crate::context::spatial_context_factory::SpatialContextFactoryOps;
use crate::io::polyshape_writer::PolyshapeWriter;
use crate::io::shape_io::{self, ShapeIO};
use crate::io::shape_reader::{read_all, ShapeReader};
use crate::shape::shape::Shape;
use crate::shape::shape_factory::{PointsBuilder, ShapeFactory};

/// Equivalent to `org.locationtech.spatial4j.io.PolyshapeReader` (`public
/// class PolyshapeReader implements ShapeReader`): reads the format
/// [`PolyshapeWriter`] writes.
#[derive(Debug)]
pub struct PolyshapeReader {
    ctx: Weak<SpatialContext>,
}

impl PolyshapeReader {
    /// `PolyshapeReader(SpatialContext ctx, SpatialContextFactory factory)`.
    pub fn new(ctx: &Arc<SpatialContext>, _factory: &dyn SpatialContextFactoryOps) -> Self {
        Self {
            ctx: Arc::downgrade(ctx),
        }
    }

    fn ctx(&self) -> Arc<SpatialContext> {
        self.ctx
            .upgrade()
            .expect("the SpatialContext of a PolyshapeReader is alive")
    }

    /// `shpFactory`: `ctx.getShapeFactory()`.
    fn shp_factory(&self) -> Arc<dyn ShapeFactory> {
        self.ctx().get_shape_factory()
    }

    /// `read(Reader r)` (final in Java), over the UTF-16 units of `text`.
    ///
    /// # Errors
    ///
    /// `ParseException` for malformed input; `IllegalArgumentException` for
    /// a circle without a radius; `InvalidShapeException` of the factory.
    pub fn read_text(&self, text: &str) -> Result<Option<Arc<dyn Shape>>> {
        let shp_factory = self.shp_factory();
        let mut reader = XReader::new(text, Arc::clone(&shp_factory));
        let mut last_shape: Option<Arc<dyn Shape>> = None;
        let mut shapes: Option<Vec<Arc<dyn Shape>>> = None;
        while !reader.is_done() {
            let event = reader.read_key();
            if !('0'..='9').contains(&event) {
                if event == PolyshapeWriter::KEY_SEPERATOR {
                    continue; // read the next key
                }
                return Err(ParseException::error(
                    format!("expecting a shape key.  not '{event}'"),
                    -1,
                ));
            }

            if let Some(last) = last_shape.take() {
                shapes.get_or_insert_with(Vec::new).push(last);
            }
            let mut arg: Option<f64> = None;
            if reader.peek() == PolyshapeWriter::KEY_ARG_START as i32 {
                reader.read_key(); // skip the key
                arg = Some(reader.read_double());
                if reader.read_key() != PolyshapeWriter::KEY_ARG_END {
                    return Err(ParseException::error("expecting an argument end", -1));
                }
            }
            if reader.is_event() {
                return Err(ParseException::error(
                    "Invalid input. Event should be followed by data",
                    -1,
                ));
            }

            last_shape = Some(match event {
                PolyshapeWriter::KEY_POINT => {
                    let x = shp_factory.norm_x(reader.read_lat());
                    let y = shp_factory.norm_y(reader.read_lng());
                    shp_factory.point_xy(x, y)?
                }
                PolyshapeWriter::KEY_LINE => {
                    let mut line_builder = shp_factory.line_string();
                    reader.read_points(&mut *line_builder)?;
                    if let Some(arg) = arg {
                        line_builder.buffer(shp_factory.norm_dist(arg));
                    }
                    line_builder.build()?
                }
                PolyshapeWriter::KEY_BOX => {
                    let lat1 = shp_factory.norm_x(reader.read_lat());
                    let lon1 = shp_factory.norm_y(reader.read_lng());
                    let lat2 = shp_factory.norm_x(reader.read_lat());
                    let lon2 = shp_factory.norm_y(reader.read_lng());
                    shp_factory.rect(lat1, lat2, lon1, lon2)?
                }
                PolyshapeWriter::KEY_MULTIPOINT => {
                    let mut builder = shp_factory.multi_point();
                    reader.read_points(&mut *builder)?;
                    builder.build()?
                }
                PolyshapeWriter::KEY_CIRCLE => {
                    let Some(arg) = arg else {
                        return Err(LuceneError::illegal_argument(
                            "the input should have a radius argument",
                        ));
                    };
                    let x = shp_factory.norm_x(reader.read_lat());
                    let y = shp_factory.norm_y(reader.read_lng());
                    shp_factory.circle(x, y, shp_factory.norm_dist(arg))?
                }
                PolyshapeWriter::KEY_POLYGON => self.read_polygon(&mut reader)?,
                _ => return Err(ParseException::error(format!("unhandled key: {event}"), -1)),
            });
        }

        if let Some(mut shapes) = shapes {
            if let Some(last) = last_shape {
                shapes.push(last);
            }
            let mut multi_builder = shp_factory.multi_shape();
            for shp in shapes {
                multi_builder.add(shp);
            }
            return Ok(Some(multi_builder.build()?));
        }
        Ok(last_shape)
    }

    /// `readPolygon(XReader reader)`.
    ///
    /// # Errors
    ///
    /// `UnsupportedOperationException` of a factory without polygons; the
    /// errors of the factory.
    pub fn read_polygon(&self, reader: &mut XReader) -> Result<Arc<dyn Shape>> {
        let mut poly_builder = self.shp_factory().polygon()?;
        reader.read_points(&mut *poly_builder)?;
        if !reader.is_done() && reader.peek() == PolyshapeWriter::KEY_ARG_START as i32 {
            while reader.is_event() && reader.peek() == PolyshapeWriter::KEY_ARG_START as i32 {
                reader.read_key(); // eat the event;
                let mut hole = poly_builder.hole();
                reader.read_points(&mut *hole)?;
                hole.end_hole()?;
            }
        }
        poly_builder.build()
    }
}

impl ShapeIO for PolyshapeReader {
    fn get_format_name(&self) -> &'static str {
        shape_io::POLY
    }
}

impl ShapeReader for PolyshapeReader {
    fn read(&self, value: &str) -> Result<Option<Arc<dyn Shape>>> {
        let text = value.trim_matches(|c: char| c <= ' ');
        self.read_text(text)
    }

    fn read_if_supported(&self, value: &str) -> Result<Option<Arc<dyn Shape>>> {
        let v = value.trim_matches(|c: char| c <= ' ');
        let Some(first) = v.chars().next() else {
            return Err(LuceneError::from(LuceneErrorKind::StringIndexOutOfBounds(
                "Index 0 out of bounds for length 0".to_string(),
            )));
        };
        if first.is_ascii_digit() {
            match self.read_text(v) {
                Ok(shape) => return Ok(shape),
                Err(e) if ParseException::of(&e).is_some() || e.is_io_exception() => {
                    // Java catches ParseException and IOException here.
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    fn read_reader(&self, reader: &mut dyn std::io::Read) -> Result<Option<Arc<dyn Shape>>> {
        let s = read_all(reader)?;
        self.read_text(&s)
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Equivalent to `org.locationtech.spatial4j.io.PolyshapeReader.XReader`
/// (`public static class`): reads keys and delta-encoded coordinates from
/// a `java.io.Reader`, here the UTF-16 units of a string.
pub struct XReader {
    lat: i32,
    lng: i32,
    head: i32,
    input: Vec<u16>,
    pos: usize,
    shp_factory: Arc<dyn ShapeFactory>,
}

impl XReader {
    /// `XReader(Reader input, ShapeFactory shpFactory)`.
    pub fn new(input: &str, shp_factory: Arc<dyn ShapeFactory>) -> Self {
        let mut reader = Self {
            lat: 0,
            lng: 0,
            head: -1,
            input: input.encode_utf16().collect(),
            pos: 0,
            shp_factory,
        };
        reader.head = reader.read();
        reader
    }

    /// `input.read()`: the next UTF-16 unit, or -1 at the end.
    fn read(&mut self) -> i32 {
        match self.input.get(self.pos) {
            Some(&u) => {
                self.pos += 1;
                i32::from(u)
            }
            None => -1,
        }
    }

    /// `readPoints(T builder)`.
    ///
    /// # Errors
    ///
    /// The `InvalidShapeException` of the builder.
    pub fn read_points(&mut self, builder: &mut dyn PointsBuilder) -> Result<()> {
        while self.is_data() {
            let lat = self.read_lat();
            let x = self.shp_factory.norm_x(lat);
            let lng = self.read_lng();
            let y = self.shp_factory.norm_y(lng);
            builder.point_xy(x, y)?;
        }
        Ok(())
    }

    /// `readLat()`.
    pub fn read_lat(&mut self) -> f64 {
        self.lat = self.lat.wrapping_add(self.read_int());
        f64::from(self.lat) * 1e-5
    }

    /// `readLng()`.
    pub fn read_lng(&mut self) -> f64 {
        self.lng = self.lng.wrapping_add(self.read_int());
        f64::from(self.lng) * 1e-5
    }

    /// `readDouble()`.
    pub fn read_double(&mut self) -> f64 {
        f64::from(self.read_int()) * 1e-5
    }

    /// `peek()`.
    pub fn peek(&self) -> i32 {
        self.head
    }

    /// `readKey()`: `(char) head`, `'\u{FFFF}'` at the end.
    pub fn read_key(&mut self) -> char {
        self.lat = 0;
        self.lng = 0; // reset the offset
        let key = char::from_u32(self.head as u16 as u32).unwrap_or('\u{FFFD}');
        self.head = self.read();
        key
    }

    /// `isData()`.
    pub fn is_data(&self) -> bool {
        self.head >= i32::from(b'?')
    }

    /// `isDone()`.
    pub fn is_done(&self) -> bool {
        self.head < 0
    }

    /// `isEvent()`.
    pub fn is_event(&self) -> bool {
        self.head > 0 && self.head < i32::from(b'?')
    }

    /// `readInt()` (package-private).
    pub(crate) fn read_int(&mut self) -> i32 {
        let mut b;
        let mut result: i32 = 1;
        let mut shift: u32 = 0;
        loop {
            b = self.head - 63 - 1;
            result = result.wrapping_add(b.wrapping_shl(shift));
            shift += 5;
            self.head = self.read();
            if b < 0x1f {
                break;
            }
        }
        if (result & 1) != 0 {
            !(result >> 1)
        } else {
            result >> 1
        }
    }
}
