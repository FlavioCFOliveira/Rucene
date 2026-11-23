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

//! Port of `DefaultEncoder.java`, of the Java package
//! `org.apache.lucene.search.highlight`.

use crate::highlight::encoder::Encoder;

/// Simple Encoder that does not encode the text.
///
/// Equivalent to `org.apache.lucene.search.highlight.DefaultEncoder`.
#[derive(Debug, Default, Clone)]
pub struct DefaultEncoder;

impl DefaultEncoder {
    /// Equivalent to `new DefaultEncoder()`.
    pub fn new() -> Self {
        Self
    }
}

impl Encoder for DefaultEncoder {
    fn encode_text(&self, original_text: &str) -> String {
        original_text.to_string()
    }
}
