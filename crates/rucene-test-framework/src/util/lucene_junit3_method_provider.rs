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

//! Port of `org.apache.lucene.tests.util.LuceneJUnit3MethodProvider`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::{ClassModel, TestMethodProvider};

/// Port of `LuceneJUnit3MethodProvider`: "backwards compatible test* method
/// provider (public, non-static)."
#[derive(Debug, Default, Clone, Copy)]
pub struct LuceneJUnit3MethodProvider;

impl LuceneJUnit3MethodProvider {
    /// `LuceneJUnit3MethodProvider.class.getName()`.
    pub const CLASS_NAME: &'static str = "org.apache.lucene.tests.util.LuceneJUnit3MethodProvider";
}

impl TestMethodProvider for LuceneJUnit3MethodProvider {
    /// `getTestMethods(suiteClass, classModel)`: every method without an
    /// override or shadow whose name starts with `test` and which is public,
    /// not static and takes no parameters.
    fn get_test_methods(&self, _suite_class: &Class, class_model: &ClassModel) -> Vec<Arc<Method>> {
        let mut result = Vec::new();
        for mm in class_model.methods() {
            // Skip any methods that have overrieds/ shadows.
            if class_model.down(mm).is_some() {
                continue;
            }
            let m = &mm.element;
            if m.name().starts_with("test")
                && (m.get_modifiers() & modifier::PUBLIC) != 0
                && (m.get_modifiers() & modifier::STATIC) == 0
                && m.parameter_types().is_empty()
            {
                result.push(Arc::clone(m));
            }
        }
        result
    }
}
