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

//! The type declared in `MorphData.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

/// High-level interface that represents morphological information in a
/// dictionary
///
/// Equivalent to `org.apache.lucene.analysis.morph.MorphData`.
pub trait MorphData: Send + Sync {
    /// Get left id of specified word
    ///
    /// Equivalent to `MorphData.getLeftId(int)`.
    fn get_left_id(&self, morph_id: i32) -> i32;

    /// Get right id of specified word
    ///
    /// Equivalent to `MorphData.getRightId(int)`.
    fn get_right_id(&self, morph_id: i32) -> i32;

    /// Get word cost of specified word
    ///
    /// Equivalent to `MorphData.getWordCost(int)`.
    fn get_word_cost(&self, morph_id: i32) -> i32;
}
