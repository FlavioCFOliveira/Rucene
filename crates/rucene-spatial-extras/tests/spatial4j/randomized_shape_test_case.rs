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

//! Port of `RandomizedShapeTestCase.java` (package
//! `org.apache.lucene.spatial.spatial4j`): a base test class with utility
//! methods to help test shapes. Extends from RandomizedTest.

use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::junit::Assert;
use rucene_test_framework::util::LuceneTestCase;

/// The abstract class `RandomizedShapeTestCase extends LuceneTestCase`; its
/// field `ctx` is on each subclass's instance type.
pub fn randomized_shape_test_case_class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.spatial.spatial4j.RandomizedShapeTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .build()
    });
    Arc::clone(&CLASS)
}

/// `checkShapesImplementEquals(Class<?>[] classes)` (`public static`).
pub fn check_shapes_implement_equals(classes: &[Arc<Class>]) {
    for clazz in classes {
        let declares = |name: &str| clazz.declared_methods().iter().any(|m| m.name() == name);
        if !declares("equals") {
            Assert::fail(&format!(
                "Shape needs to define 'equals' : {}",
                clazz.name()
            ));
        }
        if !declares("hashCode") {
            Assert::fail(&format!(
                "Shape needs to define 'hashCode' : {}",
                clazz.name()
            ));
        }
    }
}

/// `divisible(double v, double divisible)` (`public static`).
pub fn divisible(v: f64, divisible: f64) -> f64 {
    f64::from(((randomizedtesting::jdk::math::round(v / divisible) as f64) * divisible) as i32)
}
