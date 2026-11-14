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

//! Skeletons of the types declared in `SpanNearQuery.java`, of the Java
//! package `org.apache.lucene.queries.spans`.

/// Java: `public class`.
///
/// Skeleton of `org.apache.lucene.queries.spans.SpanNearQuery`; ported by rmp task #480.
pub struct SpanNearQuery;

/// Java: `public static class`.
///
/// Skeleton of `org.apache.lucene.queries.spans.SpanNearQuery.Builder`; ported by rmp task #480.
pub struct Builder;

/// Java: `public class`.
///
/// Skeleton of `org.apache.lucene.queries.spans.SpanNearQuery.SpanNearWeight`; ported by rmp task #480.
pub struct SpanNearWeight;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.queries.spans.SpanNearQuery.SpanGapQuery`; ported by rmp task #480.
pub(crate) struct SpanGapQuery;

/// Java: `private class`.
///
/// Skeleton of `org.apache.lucene.queries.spans.SpanNearQuery.SpanGapQuery.SpanGapWeight`; ported by rmp task #480.
pub(crate) struct SpanGapWeight;

/// Java: `static class` (package-private).
///
/// Skeleton of `org.apache.lucene.queries.spans.SpanNearQuery.GapSpans`; ported by rmp task #480.
pub(crate) struct GapSpans;
