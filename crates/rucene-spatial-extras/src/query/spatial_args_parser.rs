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

//! The types declared in `SpatialArgsParser.java`, of the Java package
//! `org.apache.lucene.spatial.query`.

use std::sync::Arc;

use rucene::error::{LuceneError, Result};
use rucene::util::extra::ParseException;
use rucene::util::java_collections::JavaHashMap;
use rucene::util::java_fmt::format_fixed;
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::shape::Shape;

use crate::jdk::{d2s, parse_boolean, parse_double};
use crate::query::spatial_args::SpatialArgs;
use crate::query::spatial_operation::SpatialOperation;

/// Equivalent to `org.apache.lucene.spatial.query.SpatialArgsParser`
/// (`public class`): parses a string that usually looks like
/// `OPERATION(SHAPE)` into a [`SpatialArgs`] object. The set of operations
/// supported are defined in [`SpatialOperation`], such as `Intersects` and
/// `IsWithin`. The shape portion is defined by WKT `WKTReader`, but it can
/// be overridden/customized via `parse_shape`. There are some optional
/// name-value pair parameters that follow the closing parenthesis, e.g.
/// `Intersects(ENVELOPE(-10,-8,22,20)) distErrPct=0.025`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpatialArgsParser;

impl SpatialArgsParser {
    /// `DIST_ERR_PCT`.
    pub const DIST_ERR_PCT: &'static str = "distErrPct";
    /// `DIST_ERR`.
    pub const DIST_ERR: &'static str = "distErr";

    /// `SpatialArgsParser()`.
    pub fn new() -> Self {
        Self
    }

    /// `writeSpatialArgs(SpatialArgs args)`: writes a close approximation
    /// to the parsed input format.
    pub(crate) fn write_spatial_args(args: &SpatialArgs) -> String {
        let mut str = String::new();
        str.push_str(args.get_operation().get_name());
        str.push('(');
        str.push_str(&args.get_shape().to_string());
        if let Some(pct) = args.get_dist_err_pct() {
            // String.format(Locale.ROOT, "%.2f%%", args.getDistErrPct() * 100d)
            str.push_str(" distErrPct=");
            str.push_str(&format_fixed(pct * 100.0, 2));
            str.push('%');
        }
        if let Some(dist_err) = args.get_dist_err() {
            str.push_str(" distErr=");
            str.push_str(&d2s(dist_err));
        }
        str.push(')');
        str
    }

    /// `parse(String v, SpatialContext ctx)`: parses a string such as
    /// `Intersects(ENVELOPE(-10,-8,22,20)) distErrPct=0.025`.
    ///
    /// # Errors
    ///
    /// `ParseException` if the strategy can't parse the shape or the
    /// arguments are malformed; `InvalidShapeException` for an invalid
    /// shape; `IllegalArgumentException` for an unknown operation or unused
    /// parameters.
    pub fn parse(&self, v: &str, ctx: &SpatialContext) -> Result<SpatialArgs> {
        let units: Vec<u16> = v.encode_utf16().collect();
        let idx = units.iter().position(|&u| u == u16::from(b'(')).map_or(-1, |i| i as i32);
        let edx = units.iter().rposition(|&u| u == u16::from(b')')).map_or(-1, |i| i as i32);
        if idx < 0 || idx > edx {
            return Err(ParseException::error(format!("missing parens: {v}"), -1));
        }
        let sub = |a: i32, b: i32| String::from_utf16_lossy(&units[a as usize..b as usize]);
        let op = SpatialOperation::get(java_trim(&sub(0, idx)))?;
        let body = sub(idx + 1, edx);
        let body = java_trim(&body);
        if body.is_empty() {
            return Err(ParseException::error(format!("missing body : {v}"), idx + 1));
        }
        let shape = self.parse_shape(body, ctx)?;
        let args = self.new_spatial_args(op, shape);
        if units.len() as i32 > edx + 1 {
            let rest = sub(edx + 1, units.len() as i32);
            let body = java_trim(&rest);
            if !body.is_empty() {
                let mut aa = Self::parse_map(body);
                self.read_name_value_pairs(&args, &mut aa)?;
                if !aa.is_empty() {
                    return Err(LuceneError::illegal_argument(format!(
                        "unused parameters: {}",
                        map_to_string(&aa)
                    )));
                }
            }
        }
        args.validate()?;
        Ok(args)
    }

    /// `newSpatialArgs(SpatialOperation op, Shape shape)`.
    pub fn new_spatial_args(&self, op: SpatialOperation, shape: Arc<dyn Shape>) -> SpatialArgs {
        SpatialArgs::new(op, shape)
    }

    /// `readNameValuePairs(SpatialArgs args, Map<String, String> nameValPairs)`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException` for a value that is not a number.
    pub fn read_name_value_pairs(&self, args: &SpatialArgs, name_val_pairs: &mut JavaHashMap<String>) -> Result<()> {
        args.set_dist_err_pct(Self::read_double(name_val_pairs.remove(Self::DIST_ERR_PCT).as_deref())?);
        args.set_dist_err(Self::read_double(name_val_pairs.remove(Self::DIST_ERR).as_deref())?);
        Ok(())
    }

    /// `parseShape(String str, SpatialContext ctx)`.
    ///
    /// # Errors
    ///
    /// `ParseException` and `InvalidShapeException` of the WKT reader.
    pub fn parse_shape(&self, str: &str, ctx: &SpatialContext) -> Result<Arc<dyn Shape>> {
        let reader = ctx
            .get_formats()
            .get_wkt_reader()
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: no WKT reader"));
        match reader.read(str) {
            Ok(Some(shape)) => Ok(shape),
            // the WKT reader never returns null
            Ok(None) => panic!("java.lang.NullPointerException"),
            // impossible but whatever
            Err(e) if e.is_io_exception() => Err(ParseException::error(e.to_string(), 0)),
            Err(e) => Err(e),
        }
    }

    /// `readDouble(String v)`.
    ///
    /// # Errors
    ///
    /// `NumberFormatException`.
    pub fn read_double(v: Option<&str>) -> Result<Option<f64>> {
        v.map(parse_double).transpose()
    }

    /// `readBool(String v, boolean defaultValue)`.
    pub fn read_bool(v: Option<&str>, default_value: bool) -> bool {
        v.map_or(default_value, parse_boolean)
    }

    /// `parseMap(String body)`: parses "a=b c=d f" (whitespace separated)
    /// into name-value pairs. If there is no '=' as in 'f' above then it's
    /// short for f=f.
    pub fn parse_map(body: &str) -> JavaHashMap<String> {
        let mut map = JavaHashMap::new();
        // new StringTokenizer(body, " \n\t")
        for a in body.split([' ', '\n', '\t']).filter(|t| !t.is_empty()) {
            match a.find('=') {
                Some(idx) if idx > 0 => {
                    let k = &a[..idx];
                    let v = &a[idx + 1..];
                    map.insert(k.to_string(), v.to_string());
                }
                _ => {
                    map.insert(a.to_string(), a.to_string());
                }
            }
        }
        map
    }
}

/// `String.trim()`: strips the code points `<= ' '` at both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `AbstractMap.toString()`: `{k1=v1, k2=v2}` in iteration order.
fn map_to_string(map: &JavaHashMap<String>) -> String {
    let entries: Vec<String> = map.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{{{}}}", entries.join(", "))
}
