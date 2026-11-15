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

//! Port of `UniqueFieldAttributeImpl.java`.

use std::any::{Any, TypeId};
use std::sync::OnceLock;

use rucene::util::attribute::{Attribute, AttributeImpl, AttributeReflector};
use rucene::util::java_collections::java_string_hash_code;
use rucene_queryparser::jdk::char_sequence::CharSequence;

use super::unique_field_attribute::UniqueFieldAttribute;

/// This attribute is used by the `UniqueFieldQueryNodeProcessor` processor.
/// It holds a value that defines which is the unique field name that should
/// be set in every `FieldableNode`.
#[derive(Debug, Clone)]
pub struct UniqueFieldAttributeImpl {
    unique_field: CharSequence,
}

impl Default for UniqueFieldAttributeImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl UniqueFieldAttributeImpl {
    /// `new UniqueFieldAttributeImpl()`: `clear()`.
    pub fn new() -> Self {
        let mut a = UniqueFieldAttributeImpl {
            unique_field: CharSequence::from(""),
        };
        a.clear();
        a
    }

    /// `hashCode()`: `uniqueField.hashCode()` (a `String`'s hash; the
    /// sequences this attribute holds are strings).
    pub fn hash_code(&self) -> i32 {
        java_string_hash_code(&self.unique_field.to_string())
    }
}

/// `equals(Object other)`: `other instanceof UniqueFieldAttributeImpl` and
/// `((UniqueFieldAttributeImpl) other).uniqueField.equals(this.uniqueField)`.
impl PartialEq for UniqueFieldAttributeImpl {
    fn eq(&self, other: &Self) -> bool {
        other.unique_field == self.unique_field
    }
}

impl Attribute for UniqueFieldAttributeImpl {}

impl UniqueFieldAttribute for UniqueFieldAttributeImpl {
    fn set_unique_field(&mut self, unique_field: CharSequence) {
        self.unique_field = unique_field;
    }

    fn get_unique_field(&self) -> CharSequence {
        self.unique_field.clone()
    }
}

impl AttributeImpl for UniqueFieldAttributeImpl {
    fn clear(&mut self) {
        self.unique_field = CharSequence::from("");
    }

    /// `copyTo(AttributeImpl target)`: an `IllegalArgumentException` for a
    /// target of another class (named by its Rust type: an attribute
    /// implementation keeps no Java class name).
    fn copy_to(&self, target: &mut dyn AttributeImpl) {
        let name = target.impl_type_name();
        match target.as_any_mut().downcast_mut::<UniqueFieldAttributeImpl>() {
            Some(unique_field_attr) => {
                unique_field_attr.unique_field = CharSequence::from(self.unique_field.to_string());
            }
            None => randomizedtesting::jdk::exception::throw(
                "java.lang.IllegalArgumentException",
                format!(
                    "cannot copy the values from attribute UniqueFieldAttribute to an instance of {name}"
                ),
            ),
        }
    }

    fn reflect_with(&self, reflector: &mut dyn AttributeReflector) {
        reflector.reflect(
            TypeId::of::<dyn UniqueFieldAttribute>(),
            "UniqueFieldAttribute",
            "uniqueField",
            &self.unique_field,
        );
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
        static IDS: OnceLock<&'static [TypeId]> = OnceLock::new();
        IDS.get_or_init(|| {
            let ids = vec![
                TypeId::of::<UniqueFieldAttributeImpl>(),
                TypeId::of::<dyn UniqueFieldAttribute>(),
            ];
            Box::leak(ids.into_boxed_slice())
        })
    }
}
