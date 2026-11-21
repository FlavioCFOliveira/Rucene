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

//! The type declared in `CharacterDefinitionWriter.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use rucene::codecs::codec_util::write_header;
use rucene::error::Result;
use rucene::store::java_nio_file::Path;
use rucene::store::DataOutput;

use super::character_definition::{CharacterDefinition, LookupCharacterClass};
use super::dictionary_entry_writer::{create_parent_directories, write_file};

/// Writes character definition file
///
/// Equivalent to `org.apache.lucene.analysis.morph.CharacterDefinitionWriter<T
/// extends CharacterDefinition>`; Java's `Class<T> implClazz` is held as the
/// class's binary name, the only thing the writer reads from it.
pub struct CharacterDefinitionWriter {
    impl_class_name: String,
    character_category_map: Vec<i8>,
    class_count: usize,
    invoke_map: Vec<bool>,
    group_map: Vec<bool>,
    lookup_char_class: Box<dyn LookupCharacterClass>,
}

impl std::fmt::Debug for CharacterDefinitionWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CharacterDefinitionWriter")
            .field("impl_class_name", &self.impl_class_name)
            .field("class_count", &self.class_count)
            .field("invoke_map", &self.invoke_map)
            .field("group_map", &self.group_map)
            .finish_non_exhaustive()
    }
}

/// `String.split(" ")` of the JDK: the pieces between spaces, trailing empty
/// pieces removed; an empty string gives one empty piece.
fn java_split_space(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return vec![s];
    }
    let mut parts: Vec<&str> = s.split(' ').collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

impl CharacterDefinitionWriter {
    /// Constructor for building. TODO: remove write access
    ///
    /// Equivalent to `CharacterDefinitionWriter(Class<T> implClazz, byte
    /// defaultValue, int classCount, CharacterDefinition.LookupCharacterClass
    /// lookupCharClass)`; `impl_class_name` is `implClazz.getName()`.
    pub fn new(
        impl_class_name: impl Into<String>,
        default_value: i8,
        class_count: usize,
        lookup_char_class: Box<dyn LookupCharacterClass>,
    ) -> Self {
        Self {
            impl_class_name: impl_class_name.into(),
            character_category_map: vec![default_value; 0x10000],
            invoke_map: vec![false; class_count],
            group_map: vec![false; class_count],
            class_count,
            lookup_char_class,
        }
    }

    /// Put mapping from unicode code point to character class.
    ///
    /// Equivalent to `CharacterDefinitionWriter.putCharacterCategory(int codePoint,
    /// String characterClassName)`.
    ///
    /// # Panics
    ///
    /// For a code point above U+FFFF or a name of spaces only, where Java
    /// throws `ArrayIndexOutOfBoundsException`.
    pub fn put_character_category(&mut self, code_point: i32, character_class_name: &str) {
        let mut character_class_name = java_split_space(character_class_name)[0]; // use first
                                                                                  // category
                                                                                  // class

        // Override Nakaguro
        if code_point == 0x30FB {
            character_class_name = "SYMBOL";
        }
        self.character_category_map[code_point as usize] = self
            .lookup_char_class
            .lookup_character_class(character_class_name);
    }

    /// Equivalent to `CharacterDefinitionWriter.putInvokeDefinition(String
    /// characterClassName, int invoke, int group, int length)`; as in Java,
    /// `length` is ignored.
    ///
    /// # Panics
    ///
    /// For a class outside the maps, where Java throws
    /// `ArrayIndexOutOfBoundsException`.
    pub fn put_invoke_definition(
        &mut self,
        character_class_name: &str,
        invoke: i32,
        group: i32,
        _length: i32,
    ) {
        let character_class = self
            .lookup_char_class
            .lookup_character_class(character_class_name);
        let index = usize::try_from(character_class).unwrap_or_else(|_| {
            panic!(
                "java.lang.ArrayIndexOutOfBoundsException: Index {character_class} out of bounds"
            )
        });
        self.invoke_map[index] = invoke == 1;
        self.group_map[index] = group == 1;
        // TODO: length def ignored
    }

    /// Equivalent to the private `CharacterDefinitionWriter.getBaseFileName()`.
    fn get_base_file_name(&self) -> String {
        self.impl_class_name.replace('.', "/")
    }

    /// Equivalent to `CharacterDefinitionWriter.write(Path baseDir, String
    /// charDefCodecHeader, int charDefCodecVersion)`.
    ///
    /// # Errors
    ///
    /// What creating and writing the file raise.
    pub fn write(
        &self,
        base_dir: &Path,
        char_def_codec_header: &str,
        char_def_codec_version: i32,
    ) -> Result<()> {
        let path = base_dir.resolve_str(&format!(
            "{}{}",
            self.get_base_file_name(),
            CharacterDefinition::FILENAME_SUFFIX
        ));
        create_parent_directories(&path)?;
        write_file(&path, |out| {
            write_header(out, char_def_codec_header, char_def_codec_version)?;
            let bytes: Vec<u8> = self
                .character_category_map
                .iter()
                .map(|&b| b as u8)
                .collect();
            out.write_bytes(&bytes, 0, bytes.len())?;
            for i in 0..self.class_count {
                let b = (if self.invoke_map[i] { 0x01 } else { 0x00 })
                    | (if self.group_map[i] { 0x02 } else { 0x00 });
                out.write_byte(b)?;
            }
            Ok(())
        })
    }
}
