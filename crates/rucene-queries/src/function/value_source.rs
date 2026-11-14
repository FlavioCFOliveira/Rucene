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

//! Skeletons of the types declared in `ValueSource.java`, of the Java
//! package `org.apache.lucene.queries.function`.

/// Java: `public abstract class`.
///
/// Skeleton of `org.apache.lucene.queries.function.ValueSource`; ported by rmp task #478.
pub struct ValueSource;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.queries.function.ValueSource.ScorableView`; ported by rmp task #478.
pub(crate) struct ScorableView;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.queries.function.ValueSource.WrappedLongValuesSource`; ported by rmp task #478.
pub(crate) struct WrappedLongValuesSource;

/// Java: `static class` (package-private).
///
/// Skeleton of `org.apache.lucene.queries.function.ValueSource.WrappedDoubleValuesSource`; ported by rmp task #478.
pub(crate) struct WrappedDoubleValuesSource;

/// Java: `private static class`.
///
/// Skeleton of `org.apache.lucene.queries.function.ValueSource.FromDoubleValuesSource`; ported by rmp task #478.
pub(crate) struct FromDoubleValuesSource;

/// Java: `class` (package-private).
///
/// Skeleton of `org.apache.lucene.queries.function.ValueSource.ValueSourceSortField`; ported by rmp task #478.
pub(crate) struct ValueSourceSortField;

/// Java: `class` (package-private).
///
/// Skeleton of `org.apache.lucene.queries.function.ValueSource.ValueSourceComparatorSource`; ported by rmp task #478.
pub(crate) struct ValueSourceComparatorSource;

/// Java: `class` (package-private).
///
/// Skeleton of `org.apache.lucene.queries.function.ValueSource.ValueSourceComparator`; ported by rmp task #478.
pub(crate) struct ValueSourceComparator;
