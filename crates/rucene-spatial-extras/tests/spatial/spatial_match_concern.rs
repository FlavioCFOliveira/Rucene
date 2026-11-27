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

//! Port of `SpatialMatchConcern.java` (package `org.apache.lucene.spatial`).

/// `SpatialMatchConcern` (`public class`).
#[derive(Clone, Copy, Debug)]
pub struct SpatialMatchConcern {
    /// `orderIsImportant`.
    pub order_is_important: bool,
    /// `resultsAreSuperset`: if the strategy can not give exact answers, but
    /// used to limit results.
    pub results_are_superset: bool,
}

impl SpatialMatchConcern {
    /// `EXACT`.
    pub const EXACT: SpatialMatchConcern = SpatialMatchConcern {
        order_is_important: true,
        results_are_superset: false,
    };
    /// `FILTER`.
    pub const FILTER: SpatialMatchConcern = SpatialMatchConcern {
        order_is_important: false,
        results_are_superset: false,
    };
    /// `SUPERSET`.
    pub const SUPERSET: SpatialMatchConcern = SpatialMatchConcern {
        order_is_important: false,
        results_are_superset: true,
    };
}
