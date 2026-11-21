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

//! Port of `BoolFunction.java`, of the Java package
//! `org.apache.lucene.queries.function.valuesource`.

use crate::function::value_source::ValueSource;

/// Abstract parent class for those [`ValueSource`] implementations which apply
/// boolean logic to their values.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.queries.function.valuesource.BoolFunction`, which
/// declares no member of its own; a Rust value source that extends it
/// implements this marker trait.
pub trait BoolFunction: ValueSource {}
