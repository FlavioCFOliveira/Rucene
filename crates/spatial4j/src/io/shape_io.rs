/*******************************************************************************
 * Copyright (c) 2015 VoyagerSearch
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `ShapeIO.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

/// Equivalent to `org.locationtech.spatial4j.io.ShapeIO` (`public
/// interface`): a named shape format.
pub trait ShapeIO {
    /// `getFormatName()`.
    fn get_format_name(&self) -> &'static str;
}

/// `ShapeIO.WKT`.
pub const WKT: &str = "WKT";
/// `ShapeIO.GeoJSON`.
pub const GEO_JSON: &str = "GeoJSON";
/// `ShapeIO.POLY`.
pub const POLY: &str = "POLY";
/// `ShapeIO.LEGACY`.
pub const LEGACY: &str = "LEGACY";
