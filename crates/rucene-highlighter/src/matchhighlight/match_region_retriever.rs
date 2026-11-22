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

//! Skeletons of the types declared in `MatchRegionRetriever.java`, of the Java
//! package `org.apache.lucene.search.matchhighlight`.

/// Java: `public class`.
///
/// Skeleton of `org.apache.lucene.search.matchhighlight.MatchRegionRetriever`; ported by rmp task #617.
pub struct MatchRegionRetriever;

/// Java: `public interface`.
///
/// Skeleton of `org.apache.lucene.search.matchhighlight.MatchRegionRetriever.MatchOffsetsConsumer`; ported by rmp task #617.
pub trait MatchOffsetsConsumer {}

/// Java: `public interface`.
///
/// Skeleton of `org.apache.lucene.search.matchhighlight.MatchRegionRetriever.FieldValueProvider`; ported by rmp task #617.
pub trait FieldValueProvider {}

/// Java: `private record`.
///
/// Skeleton of `org.apache.lucene.search.matchhighlight.MatchRegionRetriever.DocHighlightData`; ported by rmp task #617.
pub(crate) struct DocHighlightData;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.search.matchhighlight.MatchRegionRetriever.MatchesIteratorWithLimit`; ported by rmp task #617.
pub(crate) struct MatchesIteratorWithLimit;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.search.matchhighlight.MatchRegionRetriever.StoredFieldsVisitor`; ported by rmp task #617.
pub(crate) struct StoredFieldsVisitor;
