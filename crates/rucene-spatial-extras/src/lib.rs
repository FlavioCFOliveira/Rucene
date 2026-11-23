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
//! From the module's `overview.html`: the spatial-extras module, new to Lucene
//! 6.5, is the new home for the original Lucene spatial module. The principle
//! interface to this module is a [`spatial_strategy::SpatialStrategy`], which
//! encapsulates an approach to indexing and searching based on shapes.
//! Different strategies have different features and performance profiles,
//! documented at each strategy implementation. For sample code showing how to
//! use the API, see `SpatialExample.java` in the tests (ported as
//! `tests/spatial/test_spatial_example.rs`). The module uses Spatial4j (the
//! crate `spatial4j`) for shape implementations (point, rectangle, circle; geo
//! and plain 2D Euclidean contexts), shape parsing and serialization including
//! Well-Known Text, and distance math. The prefix-tree approach that originated
//! as SOLR-2155 (trie/prefix-tree algorithms with a geohash encoding) is
//! implemented in `RecursivePrefixTreeStrategy`.
//!
//! **Divergence.** The JTS Topology Suite integration of Spatial4j (polygons
//! in a `SpatialContext.GEO` context through `JtsSpatialContextFactory`) is
//! not ported (user decision, sprint 17); Geo3D shapes
//! ([`spatial4j::geo3d_spatial_context_factory`]) provide polygons.
//!
//! **Divergence.** As in `rucene-spatial3d`, `java.lang.Math` calls go through
//! the fdlibm port [`rucene::util::java_math`] (bit-exact with a JVM run with
//! `-XX:-UseLibmIntrinsic`).
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
