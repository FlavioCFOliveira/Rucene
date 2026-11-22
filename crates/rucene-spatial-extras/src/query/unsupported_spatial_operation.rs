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

use rucene::error::LuceneError;

use crate::query::spatial_operation::SpatialOperation;

/// Equivalent to `org.apache.lucene.spatial.query.UnsupportedSpatialOperation`
/// (`public class UnsupportedSpatialOperation extends
/// UnsupportedOperationException`): exception thrown when the
/// `SpatialStrategy` cannot implement the requested operation.
///
/// The exception is a [`LuceneError`] of the core's
/// `UnsupportedOperation` kind (its Java superclass), whose message is the
/// operation's name.
#[derive(Debug)]
pub struct UnsupportedSpatialOperation;

impl UnsupportedSpatialOperation {
    /// `UnsupportedSpatialOperation(SpatialOperation op)`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(op: SpatialOperation) -> LuceneError {
        LuceneError::unsupported_operation(op.get_name())
    }
}
