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

//! Port of `MinPayloadFunction.java`, of the Java package
//! `org.apache.lucene.queries.payloads`.

use std::any::Any;

use crate::payloads::payload_function::{class_based_hash, java_math_min, PayloadFunction};

/// Calculates the minimum payload seen.
///
/// Equivalent to `org.apache.lucene.queries.payloads.MinPayloadFunction`.
#[derive(Debug, Default, Clone, Copy)]
pub struct MinPayloadFunction;

impl MinPayloadFunction {
    /// Creates the function.
    ///
    /// Equivalent to the implicit `MinPayloadFunction()`.
    pub fn new() -> Self {
        Self
    }
}

impl PayloadFunction for MinPayloadFunction {
    fn current_score(
        &self,
        _doc_id: i32,
        _field: &str,
        _start: i32,
        _end: i32,
        num_payloads_seen: i32,
        current_score: f32,
        current_payload_score: f32,
    ) -> f32 {
        let _ = num_payloads_seen;
        if num_payloads_seen == 0 {
            current_payload_score
        } else {
            java_math_min(current_payload_score, current_score)
        }
    }

    fn doc_score(
        &self,
        _doc_id: i32,
        _field: &str,
        num_payloads_seen: i32,
        payload_score: f32,
    ) -> f32 {
        if num_payloads_seen > 0 {
            payload_score
        } else {
            1.0
        }
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.queries.payloads.MinPayloadFunction"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Equivalent to `MinPayloadFunction.equals(Object)`: equal to any instance of the
    /// same class.
    fn function_eq(&self, other: &dyn PayloadFunction) -> bool {
        other.as_any().is::<MinPayloadFunction>()
    }

    /// Equivalent to `MinPayloadFunction.hashCode()`, `31 * 1 + getClass().hashCode()`.
    fn function_hash(&self) -> i32 {
        class_based_hash("org.apache.lucene.queries.payloads.MinPayloadFunction")
    }
}
