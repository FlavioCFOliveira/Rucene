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

//! Skeletons of the types declared in `LineDocSource.java`, of the Java
//! package `org.apache.lucene.benchmark.byTask.feeds`.

/// Java: `public class`.
///
/// Skeleton of `org.apache.lucene.benchmark.byTask.feeds.LineDocSource`; ported by rmp task #625.
pub struct LineDocSource;

/// Java: `public abstract static class`.
///
/// Skeleton of `org.apache.lucene.benchmark.byTask.feeds.LineDocSource.LineParser`; ported by rmp task #625.
pub struct LineParser;

/// Java: `public static class`.
///
/// Skeleton of `org.apache.lucene.benchmark.byTask.feeds.LineDocSource.SimpleLineParser`; ported by rmp task #625.
pub struct SimpleLineParser;

/// Java: `public static class`.
///
/// Skeleton of `org.apache.lucene.benchmark.byTask.feeds.LineDocSource.HeaderLineParser`; ported by rmp task #625.
pub struct HeaderLineParser;

/// Java: `private enum`.
///
/// Skeleton of `org.apache.lucene.benchmark.byTask.feeds.LineDocSource.HeaderLineParser.FieldName`; ported by rmp task #625.
pub(crate) enum FieldName {
    /// Java constant `NAME`.
    Name,
    /// Java constant `TITLE`.
    Title,
    /// Java constant `DATE`.
    Date,
    /// Java constant `BODY`.
    Body,
    /// Java constant `PROP`.
    Prop,
}
