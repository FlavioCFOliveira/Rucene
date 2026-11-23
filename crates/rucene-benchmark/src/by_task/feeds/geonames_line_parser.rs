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

//! The types declared in `GeonamesLineParser.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.feeds`.

use rucene::error::{LuceneError, Result};

use crate::by_task::feeds::doc_data::DocData;
use crate::by_task::feeds::line_doc_source::LineParser;
use crate::jdk;

/// Equivalent to `org.apache.lucene.benchmark.byTask.feeds.GeonamesLineParser`
/// (`public class GeonamesLineParser extends LineDocSource.LineParser`): a
/// line parser for Geonames.org data. See
/// <http://download.geonames.org/export/dump/readme.txt> ('geoname' table).
/// Requires `SpatialDocMaker`.
pub struct GeonamesLineParser {
    header: Option<Vec<String>>,
}

impl GeonamesLineParser {
    /// `GeonamesLineParser(String[] header)`: this header will be ignored;
    /// the geonames format is fixed and doesn't have a header line.
    pub fn new(header: Option<Vec<String>>) -> Self {
        GeonamesLineParser { header }
    }
}

/// `ArrayIndexOutOfBoundsException` of `parts[index]`.
fn index_out_of_bounds(index: usize, length: usize) -> LuceneError {
    LuceneError::unchecked(
        "java.lang.ArrayIndexOutOfBoundsException",
        format!("Index {index} out of bounds for length {length}"),
    )
}

impl LineParser for GeonamesLineParser {
    fn header(&self) -> Option<&[String]> {
        self.header.as_deref()
    }

    fn parse_line(&self, doc_data: &mut DocData, line: &str) -> Result<()> {
        // `line.split("\\t", 7)`: no more than first 6 fields needed
        let parts: Vec<&str> = line.splitn(7, '\t').collect();
        let part = |i: usize| {
            parts
                .get(i)
                .copied()
                .ok_or_else(|| index_out_of_bounds(i, parts.len()))
        };
        //    Sample data line:
        // 3578267, Morne du Vitet, Morne du Vitet, 17.88333, -62.8, ...
        // ID, Name, Alternate name (unused), Lat, Lon, ...
        doc_data.set_id(jdk::parse_int(part(0)?)?); // note: overwrites ID assigned by LineDocSource
        doc_data.set_name(Some(part(1)?.to_string()));
        let latitude = part(4)?;
        let longitude = part(5)?;
        doc_data.set_body(Some(format!("POINT({longitude} {latitude})"))); // WKT is x y order
        Ok(())
    }
}
