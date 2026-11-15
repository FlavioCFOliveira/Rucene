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

//! Port of `StringUtils.java`, of the Java package
//! `org.apache.lucene.queryparser.flexible.core.util`.

use crate::jdk::char_sequence::CharSequence;

/// String manipulation routines
///
/// Equivalent to the final class
/// `org.apache.lucene.queryparser.flexible.core.util.StringUtils`.
#[derive(Debug)]
pub struct StringUtils {
    _no_instances: (),
}

impl StringUtils {
    /// Equivalent to `StringUtils.toString(Object obj)`: `obj.toString()`,
    /// or `null` for `null`.
    pub fn to_string(obj: Option<&CharSequence>) -> Option<String> {
        obj.map(CharSequence::to_string)
    }
}
