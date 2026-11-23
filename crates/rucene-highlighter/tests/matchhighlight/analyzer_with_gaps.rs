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

//! Port of `AnalyzerWithGaps.java`: an analyzer for tests that has a
//! predefined offset and position gap.
//!
//! Java passes `delegate.getReuseStrategy()` as the fallback strategy; the
//! delegates of the tests are plain anonymous analyzers, whose strategy is
//! `GLOBAL_REUSE_STRATEGY`.

use std::fmt;
use std::sync::Arc;

use rucene::analysis::{
    Analyzer, DelegatingAnalyzerWrapper, DelegatingAnalyzerWrapperLogic, GlobalReuseStrategy,
};

/// `new AnalyzerWithGaps(offsetGap, positionGap, delegate)`.
pub fn analyzer_with_gaps(
    offset_gap: i32,
    position_gap: i32,
    delegate: Arc<dyn Analyzer>,
) -> Arc<dyn Analyzer> {
    Arc::new(DelegatingAnalyzerWrapper::new(
        Box::new(GlobalReuseStrategy),
        Box::new(Gaps {
            delegate,
            offset_gap,
            position_gap,
        }),
    ))
}

struct Gaps {
    delegate: Arc<dyn Analyzer>,
    offset_gap: i32,
    position_gap: i32,
}

impl fmt::Debug for Gaps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnalyzerWithGaps")
            .field("offset_gap", &self.offset_gap)
            .field("position_gap", &self.position_gap)
            .finish_non_exhaustive()
    }
}

impl DelegatingAnalyzerWrapperLogic for Gaps {
    fn get_wrapped_analyzer(&self, _field_name: Option<&str>) -> &dyn Analyzer {
        &*self.delegate
    }

    fn get_offset_gap(&self, _field_name: Option<&str>) -> i32 {
        self.offset_gap
    }

    fn get_position_increment_gap(&self, _field_name: Option<&str>) -> i32 {
        self.position_gap
    }
}
