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

//! The type declared in `BinaryDictionaryWriter.java`, of the Java package
//! `org.apache.lucene.analysis.morph`.

use rucene::codecs::codec_util::write_header;
use rucene::error::{LuceneError, Result};
use rucene::store::java_nio_file::Path;
use rucene::store::DataOutput;
use rucene::util::ArrayUtil;

use super::binary_dictionary::BinaryDictionary;
use super::dictionary_entry_writer::{
    create_parent_directories, write_file, write_pos_dict, DictionaryEntryWriterSubclass,
};

/// Abstract base dictionary writer class.
///
/// Equivalent to `org.apache.lucene.analysis.morph.BinaryDictionaryWriter<T
/// extends BinaryDictionary<? extends MorphData>>`, an abstract class: this
/// struct holds its fields, and a concrete writer holds a
/// `BinaryDictionaryWriter` and implements [`BinaryDictionaryWriterSubclass`].
/// Java's `Class<T> implClazz` is held as the class's binary name, the only
/// thing the writer reads from it.
pub struct BinaryDictionaryWriter {
    impl_class_name: String,
    target_map_end_offset: i32,
    last_word_id: i32,
    last_source_id: i32,
    target_map: Vec<i32>,
    target_map_offsets: Vec<i32>,
    /// Equivalent to the protected `BinaryDictionaryWriter.entryWriter`.
    pub entry_writer: Box<dyn DictionaryEntryWriterSubclass>,
}

impl std::fmt::Debug for BinaryDictionaryWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinaryDictionaryWriter")
            .field("impl_class_name", &self.impl_class_name)
            .field("target_map_end_offset", &self.target_map_end_offset)
            .field("last_word_id", &self.last_word_id)
            .field("last_source_id", &self.last_source_id)
            .finish_non_exhaustive()
    }
}

impl BinaryDictionaryWriter {
    /// Equivalent to the protected `BinaryDictionaryWriter(Class<T> implClazz,
    /// DictionaryEntryWriter entryWriter)`; `impl_class_name` is
    /// `implClazz.getName()`.
    pub fn new(
        impl_class_name: impl Into<String>,
        entry_writer: Box<dyn DictionaryEntryWriterSubclass>,
    ) -> Self {
        Self {
            impl_class_name: impl_class_name.into(),
            target_map_end_offset: 0,
            last_word_id: -1,
            last_source_id: -1,
            target_map: vec![0; 8192],
            target_map_offsets: vec![0; 8192],
            entry_writer,
        }
    }

    /// put the entry in map
    ///
    /// Returns current position of buffer, which will be wordId of next
    /// entry.
    ///
    /// Equivalent to `BinaryDictionaryWriter.put(String[])`.
    ///
    /// # Errors
    ///
    /// What the entry writer's `putEntry` raises.
    pub fn put(&mut self, entry: &[String]) -> Result<i32> {
        self.entry_writer.put_entry(entry)
    }

    /// Equivalent to the protected `BinaryDictionaryWriter.addMapping(int, int)`.
    ///
    /// # Errors
    ///
    /// `IllegalStateException` (`words out of order: …`, `source ids not in
    /// increasing order: …`) for out-of-order ids.
    pub fn add_mapping(&mut self, source_id: i32, word_id: i32) -> Result<()> {
        if word_id <= self.last_word_id {
            return Err(LuceneError::illegal_state(format!(
                "words out of order: {word_id} vs lastID: {}",
                self.last_word_id
            )));
        }
        if source_id > self.last_source_id {
            self.target_map_offsets =
                ArrayUtil::grow_int(&self.target_map_offsets, (source_id + 1) as usize);
            for i in self.last_source_id + 1..=source_id {
                self.target_map_offsets[i as usize] = self.target_map_end_offset;
            }
        } else if source_id != self.last_source_id {
            return Err(LuceneError::illegal_state(format!(
                "source ids not in increasing order: lastSourceId={} vs sourceId={source_id}",
                self.last_source_id
            )));
        }
        self.target_map =
            ArrayUtil::grow_int(&self.target_map, (self.target_map_end_offset + 1) as usize);
        self.target_map[self.target_map_end_offset as usize] = word_id;
        self.target_map_end_offset += 1;
        self.last_source_id = source_id;
        self.last_word_id = word_id;
        Ok(())
    }

    /// Write dictionary in file Dictionary format is: [Size of
    /// dictionary(int)], [entry:{left id(short)}{right id(short)}{word
    /// cost(short)}{length of pos info(short)}{pos info(char)}], [entry...],
    /// [entry...].....
    ///
    /// Equivalent to the protected `BinaryDictionaryWriter.write(Path, String,
    /// String, String, int)`: the dictionary (`$buffer.dat`), the POS
    /// dictionary (`$posDict.dat`) and the target map (`$targetMap.dat`).
    ///
    /// # Errors
    ///
    /// If an I/O error occurs writing the dictionary files.
    pub fn write_files(
        &mut self,
        base_dir: &Path,
        target_map_codec_header: &str,
        pos_dict_codec_header: &str,
        dict_codec_header: &str,
        dict_codec_version: i32,
    ) -> Result<()> {
        let base_name = self.get_base_file_name();
        self.entry_writer.base_mut().write_dictionary(
            &base_dir.resolve_str(&format!(
                "{base_name}{}",
                BinaryDictionary::DICT_FILENAME_SUFFIX
            )),
            dict_codec_header,
            dict_codec_version,
        )?;
        write_pos_dict(
            self.entry_writer.as_mut(),
            &base_dir.resolve_str(&format!(
                "{base_name}{}",
                BinaryDictionary::POSDICT_FILENAME_SUFFIX
            )),
            pos_dict_codec_header,
            dict_codec_version,
        )?;
        self.write_target_map(
            &base_dir.resolve_str(&format!(
                "{base_name}{}",
                BinaryDictionary::TARGETMAP_FILENAME_SUFFIX
            )),
            target_map_codec_header,
            dict_codec_version,
        )
    }

    /// Equivalent to the protected final `BinaryDictionaryWriter.getBaseFileName()`:
    /// `implClazz.getName().replace('.', '/')`.
    pub fn get_base_file_name(&self) -> String {
        self.impl_class_name.replace('.', "/")
    }

    // TODO: maybe this int[] should instead be the output to the FST...
    /// Equivalent to the private `BinaryDictionaryWriter.writeTargetMap(Path, String, int)`.
    fn write_target_map(
        &self,
        path: &Path,
        target_map_codec_header: &str,
        dict_codec_version: i32,
    ) -> Result<()> {
        create_parent_directories(path)?;
        write_file(path, |out| {
            write_header(out, target_map_codec_header, dict_codec_version)?;
            let num_source_ids = self.last_source_id + 1;
            out.write_v_int(self.target_map_end_offset)?; // <-- size of main array
            out.write_v_int(num_source_ids + 1)?; // <-- size of offset array (+ 1 more entry)
            let mut prev: i32 = 0;
            let mut source_id: usize = 0;
            for ofs in 0..self.target_map_end_offset {
                let val = self.target_map[ofs as usize];
                let delta = val.wrapping_sub(prev);
                if ofs == self.target_map_offsets[source_id] {
                    out.write_v_int((delta << 1) | 0x01)?;
                    source_id += 1;
                } else {
                    out.write_v_int(delta << 1)?;
                }
                prev = prev.wrapping_add(delta);
            }
            if source_id as i32 != num_source_ids {
                return Err(LuceneError::illegal_state(format!(
                    "sourceId:{source_id} != numSourceIds:{num_source_ids}"
                )));
            }
            Ok(())
        })
    }
}

/// A concrete dictionary writer: a subclass of `BinaryDictionaryWriter`.
pub trait BinaryDictionaryWriterSubclass {
    /// The `BinaryDictionaryWriter` this writer extends.
    fn base(&self) -> &BinaryDictionaryWriter;

    /// The `BinaryDictionaryWriter` this writer extends, for writing.
    fn base_mut(&mut self) -> &mut BinaryDictionaryWriter;

    /// Write whole dictionary in a directory.
    ///
    /// Equivalent to the abstract `BinaryDictionaryWriter.write(Path baseDir)`.
    ///
    /// # Errors
    ///
    /// If an I/O error occurs writing the dictionary files.
    fn write(&mut self, base_dir: &Path) -> Result<()>;
}
