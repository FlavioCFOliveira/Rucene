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

//! Skeletons of the types declared in `IndexReaderFunctions.java`, of the Java
//! package `org.apache.lucene.queries.function`.

/// Java: `public final class`.
///
/// Skeleton of `org.apache.lucene.queries.function.IndexReaderFunctions`; ported by rmp task #478.
pub struct IndexReaderFunctions;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.queries.function.IndexReaderFunctions.SumTotalTermFreqValuesSource`; ported by rmp task #478.
pub(crate) struct SumTotalTermFreqValuesSource;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.queries.function.IndexReaderFunctions.NoCacheConstantLongValuesSource`; ported by rmp task #478.
pub(crate) struct NoCacheConstantLongValuesSource;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.queries.function.IndexReaderFunctions.TermFreqDoubleValuesSource`; ported by rmp task #478.
pub(crate) struct TermFreqDoubleValuesSource;

/// Java: `private interface`.
///
/// Skeleton of `org.apache.lucene.queries.function.IndexReaderFunctions.ReaderFunction`; ported by rmp task #478.
pub(crate) trait ReaderFunction {}

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.queries.function.IndexReaderFunctions.IndexReaderDoubleValuesSource`; ported by rmp task #478.
pub(crate) struct IndexReaderDoubleValuesSource;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.queries.function.IndexReaderFunctions.NoCacheConstantDoubleValuesSource`; ported by rmp task #478.
pub(crate) struct NoCacheConstantDoubleValuesSource;
