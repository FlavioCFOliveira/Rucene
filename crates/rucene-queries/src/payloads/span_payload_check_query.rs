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

//! Skeletons of the types declared in `SpanPayloadCheckQuery.java`, of the Java
//! package `org.apache.lucene.queries.payloads`.

/// Java: `public class`.
///
/// Skeleton of `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery`; ported by rmp task #481.
pub struct SpanPayloadCheckQuery;

/// Java: `public enum`.
///
/// Skeleton of `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery.PayloadType`; ported by rmp task #481.
pub enum PayloadType {
    /// Java constant `INT`.
    Int,
    /// Java constant `FLOAT`.
    Float,
    /// Java constant `STRING`.
    String,
}

/// Java: `public enum`.
///
/// Skeleton of `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery.MatchOperation`; ported by rmp task #481.
pub enum MatchOperation {
    /// Java constant `EQ`.
    Eq,
    /// Java constant `GT`.
    Gt,
    /// Java constant `GTE`.
    Gte,
    /// Java constant `LT`.
    Lt,
    /// Java constant `LTE`.
    Lte,
}

/// Java: `public class`.
///
/// Skeleton of `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery.SpanPayloadCheckWeight`; ported by rmp task #481.
pub struct SpanPayloadCheckWeight;

/// Java: `private class`.
///
/// Skeleton of `org.apache.lucene.queries.payloads.SpanPayloadCheckQuery.PayloadChecker`; ported by rmp task #481.
pub(crate) struct PayloadChecker;
