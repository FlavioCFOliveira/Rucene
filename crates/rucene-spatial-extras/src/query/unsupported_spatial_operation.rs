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

//! The types declared in `UnsupportedSpatialOperation.java`, of the Java
//! package `org.apache.lucene.spatial.query`.

use std::any::Any;
use std::sync::Arc;

use rucene::error::{JavaThrowable, LuceneError, LuceneErrorKind};

use crate::query::spatial_operation::SpatialOperation;

/// Equivalent to `org.apache.lucene.spatial.query.UnsupportedSpatialOperation`
/// (`public class UnsupportedSpatialOperation extends
/// UnsupportedOperationException`): exception thrown when the
/// `SpatialStrategy` cannot implement the requested operation.
///
/// The exception is a [`LuceneError`] of the core's `Throwable` kind holding
/// this class, so its class name is
/// `org.apache.lucene.spatial.query.UnsupportedSpatialOperation`,
/// [`LuceneError::is_unsupported_operation_exception`] accepts it (its Java
/// superclass), and its message is the operation's name.
#[derive(Debug)]
pub struct UnsupportedSpatialOperation {
    /// `getMessage()`: `op.getName()`.
    message: String,
}

impl UnsupportedSpatialOperation {
    /// `UnsupportedSpatialOperation(SpatialOperation op)`: `super(op.getName())`
    /// (`UnsupportedSpatialOperation.java:28-30`).
    #[allow(clippy::new_ret_no_self)]
    pub fn new(op: SpatialOperation) -> LuceneError {
        LuceneError::from(LuceneErrorKind::Throwable(Arc::new(
            UnsupportedSpatialOperation {
                message: op.get_name().to_string(),
            },
        )))
    }
}

impl JavaThrowable for UnsupportedSpatialOperation {
    fn java_class(&self) -> &'static str {
        "org.apache.lucene.spatial.query.UnsupportedSpatialOperation"
    }

    fn get_message(&self) -> Option<&str> {
        Some(&self.message)
    }

    fn is_unsupported_operation_exception(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
