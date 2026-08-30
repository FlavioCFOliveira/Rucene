//! Graph merging, ported from `org.apache.lucene.util.hnsw.HnswGraphMerger` and
//! `org.apache.lucene.util.hnsw.ConcurrentHnswMerger`.

#![deny(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use crate::codecs::hnsw::graph_provider::HnswGraphProvider;
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::error::Result;
use crate::index::field_infos::FieldInfo;
use crate::index::merge::DocMap;
use crate::index::vector_values::KnnVectorValues;
use crate::index::VectorEncoding;
use crate::search::NO_MORE_DOCS;
use crate::util::{Bits, FixedBitSet};

use super::builder::{HnswBuilder, DEFAULT_RAND_SEED};
use super::concurrent_merge_builder::HnswConcurrentMergeBuilder;
use super::incremental_merger::IncrementalHnswGraphMerger;
use super::initialized_builder::InitializedHnswGraphBuilder;
use super::on_heap::OnHeapHnswGraph;
use super::scorer::RandomVectorScorerSupplier;

/// Abstraction of merging multiple graphs into one on-heap graph.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HnswGraphMerger`.
///
/// # Divergences from Lucene 10.5.0
///
/// * `add_reader` takes the [`HnswGraphProvider`] explicitly, because Rust cannot
///   cast a `dyn KnnVectorsReader` to a `dyn HnswGraphProvider` the way Java's
///   `instanceof` does. `None` stands for a reader that is not a graph provider.
/// * `merge` takes no `InfoStream`: the port's builders have none.
pub trait HnswGraphMerger {
    /// Adds a reader to the graph merger, recording its state.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from the merge state fails.
    fn add_reader(
        &mut self,
        reader: Arc<dyn KnnVectorsReader>,
        graph_provider: Option<Arc<dyn HnswGraphProvider>>,
        doc_map: Arc<DocMap>,
        live_docs: Option<&dyn Bits>,
    ) -> Result<()>;

    /// Merges the added readers and produces the on-heap graph.
    ///
    /// `merged_vector_values` is the view of the vectors in the merged segment and
    /// `max_ord` the number of vectors that will be added to the graph.
    ///
    /// # Errors
    ///
    /// Returns an error if the merge fails.
    fn merge(
        &mut self,
        merged_vector_values: &dyn KnnVectorValues,
        max_ord: i32,
    ) -> Result<OnHeapHnswGraph>;
}

/// Merges graphs by handing the work to [`HnswConcurrentMergeBuilder`].
///
/// Equivalent to `org.apache.lucene.util.hnsw.ConcurrentHnswMerger`, which extends
/// `IncrementalHnswGraphMerger`; this port holds one instead, because Rust has no
/// implementation inheritance. See [`HnswConcurrentMergeBuilder`] for how the
/// workers are executed.
pub struct ConcurrentHnswMerger {
    inner: IncrementalHnswGraphMerger,
    num_worker: i32,
}

impl ConcurrentHnswMerger {
    /// Creates a merger driving `num_worker` merge workers.
    pub fn new(
        field_info: FieldInfo,
        scorer_supplier: Arc<dyn RandomVectorScorerSupplier>,
        m: i32,
        beam_width: i32,
        num_worker: i32,
    ) -> Self {
        Self {
            inner: IncrementalHnswGraphMerger::new(field_info, scorer_supplier, m, beam_width),
            num_worker,
        }
    }

    /// Builds the graph builder that produces the merged graph.
    ///
    /// Equivalent to `ConcurrentHnswMerger.createBuilder`.
    ///
    /// # Errors
    ///
    /// Returns an error if reading a source graph fails.
    pub fn create_builder(
        &mut self,
        merged_vector_values: &dyn KnnVectorValues,
        max_ord: i32,
    ) -> Result<Box<dyn HnswBuilder>> {
        let m = self.inner.m();
        let beam_width = self.inner.beam_width();
        let mut initialized_nodes: Option<FixedBitSet> = None;

        let graph = match self.inner.largest_graph_reader.clone() {
            None => OnHeapHnswGraph::new(m, max_ord),
            Some(largest) => {
                let mut initializer_graph = largest
                    .graph_provider
                    .get_graph(&self.inner.field_info().name)?;
                if initializer_graph.size() == 0 {
                    OnHeapHnswGraph::new(m, max_ord)
                } else {
                    let mut nodes = FixedBitSet::new(max_ord.max(0) as usize);
                    let old_to_new_ordinal_map = Self::get_new_ord_mapping(
                        self.inner.field_info(),
                        largest.reader.as_ref(),
                        largest.init_doc_map.as_ref(),
                        largest.graph_size,
                        merged_vector_values,
                        &mut nodes,
                    )?;
                    initialized_nodes = Some(nodes);
                    InitializedHnswGraphBuilder::init_graph(
                        initializer_graph.as_mut(),
                        &old_to_new_ordinal_map,
                        max_ord,
                        beam_width,
                        self.inner.scorer_supplier().as_ref(),
                    )?
                }
            }
        };
        Ok(Box::new(HnswConcurrentMergeBuilder::new(
            self.num_worker,
            self.inner.scorer_supplier().as_ref(),
            m,
            beam_width,
            graph,
            initialized_nodes,
        )?))
    }

    /// Creates a new mapping from old ordinals to new ordinals.
    ///
    /// Equivalent to `ConcurrentHnswMerger.getNewOrdMapping`.
    fn get_new_ord_mapping(
        field_info: &FieldInfo,
        init_reader: &dyn KnnVectorsReader,
        init_doc_map: &DocMap,
        init_graph_size: i32,
        merged_vector_values: &dyn KnnVectorValues,
        initialized_nodes: &mut FixedBitSet,
    ) -> Result<Vec<i32>> {
        let values: Box<dyn KnnVectorValues> = match field_info.get_vector_encoding() {
            VectorEncoding::BYTE => init_reader.get_byte_vector_values(&field_info.name)?,
            VectorEncoding::FLOAT32 => init_reader.get_float_vector_values(&field_info.name)?,
        };
        let mut initializer_iterator = values.iterator()?;

        let mut new_id_to_old_ordinal: HashMap<i32, i32> =
            HashMap::with_capacity(init_graph_size.max(0) as usize);
        let mut max_new_doc_id = -1i32;
        loop {
            let doc_id = initializer_iterator.next_doc()?;
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_id = init_doc_map(doc_id);
            if new_id == -1 {
                continue;
            }
            max_new_doc_id = max_new_doc_id.max(new_id);
            debug_assert!(!new_id_to_old_ordinal.contains_key(&new_id));
            new_id_to_old_ordinal.insert(new_id, initializer_iterator.index());
        }

        if max_new_doc_id == -1 {
            return Ok(Vec::new());
        }
        let mut old_to_new_ordinal_map = vec![-1i32; init_graph_size.max(0) as usize];
        let mut merged_vector_iterator = merged_vector_values.iterator()?;
        loop {
            let new_doc_id = merged_vector_iterator.next_doc()?;
            if new_doc_id > max_new_doc_id {
                break;
            }
            let old_ord = new_id_to_old_ordinal
                .get(&new_doc_id)
                .copied()
                .unwrap_or(-1);
            if old_ord != -1 {
                let new_ord = merged_vector_iterator.index();
                initialized_nodes.set(new_ord as usize);
                old_to_new_ordinal_map[old_ord as usize] = new_ord;
            }
        }
        Ok(old_to_new_ordinal_map)
    }
}

impl HnswGraphMerger for ConcurrentHnswMerger {
    fn add_reader(
        &mut self,
        reader: Arc<dyn KnnVectorsReader>,
        graph_provider: Option<Arc<dyn HnswGraphProvider>>,
        doc_map: Arc<DocMap>,
        live_docs: Option<&dyn Bits>,
    ) -> Result<()> {
        self.inner
            .add_reader(reader, graph_provider, doc_map, live_docs)
    }

    fn merge(
        &mut self,
        merged_vector_values: &dyn KnnVectorValues,
        max_ord: i32,
    ) -> Result<OnHeapHnswGraph> {
        let mut builder = self.create_builder(merged_vector_values, max_ord)?;
        builder.build(max_ord)?;
        builder.into_completed_graph()
    }
}

/// Keeps the default seed reachable from this module, as Lucene reaches
/// `HnswGraphBuilder.randSeed` from its mergers.
pub const RAND_SEED: u64 = DEFAULT_RAND_SEED;
