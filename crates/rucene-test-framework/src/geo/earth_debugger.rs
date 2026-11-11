/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.geo.EarthDebugger`: draws shapes on the
//! earth surface and renders using the very cool http://www.webglearth.org.
//!
//! Just instantiate this class, add the things you want plotted, and call
//! [`EarthDebugger::finish`] to get the resulting HTML that you should save
//! and load with a browser.

use std::fmt::Write;

use rucene::error::{LuceneError, Result};
use rucene::geo::{Polygon, Rectangle};
use rucene::util::java_fmt::double_to_string;
use rucene::util::sloppy_math::SloppyMath;

/// `StringBuilder.append(double)`.
fn d(v: f64) -> String {
    double_to_string(v)
}

/// `MAX_KM_PER_STEP`.
const MAX_KM_PER_STEP: f64 = 100.0;

/// Port of `EarthDebugger`.
pub struct EarthDebugger {
    b: String,
    next_shape: i32,
    finished: bool,
}

impl Default for EarthDebugger {
    fn default() -> Self {
        Self::new()
    }
}

impl EarthDebugger {
    /// The page header.
    fn header() -> String {
        let mut b = String::new();
        b.push_str("<!DOCTYPE HTML>\n");
        b.push_str("<html>\n");
        b.push_str("  <head>\n");
        b.push_str("    <script src=\"http://www.webglearth.com/v2/api.js\"></script>\n");
        b.push_str("    <script>\n");
        b.push_str("      function initialize() {\n");
        b
    }

    /// `EarthDebugger()`.
    pub fn new() -> Self {
        let mut b = Self::header();
        b.push_str("        var earth = new WE.map('earth_div');\n");
        EarthDebugger {
            b,
            next_shape: 0,
            finished: false,
        }
    }

    /// `EarthDebugger(double centerLat, double centerLon, double
    /// altitudeMeters)`.
    pub fn with_center(center_lat: f64, center_lon: f64, altitude_meters: f64) -> Self {
        let mut b = Self::header();
        let _ = writeln!(
            b,
            "        var earth = new WE.map('earth_div', {{center: [{}, {}], altitude: {}}});",
            d(center_lat),
            d(center_lon),
            d(altitude_meters)
        );
        EarthDebugger {
            b,
            next_shape: 0,
            finished: false,
        }
    }

    /// `addPolygon(Polygon poly)`.
    pub fn add_polygon(&mut self, poly: &Polygon) {
        self.add_polygon_with_color(poly, "#00ff00");
    }

    /// `addPolygon(Polygon poly, String color)`.
    pub fn add_polygon_with_color(&mut self, poly: &Polygon, color: &str) {
        let name = format!("poly{}", self.next_shape);
        self.next_shape += 1;

        let _ = writeln!(self.b, "        var {name} = WE.polygon([");
        let poly_lats = poly.get_polylats();
        let poly_lons = poly.get_polylons();
        for i in 0..poly_lats.len() {
            let _ = writeln!(
                self.b,
                "          [{}, {}],",
                d(poly_lats[i]),
                d(poly_lons[i])
            );
        }
        let _ = writeln!(
            self.b,
            "        ], {{color: '{color}', fillColor: \"#000000\", fillOpacity: 0.0001}});"
        );
        let _ = writeln!(self.b, "        {name}.addTo(earth);");

        for hole in poly.get_holes() {
            self.add_polygon_with_color(hole, "#ffffff");
        }
    }

    /// `getStepCount(double minLat, double maxLat, double minLon, double
    /// maxLon)` (`private`).
    fn get_step_count(min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> i32 {
        let distance_meters = SloppyMath::haversin_meters(min_lat, min_lon, max_lat, max_lon);
        1.max(java_round((distance_meters / 1000.0) / MAX_KM_PER_STEP) as i32)
    }

    /// `drawSegment(double minLat, double maxLat, double minLon, double
    /// maxLon)` (`private`).
    fn draw_segment(&mut self, min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) {
        let steps = Self::get_step_count(min_lat, max_lat, min_lon, max_lon);
        for i in 0..steps {
            let i = f64::from(i);
            let steps = f64::from(steps);
            let _ = writeln!(
                self.b,
                "          [{}, {}],",
                d(min_lat + (max_lat - min_lat) * i / steps),
                d(min_lon + (max_lon - min_lon) * i / steps)
            );
        }
    }

    /// `addRect(double minLat, double maxLat, double minLon, double
    /// maxLon)`.
    pub fn add_rect(&mut self, min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) {
        self.add_rect_with_color(min_lat, max_lat, min_lon, max_lon, "#ff0000");
    }

    /// `addRect(double minLat, double maxLat, double minLon, double maxLon,
    /// String color)`.
    pub fn add_rect_with_color(
        &mut self,
        min_lat: f64,
        max_lat: f64,
        min_lon: f64,
        max_lon: f64,
        color: &str,
    ) {
        let name = format!("rect{}", self.next_shape);
        self.next_shape += 1;

        let _ = writeln!(
            self.b,
            "        // lat: {} TO {}; lon: {} TO {}",
            d(min_lat),
            d(max_lat),
            d(min_lon),
            d(max_lon)
        );
        let _ = writeln!(self.b, "        var {name} = WE.polygon([");

        self.b.push_str("          // min -> max lat, min lon\n");
        self.draw_segment(min_lat, max_lat, min_lon, min_lon);

        self.b.push_str("          // max lat, min -> max lon\n");
        self.draw_segment(max_lat, max_lat, min_lon, max_lon);

        self.b.push_str("          // max -> min lat, max lon\n");
        self.draw_segment(max_lat, min_lat, max_lon, max_lon);

        self.b.push_str("          // min lat, max -> min lon\n");
        self.draw_segment(min_lat, min_lat, max_lon, min_lon);

        self.b.push_str("          // min lat, min lon\n");
        let _ = writeln!(self.b, "          [{}, {}]", d(min_lat), d(min_lon));
        let _ = writeln!(
            self.b,
            "        ], {{color: \"{color}\", fillColor: \"{color}\"}});"
        );
        let _ = writeln!(self.b, "        {name}.addTo(earth);");
    }

    /// `addLatLine(double lat, double minLon, double maxLon)`.
    pub fn add_lat_line(&mut self, lat: f64, min_lon: f64, max_lon: f64) {
        let name = format!("latline{}", self.next_shape);
        self.next_shape += 1;
        let _ = writeln!(self.b, "        var {name} = WE.polygon([");
        let steps = f64::from(Self::get_step_count(lat, min_lon, lat, max_lon));
        let mut lon = min_lon;
        while lon <= max_lon {
            let _ = writeln!(self.b, "          [{}, {}],", d(lat), d(lon));
            lon += (max_lon - min_lon) / steps;
        }
        let _ = writeln!(self.b, "          [{}, {}],", d(lat), d(max_lon));
        lon -= (max_lon - min_lon) / steps;
        while lon >= min_lon {
            let _ = writeln!(self.b, "          [{}, {}],", d(lat), d(lon));
            lon -= (max_lon - min_lon) / steps;
        }
        self.b.push_str(
            "        ], {color: \"#ff0000\", fillColor: \"#ffffff\", opacity: 1, fillOpacity: 0.0001});\n",
        );
        let _ = writeln!(self.b, "        {name}.addTo(earth);");
    }

    /// `addLonLine(double minLat, double maxLat, double lon)`.
    pub fn add_lon_line(&mut self, min_lat: f64, max_lat: f64, lon: f64) {
        let name = format!("lonline{}", self.next_shape);
        self.next_shape += 1;
        let _ = writeln!(self.b, "        var {name} = WE.polygon([");
        let steps = f64::from(Self::get_step_count(min_lat, lon, max_lat, lon));
        let mut lat = min_lat;
        while lat <= max_lat {
            let _ = writeln!(self.b, "          [{}, {}],", d(lat), d(lon));
            lat += (max_lat - min_lat) / steps;
        }
        let _ = writeln!(self.b, "          [{}, {}],", d(max_lat), d(lon));
        lat -= (max_lat - min_lat) / 36.0;
        while lat >= min_lat {
            let _ = writeln!(self.b, "          [{}, {}],", d(lat), d(lon));
            lat -= (max_lat - min_lat) / steps;
        }
        self.b.push_str(
            "        ], {color: \"#ff0000\", fillColor: \"#ffffff\", opacity: 1, fillOpacity: 0.0001});\n",
        );
        let _ = writeln!(self.b, "        {name}.addTo(earth);");
    }

    /// `addPoint(double lat, double lon)`.
    pub fn add_point(&mut self, lat: f64, lon: f64) {
        let _ = writeln!(
            self.b,
            "        WE.marker([{}, {}]).addTo(earth);",
            d(lat),
            d(lon)
        );
    }

    /// `addCircle(double centerLat, double centerLon, double radiusMeters,
    /// boolean alsoAddBBox)`.
    ///
    /// # Errors
    ///
    /// What `Rectangle.fromPointDistance` throws.
    pub fn add_circle(
        &mut self,
        center_lat: f64,
        center_lon: f64,
        radius_meters: f64,
        also_add_bbox: bool,
    ) -> Result<()> {
        self.add_point(center_lat, center_lon);
        let name = format!("circle{}", self.next_shape);
        self.next_shape += 1;
        let _ = writeln!(self.b, "        var {name} = WE.polygon([");
        inverse_haversin(&mut self.b, center_lat, center_lon, radius_meters);
        self.b.push_str(
            "        ], {color: '#00ff00', fillColor: \"#000000\", fillOpacity: 0.0001 });\n",
        );
        let _ = writeln!(self.b, "        {name}.addTo(earth);");

        if also_add_bbox {
            let r#box = Rectangle::from_point_distance(center_lat, center_lon, radius_meters)?;
            self.add_rect(
                r#box.min_lat(),
                r#box.max_lat(),
                r#box.min_lon(),
                r#box.max_lon(),
            );
            self.add_lat_line(
                Rectangle::axis_lat(center_lat, radius_meters),
                r#box.min_lon(),
                r#box.max_lon(),
            );
        }
        Ok(())
    }

    /// `finish()`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException("already finished")`.
    pub fn finish(&mut self) -> Result<String> {
        if self.finished {
            return Err(LuceneError::illegal_state("already finished"));
        }
        self.finished = true;
        self.b.push_str(
            "        WE.tileLayer('http://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png',{\n",
        );
        self.b
            .push_str("          attribution: '© OpenStreetMap contributors'\n");
        self.b.push_str("        }).addTo(earth);\n");
        self.b.push_str("      }\n");
        self.b.push_str("    </script>\n");
        self.b.push_str("    <style>\n");
        self.b
            .push_str("      html, body{padding: 0; margin: 0;}\n");
        self.b.push_str(
            "      #earth_div{top: 0; right: 0; bottom: 0; left: 0; position: absolute !important;}\n",
        );
        self.b.push_str("    </style>\n");
        self.b
            .push_str("    <title>WebGL Earth API: Hello World</title>\n");
        self.b.push_str("  </head>\n");
        self.b.push_str("  <body onload=\"initialize()\">\n");
        self.b.push_str("    <div id=\"earth_div\"></div>\n");
        self.b.push_str("  </body>\n");
        self.b.push_str("</html>\n");
        Ok(self.b.clone())
    }
}

/// `Math.round(double)`: the closest `long`, ties rounding up.
fn java_round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// `inverseHaversin(StringBuilder b, double centerLat, double centerLon,
/// double radiusMeters)` (`private static`).
fn inverse_haversin(b: &mut String, center_lat: f64, center_lon: f64, radius_meters: f64) {
    let mut angle = 0.0f64;
    let steps = 100.0;

    'new_angle: while angle < 360.0 {
        let x = angle.to_radians().cos();
        let y = angle.to_radians().sin();
        let mut factor = 2.0;
        let mut step = 1.0;
        let mut last = 0;
        let mut last_distance_meters = 0.0;
        loop {
            let lat = wrap_lat(center_lat + y * factor);
            let lon = wrap_lon(center_lon + x * factor);
            let distance_meters = SloppyMath::haversin_meters(center_lat, center_lon, lat, lon);

            if last == 1 && distance_meters < last_distance_meters {
                // For large enough circles, some angles are not possible:
                angle += 360.0 / steps;
                continue 'new_angle;
            }
            if last == -1 && distance_meters > last_distance_meters {
                // For large enough circles, some angles are not possible:
                angle += 360.0 / steps;
                continue 'new_angle;
            }
            last_distance_meters = distance_meters;

            if (distance_meters - radius_meters).abs() < 0.1 {
                let _ = writeln!(b, "          [{}, {}],", d(lat), d(lon));
                break;
            }
            if distance_meters > radius_meters {
                // too big
                factor -= step;
                if last == 1 {
                    step /= 2.0;
                }
                last = -1;
            } else if distance_meters < radius_meters {
                // too small
                factor += step;
                if last == -1 {
                    step /= 2.0;
                }
                last = 1;
            }
        }
        angle += 360.0 / steps;
    }
}

/// `wrapLat(double lat)` (`private static`).
fn wrap_lat(lat: f64) -> f64 {
    if lat > 90.0 {
        180.0 - lat
    } else if lat < -90.0 {
        -180.0 - lat
    } else {
        lat
    }
}

/// `wrapLon(double lon)` (`private static`).
fn wrap_lon(lon: f64) -> f64 {
    if lon > 180.0 {
        lon - 360.0
    } else if lon < -180.0 {
        lon + 360.0
    } else {
        lon
    }
}
