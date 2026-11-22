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

//! This crate ports Apache Lucene 10.5.0 `lucene/spatial-extras` to Rust: the
//! spatial strategies over spatial4j shapes (recursive and term prefix trees,
//! including the S2 tree, bounding box, serialized doc values, point vector and
//! composite) and the spatial4j bridge to `spatial3d`.
//!
//! Every type of the module is ported faithfully; each carries an
//! `Equivalent to` claim naming its Java type, and each unavoidable
//! divergence is declared where it occurs, in a paragraph led by
//! **Divergence.**
//!
//! The module tree mirrors the Java packages below the package root
//! `org.apache.lucene.spatial`: the types of that package sit at the crate root
//! and `org.apache.lucene.spatial.prefix.tree` is the module [`prefix::tree`].
//! Each Java source file is one module, named in snake_case
//! (`SpatialStrategy.java` is [`spatial_strategy`]); a Java package name that is
//! not snake_case becomes a snake_case module, and one that is a Rust keyword a
//! raw identifier. A nested Java type is declared in the module of its source
//! file under its own name; the name of its enclosing type is prefixed only when
//! its own name is a name of the Rust prelude or the name of another item of
//! that module, and the Java kind is appended when the prefixed name is itself
//! taken.

// Some Java fields are only read by assertions or by methods no other type
// calls (e.g. `DateRangePrefixTree`'s year bounds), as in Java.
#![allow(dead_code)]

pub(crate) mod jdk;
pub(crate) mod search_util;

pub mod bbox;
pub mod composite;
pub mod prefix;
pub mod query;
pub mod serialized;
pub mod shape_values;
pub mod shape_values_source;
pub mod spatial4j;
pub mod spatial_strategy;
pub mod util;
pub mod vector;
