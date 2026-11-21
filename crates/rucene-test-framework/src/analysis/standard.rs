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

//! Port of `org.apache.lucene.tests.analysis.standard`: the Unicode test data
//! that `StandardTokenizer`'s tests run, generated from the Java classes by
//! `tools/gen/standard_unicode_tests.py`.

pub mod emoji_tokenization_test_unicode_12_1;
pub mod word_break_test_unicode_12_1_0;

pub use emoji_tokenization_test_unicode_12_1::EmojiTokenizationTestUnicode_12_1;
pub use word_break_test_unicode_12_1_0::WordBreakTestUnicode_12_1_0;
