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

//! Port of `TaxonomyMergeUtils.java`, of the Java package
//! `org.apache.lucene.facet.taxonomy`.

use std::sync::Arc;

use rucene::error::Result;
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{directory_reader, CodecReader, IndexReader, LeafReader, TwoPhaseCommit};
use rucene::store::Directory;

use crate::facets_config::FacetsConfig;
use crate::taxonomy::directory::directory_taxonomy_writer::{DirectoryTaxonomyWriter, OrdinalMap};
use crate::taxonomy::ordinal_mapping_leaf_reader::OrdinalMappingLeafReader;

/// Utility methods for merging index and taxonomy directories.
///
/// Equivalent to `org.apache.lucene.facet.taxonomy.TaxonomyMergeUtils`.
pub struct TaxonomyMergeUtils;

impl TaxonomyMergeUtils {
    /// Merges the given taxonomy and index directories and commits the
    /// changes to the given writers.
    ///
    /// Equivalent to `merge(Directory, Directory, OrdinalMap, IndexWriter,
    /// DirectoryTaxonomyWriter, FacetsConfig)`.
    pub fn merge(
        src_index_dir: Arc<dyn Directory>,
        src_taxo_dir: Arc<dyn Directory>,
        map: &mut dyn OrdinalMap,
        dest_index_writer: &IndexWriter,
        dest_taxo_writer: &DirectoryTaxonomyWriter,
        src_config: &FacetsConfig,
    ) -> Result<()> {
        // merge the taxonomies
        dest_taxo_writer.add_taxonomy(src_taxo_dir, map)?;
        let ordinal_map = map.get_map()?;
        let reader = directory_reader::open(src_index_dir)?;
        let result = (|| -> Result<()> {
            let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
            let mut wrapped_leaves: Vec<Arc<dyn CodecReader>> = Vec::with_capacity(leaves.len());
            for leaf in &leaves {
                let mapping: Arc<dyn LeafReader> = Arc::new(OrdinalMappingLeafReader::new(
                    leaf.leaf_reader(),
                    ordinal_map.clone(),
                    src_config,
                )?);
                wrapped_leaves.push(SlowCodecReaderWrapper::wrap(mapping)?);
            }
            dest_index_writer.add_indexes_codec_readers(&wrapped_leaves)?;

            // commit changes to taxonomy and index respectively.
            dest_taxo_writer.commit()?;
            dest_index_writer.commit()?;
            Ok(())
        })();
        let closed = reader.close();
        result?;
        closed
    }
}
