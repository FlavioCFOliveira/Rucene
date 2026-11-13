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

//! Skeletons of the types declared in `FrenchAnalyzer.java`, of the Java
//! package `org.apache.lucene.analysis.fr`.

/// Java: `public final class`.
///
/// Skeleton of `org.apache.lucene.analysis.fr.FrenchAnalyzer`; ported by rmp task #468.
pub struct FrenchAnalyzer;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.analysis.fr.FrenchAnalyzer.DefaultSetHolder`; ported by rmp task #468.
pub(crate) struct DefaultSetHolder;

/// Default set of articles for `ElisionFilter`.
///
/// The public static field `DEFAULT_ARTICLES` of Lucene 10.5.0's
/// `org.apache.lucene.analysis.fr.FrenchAnalyzer`,
/// `CharArraySet.unmodifiableSet(new CharArraySet(Arrays.asList("l", "m", "t",
/// "qu", "n", "s", "j", "d", "c", "jusqu", "quoiqu", "lorsqu", "puisqu"),
/// true))`; ported ahead of its class by rmp task #448, whose
/// `ElisionFilterFactory` reads it (the class is task #468's).
pub static DEFAULT_ARTICLES: std::sync::LazyLock<rucene::analysis::CharArraySet> =
    std::sync::LazyLock::new(|| {
        rucene::analysis::CharArraySet::unmodifiable_set(Some(
            rucene::analysis::CharArraySet::from_iter(
                [
                    "l", "m", "t", "qu", "n", "s", "j", "d", "c", "jusqu", "quoiqu", "lorsqu",
                    "puisqu",
                ],
                true,
            ),
        ))
    });
