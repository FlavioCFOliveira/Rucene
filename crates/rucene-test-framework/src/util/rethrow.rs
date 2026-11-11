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

//! Port of `org.apache.lucene.tests.util.Rethrow`.

use std::any::Any;

/// Sneaky: rethrowing checked exceptions as unchecked ones. Eh, it is
/// sometimes useful...
///
/// Pulled from [Java Puzzlers](http://www.javapuzzlers.com).
///
/// Port of `Rethrow` (`final`, no instances).
pub struct Rethrow(());

impl Rethrow {
    /// `rethrow(Throwable t)`: rethrows `t` (identical object).
    ///
    /// **Divergence (declared).** Java throws any `Throwable` past the
    /// compiler's checked-exception analysis; the Rust counterpart of a
    /// thrown object is a panic payload, which this resumes unchanged
    /// (`std::panic::resume_unwind`), as randomizedtesting's `Rethrow` does
    /// in this workspace.
    pub fn rethrow(t: Box<dyn Any + Send>) -> ! {
        std::panic::resume_unwind(t)
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    use super::Rethrow;

    /// The payload that comes back is the identical object.
    #[test]
    fn rethrow_resumes_the_identical_payload() {
        let payload: Box<dyn std::any::Any + Send> = Box::new(42u64);
        let address = &*payload as *const dyn std::any::Any as *const u8 as usize;
        let _quiet = randomizedtesting::jdk::exception::QuietPanics::new();
        let caught = catch_unwind(AssertUnwindSafe(|| Rethrow::rethrow(payload))).unwrap_err();
        assert_eq!(
            &*caught as *const dyn std::any::Any as *const u8 as usize,
            address
        );
        assert_eq!(caught.downcast_ref::<u64>(), Some(&42));
    }
}
