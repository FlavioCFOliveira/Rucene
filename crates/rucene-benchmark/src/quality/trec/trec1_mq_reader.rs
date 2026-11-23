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

//! The types declared in `Trec1MQReader.java`, of the Java package
//! `org.apache.lucene.benchmark.quality.trec`.

use rucene::error::{LuceneError, Result};
use rucene::util::java_collections::JavaHashMap;
use rucene_analysis_common::jdk::io::{BufferedReader, ReadChars};

use crate::jdk;
use crate::quality::quality_query::{sort_quality_queries, QualityQuery};

/// Equivalent to `org.apache.lucene.benchmark.quality.trec.Trec1MQReader`:
/// read topics of TREC 1MQ track.
///
/// Expects this topic format -
///
/// ```text
///   qnum:qtext
/// ```
///
/// Comment lines starting with '#' are ignored.
///
/// All topics will have a single name value pair.
#[derive(Debug)]
pub struct Trec1MQReader {
    name: String,
}

impl Trec1MQReader {
    /// `Trec1MQReader(String name)`: `name` is the name of name-value pair to
    /// set for all queries.
    pub fn new(name: &str) -> Self {
        Trec1MQReader {
            name: name.to_string(),
        }
    }

    /// `readQueries(BufferedReader reader)`: read quality queries from trec
    /// 1MQ format topics file.
    ///
    /// # Errors
    ///
    /// If cannot read the queries; `StringIndexOutOfBoundsException` for a
    /// line without `:`.
    pub fn read_queries<S: ReadChars>(
        &self,
        mut reader: BufferedReader<S>,
    ) -> Result<Vec<QualityQuery>> {
        let mut res = Vec::new();
        let result = (|| -> Result<()> {
            while let Some(line) = jdk::read_line(&mut reader)? {
                let line = jdk::java_trim(&line);
                if line.starts_with('#') {
                    continue;
                }
                // id
                let Some(k) = line.find(':') else {
                    return Err(LuceneError::unchecked(
                        "java.lang.StringIndexOutOfBoundsException",
                        format!("begin 0, end -1, length {}", jdk::utf16_len(line)),
                    ));
                };
                let id = jdk::java_trim(&line[..k]).to_string();
                // qtext
                let qtext = jdk::java_trim(&line[k + 1..]).to_string();
                // we got a topic!
                let mut fields: JavaHashMap<String> = JavaHashMap::new();
                fields.insert(self.name.clone(), qtext);
                // System.out.println("id: "+id+" qtext: "+qtext+"  line: "+line);
                res.push(QualityQuery::new(id, fields));
            }
            Ok(())
        })();
        let closed = reader.close();
        result?;
        closed?;
        // sort result array (by ID)
        sort_quality_queries(&mut res);
        Ok(res)
    }
}
