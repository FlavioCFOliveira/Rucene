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

//! Skeletons of the types declared in `QueryNodeOperation.java`, of the Java
//! package `org.apache.lucene.queryparser.flexible.core.util`.

/// Java: `public final class`.
///
/// Skeleton of `org.apache.lucene.queryparser.flexible.core.util.QueryNodeOperation`; ported by rmp task #487.
pub struct QueryNodeOperation;

/// Java: `private enum`.
///
/// Skeleton of `org.apache.lucene.queryparser.flexible.core.util.QueryNodeOperation.ANDOperation`; ported by rmp task #487.
pub(crate) enum ANDOperation {
    /// Java constant `BOTH`.
    Both,
    /// Java constant `Q1`.
    Q1,
    /// Java constant `Q2`.
    Q2,
    /// Java constant `NONE`.
    None,
}
