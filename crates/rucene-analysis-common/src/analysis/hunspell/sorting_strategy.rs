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

//! How a dictionary's entries are sorted while loading, ported from
//! `org.apache.lucene.analysis.hunspell.SortingStrategy`.

use std::sync::Arc;

use rucene::codecs::codec_util;
use rucene::error::Result;
use rucene::store::{Directory, DEFAULT_IO_CONTEXT};
use rucene::util::bytes_ref_array::BytesRefIterator;
use rucene::util::{
    ByteSequencesReader, ByteSequencesWriter, IOUtils, OfflineSorter, OfflineSorterComparator,
};

use crate::jdk::charset::StandardCharset;

/// The strategy defining how a Hunspell dictionary should be loaded, with
/// different tradeoffs. The entries should be sorted in a special way, and
/// this can be done either in-memory (faster, but temporarily allocating
/// more memory) or using disk (slower, but not needing much memory).
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.SortingStrategy`, an
/// abstract class whose only subclasses are the anonymous ones of
/// [`offline`](Self::offline) and [`in_memory`](Self::in_memory).
#[derive(Clone)]
pub struct SortingStrategy {
    kind: Kind,
}

/// The anonymous subclass of a [`SortingStrategy`].
#[derive(Clone)]
enum Kind {
    Offline {
        temp_dir: Arc<dyn Directory>,
        temp_file_name_prefix: String,
    },
    InMemory,
}

impl std::fmt::Debug for SortingStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            Kind::Offline {
                temp_file_name_prefix,
                ..
            } => f
                .debug_struct("SortingStrategy::offline")
                .field("temp_file_name_prefix", temp_file_name_prefix)
                .finish_non_exhaustive(),
            Kind::InMemory => f.write_str("SortingStrategy::in_memory"),
        }
    }
}

/// Collects the entries of the `*.dic` files.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.SortingStrategy$EntryAccumulator`.
pub(crate) trait EntryAccumulator {
    /// Equivalent to `addEntry(String)`.
    ///
    /// # Errors
    ///
    /// What writing the entry raises.
    fn add_entry(&mut self, entry: &[u16]) -> Result<()>;

    /// Equivalent to `finishAndSort()`.
    ///
    /// # Errors
    ///
    /// What sorting raises.
    fn finish_and_sort(self: Box<Self>) -> Result<Box<dyn EntrySupplier>>;
}

/// Returns the sorted entries.
///
/// Equivalent to `org.apache.lucene.analysis.hunspell.SortingStrategy$EntrySupplier`,
/// a `Closeable`.
pub(crate) trait EntrySupplier {
    /// Equivalent to `wordCount()`.
    fn word_count(&self) -> i32;

    /// The next line or `None` if the end is reached.
    ///
    /// Equivalent to `next()`.
    ///
    /// # Errors
    ///
    /// What reading raises.
    fn next(&mut self) -> Result<Option<Vec<u16>>>;

    /// Equivalent to `close()`.
    ///
    /// # Errors
    ///
    /// What closing raises.
    fn close(&mut self) -> Result<()>;
}

impl SortingStrategy {
    /// Equivalent to the abstract `start()`.
    ///
    /// # Errors
    ///
    /// What creating the temporary output raises.
    pub(crate) fn start(&self) -> Result<Box<dyn EntryAccumulator>> {
        match &self.kind {
            Kind::Offline {
                temp_dir,
                temp_file_name_prefix,
            } => {
                let output = temp_dir.create_temp_output(
                    temp_file_name_prefix,
                    "dat",
                    &*DEFAULT_IO_CONTEXT,
                )?;
                let output_name = output.name().to_string();
                Ok(Box::new(OfflineAccumulator {
                    temp_dir: Arc::clone(temp_dir),
                    temp_file_name_prefix: temp_file_name_prefix.clone(),
                    output_name,
                    writer: ByteSequencesWriter::new(output),
                    word_count: 0,
                }))
            }
            Kind::InMemory => Ok(Box::new(InMemoryAccumulator {
                entries: Vec::new(),
            })),
        }
    }

    /// An "offline" strategy that creates temporary files in the given
    /// directory and uses them for sorting with [`OfflineSorter`]. It's
    /// slower than [`in_memory`](Self::in_memory), but doesn't need to load
    /// the entire dictionary into memory.
    ///
    /// Equivalent to `SortingStrategy.offline(Directory, String)`.
    pub fn offline(temp_dir: Arc<dyn Directory>, temp_file_name_prefix: &str) -> SortingStrategy {
        SortingStrategy {
            kind: Kind::Offline {
                temp_dir,
                temp_file_name_prefix: temp_file_name_prefix.to_string(),
            },
        }
    }

    /// The strategy that loads all entries as `String` objects and sorts
    /// them in memory. The entries are then stored in a more compressed way,
    /// and the strings are gc-ed, but the loading itself needs
    /// `O(dictionary_size)` memory.
    ///
    /// Equivalent to `SortingStrategy.inMemory()`.
    pub fn in_memory() -> SortingStrategy {
        SortingStrategy {
            kind: Kind::InMemory,
        }
    }
}

/// The accumulator of [`SortingStrategy::offline`]: the entries are written
/// as UTF-8 byte sequences to a temporary file.
struct OfflineAccumulator {
    temp_dir: Arc<dyn Directory>,
    temp_file_name_prefix: String,
    output_name: String,
    writer: ByteSequencesWriter,
    word_count: i32,
}

impl EntryAccumulator for OfflineAccumulator {
    fn add_entry(&mut self, entry: &[u16]) -> Result<()> {
        self.word_count += 1;
        self.writer.write(&StandardCharset::Utf8.encode(entry))
    }

    fn finish_and_sort(self: Box<Self>) -> Result<Box<dyn EntrySupplier>> {
        let OfflineAccumulator {
            temp_dir,
            temp_file_name_prefix,
            output_name,
            mut writer,
            word_count,
        } = *self;
        codec_util::write_footer(writer.out())?;
        writer.close()?;
        let sorted_file = sort_words_offline(&temp_dir, &temp_file_name_prefix, &output_name)?;
        let reader =
            ByteSequencesReader::new(temp_dir.open_checksum_input(&sorted_file)?, &sorted_file);
        Ok(Box::new(OfflineSupplier {
            temp_dir,
            sorted_file,
            reader,
            word_count,
            success: false,
        }))
    }
}

/// Equivalent to `sortWordsOffline()`: sorts the written file and deletes
/// it, ignoring a failure to delete when sorting failed.
fn sort_words_offline(
    temp_dir: &Arc<dyn Directory>,
    temp_file_name_prefix: &str,
    output_name: &str,
) -> Result<String> {
    let sorted = OfflineSorter::with_comparator(
        Arc::clone(temp_dir),
        temp_file_name_prefix,
        OfflineSorterComparator::Natural,
    )
    .and_then(|mut sorter| sorter.sort(output_name));
    match sorted {
        Ok(sorted) => {
            temp_dir.delete_file(output_name)?;
            Ok(sorted)
        }
        Err(e) => {
            IOUtils::delete_files_ignoring_exceptions_in(&**temp_dir, [output_name]);
            Err(e)
        }
    }
}

/// The supplier of [`SortingStrategy::offline`], reading the sorted file.
struct OfflineSupplier {
    temp_dir: Arc<dyn Directory>,
    sorted_file: String,
    reader: ByteSequencesReader,
    word_count: i32,
    success: bool,
}

impl EntrySupplier for OfflineSupplier {
    fn word_count(&self) -> i32 {
        self.word_count
    }

    fn next(&mut self) -> Result<Option<Vec<u16>>> {
        match self.reader.next()? {
            None => {
                self.success = true;
                Ok(None)
            }
            Some(scratch) => {
                // `scratch.utf8ToString()`
                let string = scratch.utf8_to_string()?;
                Ok(Some(string.encode_utf16().collect()))
            }
        }
    }

    fn close(&mut self) -> Result<()> {
        self.reader.close()?;
        if self.success {
            self.temp_dir.delete_file(&self.sorted_file)
        } else {
            IOUtils::delete_files_ignoring_exceptions_in(&*self.temp_dir, [&self.sorted_file]);
            Ok(())
        }
    }
}

/// The accumulator of [`SortingStrategy::in_memory`].
struct InMemoryAccumulator {
    entries: Vec<Vec<u16>>,
}

impl EntryAccumulator for InMemoryAccumulator {
    fn add_entry(&mut self, entry: &[u16]) -> Result<()> {
        self.entries.push(entry.to_vec());
        Ok(())
    }

    fn finish_and_sort(self: Box<Self>) -> Result<Box<dyn EntrySupplier>> {
        let mut entries = self.entries;
        // `Comparator.naturalOrder()` of `String`: UTF-16 unit order
        entries.sort();
        Ok(Box::new(InMemorySupplier { entries, i: 0 }))
    }
}

/// The supplier of [`SortingStrategy::in_memory`].
struct InMemorySupplier {
    entries: Vec<Vec<u16>>,
    i: usize,
}

impl EntrySupplier for InMemorySupplier {
    fn word_count(&self) -> i32 {
        self.entries.len() as i32
    }

    fn next(&mut self) -> Result<Option<Vec<u16>>> {
        if self.i < self.entries.len() {
            self.i += 1;
            Ok(Some(self.entries[self.i - 1].clone()))
        } else {
            Ok(None)
        }
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
