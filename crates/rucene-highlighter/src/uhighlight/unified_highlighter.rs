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

//! Skeletons of the types declared in `UnifiedHighlighter.java`, of the Java
//! package `org.apache.lucene.search.uhighlight`.

/// Java: `public class`.
///
/// Skeleton of `org.apache.lucene.search.uhighlight.UnifiedHighlighter`; ported by rmp task #617.
pub struct UnifiedHighlighter;

/// Java: `public static class`.
///
/// Skeleton of `org.apache.lucene.search.uhighlight.UnifiedHighlighter.Builder`; ported by rmp task #617.
pub struct Builder;

/// Java: `public enum`.
///
/// Skeleton of `org.apache.lucene.search.uhighlight.UnifiedHighlighter.OffsetSource`; ported by rmp task #617.
pub enum OffsetSource {
    /// Java constant `POSTINGS`.
    Postings,
    /// Java constant `TERM_VECTORS`.
    TermVectors,
    /// Java constant `ANALYSIS`.
    Analysis,
    /// Java constant `POSTINGS_WITH_TERM_VECTORS`.
    PostingsWithTermVectors,
    /// Java constant `NONE_NEEDED`.
    NoneNeeded,
}

/// Java: `protected static class`.
///
/// Skeleton of `org.apache.lucene.search.uhighlight.UnifiedHighlighter.LimitedStoredFieldVisitor`; ported by rmp task #617.
pub struct LimitedStoredFieldVisitor;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.search.uhighlight.UnifiedHighlighter.TermVectorReusingLeafReader`; ported by rmp task #617.
pub(crate) struct TermVectorReusingLeafReader;

/// Java: `public enum`.
///
/// Skeleton of `org.apache.lucene.search.uhighlight.UnifiedHighlighter.HighlightFlag`; ported by rmp task #617.
pub enum HighlightFlag {
    /// Java constant `PHRASES`.
    Phrases,
    /// Java constant `MULTI_TERM_QUERY`.
    MultiTermQuery,
    /// Java constant `PASSAGE_RELEVANCY_OVER_SPEED`.
    PassageRelevancyOverSpeed,
    /// Java constant `WEIGHT_MATCHES`.
    WeightMatches,
}
