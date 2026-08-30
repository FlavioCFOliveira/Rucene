//! Port of `org.apache.lucene.util.hnsw.IncrementalHnswGraphMerger`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::codecs::hnsw::graph_provider::HnswGraphProvider;
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::error::{LuceneError, Result};
use crate::index::field_infos::FieldInfo;
use crate::index::merge::DocMap;
use crate::index::vector_values::KnnVectorValues;
use crate::index::VectorEncoding;
use crate::search::NO_MORE_DOCS;
use crate::util::{Bits, FixedBitSet};

use super::builder::{HnswBuilder, HnswGraphBuilder, DEFAULT_RAND_SEED};
use super::merger::HnswGraphMerger;
use super::merging_builder::MergingHnswGraphBuilder;
use super::on_heap::OnHeapHnswGraph;
use super::scorer::RandomVectorScorerSupplier;
use super::HnswGraph;

/// The maximum acceptable deletion percentage for a graph to be considered as the
/// base graph.
///
/// Equivalent to `IncrementalHnswGraphMerger.DELETE_PCT_THRESHOLD`. Graphs with
/// deletion percentages above this threshold are not used for initialization, as
/// they may have degraded connectivity. A value of 40 means that if more than 40% of
/// the graph's original vectors have been deleted, the graph is not selected as the
/// base.
const DELETE_PCT_THRESHOLD: i32 = 40;

/// Represents a vector reader that contains graph info.
///
/// Equivalent to the `IncrementalHnswGraphMerger.GraphReader` record.
#[derive(Clone)]
pub struct GraphReader {
    /// The reader the vectors come from.
    pub reader: Arc<dyn KnnVectorsReader>,
    /// The graph the reader stores for the merged field.
    pub graph_provider: Arc<dyn HnswGraphProvider>,
    /// Maps this reader's doc ids into the merged segment.
    pub init_doc_map: Arc<DocMap>,
    /// How many nodes the reader's graph holds.
    pub graph_size: i32,
    /// Identifies this reader among the ones added, standing in for the record
    /// identity Java's `List.contains` compares.
    id: usize,
}

/// Merges multiple graphs in a single thread, in an incremental fashion.
///
/// Equivalent to `org.apache.lucene.util.hnsw.IncrementalHnswGraphMerger`.
///
/// # Divergences from Lucene 10.5.0
///
/// * Java recovers the graph from a reader with
///   `reader instanceof HnswGraphProvider` (unwrapping a
///   `PerFieldKnnVectorsFormat.FieldsReader` first). Rust cannot cast a `dyn
///   KnnVectorsReader` to a `dyn HnswGraphProvider`, so [`HnswGraphMerger::add_reader`]
///   takes the provider explicitly; passing `None` is the counterpart of a reader
///   that is not an `HnswGraphProvider`, and the reader is then skipped exactly as
///   Java skips it.
/// * `merge` takes no `InfoStream`: the port's [`HnswGraphBuilder`] has no info
///   stream to set.
pub struct IncrementalHnswGraphMerger {
    field_info: FieldInfo,
    scorer_supplier: Arc<dyn RandomVectorScorerSupplier>,
    m: i32,
    beam_width: i32,
    pub(crate) graph_readers: Vec<GraphReader>,
    pub(crate) largest_graph_reader: Option<GraphReader>,
    num_readers: usize,
}

impl IncrementalHnswGraphMerger {
    /// Creates a merger for the field being merged.
    pub fn new(
        field_info: FieldInfo,
        scorer_supplier: Arc<dyn RandomVectorScorerSupplier>,
        m: i32,
        beam_width: i32,
    ) -> Self {
        Self {
            field_info,
            scorer_supplier,
            m,
            beam_width,
            graph_readers: Vec::new(),
            largest_graph_reader: None,
            num_readers: 0,
        }
    }

    /// The field being merged.
    pub fn field_info(&self) -> &FieldInfo {
        &self.field_info
    }

    /// The scorer supplier the merged graph is built with.
    pub(crate) fn scorer_supplier(&self) -> &Arc<dyn RandomVectorScorerSupplier> {
        &self.scorer_supplier
    }

    /// The maximum number of connections per node.
    pub fn m(&self) -> i32 {
        self.m
    }

    /// The beam width used while building.
    pub fn beam_width(&self) -> i32 {
        self.beam_width
    }

    /// How many readers were offered to this merger, including skipped ones.
    pub fn num_readers(&self) -> usize {
        self.num_readers
    }

    /// Returns the vector values of `reader` for the merged field, following the
    /// field's encoding.
    fn vector_values(&self, reader: &dyn KnnVectorsReader) -> Result<Box<dyn KnnVectorValues>> {
        match self.field_info.get_vector_encoding() {
            VectorEncoding::BYTE => Ok(reader.get_byte_vector_values(&self.field_info.name)?),
            VectorEncoding::FLOAT32 => Ok(reader.get_float_vector_values(&self.field_info.name)?),
        }
    }

    /// Builds the graph builder that produces the merged graph.
    ///
    /// Equivalent to `IncrementalHnswGraphMerger.createBuilder`.
    ///
    /// # Errors
    ///
    /// Returns an error if reading a source graph fails.
    pub fn create_builder(
        &mut self,
        merged_vector_values: &dyn KnnVectorValues,
        max_ord: i32,
    ) -> Result<Box<dyn HnswBuilder>> {
        let Some(largest) = self.largest_graph_reader.clone() else {
            return Ok(Box::new(HnswGraphBuilder::create(
                self.scorer_supplier.as_ref(),
                self.m,
                self.beam_width,
                DEFAULT_RAND_SEED,
                max_ord,
            )?));
        };
        if !self.graph_readers.iter().any(|r| r.id == largest.id) {
            self.graph_readers.insert(0, largest);
        } else {
            self.graph_readers
                .sort_by_key(|r| std::cmp::Reverse(r.graph_size));
        }
        let mut initialized_nodes = if self.graph_readers.len() == self.num_readers {
            None
        } else {
            Some(FixedBitSet::new(max_ord.max(0) as usize))
        };
        let ord_maps =
            self.get_new_ord_mapping(merged_vector_values, initialized_nodes.as_mut())?;
        let mut graphs: Vec<Box<dyn HnswGraph>> = Vec::with_capacity(self.graph_readers.len());
        for graph_reader in &self.graph_readers {
            let graph = graph_reader
                .graph_provider
                .get_graph(&self.field_info.name)?;
            if graph.size() == 0 {
                return Err(LuceneError::IllegalState(
                    "Graph should not be empty".to_string(),
                ));
            }
            graphs.push(graph);
        }
        Ok(Box::new(MergingHnswGraphBuilder::from_graphs(
            self.scorer_supplier.as_ref(),
            self.m,
            self.beam_width,
            DEFAULT_RAND_SEED,
            graphs,
            ord_maps,
            max_ord,
            initialized_nodes,
        )?))
    }

    /// Builds the old-to-new ordinal maps of every graph reader.
    ///
    /// Equivalent to `IncrementalHnswGraphMerger.getNewOrdMapping`.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the merge state fails.
    pub fn get_new_ord_mapping(
        &self,
        merged_vector_values: &dyn KnnVectorValues,
        mut initialized_nodes: Option<&mut FixedBitSet>,
    ) -> Result<Vec<Vec<i32>>> {
        let num_graphs = self.graph_readers.len();
        let mut new_doc_id_to_old_ordinals: Vec<HashMap<i32, i32>> = Vec::with_capacity(num_graphs);
        let mut old_to_new_ordinal_map: Vec<Vec<i32>> = Vec::with_capacity(num_graphs);
        for graph_reader in &self.graph_readers {
            let values = self.vector_values(graph_reader.reader.as_ref())?;
            let mut vectors_iter = values.iterator()?;
            let mut mapping: HashMap<i32, i32> =
                HashMap::with_capacity(graph_reader.graph_size.max(0) as usize);
            loop {
                let doc_id = vectors_iter.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                let new_doc_id = (graph_reader.init_doc_map)(doc_id);
                mapping.insert(new_doc_id, vectors_iter.index());
            }
            new_doc_id_to_old_ordinals.push(mapping);
            old_to_new_ordinal_map.push(vec![-1i32; graph_reader.graph_size.max(0) as usize]);
        }

        let mut merged_vector_iterator = merged_vector_values.iterator()?;
        loop {
            let doc_id = merged_vector_iterator.next_doc()?;
            // Java's loop condition is `docId < NO_MORE_DOCS`; a `DocIdSetIterator`
            // never returns a value above the sentinel, so the two are the same test.
            if doc_id == NO_MORE_DOCS {
                break;
            }
            let new_ord = merged_vector_iterator.index();
            for i in 0..num_graphs {
                let old_ord = new_doc_id_to_old_ordinals[i]
                    .get(&doc_id)
                    .copied()
                    .unwrap_or(-1);
                if old_ord != -1 {
                    old_to_new_ordinal_map[i][old_ord as usize] = new_ord;
                    if let Some(initialized) = initialized_nodes.as_deref_mut() {
                        initialized.set(new_ord as usize);
                    }
                    break;
                }
            }
        }
        Ok(old_to_new_ordinal_map)
    }

    /// Counts the vectors of `values` whose document is live.
    ///
    /// Equivalent to `IncrementalHnswGraphMerger.countLiveVectors`.
    fn count_live_vectors(
        live_docs: Option<&dyn Bits>,
        values: &dyn KnnVectorValues,
    ) -> Result<i32> {
        let Some(live_docs) = live_docs else {
            return Ok(values.size());
        };

        let mut count = 0;
        let mut iterator = values.iterator()?;
        loop {
            let doc = iterator.next_doc()?;
            if doc == NO_MORE_DOCS {
                break;
            }
            if live_docs.get(doc as usize) {
                count += 1;
            }
        }
        Ok(count)
    }
}

impl HnswGraphMerger for IncrementalHnswGraphMerger {
    fn add_reader(
        &mut self,
        reader: Arc<dyn KnnVectorsReader>,
        graph_provider: Option<Arc<dyn HnswGraphProvider>>,
        doc_map: Arc<DocMap>,
        live_docs: Option<&dyn Bits>,
    ) -> Result<()> {
        let id = self.num_readers;
        self.num_readers += 1;
        let Some(graph_provider) = graph_provider else {
            return Ok(());
        };
        let graph = graph_provider.get_graph(&self.field_info.name)?;
        if graph.size() == 0 {
            return Ok(());
        }

        let knn_vector_values = self.vector_values(reader.as_ref())?;

        let candidate_vector_count =
            Self::count_live_vectors(live_docs, knn_vector_values.as_ref())?;
        let graph_size = graph.size();

        let graph_reader = GraphReader {
            reader,
            graph_provider,
            init_doc_map: doc_map,
            graph_size,
            id,
        };

        let delete_pct = ((graph_size - candidate_vector_count) * 100) / graph_size;

        let beats_largest = match &self.largest_graph_reader {
            None => true,
            Some(largest) => candidate_vector_count > largest.graph_size,
        };
        if delete_pct <= DELETE_PCT_THRESHOLD && beats_largest {
            self.largest_graph_reader = Some(graph_reader.clone());
        }

        // If the graph has no deletes.
        if candidate_vector_count == graph_size {
            self.graph_readers.push(graph_reader);
        }

        Ok(())
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
