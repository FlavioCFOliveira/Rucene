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

//! Skeletons of the types declared in `QueryParser.java`, of the Java
//! package `org.apache.lucene.queryparser.classic`.

/// Java: `public class`.
///
/// Skeleton of `org.apache.lucene.queryparser.classic.QueryParser`; ported by rmp task #484.
pub struct QueryParser;

/// Java: `public static enum`.
///
/// Skeleton of `org.apache.lucene.queryparser.classic.QueryParser.Operator`; ported by rmp task #484.
pub enum Operator {
    /// Java constant `OR`.
    Or,
    /// Java constant `AND`.
    And,
}

/// Java: `private static final class`.
///
/// Skeleton of `org.apache.lucene.queryparser.classic.QueryParser.LookaheadSuccess`; ported by rmp task #484.
pub(crate) struct LookaheadSuccess;

/// Java: `static final class` (package-private).
///
/// Skeleton of `org.apache.lucene.queryparser.classic.QueryParser.JJCalls`; ported by rmp task #484.
pub(crate) struct JJCalls;
