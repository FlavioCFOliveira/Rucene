/*******************************************************************************
 * Copyright (c) 2015 VoyagerSearch
 * All rights reserved. This program and the accompanying materials
 * are made available under the terms of the Apache License, Version 2.0 which
 * accompanies this distribution and is available at
 *    http://www.apache.org/licenses/LICENSE-2.0.txt
 ******************************************************************************/

//! The types declared in `GeoJSONWriter.java`, of the Java package
//! `org.locationtech.spatial4j.io`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::Result;
use rucene::util::java_fmt::DecimalFormat;

use crate::context::spatial_context::SpatialContext;
use crate::context::spatial_context_factory::SpatialContextFactoryOps;
use crate::distance::distance_utils::DistanceUtils;
use crate::io::legacy_shape_writer::LegacyShapeWriter;
use crate::io::shape_io::{self, ShapeIO};
use crate::io::shape_writer::{io_error, ShapeWriter};
use crate::shape::r#impl::buffered_line::BufferedLine;
use crate::shape::r#impl::buffered_line_string::BufferedLineString;
use crate::shape::r#impl::geo_circle::GeoCircle;
use crate::shape::shape::Shape;

/// `GeoJSONReader.BUFFER`: the GeoJSON property of a buffer distance.
const BUFFER: &str = "buffer";
/// `GeoJSONReader.BUFFER_UNITS`.
const BUFFER_UNITS: &str = "buffer_units";

/// Equivalent to `org.locationtech.spatial4j.io.GeoJSONWriter` (`public
/// class GeoJSONWriter implements ShapeWriter`): writes shapes as GeoJSON.
#[derive(Debug)]
pub struct GeoJSONWriter;

impl GeoJSONWriter {
    /// `GeoJSONWriter(SpatialContext ctx, SpatialContextFactory factory)`.
    pub fn new(_ctx: &Arc<SpatialContext>, _factory: &dyn SpatialContextFactoryOps) -> Self {
        Self
    }

    /// `write(Writer output, NumberFormat nf, double... coords)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the output.
    pub fn write_coords(
        &self,
        output: &mut dyn std::fmt::Write,
        nf: &DecimalFormat,
        coords: &[f64],
    ) -> Result<()> {
        output.write_char('[').map_err(io_error)?;
        for (i, c) in coords.iter().enumerate() {
            if c.is_nan() {
                break; // empty point or no more coordinates
            }
            if i > 0 {
                output.write_char(',').map_err(io_error)?;
            }
            output.write_str(&nf.format(*c)).map_err(io_error)?;
        }
        output.write_char(']').map_err(io_error)
    }

    /// `writeDistance(Writer output, NumberFormat nf, double dist, boolean
    /// isGeo, String distProperty, String distUnitsProperty)`.
    ///
    /// # Errors
    ///
    /// `IOException` from the output.
    pub fn write_distance(
        &self,
        output: &mut dyn std::fmt::Write,
        nf: &DecimalFormat,
        dist: f64,
        is_geo: bool,
        dist_property: &str,
        dist_units_property: &str,
    ) -> Result<()> {
        write!(output, ",\"{dist_property}\":").map_err(io_error)?;
        if is_geo {
            let dist_km = DistanceUtils::degrees2_dist(dist, DistanceUtils::EARTH_MEAN_RADIUS_KM);
            output.write_str(&nf.format(dist_km)).map_err(io_error)?;
            output.write_str(",\"properties\":{").map_err(io_error)?;
            write!(output, "\"{dist_units_property}\":\"km\"}}").map_err(io_error)?;
        } else {
            output.write_str(&nf.format(dist)).map_err(io_error)?;
        }
        Ok(())
    }
}

impl ShapeIO for GeoJSONWriter {
    fn get_format_name(&self) -> &'static str {
        shape_io::GEO_JSON
    }
}

impl ShapeWriter for GeoJSONWriter {
    fn write(&self, output: &mut dyn std::fmt::Write, shape: &dyn Shape) -> Result<()> {
        let nf = LegacyShapeWriter::make_number_format(6);
        if let Some(v) = shape.as_point() {
            output
                .write_str("{\"type\":\"Point\",\"coordinates\":")
                .map_err(io_error)?;
            self.write_coords(output, &nf, &[v.get_x(), v.get_y()])?;
            output.write_char('}').map_err(io_error)?;
            return Ok(());
        }
        if let Some(v) = shape.as_rectangle() {
            output
                .write_str("{\"type\":\"Polygon\",\"coordinates\":[[")
                .map_err(io_error)?;
            self.write_coords(output, &nf, &[v.get_min_x(), v.get_min_y()])?;
            output.write_char(',').map_err(io_error)?;
            self.write_coords(output, &nf, &[v.get_min_x(), v.get_max_y()])?;
            output.write_char(',').map_err(io_error)?;
            self.write_coords(output, &nf, &[v.get_max_x(), v.get_max_y()])?;
            output.write_char(',').map_err(io_error)?;
            self.write_coords(output, &nf, &[v.get_max_x(), v.get_min_y()])?;
            output.write_char(',').map_err(io_error)?;
            self.write_coords(output, &nf, &[v.get_min_x(), v.get_min_y()])?;
            output.write_str("]]}").map_err(io_error)?;
            return Ok(());
        }
        if let Some(v) = shape.as_any().downcast_ref::<BufferedLine>() {
            output
                .write_str("{\"type\":\"LineString\",\"coordinates\":[")
                .map_err(io_error)?;
            self.write_coords(output, &nf, &[v.get_a().get_x(), v.get_a().get_y()])?;
            output.write_char(',').map_err(io_error)?;
            self.write_coords(output, &nf, &[v.get_b().get_x(), v.get_b().get_y()])?;
            output.write_char(',').map_err(io_error)?;
            output.write_str("]").map_err(io_error)?;
            if v.get_buf() > 0.0 {
                output.write_char(',').map_err(io_error)?;
                output.write_str("\"buffer\":").map_err(io_error)?;
                output
                    .write_str(&nf.format(v.get_buf()))
                    .map_err(io_error)?;
            }
            output.write_char('}').map_err(io_error)?;
            return Ok(());
        }
        if let Some(v) = shape.as_any().downcast_ref::<BufferedLineString>() {
            output
                .write_str("{\"type\":\"LineString\",\"coordinates\":[")
                .map_err(io_error)?;
            let mut last: Option<&Arc<BufferedLine>> = None;
            for seg in v.get_segment_lines() {
                if last.is_some() {
                    output.write_char(',').map_err(io_error)?;
                }
                self.write_coords(output, &nf, &[seg.get_a().get_x(), seg.get_a().get_y()])?;
                last = Some(seg);
            }
            if let Some(last) = last {
                output.write_char(',').map_err(io_error)?;
                self.write_coords(output, &nf, &[last.get_b().get_x(), last.get_b().get_y()])?;
            }
            output.write_str("]").map_err(io_error)?;
            if v.get_buf() > 0.0 {
                let is_geo = shape.get_context().is_some_and(|c| c.is_geo());
                self.write_distance(output, &nf, v.get_buf(), is_geo, BUFFER, BUFFER_UNITS)?;
            }
            output.write_char('}').map_err(io_error)?;
            return Ok(());
        }
        if let Some(v) = shape.as_circle() {
            let center = v.get_center();
            output
                .write_str("{\"type\":\"Circle\",\"coordinates\":")
                .map_err(io_error)?;
            self.write_coords(output, &nf, &[center.get_x(), center.get_y()])?;
            let is_geo_circle = shape.as_any().is::<GeoCircle>();
            self.write_distance(
                output,
                &nf,
                v.get_radius(),
                is_geo_circle,
                "radius",
                "radius_units",
            )?;
            output.write_str("}").map_err(io_error)?;
            return Ok(());
        }
        if let Some(v) = shape.as_shape_collection() {
            output
                .write_str("{\"type\":\"GeometryCollection\",\"geometries\":[")
                .map_err(io_error)?;
            for i in 0..v.size() {
                if i > 0 {
                    output.write_char(',').map_err(io_error)?;
                }
                self.write(output, &**v.get(i))?;
            }
            output.write_str("]}").map_err(io_error)?;
            return Ok(());
        }
        output
            .write_str("{\"type\":\"Unknown\",\"wkt\":\"")
            .map_err(io_error)?;
        output
            .write_str(&LegacyShapeWriter::write_shape(shape))
            .map_err(io_error)?;
        output.write_str("\"}").map_err(io_error)?;
        Ok(())
    }

    /// `toString(Shape)`: Java wraps an `IOException` of the in-memory writer
    /// in a `RuntimeException`, which cannot happen.
    fn to_string_of(&self, shape: &dyn Shape) -> String {
        let mut buffer = String::new();
        self.write(&mut buffer, shape)
            .unwrap_or_else(|e| panic!("java.lang.RuntimeException: {e}"));
        buffer
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
