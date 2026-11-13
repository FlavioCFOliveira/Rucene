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

//! Skeletons of the Java package `org.apache.lucene.analysis.compound.hyphenation`.

pub mod byte_vector;
pub mod char_vector;
pub mod hyphen;
#[allow(clippy::module_inception)]
pub mod hyphenation;
pub mod hyphenation_tree;
pub mod pattern_consumer;
pub mod pattern_parser;
pub mod ternary_tree;
