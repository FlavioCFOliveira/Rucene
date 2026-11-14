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

//! The types declared in `CharacterDefinition.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use std::io::Read;

use rucene::codecs::codec_util::check_header;
use rucene::error::Result;
use rucene::store::{DataInput, InputStreamDataInput};
use rucene::util::IOSupplier;

/// Character category data.
///
/// Equivalent to `org.apache.lucene.analysis.morph.CharacterDefinition`, an
/// abstract class without abstract methods: a concrete definition holds a
/// `CharacterDefinition`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterDefinition {
    /// Equivalent to the protected `CharacterDefinition.characterCategoryMap`
    /// (`byte[0x10000]`).
    pub character_category_map: Vec<i8>,
    invoke_map: Vec<bool>,
    group_map: Vec<bool>,
}

impl CharacterDefinition {
    /// Equivalent to `CharacterDefinition.FILENAME_SUFFIX`.
    pub const FILENAME_SUFFIX: &'static str = ".dat";

    /// Equivalent to the protected `CharacterDefinition(IOSupplier<InputStream>
    /// charDefResource, String charDefCodecHeader, int charDefCodecVersion,
    /// int classCount)`.
    ///
    /// # Errors
    ///
    /// What the resource and `CodecUtil.checkHeader` raise, and
    /// `EOFException` for a short file.
    pub fn new(
        char_def_resource: &mut dyn IOSupplier<Box<dyn Read + Send>>,
        char_def_codec_header: &str,
        char_def_codec_version: i32,
        class_count: usize,
    ) -> Result<Self> {
        let is = char_def_resource.get()?;
        let mut input = InputStreamDataInput::new(is);
        check_header(
            &mut input,
            char_def_codec_header,
            char_def_codec_version,
            char_def_codec_version,
        )?;
        let mut map = vec![0u8; 0x10000];
        let len = map.len();
        input.read_bytes(Some(&mut map), 0, len)?;
        let mut invoke_map = vec![false; class_count];
        let mut group_map = vec![false; class_count];
        for i in 0..class_count {
            let b = input.read_byte()?;
            invoke_map[i] = (b & 0x01) != 0;
            group_map[i] = (b & 0x02) != 0;
        }
        Ok(Self {
            character_category_map: map.into_iter().map(|b| b as i8).collect(),
            invoke_map,
            group_map,
        })
    }

    /// Equivalent to `CharacterDefinition.getCharacterClass(char)`.
    pub fn get_character_class(&self, c: u16) -> i8 {
        self.character_category_map[c as usize]
    }

    /// Equivalent to `CharacterDefinition.isInvoke(char)`.
    ///
    /// # Panics
    ///
    /// For a class outside the map, where Java throws
    /// `ArrayIndexOutOfBoundsException`.
    pub fn is_invoke(&self, c: u16) -> bool {
        self.invoke_map[class_index(self.character_category_map[c as usize])]
    }

    /// Equivalent to `CharacterDefinition.isGroup(char)`.
    ///
    /// # Panics
    ///
    /// For a class outside the map, where Java throws
    /// `ArrayIndexOutOfBoundsException`.
    pub fn is_group(&self, c: u16) -> bool {
        self.group_map[class_index(self.character_category_map[c as usize])]
    }
}

/// A class byte as an array index: a negative one panics, as Java's
/// `ArrayIndexOutOfBoundsException`.
fn class_index(class: i8) -> usize {
    usize::try_from(class).unwrap_or_else(|_| {
        panic!("java.lang.ArrayIndexOutOfBoundsException: Index {class} out of bounds")
    })
}

/// Functional interface to lookup character class
///
/// Equivalent to `org.apache.lucene.analysis.morph.CharacterDefinition$LookupCharacterClass`;
/// a closure `Fn(&str) -> i8` implements it.
pub trait LookupCharacterClass: Send + Sync {
    /// looks up character class for given class name
    ///
    /// Equivalent to `LookupCharacterClass.lookupCharacterClass(String)`.
    fn lookup_character_class(&self, character_class_name: &str) -> i8;
}

impl<F: Fn(&str) -> i8 + Send + Sync> LookupCharacterClass for F {
    fn lookup_character_class(&self, character_class_name: &str) -> i8 {
        self(character_class_name)
    }
}
