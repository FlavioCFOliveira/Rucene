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

//! Port of the test classes of Lucene 10.5.0
//! `lucene/queryparser/src/test/org/apache/lucene/queryparser/surround/query/`
//! that rmp task #492 lists, each run as a suite by the ported runner:
//! `Test01Exceptions`, `Test02Boolean`, `Test03Distance`, `TestSrndQuery`;
//! the test helpers `BooleanQueryTestFacade`, `ExceptionQueryTestFacade` and
//! `SingleFieldTestDb`.

mod qp_support;

mod surround {
    pub mod boolean_query_test_facade;
    pub mod exception_query_test_facade;
    pub mod single_field_test_db;
    pub mod test01_exceptions;
    pub mod test02_boolean;
    pub mod test03_distance;
    pub mod test_srnd_query;
}

use qp_support::core_support::run;

#[test]
fn test01_exceptions() {
    run(surround::test01_exceptions::class_def);
}

#[test]
fn test02_boolean() {
    run(surround::test02_boolean::class_def);
}

#[test]
fn test03_distance() {
    run(surround::test03_distance::class_def);
}

#[test]
fn test_srnd_query() {
    run(surround::test_srnd_query::class_def);
}
