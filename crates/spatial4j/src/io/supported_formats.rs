/*******************************************************************************
 * Copyright (c) 2015 VoyagerSearch
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `SupportedFormats.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::sync::Arc;

use rucene::error::Result;

use crate::io::shape_io;
use crate::io::shape_reader::ShapeReader;
use crate::io::shape_writer::ShapeWriter;
use crate::shape::shape::Shape;

/// Equivalent to `org.locationtech.spatial4j.io.SupportedFormats` (`public
/// class`): the readers and writers a context supports.
#[derive(Clone)]
pub struct SupportedFormats {
    readers: Vec<Arc<dyn ShapeReader>>,
    writers: Vec<Arc<dyn ShapeWriter>>,
    wkt_reader: Option<Arc<dyn ShapeReader>>,
    wkt_writer: Option<Arc<dyn ShapeWriter>>,
    geo_json_reader: Option<Arc<dyn ShapeReader>>,
    geo_json_writer: Option<Arc<dyn ShapeWriter>>,
}

impl std::fmt::Debug for SupportedFormats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupportedFormats")
            .field("readers", &self.readers.iter().map(|r| r.get_format_name()).collect::<Vec<_>>())
            .field("writers", &self.writers.iter().map(|w| w.get_format_name()).collect::<Vec<_>>())
            .finish()
    }
}

impl SupportedFormats {
    /// `SupportedFormats(List<ShapeReader> readers, List<ShapeWriter>
    /// writers)`.
    pub fn new(readers: Vec<Arc<dyn ShapeReader>>, writers: Vec<Arc<dyn ShapeWriter>>) -> Self {
        let mut formats = Self {
            readers,
            writers,
            wkt_reader: None,
            wkt_writer: None,
            geo_json_reader: None,
            geo_json_writer: None,
        };
        formats.wkt_reader = formats.get_reader(shape_io::WKT);
        formats.wkt_writer = formats.get_writer(shape_io::WKT);
        formats.geo_json_reader = formats.get_reader(shape_io::GEO_JSON);
        formats.geo_json_writer = formats.get_writer(shape_io::GEO_JSON);
        formats
    }

    /// `getReaders()`.
    pub fn get_readers(&self) -> &[Arc<dyn ShapeReader>] {
        &self.readers
    }

    /// `getWriters()`.
    pub fn get_writers(&self) -> &[Arc<dyn ShapeWriter>] {
        &self.writers
    }

    /// `getReader(String fmt)`: the first reader of the format.
    pub fn get_reader(&self, fmt: &str) -> Option<Arc<dyn ShapeReader>> {
        self.readers.iter().find(|f| fmt == f.get_format_name()).cloned()
    }

    /// `getWriter(String fmt)`: the first writer of the format.
    pub fn get_writer(&self, fmt: &str) -> Option<Arc<dyn ShapeWriter>> {
        self.writers.iter().find(|f| fmt == f.get_format_name()).cloned()
    }

    /// `getWktReader()`.
    pub fn get_wkt_reader(&self) -> Option<Arc<dyn ShapeReader>> {
        self.wkt_reader.clone()
    }

    /// `getWktWriter()`.
    pub fn get_wkt_writer(&self) -> Option<Arc<dyn ShapeWriter>> {
        self.wkt_writer.clone()
    }

    /// `getGeoJsonReader()`.
    pub fn get_geo_json_reader(&self) -> Option<Arc<dyn ShapeReader>> {
        self.geo_json_reader.clone()
    }

    /// `getGeoJsonWriter()`.
    pub fn get_geo_json_writer(&self) -> Option<Arc<dyn ShapeWriter>> {
        self.geo_json_writer.clone()
    }

    /// `read(String value)`: the shape of the first reader that supports
    /// the value; `None` (Java `null`) when none does.
    ///
    /// # Errors
    ///
    /// `InvalidShapeException` of a reader.
    pub fn read(&self, value: &str) -> Result<Option<Arc<dyn Shape>>> {
        for format in &self.readers {
            if let Some(v) = format.read_if_supported(value)? {
                return Ok(Some(v));
            }
        }
        Ok(None)
    }
}
