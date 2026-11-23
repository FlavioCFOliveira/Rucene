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

//! Implementation of `ScriptAttribute` that stores the script as an integer,
//! ported from
//! `org.apache.lucene.analysis.icu.tokenattributes.ScriptAttributeImpl`.

use std::any::{Any, TypeId};
use std::sync::OnceLock;

use icu4j::lang::u_script::UScript;
use rucene::util::attribute::{Attribute, AttributeImpl, AttributeReflector};
use rucene::util::IOUtils;

use super::script_attribute::ScriptAttribute;

/// Implementation of [`ScriptAttribute`] that stores the script as an
/// integer.
///
/// Equivalent to
/// `org.apache.lucene.analysis.icu.tokenattributes.ScriptAttributeImpl`
/// (`@lucene.experimental`).
#[derive(Clone, Debug)]
pub struct ScriptAttributeImpl {
    code: i32,
}

impl ScriptAttributeImpl {
    /// Initializes this attribute with `UScript.COMMON`.
    ///
    /// Equivalent to `ScriptAttributeImpl()`.
    pub fn new() -> Self {
        Self {
            code: UScript::COMMON,
        }
    }

    /// The attribute interfaces this implementation covers: itself and
    /// `ScriptAttribute`.
    pub fn interfaces() -> &'static [TypeId] {
        static IDS: OnceLock<Vec<TypeId>> = OnceLock::new();
        IDS.get_or_init(|| {
            vec![
                TypeId::of::<ScriptAttributeImpl>(),
                TypeId::of::<dyn ScriptAttribute>(),
            ]
        })
    }
}

impl Default for ScriptAttributeImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl Attribute for ScriptAttributeImpl {}

impl ScriptAttribute for ScriptAttributeImpl {
    /// Equivalent to `ScriptAttributeImpl.getCode()`.
    fn get_code(&self) -> i32 {
        self.code
    }

    /// Equivalent to `ScriptAttributeImpl.setCode(int)`.
    fn set_code(&mut self, code: i32) {
        self.code = code;
    }

    /// Equivalent to `ScriptAttributeImpl.getName()`, `UScript.getName(code)`.
    ///
    /// # Panics
    ///
    /// With the `IllegalArgumentException` `UScript.getName` throws for an
    /// invalid script code (an unchecked exception in Java).
    fn get_name(&self) -> Option<String> {
        UScript::get_name(self.code).unwrap_or_else(|e| IOUtils::rethrow_always(e))
    }

    /// Equivalent to `ScriptAttributeImpl.getShortName()`,
    /// `UScript.getShortName(code)`.
    ///
    /// # Panics
    ///
    /// As [`get_name`](Self::get_name).
    fn get_short_name(&self) -> Option<String> {
        UScript::get_short_name(self.code).unwrap_or_else(|e| IOUtils::rethrow_always(e))
    }
}

impl AttributeImpl for ScriptAttributeImpl {
    /// Equivalent to `ScriptAttributeImpl.clear()`: `code = UScript.COMMON`.
    fn clear(&mut self) {
        self.code = UScript::COMMON;
    }

    /// Equivalent to `ScriptAttributeImpl.copyTo(AttributeImpl)`:
    /// `((ScriptAttribute) target).setCode(code)`.
    ///
    /// # Panics
    ///
    /// With Java's `ClassCastException` when `target` is not a
    /// `ScriptAttribute`.
    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        match target.as_any_mut().downcast_mut::<ScriptAttributeImpl>() {
            Some(t) => t.set_code(self.code),
            None => rucene::store::java_nio_file::throw_unchecked(
                "java.lang.ClassCastException",
                Some(
                    "class cannot be cast to class \
                     org.apache.lucene.analysis.icu.tokenattributes.ScriptAttribute",
                ),
            ),
        }
    }

    /// Equivalent to `ScriptAttributeImpl.reflectWith(AttributeReflector)`.
    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        // when wordbreaking CJK, we use the 15924 code Japanese (Han+Hiragana+Katakana) to
        // mark runs of Chinese/Japanese. our use is correct (as for chinese Han is a subset),
        // but this is just to help prevent confusion.
        let name = if self.code == UScript::JAPANESE {
            Some("Chinese/Japanese".to_string())
        } else {
            self.get_name()
        };
        match &name {
            Some(name) => reflector.reflect(
                TypeId::of::<dyn ScriptAttribute>(),
                std::any::type_name::<ScriptAttributeImpl>(),
                "script",
                name,
            ),
            None => reflector.reflect(
                TypeId::of::<dyn ScriptAttribute>(),
                std::any::type_name::<ScriptAttributeImpl>(),
                "script",
                &name,
            ),
        }
    }

    fn clone_box(&self) -> Box<dyn AttributeImpl> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn attribute_interfaces(&self) -> &'static [TypeId] {
        Self::interfaces()
    }

    /// Equivalent to `ScriptAttributeImpl.equals(Object)`: another
    /// `ScriptAttributeImpl` with the same code.
    fn equals(&self, other: &dyn AttributeImpl) -> bool {
        other
            .as_any()
            .downcast_ref::<ScriptAttributeImpl>()
            .is_some_and(|o| o.code == self.code)
    }

    /// Equivalent to `ScriptAttributeImpl.hashCode()`: the code.
    fn hash_code(&self) -> i32 {
        self.code
    }
}
