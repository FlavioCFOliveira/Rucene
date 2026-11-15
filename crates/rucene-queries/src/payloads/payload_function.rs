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

//! Port of `PayloadFunction.java`, of the Java package
//! `org.apache.lucene.queries.payloads`.

use std::any::Any;
use std::fmt::Debug;

use rucene::search::similarities::Explanation;

/// An abstract class that defines a way for payload queries to customize
/// scoring based on payload values.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.payloads.PayloadFunction`. Java's abstract
/// `equals` and `hashCode` are [`function_eq`](Self::function_eq) and
/// [`function_hash`](Self::function_hash); `getClass()` is
/// [`java_class_name`](Self::java_class_name).
pub trait PayloadFunction: Debug + Send + Sync + 'static {
    /// Calculates the score up to this point for this doc and field.
    ///
    /// Equivalent to
    /// `PayloadFunction.currentScore(int, String, int, int, int, float, float)`.
    ///
    /// * `doc_id` — the current doc;
    /// * `field` — the field;
    /// * `start` — the start position of the matching span;
    /// * `end` — the end position of the matching span;
    /// * `num_payloads_seen` — the number of payloads seen so far;
    /// * `current_score` — the current score so far;
    /// * `current_payload_score` — the score for the current payload.
    #[allow(clippy::too_many_arguments)]
    fn current_score(
        &self,
        doc_id: i32,
        field: &str,
        start: i32,
        end: i32,
        num_payloads_seen: i32,
        current_score: f32,
        current_payload_score: f32,
    ) -> f32;

    /// Calculates the final score for all the payloads seen so far for this
    /// doc/field.
    ///
    /// Equivalent to `PayloadFunction.docScore(int, String, int, float)`.
    fn doc_score(
        &self,
        doc_id: i32,
        field: &str,
        num_payloads_seen: i32,
        payload_score: f32,
    ) -> f32;

    /// Explains the final score.
    ///
    /// Equivalent to `PayloadFunction.explain(int, String, int, float)`.
    fn explain(
        &self,
        doc_id: i32,
        field: &str,
        num_payloads_seen: i32,
        payload_score: f32,
    ) -> Explanation {
        Explanation::matched(
            self.doc_score(doc_id, field, num_payloads_seen, payload_score),
            format!(
                "{}.docScore()",
                java_simple_class_name(self.java_class_name())
            ),
            Vec::new(),
        )
    }

    /// The Java binary name of this function's class: `getClass().getName()`.
    fn java_class_name(&self) -> &'static str;

    /// Returns this function as [`Any`], for [`function_eq`](Self::function_eq).
    fn as_any(&self) -> &dyn Any;

    /// Equivalent to the abstract `PayloadFunction.equals(Object)`.
    fn function_eq(&self, other: &dyn PayloadFunction) -> bool;

    /// Equivalent to the abstract `PayloadFunction.hashCode()`.
    fn function_hash(&self) -> i32;
}

/// `Class.getSimpleName()` of a class given its binary name: the part after
/// the last `.` and the last `$`.
pub(crate) fn java_simple_class_name(binary_name: &str) -> &str {
    let name = binary_name.rsplit('.').next().unwrap_or(binary_name);
    name.rsplit('$').next().unwrap_or(name)
}

/// The `hashCode()` every built-in payload function declares:
/// `31 * 1 + getClass().hashCode()`, where the class's identity hash is
/// replaced by
/// [`java_identity_hash_code_stand_in`](rucene::util::java_collections::java_identity_hash_code_stand_in).
pub(crate) fn class_based_hash(class_name: &str) -> i32 {
    let prime = 31i32;
    let mut result = 1i32;
    result = prime
        .wrapping_mul(result)
        .wrapping_add(rucene::util::java_collections::java_identity_hash_code_stand_in(class_name));
    result
}

/// Java's `Math.max(float, float)`, which propagates `NaN` and orders `-0.0`
/// below `0.0`.
pub(crate) fn java_math_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.to_bits() == (-0.0f32).to_bits() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// Java's `Math.min(float, float)`, which propagates `NaN` and orders `-0.0`
/// below `0.0`.
pub(crate) fn java_math_min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.to_bits() == (-0.0f32).to_bits() {
        return b;
    }
    if a <= b {
        a
    } else {
        b
    }
}
