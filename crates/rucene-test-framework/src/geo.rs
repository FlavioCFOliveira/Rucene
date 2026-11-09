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

//! Port of `org.apache.lucene.tests.geo` (rmp #358): [`geo_test_util`]
//! (`GeoTestUtil`), [`shape_test_util`] (`ShapeTestUtil`),
//! [`earth_debugger`] (`EarthDebugger`).
//!
//! The shape resources of the package
//! (`lucene/test-framework/src/resources/org/apache/lucene/tests/geo/*.gz`,
//! Apache License 2.0, Copyright The Apache Software Foundation, see the
//! Lucene `NOTICE.txt`) are copied unchanged under `src/geo/resources/` and
//! read by [`geo_test_util::read_shape`].

pub mod earth_debugger;
pub mod geo_test_util;
pub mod shape_test_util;

pub use earth_debugger::EarthDebugger;
