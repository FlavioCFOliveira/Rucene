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

//! This attribute stores the UTR #24 script value for a token of text,
//! ported from `org.apache.lucene.analysis.icu.tokenattributes.ScriptAttribute`.

use rucene::util::attribute::{Attribute, AttributeImpl, AttributeInterface};

use super::script_attribute_impl::ScriptAttributeImpl;

/// This attribute stores the UTR #24 script value for a token of text.
///
/// Equivalent to `org.apache.lucene.analysis.icu.tokenattributes.ScriptAttribute`
/// (`@lucene.experimental`).
pub trait ScriptAttribute: Attribute {
    /// Get the numeric code for this script value. This is the constant
    /// value from `UScript`.
    ///
    /// Equivalent to `getCode()`.
    fn get_code(&self) -> i32;

    /// Set the numeric code for this script value. This is the constant
    /// value from `UScript`.
    ///
    /// Equivalent to `setCode(int)`.
    fn set_code(&mut self, code: i32);

    /// Get the full name: the UTR #24 full name (`None` is Java's `null`).
    ///
    /// Equivalent to `getName()`.
    fn get_name(&self) -> Option<String>;

    /// Get the abbreviated name: the UTR #24 abbreviated name (`None` is
    /// Java's `null`).
    ///
    /// Equivalent to `getShortName()`.
    fn get_short_name(&self) -> Option<String>;
}

/// `(ScriptAttribute) att`: the cast an attribute source makes to hand out
/// the interface.
///
/// **Divergence from Lucene 10.5.0.** Java casts any class implementing the
/// interface; Rust cannot cast a `dyn AttributeImpl` to another trait
/// object, so the cast succeeds for [`ScriptAttributeImpl`], the one
/// implementation of the interface Lucene 10.5.0 ships (the closed set the
/// test framework's `Att` casts through, for the same reason).
impl AttributeInterface for dyn ScriptAttribute {
    fn cast(att: &dyn AttributeImpl) -> Option<&Self> {
        att.as_any()
            .downcast_ref::<ScriptAttributeImpl>()
            .map(|a| a as &dyn ScriptAttribute)
    }

    fn cast_mut(att: &mut dyn AttributeImpl) -> Option<&mut Self> {
        att.as_any_mut()
            .downcast_mut::<ScriptAttributeImpl>()
            .map(|a| a as &mut dyn ScriptAttribute)
    }
}
