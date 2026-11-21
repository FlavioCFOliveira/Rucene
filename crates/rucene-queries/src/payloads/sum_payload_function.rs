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

//! Port of `SumPayloadFunction.java`, of the Java package
//! `org.apache.lucene.queries.payloads`.

use std::any::Any;

use crate::payloads::payload_function::{class_based_hash, PayloadFunction};

/// Calculates the final score as the sum of scores of all payloads seen.
///
/// Equivalent to `org.apache.lucene.queries.payloads.SumPayloadFunction`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SumPayloadFunction;

impl SumPayloadFunction {
    /// Creates the function.
    ///
    /// Equivalent to the implicit `SumPayloadFunction()`.
    pub fn new() -> Self {
        Self
    }
}

impl PayloadFunction for SumPayloadFunction {
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
        current_payload_score + current_score
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
        "org.apache.lucene.queries.payloads.SumPayloadFunction"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Equivalent to `SumPayloadFunction.equals(Object)`: equal to any instance of the
    /// same class.
    fn function_eq(&self, other: &dyn PayloadFunction) -> bool {
        other.as_any().is::<SumPayloadFunction>()
    }

    /// Equivalent to `SumPayloadFunction.hashCode()`, `31 * 1 + getClass().hashCode()`.
    fn function_hash(&self) -> i32 {
        class_based_hash("org.apache.lucene.queries.payloads.SumPayloadFunction")
    }
}
