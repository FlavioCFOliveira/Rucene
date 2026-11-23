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

//! Port of `SpatialTestData.java` (package `org.apache.lucene.spatial`).
// This class is modelled after SpatialTestQuery.
// Before Lucene 4.7, this was a bit different in Spatial4j as SampleData & SampleDataReader.

use std::sync::Arc;

use rucene::error::{LuceneError, LuceneErrorKind, Result};
use spatial4j::context::spatial_context::SpatialContext;
use spatial4j::shape::shape::Shape;

use crate::spatial::spatial_test_query::{java_split, read_lines};

/// `SpatialTestData` (`public class`).
#[derive(Clone, Debug)]
pub struct SpatialTestData {
    /// `id`.
    pub id: String,
    /// `name`.
    pub name: String,
    /// `shape`.
    pub shape: Arc<dyn Shape>,
}

impl SpatialTestData {
    /// `getTestData(InputStream in, SpatialContext ctx)`: reads the stream,
    /// consuming a format that is a tab-separated values of 3 columns: an
    /// "id", a "name" and the "shape". Empty lines and lines starting with a
    /// '#' are skipped. The stream is closed.
    ///
    /// # Errors
    ///
    /// Java's `RuntimeException` for a bad line or a shape that does not
    /// parse; the I/O errors of reading.
    pub fn get_test_data(input: &[u8], ctx: &SpatialContext) -> Result<Vec<SpatialTestData>> {
        let mut results = Vec::new();
        for line in read_lines(input)? {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let vals = java_split(&line, '\t');
            if vals.len() != 3 {
                return Err(LuceneError::unchecked(
                    "java.lang.RuntimeException",
                    format!("bad format; expecting 3 tab-separated values for line: {line}"),
                ));
            }
            let shape = ctx.read_shape_from_wkt(&vals[2]).map_err(|e| {
                LuceneError::from(LuceneErrorKind::Unchecked {
                    class: "java.lang.RuntimeException",
                    message: Some(e.to_string()),
                    cause: Some(std::sync::Arc::new(e)),
                })
            })?;
            results.push(SpatialTestData {
                id: vals[0].clone(),
                name: vals[1].clone(),
                shape,
            });
        }
        Ok(results)
    }
}
