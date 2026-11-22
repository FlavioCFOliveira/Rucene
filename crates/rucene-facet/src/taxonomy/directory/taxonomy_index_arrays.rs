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

//! Port of `TaxonomyIndexArrays.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy.directory`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, OnceLock};

use rucene::error::{LuceneError, Result};
use rucene::index::IndexReader;
use rucene::internal::hppc::IntHashSet;
use rucene::util::{Accountable, ArrayUtil, RamUsageEstimator};

use crate::taxonomy::directory::consts::Consts;
use crate::taxonomy::parallel_taxonomy_arrays::{IntArray, ParallelTaxonomyArrays};
use crate::taxonomy::taxonomy_reader::INVALID_ORDINAL;

const CHUNK_SIZE_BITS: u32 = 13;
/// Equivalent to `TaxonomyIndexArrays.CHUNK_SIZE`.
pub(crate) const CHUNK_SIZE: usize = 1 << CHUNK_SIZE_BITS;
const CHUNK_MASK: usize = CHUNK_SIZE - 1;

/// One chunk of a [`ChunkedIntArray`]: Java's `int[]`, which several arrays
/// may share by reference and write through; atomics make the shared writes
/// expressible.
type Chunk = Arc<[AtomicI32]>;

fn new_chunk(len: usize) -> Chunk {
    (0..len).map(|_| AtomicI32::new(0)).collect()
}

/// Equivalent to
/// `org.apache.lucene.facet.taxonomy.directory.TaxonomyIndexArrays.ChunkedIntArray`.
#[derive(Debug)]
pub(crate) struct ChunkedIntArray {
    pub(crate) values: Vec<Chunk>,
}

impl ChunkedIntArray {
    fn new(values: Vec<Chunk>) -> Self {
        Self { values }
    }

    /// Equivalent to `set(int, int)`.
    pub(crate) fn set(&self, i: i32, val: i32) {
        let i = i as usize;
        self.values[i >> CHUNK_SIZE_BITS][i & CHUNK_MASK].store(val, Ordering::Relaxed);
    }
}

impl IntArray for ChunkedIntArray {
    fn get(&self, i: i32) -> i32 {
        let i = i as usize;
        self.values[i >> CHUNK_SIZE_BITS][i & CHUNK_MASK].load(Ordering::Relaxed)
    }

    fn length(&self) -> i32 {
        (((self.values.len() - 1) << CHUNK_SIZE_BITS) + self.values[self.values.len() - 1].len())
            as i32
    }
}

/// A `ParallelTaxonomyArrays` that are initialized from the taxonomy index.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.directory.TaxonomyIndexArrays`.
/// The lazily computed `children`/`siblings` pair Java guards with a
/// `volatile` flag and `synchronized` is a [`OnceLock`].
#[derive(Debug)]
pub(crate) struct TaxonomyIndexArrays {
    parents: ChunkedIntArray,
    children_siblings: OnceLock<(ChunkedIntArray, ChunkedIntArray)>,
}

/// Equivalent to `allocateChunkedArray(int, int)`; chunks below `start_from`
/// are left for the caller to fill, as Java leaves them `null`.
fn allocate_chunked_array(size: usize, start_from: usize) -> Vec<Option<Chunk>> {
    let chunk_count = (size >> CHUNK_SIZE_BITS) + 1;
    let mut array: Vec<Option<Chunk>> = vec![None; chunk_count];
    for slot in array.iter_mut().take(chunk_count - 1).skip(start_from) {
        *slot = Some(new_chunk(CHUNK_SIZE));
    }
    array[chunk_count - 1] = Some(new_chunk(size & CHUNK_MASK));
    array
}

/// Equivalent to `copyChunkedArray(int[][], int[][])`: shares all but the
/// last chunk, whose values are copied into the new last-copied chunk.
fn copy_chunked_array(old_array: &[Chunk], new_array: &mut [Option<Chunk>]) {
    if old_array.len() > 1 {
        for i in 0..old_array.len() - 1 {
            new_array[i] = Some(Arc::clone(&old_array[i]));
        }
    }
    let last_copy_chunk = &old_array[old_array.len() - 1];
    let target = new_array[old_array.len() - 1]
        .as_ref()
        .expect("INVARIANT: the chunk the copy lands in is allocated");
    for (i, v) in last_copy_chunk.iter().enumerate() {
        target[i].store(v.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}

fn finish(array: Vec<Option<Chunk>>) -> Vec<Chunk> {
    array
        .into_iter()
        .map(|chunk| chunk.expect("INVARIANT: every chunk is allocated or copied"))
        .collect()
}

impl TaxonomyIndexArrays {
    /// Equivalent to `TaxonomyIndexArrays(int[][])`.
    fn from_parents(parents: Vec<Chunk>) -> Self {
        Self {
            parents: ChunkedIntArray::new(parents),
            children_siblings: OnceLock::new(),
        }
    }

    /// Equivalent to `TaxonomyIndexArrays(IndexReader)`.
    pub(crate) fn new(reader: &Arc<dyn IndexReader>) -> Result<Self> {
        let mut parent_array = allocate_chunked_array(reader.max_doc() as usize, 0);
        debug_assert!(!parent_array.is_empty());
        let first_len = parent_array[0].as_ref().map_or(0, |c| c.len());
        let parent_array = if first_len > 0 {
            let array = finish(std::mem::take(&mut parent_array));
            init_parents(&array, reader, 0)?;
            // The root category has no parent.
            array[0][0].store(INVALID_ORDINAL, Ordering::Relaxed);
            array
        } else {
            finish(parent_array)
        };
        Ok(Self::from_parents(parent_array))
    }

    /// Equivalent to `TaxonomyIndexArrays(IndexReader, TaxonomyIndexArrays)`.
    pub(crate) fn with_copy_from(
        reader: &Arc<dyn IndexReader>,
        copy_from: &TaxonomyIndexArrays,
    ) -> Result<Self> {
        let mut parent_array = allocate_chunked_array(
            reader.max_doc() as usize,
            copy_from.parents.values.len() - 1,
        );
        debug_assert!(!parent_array.is_empty());
        copy_chunked_array(&copy_from.parents.values, &mut parent_array);
        let parent_array = finish(parent_array);
        init_parents(&parent_array, reader, copy_from.parents.length())?;
        let arrays = Self::from_parents(parent_array);
        if copy_from.children_siblings.get().is_some() {
            arrays.init_children_siblings(Some(copy_from));
        }
        Ok(arrays)
    }

    /// Equivalent to `initChildrenSiblings(TaxonomyIndexArrays)`.
    fn init_children_siblings(&self, copy_from: Option<&TaxonomyIndexArrays>) {
        self.children_siblings.get_or_init(|| {
            let start_from = match copy_from {
                None => 0,
                Some(copy_from) => copy_from.parents.values.len() - 1,
            };
            let parents_length = self.parents.length() as usize;
            let mut children_array = allocate_chunked_array(parents_length, start_from);
            let mut siblings_array = allocate_chunked_array(parents_length, start_from);
            let first = if let Some(copy_from) = copy_from {
                let (children, siblings) = copy_from
                    .children_siblings
                    .get()
                    .expect("INVARIANT: the copied arrays have children");
                copy_chunked_array(&children.values, &mut children_array);
                copy_chunked_array(&siblings.values, &mut siblings_array);
                copy_from.parents.length()
            } else {
                0
            };
            let mut children = ChunkedIntArray::new(finish(children_array));
            let siblings = ChunkedIntArray::new(finish(siblings_array));
            self.compute_children_siblings(&mut children, &siblings, first);
            (children, siblings)
        });
    }

    /// Equivalent to `computeChildrenSiblings(int)`.
    fn compute_children_siblings(
        &self,
        children: &mut ChunkedIntArray,
        siblings: &ChunkedIntArray,
        mut first: i32,
    ) {
        // reset the youngest child of all ordinals. while this should be done
        // only for the leaves, we don't know up front which are the leaves, so
        // we reset all of them.
        let length = self.parents.length();
        for i in first..length {
            children.set(i, INVALID_ORDINAL);
        }

        // the root category has no parent, and therefore no siblings
        if first == 0 {
            first = 1;
            siblings.set(0, INVALID_ORDINAL);
        }

        // To avoid modifying the shared chunks of the copied arrays, a child
        // chunk is reallocated before the first write into it.
        let first_chunk_start = first - (first & CHUNK_MASK as i32);
        let mut reallocated_child_chunks = IntHashSet::new();
        for i in first..length {
            let parent = self.parents.get(i);
            // The existing youngest child of the parent is the next older
            // sibling of i. note that parents[i] is always < i, so the right-
            // hand-side of the following line is already set when we get here
            siblings.set(i, children.get(parent));
            if parent < first_chunk_start {
                let chunk_idx = (parent >> CHUNK_SIZE_BITS) as usize;
                if !reallocated_child_chunks.contains(chunk_idx as i32) {
                    reallocated_child_chunks.add(chunk_idx as i32);
                    let old_children = &children.values[chunk_idx];
                    let new = new_chunk(CHUNK_SIZE);
                    for (j, v) in old_children.iter().enumerate() {
                        new[j].store(v.load(Ordering::Relaxed), Ordering::Relaxed);
                    }
                    children.values[chunk_idx] = new;
                }
            }
            children.set(parent, i);
        }
    }

    /// Adds the given ordinal/parent info and returns either a new instance
    /// if the underlying array had to grow, or this instance otherwise.
    ///
    /// Equivalent to `add(int, int)`.
    pub(crate) fn add(self: &Arc<Self>, ordinal: i32, parent_ordinal: i32) -> Arc<Self> {
        if ordinal >= self.parents.length() {
            let mut new_parents = allocate_chunked_array(
                ArrayUtil::oversize(ordinal as usize + 1, 4),
                self.parents.values.len() - 1,
            );
            copy_chunked_array(&self.parents.values, &mut new_parents);
            let new_parents = finish(new_parents);
            let ordinal = ordinal as usize;
            new_parents[ordinal >> CHUNK_SIZE_BITS][ordinal & CHUNK_MASK]
                .store(parent_ordinal, Ordering::Relaxed);
            return Arc::new(Self::from_parents(new_parents));
        }
        self.parents.set(ordinal, parent_ordinal);
        Arc::clone(self)
    }

    /// The shallow size of a `ChunkedIntArray`: an object header and its one
    /// `values` reference.
    fn shallow_size_of_chunked() -> i64 {
        RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        )
    }
}

/// Equivalent to `initParents(int[][], IndexReader, int)`.
fn init_parents(
    parents_array: &[Chunk],
    reader: &Arc<dyn IndexReader>,
    first: i32,
) -> Result<()> {
    if reader.max_doc() == first {
        return Ok(());
    }
    for leaf_context in Arc::clone(reader).leaves()? {
        let leaf_reader = leaf_context.leaf_reader();
        let leaf_doc_num = leaf_reader.max_doc();
        let doc_base = leaf_context.doc_base();
        if doc_base + leaf_doc_num <= first {
            continue;
        }
        let Some(mut parent_values) =
            leaf_reader.get_numeric_doc_values(Consts::FIELD_PARENT_ORDINAL_NDV)?
        else {
            return Err(LuceneError::corrupt_index(
                format!(
                    "Parent data field {} does not exist",
                    Consts::FIELD_PARENT_ORDINAL_NDV
                ),
                leaf_reader.to_string(),
            ));
        };
        for doc in (first - doc_base).max(0)..leaf_doc_num {
            if !parent_values.advance_exact(doc)? {
                return Err(LuceneError::corrupt_index(
                    format!("Missing parent data for category {}", doc + doc_base),
                    reader.to_string(),
                ));
            }
            let pos = (doc + doc_base) as usize;
            let value = parent_values.long_value()?;
            // `Math.toIntExact`.
            let value = i32::try_from(value).map_err(|_| LuceneError::integer_overflow())?;
            parents_array[pos >> CHUNK_SIZE_BITS][pos & CHUNK_MASK].store(value, Ordering::Relaxed);
        }
    }
    Ok(())
}

impl ParallelTaxonomyArrays for TaxonomyIndexArrays {
    fn parents(&self) -> &dyn IntArray {
        &self.parents
    }

    fn children(&self) -> &dyn IntArray {
        if self.children_siblings.get().is_none() {
            self.init_children_siblings(None);
        }
        &self.children_siblings.get().expect("INVARIANT: initialized above").0
    }

    fn siblings(&self) -> &dyn IntArray {
        if self.children_siblings.get().is_none() {
            self.init_children_siblings(None);
        }
        &self.children_siblings.get().expect("INVARIANT: initialized above").1
    }
}

impl Accountable for TaxonomyIndexArrays {
    fn ram_bytes_used(&self) -> i64 {
        let mut ram_bytes_used = RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
            + 3 * RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + 1;
        ram_bytes_used += Self::shallow_size_of_chunked();
        if self.children_siblings.get().is_some() {
            ram_bytes_used += 2 * Self::shallow_size_of_chunked();
        }
        ram_bytes_used
    }

    fn child_resources(&self) -> Vec<Box<dyn Accountable + '_>> {
        let mut resources: Vec<Box<dyn Accountable + '_>> = Vec::new();
        resources.push(Box::new(rucene::util::Accountables::named_accountable(
            "parents",
            Self::shallow_size_of_chunked(),
        )));
        if self.children_siblings.get().is_some() {
            resources.push(Box::new(rucene::util::Accountables::named_accountable(
                "children",
                Self::shallow_size_of_chunked(),
            )));
            resources.push(Box::new(rucene::util::Accountables::named_accountable(
                "siblings",
                Self::shallow_size_of_chunked(),
            )));
        }
        resources
    }
}
