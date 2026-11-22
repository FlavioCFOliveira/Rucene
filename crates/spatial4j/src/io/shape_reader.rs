/*******************************************************************************
 * Copyright (c) 2015 VoyagerSearch
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `ShapeReader.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;

use crate::io::shape_io::ShapeIO;
use crate::shape::shape::Shape;

/// Equivalent to `org.locationtech.spatial4j.io.ShapeReader` (`public
/// interface ShapeReader extends ShapeIO`): reads shapes of one format.
///
/// Java's `read(Object value)` and `readIfSupported(Object value)` read
/// `value.toString()`; they take that string here.
pub trait ShapeReader: ShapeIO + Send + Sync + Any {
    /// `read(Object value)`; `None` stands for a `null` result, which only
    /// the polyshape reader returns (for an empty value).
    ///
    /// # Errors
    ///
    /// `IOException`, `ParseException` or `InvalidShapeException`.
    fn read(&self, value: &str) -> Result<Option<Arc<dyn Shape>>>;

    /// `readIfSupported(Object value)`: `None` (Java `null`) when the value
    /// is not in this format.
    ///
    /// # Errors
    ///
    /// `InvalidShapeException`.
    fn read_if_supported(&self, value: &str) -> Result<Option<Arc<dyn Shape>>>;

    /// `read(Reader reader)`; `None` as for [`read`](Self::read).
    ///
    /// # Errors
    ///
    /// `IOException`, `ParseException` or `InvalidShapeException`.
    fn read_reader(&self, reader: &mut dyn std::io::Read) -> Result<Option<Arc<dyn Shape>>>;

    /// This reader as `Any`, for casts to its class.
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

/// Reads all of `reader` as UTF-8 text, as a `java.io.Reader` over it would.
pub(crate) fn read_all(reader: &mut dyn std::io::Read) -> Result<String> {
    let mut s = String::new();
    reader.read_to_string(&mut s)?;
    Ok(s)
}
