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

//! Port of the test classes of Lucene 10.5.0 `lucene/facet/src/test`
//! (rmp #630), each run as a suite by the ported runner.

mod facet_support;

mod taxonomy {
    pub mod test_lru_hash_map;
    pub mod writercache {
        pub mod test_2gb_char_block_array;
        pub mod test_char_block_array;
        pub mod test_lru_taxonomy_writer_cache;
    }
}

use facet_support::core_support::run;

macro_rules! suites {
    ($($name:ident => $path:path),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run($path);
            }
        )*
    };
}

suites! {
    test_lru_hash_map => taxonomy::test_lru_hash_map::class_def,
    test_2gb_char_block_array => taxonomy::writercache::test_2gb_char_block_array::class_def,
    test_char_block_array => taxonomy::writercache::test_char_block_array::class_def,
    test_lru_taxonomy_writer_cache => taxonomy::writercache::test_lru_taxonomy_writer_cache::class_def,
}
