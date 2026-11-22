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

//! Skeletons of the types declared in `FacetsConfig.java`, of the Java
//! package `org.apache.lucene.facet`.

/// Java: `public class`.
///
/// Skeleton of `org.apache.lucene.facet.FacetsConfig`; ported by rmp task #613.
pub struct FacetsConfig;

/// Java: `public enum`.
///
/// Skeleton of `org.apache.lucene.facet.FacetsConfig.DrillDownTermsIndexing`; ported by rmp task #613.
pub enum DrillDownTermsIndexing {
    /// Java constant `NONE`.
    None,
    /// Java constant `FULL_PATH_ONLY`.
    FullPathOnly,
    /// Java constant `ALL_PATHS_NO_DIM`.
    AllPathsNoDim,
    /// Java constant `DIMENSION_AND_FULL_PATH`.
    DimensionAndFullPath,
    /// Java constant `ALL`.
    All,
}

/// Java: `public static final class`.
///
/// Skeleton of `org.apache.lucene.facet.FacetsConfig.DimConfig`; ported by rmp task #613.
pub struct DimConfig;
