/*******************************************************************************
 * Copyright (c) 2015 VoyagerSearch
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `ShapeWriter.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;

use rucene::error::Result;

use crate::io::shape_io::ShapeIO;
use crate::shape::shape::Shape;

/// Equivalent to `org.locationtech.spatial4j.io.ShapeWriter` (`public
/// interface ShapeWriter extends ShapeIO`): writes shapes in one format.
pub trait ShapeWriter: ShapeIO + Send + Sync + Any {
    /// `write(Writer output, Shape shape)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the output.
    fn write(&self, output: &mut dyn std::fmt::Write, shape: &dyn Shape) -> Result<()>;

    /// `toString(Shape shape)`.
    fn to_string_of(&self, shape: &dyn Shape) -> String;

    /// This writer as `Any`, for casts to its class.
    fn as_any(&self) -> &dyn Any;
}

/// A `java.io.Writer` write that failed, as the `IOException` Java reports.
pub(crate) fn io_error(_e: std::fmt::Error) -> rucene::error::LuceneError {
    rucene::error::LuceneError::from(std::io::Error::other("write failed"))
}
