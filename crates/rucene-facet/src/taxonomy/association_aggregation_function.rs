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

//! Port of `AssociationAggregationFunction.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

/// Specify aggregation logic used in `TaxonomyFacetIntAssociations` and
/// `TaxonomyFacetFloatAssociations`.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.AssociationAggregationFunction`.
pub trait AssociationAggregationFunction: Send + Sync + std::fmt::Debug {
    /// Implement aggregation logic for integers.
    ///
    /// Equivalent to `aggregate(int, int)`.
    fn aggregate_int(&self, existing_val: i32, new_val: i32) -> i32;

    /// Implement aggregation logic for floats.
    ///
    /// Equivalent to `aggregate(float, float)`.
    fn aggregate_float(&self, existing_val: f32, new_val: f32) -> f32;
}

/// `Math.max(float, float)`: NaN if either is NaN, and `+0.0` above `-0.0`.
pub(crate) fn java_math_max_f32(a: f32, b: f32) -> f32 {
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

/// The anonymous `MAX` aggregation.
#[derive(Debug)]
pub struct MaxAggregation;

impl AssociationAggregationFunction for MaxAggregation {
    fn aggregate_int(&self, existing_val: i32, new_val: i32) -> i32 {
        existing_val.max(new_val)
    }

    fn aggregate_float(&self, existing_val: f32, new_val: f32) -> f32 {
        java_math_max_f32(existing_val, new_val)
    }
}

/// The anonymous `SUM` aggregation.
#[derive(Debug)]
pub struct SumAggregation;

impl AssociationAggregationFunction for SumAggregation {
    fn aggregate_int(&self, existing_val: i32, new_val: i32) -> i32 {
        existing_val.wrapping_add(new_val)
    }

    fn aggregate_float(&self, existing_val: f32, new_val: f32) -> f32 {
        existing_val + new_val
    }
}

/// Aggregation that computes the maximum value.
///
/// Equivalent to `AssociationAggregationFunction.MAX`.
pub static MAX: MaxAggregation = MaxAggregation;

/// Aggregation that computes the sum of values.
///
/// Equivalent to `AssociationAggregationFunction.SUM`.
pub static SUM: SumAggregation = SumAggregation;