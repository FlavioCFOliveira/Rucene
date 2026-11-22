/*******************************************************************************
 * Copyright (c) 2015 Voyager Search and MITRE
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `LegacyShapeReader.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;
use std::sync::{Arc, Weak};

use rucene::error::Result;
use rucene::util::extra::ParseException;

use crate::context::spatial_context::SpatialContext;
use crate::context::spatial_context_factory::SpatialContextFactoryOps;
use crate::exception::invalid_shape_exception::InvalidShapeException;
use crate::io::parse_utils::ParseUtils;
use crate::io::shape_io::{self, ShapeIO};
use crate::io::shape_reader::{read_all, ShapeReader};
use rucene_analysis_common::jdk::character;

use crate::jdk::parse_double;
use crate::shape::point::Point;
use crate::shape::shape::Shape;

/// `java.util.StringTokenizer(str, " ")`: the tokens between spaces.
struct StringTokenizer<'a> {
    tokens: std::iter::Peekable<std::iter::Filter<std::str::Split<'a, char>, fn(&&str) -> bool>>,
}

impl<'a> StringTokenizer<'a> {
    fn new(s: &'a str) -> Self {
        let not_empty: fn(&&str) -> bool = |t| !t.is_empty();
        Self { tokens: s.split(' ').filter(not_empty).peekable() }
    }

    /// `nextToken()`: `NoSuchElementException` when there is none.
    fn next_token(&mut self) -> Result<&'a str> {
        self.tokens.next().ok_or_else(|| {
            crate::jdk::unchecked_with_cause("java.util.NoSuchElementException", None, None)
        })
    }

    fn has_more_tokens(&mut self) -> bool {
        self.tokens.peek().is_some()
    }
}

/// Equivalent to `org.locationtech.spatial4j.io.LegacyShapeReader`
/// (`@Deprecated public class LegacyShapeReader implements ShapeReader`):
/// reads the legacy spatial4j shape format (`x y`, `lat,lon`, `minX minY
/// maxX maxY`, `Circle(x y d=dist)`).
#[derive(Debug)]
pub struct LegacyShapeReader {
    ctx: Weak<SpatialContext>,
}

impl LegacyShapeReader {
    /// `LegacyShapeReader(SpatialContext ctx, SpatialContextFactory factory)`.
    pub fn new(ctx: &Arc<SpatialContext>, _factory: &dyn SpatialContextFactoryOps) -> Self {
        Self { ctx: Arc::downgrade(ctx) }
    }

    fn ctx(&self) -> Arc<SpatialContext> {
        self.ctx.upgrade().expect("the SpatialContext of a LegacyShapeReader is alive")
    }

    /// `readShapeOrNull(String str, SpatialContext ctx)`: `None` (Java
    /// `null`) when the value starts with a letter but is not a circle.
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` for an empty or malformed value;
    /// `NumberFormatException` for a value that is not a number.
    pub fn read_shape_or_null(str: &str, ctx: &SpatialContext) -> Result<Option<Arc<dyn Shape>>> {
        if str.is_empty() {
            return Err(InvalidShapeException::error(str));
        }
        // `str.charAt(0)`: the first UTF-16 unit.
        let first = str.encode_utf16().next().expect("not empty");
        if character::is_letter(i32::from(first)) {
            if str.starts_with("Circle(") || str.starts_with("CIRCLE(") {
                if let Some(idx) = str.rfind(')').filter(|&i| i > 0) {
                    let body = &str["Circle(".len()..idx];
                    let mut st = StringTokenizer::new(body);
                    let token = st.next_token()?;
                    let pt: Arc<dyn Point> = if token.contains(',') {
                        Self::read_lat_comma_lon_point(token, ctx)?
                    } else {
                        let x = parse_double(token)?;
                        let y = parse_double(st.next_token()?)?;
                        ctx.make_point(x, y)?
                    };
                    let arg = st.next_token()?;
                    let d = match arg.find('=').filter(|&i| i > 0) {
                        Some(idx) => {
                            let k = &arg[..idx];
                            if k == "d" || k == "distance" {
                                parse_double(&arg[idx + 1..])?
                            } else {
                                return Err(InvalidShapeException::error(format!("unknown arg: {k} :: {str}")));
                            }
                        }
                        None => parse_double(arg)?,
                    };
                    if st.has_more_tokens() {
                        return Err(InvalidShapeException::error(format!(
                            "Extra arguments: {} :: {str}",
                            st.next_token()?
                        )));
                    }
                    //NOTE: we are assuming the units of 'd' is the same as that of the spatial context.
                    let circle = ctx.make_circle_at(pt, d)?;
                    return Ok(Some(circle));
                }
            }
            return Ok(None); //caller has opportunity to try other parsing
        }

        if str.contains(',') {
            return Ok(Some(Self::read_lat_comma_lon_point(str, ctx)?));
        }
        let mut st = StringTokenizer::new(str);
        let p0 = parse_double(st.next_token()?)?;
        let p1 = parse_double(st.next_token()?)?;
        if st.has_more_tokens() {
            let p2 = parse_double(st.next_token()?)?;
            let p3 = parse_double(st.next_token()?)?;
            if st.has_more_tokens() {
                return Err(InvalidShapeException::error(format!(
                    "Only 4 numbers supported (rect) but found more: {str}"
                )));
            }
            return Ok(Some(ctx.make_rectangle(p0, p2, p1, p3)?));
        }
        Ok(Some(ctx.make_point(p0, p1)?))
    }

    fn read_lat_comma_lon_point(value: &str, ctx: &SpatialContext) -> Result<Arc<dyn Point>> {
        let lat_lon = ParseUtils::parse_latitude_longitude(value)?;
        ctx.make_point(lat_lon[1], lat_lon[0])
    }
}

impl ShapeIO for LegacyShapeReader {
    fn get_format_name(&self) -> &'static str {
        shape_io::LEGACY
    }
}

impl ShapeReader for LegacyShapeReader {
    fn read(&self, value: &str) -> Result<Option<Arc<dyn Shape>>> {
        let ctx = self.ctx();
        let shape = Self::read_shape_or_null(value, &ctx)?;
        if shape.is_none() {
            return Err(ParseException::error(format!("unable to read shape: {value}"), 0));
        }
        // Java reads the value a second time for the result.
        Self::read_shape_or_null(value, &ctx)
    }

    fn read_if_supported(&self, value: &str) -> Result<Option<Arc<dyn Shape>>> {
        Self::read_shape_or_null(value, &self.ctx())
    }

    fn read_reader(&self, reader: &mut dyn std::io::Read) -> Result<Option<Arc<dyn Shape>>> {
        let s = read_all(reader)?;
        self.read(&s)
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}
