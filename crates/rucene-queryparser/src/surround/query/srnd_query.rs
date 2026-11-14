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

//! The port of `SrndQuery.java`, of the Java package
//! `org.apache.lucene.queryparser.surround.query`.

use std::any::Any;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use rucene::error::Result;
use rucene::search::boost_query::BoostQuery;
use rucene::search::Query;
use rucene::util::java_collections::java_string_hash_code;
use rucene::util::java_fmt::float_to_string;

use super::basic_query_factory::BasicQueryFactory;
use super::distance_sub_query::DistanceSubQuery;

/// The fields of Java's abstract `SrndQuery`: the weight parsed from the
/// query text, and whether one was given.
///
/// Java's `setWeight` changes a query after it is built and shared, so the
/// weight is behind a lock; [`Clone`] copies the current value, as Java's
/// shallow `clone()` copies the fields.
#[derive(Debug)]
pub struct SrndQueryState {
    weight: Mutex<(f32, bool)>,
}

impl SrndQueryState {
    /// `new SrndQuery()`: weight `1.0`, not weighted.
    pub fn new() -> Self {
        Self {
            weight: Mutex::new((1.0, false)),
        }
    }

    fn get(&self) -> (f32, bool) {
        *self.weight.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, w: f32) {
        *self.weight.lock().unwrap_or_else(PoisonError::into_inner) = (w, true);
    }
}

impl Default for SrndQueryState {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SrndQueryState {
    fn clone(&self) -> Self {
        Self {
            weight: Mutex::new(self.get()),
        }
    }
}

/// Lowest level base class for surround queries.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queryparser.surround.query.SrndQuery`, which
/// implements `Cloneable`. The methods that build a Lucene query take the
/// query by `Arc`, because Java's rewrite queries keep a reference to the
/// surround query that made them.
///
/// **Divergence.** Java's `hashCode()` mixes `getClass().hashCode()`, the
/// identity hash of the class object, which differs between JVM runs; this
/// port mixes the `String.hashCode()` of the class name instead.
pub trait SrndQuery: Debug + Send + Sync + Any {
    /// The weight state.
    fn srnd_state(&self) -> &SrndQueryState;

    /// Equivalent to `setWeight(float)`.
    fn set_weight(&self, w: f32) {
        self.srnd_state().set(w); /* as parsed from the query text */
    }

    /// Equivalent to `isWeighted()`.
    fn is_weighted(&self) -> bool {
        self.srnd_state().get().1
    }

    /// Equivalent to `getWeight()`.
    fn get_weight(&self) -> f32 {
        self.srnd_state().get().0
    }

    /// Equivalent to `getWeightString()`: `Float.toString(getWeight())`.
    fn get_weight_string(&self) -> String {
        float_to_string(self.get_weight())
    }

    /// Equivalent to `getWeightOperator()`.
    fn get_weight_operator(&self) -> String {
        "^".to_string()
    }

    /// Appends the weight part of a query.
    ///
    /// Equivalent to the protected `weightToString(StringBuilder)`.
    fn weight_to_string(&self, r: &mut String) {
        if self.is_weighted() {
            r.push_str(&self.get_weight_operator());
            r.push_str(&self.get_weight_string());
        }
    }

    /// Equivalent to `makeLuceneQueryField(String, BasicQueryFactory)`:
    /// the query without its weight, boosted by the weight when one was
    /// given (weight may be at any level in a SrndQuery).
    ///
    /// # Errors
    ///
    /// The errors of the query construction.
    fn make_lucene_query_field(
        self: Arc<Self>,
        field_name: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Arc<dyn Query>> {
        let mut q = Arc::clone(&self).make_lucene_query_field_no_boost(field_name, qf)?;
        if self.is_weighted() {
            q = Arc::new(BoostQuery::new(q, self.get_weight())?);
        }
        Ok(q)
    }

    /// Equivalent to the abstract `makeLuceneQueryFieldNoBoost(String,
    /// BasicQueryFactory)`.
    ///
    /// # Errors
    ///
    /// The errors of the query construction.
    fn make_lucene_query_field_no_boost(
        self: Arc<Self>,
        field_name: Option<&str>,
        qf: &Arc<BasicQueryFactory>,
    ) -> Result<Arc<dyn Query>>;

    /// This method is used by `hash_code` and `equals`, so that queries
    /// with the same text are equal.
    ///
    /// Equivalent to the abstract `toString()`.
    fn srnd_to_string(&self) -> String;

    /// Equivalent to `isFieldsSubQueryAcceptable()`.
    fn is_fields_sub_query_acceptable(&self) -> bool {
        true
    }

    /// A shallow copy: the weight is copied, the sub-queries are shared.
    ///
    /// Equivalent to `clone()`.
    fn clone_srnd(&self) -> Arc<dyn SrndQuery>;

    /// Equivalent to `hashCode()`: `getClass().hashCode() ^
    /// toString().hashCode()`.
    fn srnd_hash_code(&self) -> i32 {
        java_string_hash_code(self.java_class_name())
            ^ java_string_hash_code(&self.srnd_to_string())
    }

    /// Two query objects are equal if their textual representation is the
    /// same.
    ///
    /// Equivalent to `equals(Object)`.
    fn srnd_equals(&self, other: &dyn SrndQuery) -> bool {
        self.as_any().type_id() == other.as_any().type_id()
            && self.srnd_to_string() == other.srnd_to_string()
    }

    /// The Java binary class name, `getClass().getName()`.
    fn java_class_name(&self) -> &'static str;

    /// This query, for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// This query as a [`DistanceSubQuery`], when its class implements the
    /// interface: Java's `instanceof DistanceSubQuery`.
    fn as_distance_sub_query(&self) -> Option<&dyn DistanceSubQuery> {
        None
    }
}
