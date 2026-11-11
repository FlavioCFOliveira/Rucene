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

//! Port of `org.apache.lucene.tests.index.PerThreadPKLookup`: a utility
//! class to do efficient primary-key (only 1 doc contains the given term)
//! lookups by segment, re-using the enums.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::index_reader::{CacheKey, IndexReader};
use rucene::index::{PostingsEnum, TermsEnum};
use rucene::search::NO_MORE_DOCS;
use rucene::util::{Bits, BytesRef};

/// A terms enum the lookups of a reader and of the lookups reopened from it
/// share, as Java's arrays share the enum objects.
type SharedTermsEnum = Rc<RefCell<Box<dyn TermsEnum>>>;

/// The postings enum slot of a segment, shared the same way: `lookup` hands
/// the enum in it to `postings` for reuse and stores the result back.
type SharedPostingsEnum = Rc<RefCell<Option<Box<dyn PostingsEnum>>>>;

/// Port of `org.apache.lucene.tests.index.PerThreadPKLookup`.
///
/// This class is not thread safe, so it is the caller's job to create and
/// use one instance of this per thread. Do not use this if a term may appear
/// in more than one document! It will only return the first one it finds.
///
/// The enums of a segment are shared with the lookups [`reopen`](Self::reopen)
/// builds over a reader with the same segment core, as Java shares the enum
/// objects between the old and the new arrays; the sharing makes the type
/// neither `Send` nor `Sync` (the port's enums are not `Send` either).
///
/// Java keys the reusable enums on the segments' core `CacheKey`s, compared
/// by identity, as [`CacheKey`] is.
pub struct PerThreadPKLookup {
    id_field_name: String,
    terms_enums: Vec<Option<SharedTermsEnum>>,
    postings_enums: Vec<SharedPostingsEnum>,
    live_docs: Vec<Option<Box<dyn Bits>>>,
    doc_bases: Vec<i32>,
    num_enums: usize,
    has_deletions: bool,
    enum_indexes: HashMap<CacheKey, usize>,
}

impl PerThreadPKLookup {
    /// `new PerThreadPKLookup(reader, idFieldName)`.
    ///
    /// # Errors
    ///
    /// What reading the reader's leaves and their terms raises.
    pub fn new(reader: &Arc<dyn IndexReader>, id_field_name: &str) -> Result<Self> {
        Self::new_reusing(reader, id_field_name, &HashMap::new(), &[], &[])
    }

    /// The private constructor, reusing the enums of a previous lookup for
    /// the segments whose core key `prev_enum_indexes` knows.
    fn new_reusing(
        reader: &Arc<dyn IndexReader>,
        id_field_name: &str,
        prev_enum_indexes: &HashMap<CacheKey, usize>,
        reusable_terms_enums: &[Option<SharedTermsEnum>],
        reusable_postings_enums: &[SharedPostingsEnum],
    ) -> Result<Self> {
        let mut leaves = Arc::clone(reader).leaves()?;
        // Larger segments are more likely to have the id, so we sort largest to smallest by numDocs:
        leaves
            .sort_by(|c1, c2| (c2.leaf_reader().num_docs() - c1.leaf_reader().num_docs()).cmp(&0));
        let n = leaves.len();
        let mut terms_enums: Vec<Option<SharedTermsEnum>> = vec![None; n];
        let mut postings_enums: Vec<SharedPostingsEnum> =
            (0..n).map(|_| Rc::new(RefCell::new(None))).collect();
        let mut live_docs: Vec<Option<Box<dyn Bits>>> = (0..n).map(|_| None).collect();
        let mut doc_bases = vec![0i32; n];
        let mut enum_indexes = HashMap::new();
        let mut num_enums = 0usize;
        let mut has_deletions = false;
        for context in &leaves {
            let leaf_reader = context.leaf_reader();
            let cache_helper = leaf_reader.get_core_cache_helper();
            let cache_key = cache_helper.as_ref().map(|h| h.get_key().clone());
            match cache_key
                .as_ref()
                .and_then(|key| prev_enum_indexes.get(key))
            {
                Some(&seg) => {
                    // Reuse termsEnum, postingsEnum.
                    terms_enums[num_enums] = reusable_terms_enums[seg].clone();
                    postings_enums[num_enums] = Rc::clone(&reusable_postings_enums[seg]);
                }
                None => {
                    // New or empty segment.
                    if let Some(terms) = leaf_reader.terms(id_field_name)? {
                        terms_enums[num_enums] = Some(Rc::new(RefCell::new(terms.iterator()?)));
                    }
                }
            }
            if terms_enums[num_enums].is_some() {
                if let Some(key) = cache_key {
                    enum_indexes.insert(key, num_enums);
                }
                doc_bases[num_enums] = context.doc_base();
                live_docs[num_enums] = leaf_reader.get_live_docs();
                has_deletions |= Arc::clone(&leaf_reader).as_index_reader().has_deletions();
                num_enums += 1;
            }
        }
        Ok(PerThreadPKLookup {
            id_field_name: id_field_name.to_string(),
            terms_enums,
            postings_enums,
            live_docs,
            doc_bases,
            num_enums,
            has_deletions,
            enum_indexes,
        })
    }

    /// `lookup(id)`: returns docID if found, else -1.
    ///
    /// # Errors
    ///
    /// What seeking the terms enums and iterating the postings raise, and the
    /// `AssertionError` of `assert hasDeletions` when a segment holds the id
    /// only in deleted documents of a reader without deletions.
    pub fn lookup(&mut self, id: &BytesRef) -> Result<i32> {
        for seg in 0..self.num_enums {
            let terms_enum = self.terms_enums[seg]
                .as_ref()
                .expect("every enum below numEnums is set");
            let mut terms_enum = terms_enum.borrow_mut();
            if terms_enum.seek_exact(id)? {
                let mut slot = self.postings_enums[seg].borrow_mut();
                let reuse = slot.take();
                let postings_enum = slot.insert(
                    terms_enum
                        .postings(reuse, 0)?
                        .ok_or_else(rucene::error::LuceneError::null_pointer)?,
                );
                loop {
                    let doc_id = postings_enum.next_doc()?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    if self.live_docs[seg]
                        .as_ref()
                        .is_none_or(|live_docs| live_docs.get(doc_id as usize))
                    {
                        return Ok(self.doc_bases[seg] + doc_id);
                    }
                }
                java_assert!(self.has_deletions);
            }
        }
        Ok(-1)
    }

    /// `reopen(reader)`: reuse previous PerThreadPKLookup's termsEnum and
    /// postingsEnum. `None` for a `None` reader.
    ///
    /// # Errors
    ///
    /// What [`new`](Self::new) raises.
    pub fn reopen(&self, reader: Option<&Arc<dyn IndexReader>>) -> Result<Option<Self>> {
        let Some(reader) = reader else {
            return Ok(None);
        };
        Ok(Some(Self::new_reusing(
            reader,
            &self.id_field_name,
            &self.enum_indexes,
            &self.terms_enums,
            &self.postings_enums,
        )?))
    }
}

impl fmt::Debug for PerThreadPKLookup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PerThreadPKLookup")
            .field("id_field_name", &self.id_field_name)
            .field("num_enums", &self.num_enums)
            .field("has_deletions", &self.has_deletions)
            .finish_non_exhaustive()
    }
}
