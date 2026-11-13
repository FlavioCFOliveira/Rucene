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

//! The Java package `org.apache.lucene.analysis.payloads`: provides various
//! convenience classes for creating payloads on tokens.

pub mod abstract_encoder;
pub mod delimited_payload_token_filter;
pub mod delimited_payload_token_filter_factory;
pub mod float_encoder;
pub mod identity_encoder;
pub mod integer_encoder;
pub mod numeric_payload_token_filter;
pub mod numeric_payload_token_filter_factory;
pub mod payload_encoder;
pub mod payload_helper;
pub mod token_offset_payload_token_filter;
pub mod token_offset_payload_token_filter_factory;
pub mod type_as_payload_token_filter;
pub mod type_as_payload_token_filter_factory;
