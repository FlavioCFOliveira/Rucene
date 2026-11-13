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

//! Skeletons of the types declared in `EnglishAnalyzer.java`, of the Java
//! package `org.apache.lucene.analysis.en`.

/// Java: `public final class`.
///
/// Skeleton of `org.apache.lucene.analysis.en.EnglishAnalyzer`; ported by rmp task #452.
pub struct EnglishAnalyzer;

/// An unmodifiable set containing some common English words that are not
/// usually useful for searching.
///
/// The public static field `ENGLISH_STOP_WORDS_SET` of Lucene 10.5.0's
/// `org.apache.lucene.analysis.en.EnglishAnalyzer`, which its static
/// initializer builds as `CharArraySet.unmodifiableSet(new
/// CharArraySet(stopWords, false))`; ported ahead of its class by rmp task
/// #449, whose `StopFilterFactory` reads it (the class is task #452's).
pub static ENGLISH_STOP_WORDS_SET: std::sync::LazyLock<rucene::analysis::CharArraySet> =
    std::sync::LazyLock::new(|| {
        let stop_words = [
            "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in", "into",
            "is", "it", "no", "not", "of", "on", "or", "such", "that", "the", "their", "then",
            "there", "these", "they", "this", "to", "was", "will", "with",
        ];
        let stop_set = rucene::analysis::CharArraySet::from_iter(stop_words, false);
        rucene::analysis::CharArraySet::unmodifiable_set(Some(stop_set))
    });
