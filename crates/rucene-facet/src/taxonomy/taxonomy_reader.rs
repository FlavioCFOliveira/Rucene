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

//! Port of `TaxonomyReader.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use rucene::error::{LuceneError, Result};

use crate::taxonomy::facet_label::FacetLabel;
use crate::taxonomy::parallel_taxonomy_arrays::ParallelTaxonomyArrays;

/// The root category (the category with the empty path) always has the
/// ordinal 0.
///
/// Equivalent to `TaxonomyReader.ROOT_ORDINAL`.
pub const ROOT_ORDINAL: i32 = 0;

/// Ordinals are always non-negative, so a negative ordinal can be used to
/// signify an error.
///
/// Equivalent to `TaxonomyReader.INVALID_ORDINAL`.
pub const INVALID_ORDINAL: i32 = -1;

/// An iterator over a category's children.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyReader.ChildrenIterator`.
/// Java keeps the `siblings` array; the port keeps the arrays object that
/// owns it.
pub struct ChildrenIterator {
    arrays: Arc<dyn ParallelTaxonomyArrays>,
    child: i32,
}

impl ChildrenIterator {
    /// Equivalent to `ChildrenIterator(int, IntArray)`.
    pub(crate) fn new(child: i32, arrays: Arc<dyn ParallelTaxonomyArrays>) -> Self {
        Self { arrays, child }
    }

    /// Return the next child ordinal, or [`INVALID_ORDINAL`] if no more
    /// children.
    ///
    /// Equivalent to `next()` (Java's `ChildrenIterator.next()`, not an
    /// `Iterator`: `INVALID_ORDINAL` ends it).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> i32 {
        let res = self.child;
        if self.child != INVALID_ORDINAL {
            self.child = self.arrays.siblings().get(self.child);
        }
        res
    }
}

/// The reference-counting state of `TaxonomyReader`: its `closed` flag and
/// `refCount`.
#[derive(Debug)]
pub struct TaxonomyReaderBase {
    closed: AtomicBool,
    ref_count: AtomicI32,
    /// The monitor of `close()`'s `synchronized (this)`.
    close_lock: Mutex<()>,
}

impl Default for TaxonomyReaderBase {
    fn default() -> Self {
        Self {
            closed: AtomicBool::new(false),
            ref_count: AtomicI32::new(1),
            close_lock: Mutex::new(()),
        }
    }
}

/// `TaxonomyReader` is the read-only interface with which the
/// faceted-search library uses the taxonomy during search time.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyReader`. Its
/// `final` reference-counting methods are provided methods over
/// [`taxonomy_reader_base`](Self::taxonomy_reader_base).
pub trait TaxonomyReader: Send + Sync {
    /// The reference-counting state.
    fn taxonomy_reader_base(&self) -> &TaxonomyReaderBase;

    /// This reader as [`Any`], for Java's casts to a concrete reader.
    fn as_any(&self) -> &dyn Any;

    /// Performs the actual task of closing the resources that are used by the
    /// taxonomy reader.
    ///
    /// Equivalent to `doClose()`.
    fn do_close(&self) -> Result<()>;

    /// Implements the actual opening of a new `TaxonomyReader` instance if
    /// the taxonomy has changed; `None` when it has not.
    ///
    /// Equivalent to `doOpenIfChanged()`.
    fn do_open_if_changed(&self) -> Result<Option<Arc<dyn TaxonomyReader>>>;

    /// Returns a [`ParallelTaxonomyArrays`] object which can be used to
    /// efficiently traverse the taxonomy tree.
    ///
    /// Equivalent to `getParallelTaxonomyArrays()`.
    fn get_parallel_taxonomy_arrays(&self) -> Result<Arc<dyn ParallelTaxonomyArrays>>;

    /// Returns an iterator over the children of the given ordinal.
    ///
    /// Equivalent to `getChildren(int)`.
    fn get_children(&self, ordinal: i32) -> Result<ChildrenIterator> {
        let arrays = self.get_parallel_taxonomy_arrays()?;
        let child = if ordinal >= 0 {
            arrays.children().get(ordinal)
        } else {
            INVALID_ORDINAL
        };
        Ok(ChildrenIterator::new(child, arrays))
    }

    /// Retrieve user committed data.
    ///
    /// Equivalent to `getCommitUserData()`.
    fn get_commit_user_data(&self) -> Result<HashMap<String, String>>;

    /// Returns the ordinal of the category given as a path, or
    /// [`INVALID_ORDINAL`] if the category cannot be found.
    ///
    /// Equivalent to `getOrdinal(FacetLabel)`.
    fn get_ordinal(&self, category_path: &FacetLabel) -> Result<i32>;

    /// Returns the ordinals of the categories given as paths.
    ///
    /// Equivalent to `getBulkOrdinals(FacetLabel...)`.
    fn get_bulk_ordinals(&self, category_path: &[FacetLabel]) -> Result<Vec<i32>> {
        let mut ords = vec![0; category_path.len()];
        for (i, path) in category_path.iter().enumerate() {
            ords[i] = self.get_ordinal(path)?;
        }
        Ok(ords)
    }

    /// Returns ordinal for the dim + path.
    ///
    /// Equivalent to `getOrdinal(String, String...)`.
    fn get_ordinal_for(&self, dim: &str, path: &[&str]) -> Result<i32> {
        let mut full_path = Vec::with_capacity(path.len() + 1);
        full_path.push(dim.to_string());
        full_path.extend(path.iter().map(|p| p.to_string()));
        self.get_ordinal(&FacetLabel::new(full_path)?)
    }

    /// Returns the path name of the category with the given ordinal.
    ///
    /// Equivalent to `getPath(int)`; Java's `null` result is `None`.
    fn get_path(&self, ordinal: i32) -> Result<Option<FacetLabel>>;

    /// Returns the path names of the list of ordinals associated with
    /// different categories. As in Java, an implementation may reorder the
    /// given ordinals.
    ///
    /// Equivalent to `getBulkPath(int...)`.
    fn get_bulk_path(&self, ordinals: &mut [i32]) -> Result<Vec<Option<FacetLabel>>> {
        let mut facet_labels = Vec::with_capacity(ordinals.len());
        for &ordinal in ordinals.iter() {
            facet_labels.push(self.get_path(ordinal)?);
        }
        Ok(facet_labels)
    }

    /// Returns the current refCount for this taxonomy reader.
    ///
    /// Equivalent to `getRefCount()`.
    fn get_ref_count(&self) -> i32 {
        self.taxonomy_reader_base().ref_count.load(Ordering::SeqCst)
    }

    /// Returns the number of categories in the taxonomy.
    ///
    /// Equivalent to `getSize()`.
    fn get_size(&self) -> Result<i32>;

    /// Throws `AlreadyClosedException` if this `IndexReader` is closed.
    ///
    /// Equivalent to `ensureOpen()`.
    fn ensure_open(&self) -> Result<()> {
        if self.get_ref_count() <= 0 {
            return Err(LuceneError::already_closed("this TaxonomyReader is closed"));
        }
        Ok(())
    }

    /// Equivalent to `close()`.
    fn close(&self) -> Result<()> {
        let base = self.taxonomy_reader_base();
        if !base.closed.load(Ordering::SeqCst) {
            let _guard = base
                .close_lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !base.closed.load(Ordering::SeqCst) {
                self.dec_ref()?;
                base.closed.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }

    /// Expert: decreases the refCount of this TaxonomyReader instance. If the
    /// refCount drops to 0 this taxonomy reader is closed.
    ///
    /// Equivalent to `decRef()`.
    fn dec_ref(&self) -> Result<()> {
        self.ensure_open()?;
        let base = self.taxonomy_reader_base();
        let rc = base.ref_count.fetch_sub(1, Ordering::SeqCst) - 1;
        if rc == 0 {
            match self.do_close() {
                Ok(()) => {
                    base.closed.store(true, Ordering::SeqCst);
                }
                Err(e) => {
                    // Put reference back on failure
                    base.ref_count.fetch_add(1, Ordering::SeqCst);
                    return Err(e);
                }
            }
        } else if rc < 0 {
            return Err(LuceneError::illegal_state(format!(
                "too many decRef calls: refCount is {rc} after decrement"
            )));
        }
        Ok(())
    }

    /// Expert: increments the refCount of this TaxonomyReader instance.
    ///
    /// Equivalent to `incRef()`.
    fn inc_ref(&self) -> Result<()> {
        self.ensure_open()?;
        self.taxonomy_reader_base()
            .ref_count
            .fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Expert: increments the refCount of this TaxonomyReader instance only if
    /// it has not been closed yet.
    ///
    /// Equivalent to `tryIncRef()`.
    fn try_inc_ref(&self) -> bool {
        let rc = &self.taxonomy_reader_base().ref_count;
        let mut count = rc.load(Ordering::SeqCst);
        while count > 0 {
            match rc.compare_exchange(count, count + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return true,
                Err(actual) => count = actual,
            }
        }
        false
    }
}

/// If the taxonomy has changed since the provided reader was opened, open and
/// return a new `TaxonomyReader`; else, return `None`.
///
/// Equivalent to `TaxonomyReader.openIfChanged(T)`.
pub fn open_if_changed(
    old_taxo_reader: &Arc<dyn TaxonomyReader>,
) -> Result<Option<Arc<dyn TaxonomyReader>>> {
    let new_taxo_reader = old_taxo_reader.do_open_if_changed()?;
    debug_assert!(new_taxo_reader
        .as_ref()
        .is_none_or(|r| !std::ptr::addr_eq(Arc::as_ptr(r), Arc::as_ptr(old_taxo_reader))));
    Ok(new_taxo_reader)
}
