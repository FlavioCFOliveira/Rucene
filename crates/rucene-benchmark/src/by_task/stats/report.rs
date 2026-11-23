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

//! The types declared in `Report.java`, of the Java package
//! `org.apache.lucene.benchmark.byTask.stats`.

/// Equivalent to `org.apache.lucene.benchmark.byTask.stats.Report`: textual
/// report of current statistics.
#[derive(Clone, Debug)]
pub struct Report {
    text: String,
    size: i32,
    out_of: i32,
    reported: i32,
}

impl Report {
    /// `Report(String text, int size, int reported, int outOf)`.
    pub fn new(text: String, size: i32, reported: i32, out_of: i32) -> Self {
        Report { text, size, out_of, reported }
    }

    /// `getOutOf()`: total number of stats points when this report was
    /// created.
    pub fn get_out_of(&self) -> i32 {
        self.out_of
    }

    /// `getSize()`: number of lines in the report.
    pub fn get_size(&self) -> i32 {
        self.size
    }

    /// `getText()`: the report text.
    pub fn get_text(&self) -> &str {
        &self.text
    }

    /// `getReported()`: number of stats points represented in this report.
    pub fn get_reported(&self) -> i32 {
        self.reported
    }
}
