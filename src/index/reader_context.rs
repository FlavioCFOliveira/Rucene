//! Hierarchical reader context tree ported from `org.apache.lucene.index`.
//!
//! This module provides [`IndexReaderContext`], [`LeafReaderContext`] and
//! [`CompositeReaderContext`], which expose the parent/child relationships and
//! the flattened leaf view used by search and merge code.

#![deny(unsafe_code)]

use std::{
    fmt::Debug,
    sync::{Arc, Weak},
};

use crate::index::index_reader::{CompositeReader, IndexReader};

/// A node in the hierarchical reader context tree.
///
/// Equivalent to `org.apache.lucene.index.IndexReaderContext`.
pub trait IndexReaderContext: Send + Sync + Debug {
    /// Returns the parent context, if any.
    fn parent(&self) -> Option<Weak<dyn IndexReaderContext>>;

    /// Returns `true` if this context is the top-level context.
    fn is_top_level(&self) -> bool;

    /// Returns the doc base of this reader within its parent.
    fn doc_base_in_parent(&self) -> i32;

    /// Returns the ordinal of this reader within its parent.
    fn ord_in_parent(&self) -> i32;

    /// Returns the reader represented by this context.
    fn reader(&self) -> Arc<dyn IndexReader>;

    /// Returns the leaf contexts if this is a top-level context.
    ///
    /// For a [`LeafReaderContext`] this returns a singleton containing itself.
    /// For a non-top-level [`CompositeReaderContext`] this panics.
    fn leaves(self: Arc<Self>) -> Vec<Arc<LeafReaderContext>>;

    /// Returns the direct child contexts, or `None` for leaf contexts.
    fn children(&self) -> Option<Vec<Arc<dyn IndexReaderContext>>>;

    /// Returns `true` if this context represents a leaf reader.
    fn is_leaf_context(&self) -> bool;

    /// Returns this context as a leaf context, if it is one.
    fn as_leaf(self: Arc<Self>) -> Option<Arc<LeafReaderContext>>;

    /// Returns an identity value for this context.
    ///
    /// The returned value does not reference the wrapped reader and can safely
    /// outlive it.
    fn id(&self) -> usize;
}

/// Context for an atomic leaf reader.
///
/// Equivalent to `org.apache.lucene.index.LeafReaderContext`.
#[derive(Debug)]
pub struct LeafReaderContext {
    parent: Option<Weak<dyn IndexReaderContext>>,
    is_top_level: bool,
    ord_in_parent: i32,
    doc_base_in_parent: i32,
    reader: Arc<dyn IndexReader>,
    ord: i32,
    doc_base: i32,
    identity: usize,
}

impl LeafReaderContext {
    /// Creates a leaf context.
    pub fn new(
        reader: Arc<dyn IndexReader>,
        parent: Option<Weak<dyn IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Self {
        let is_top_level = parent.is_none();
        Self {
            parent,
            is_top_level,
            ord_in_parent,
            doc_base_in_parent,
            reader,
            ord: leaf_ord,
            doc_base: leaf_doc_base,
            identity: new_identity(),
        }
    }

    /// The reader's ordinal in the top-level leaves array.
    pub fn ord(&self) -> i32 {
        self.ord
    }

    /// The reader's absolute doc base.
    pub fn doc_base(&self) -> i32 {
        self.doc_base
    }
}

impl IndexReaderContext for LeafReaderContext {
    fn parent(&self) -> Option<Weak<dyn IndexReaderContext>> {
        self.parent.clone()
    }

    fn is_top_level(&self) -> bool {
        self.is_top_level
    }

    fn doc_base_in_parent(&self) -> i32 {
        self.doc_base_in_parent
    }

    fn ord_in_parent(&self) -> i32 {
        self.ord_in_parent
    }

    fn reader(&self) -> Arc<dyn IndexReader> {
        Arc::clone(&self.reader)
    }

    fn leaves(self: Arc<Self>) -> Vec<Arc<LeafReaderContext>> {
        if !self.is_top_level {
            panic!("This is not a top-level context.");
        }
        vec![self]
    }

    fn children(&self) -> Option<Vec<Arc<dyn IndexReaderContext>>> {
        None
    }

    fn is_leaf_context(&self) -> bool {
        true
    }

    fn as_leaf(self: Arc<Self>) -> Option<Arc<LeafReaderContext>> {
        Some(self)
    }

    fn id(&self) -> usize {
        self.identity
    }
}

/// Context for a composite reader.
///
/// Equivalent to `org.apache.lucene.index.CompositeReaderContext`.
#[derive(Debug)]
pub struct CompositeReaderContext {
    parent: Option<Weak<dyn IndexReaderContext>>,
    is_top_level: bool,
    ord_in_parent: i32,
    doc_base_in_parent: i32,
    reader: Arc<dyn IndexReader>,
    children: Vec<Arc<dyn IndexReaderContext>>,
    leaves: Vec<Arc<LeafReaderContext>>,
    identity: usize,
}

impl CompositeReaderContext {
    /// Creates a top-level composite context.
    pub fn new_top_level(
        reader: Arc<dyn IndexReader>,
        children: Vec<Arc<dyn IndexReaderContext>>,
        leaves: Vec<Arc<LeafReaderContext>>,
    ) -> Self {
        Self {
            parent: None,
            is_top_level: true,
            ord_in_parent: 0,
            doc_base_in_parent: 0,
            reader,
            children,
            leaves,
            identity: new_identity(),
        }
    }

    /// Creates a non-top-level composite context.
    pub fn new(
        parent: Weak<dyn IndexReaderContext>,
        reader: Arc<dyn IndexReader>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        children: Vec<Arc<dyn IndexReaderContext>>,
    ) -> Self {
        Self {
            parent: Some(parent),
            is_top_level: false,
            ord_in_parent,
            doc_base_in_parent,
            reader,
            children,
            leaves: Vec::new(),
            identity: new_identity(),
        }
    }

    /// Builds a composite context tree for `reader`.
    ///
    /// The resulting tree mirrors the Java `CompositeReaderContext.create`
    /// algorithm: each sub-reader recursively builds its own context, parent
    /// weak references are wired correctly, and the top-level context carries a
    /// flattened `leaves` list with cumulative doc bases and ordinals.
    pub fn create(
        reader: Arc<dyn CompositeReader>,
        parent: Option<Weak<dyn IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Arc<dyn IndexReaderContext> {
        Arc::new_cyclic(|this_weak: &Weak<CompositeReaderContext>| {
            let this_weak_dyn: Weak<dyn IndexReaderContext> = this_weak.clone();
            let subs = reader.get_sequential_sub_readers();
            let mut children = Vec::with_capacity(subs.len());
            let mut leaves = Vec::new();
            let mut next_doc_base = 0;
            let mut next_leaf_ord = leaf_ord;
            let mut next_leaf_doc_base = leaf_doc_base;

            for (ord, sub) in subs.iter().enumerate() {
                let sub = Arc::clone(sub);
                let sub_max_doc = sub.max_doc();
                let child = sub.build_context(
                    Some(this_weak_dyn.clone()),
                    ord as i32,
                    next_doc_base,
                    next_leaf_ord,
                    next_leaf_doc_base,
                );
                let child_leaves = collect_leaves(&child);
                next_leaf_ord += child_leaves.len() as i32;
                next_leaf_doc_base += sub_max_doc;
                leaves.extend(child_leaves);
                children.push(child);
                next_doc_base += sub_max_doc;
            }

            assert_eq!(
                next_doc_base,
                reader.max_doc(),
                "CompositeReader maxDoc must match sum of sub-reader maxDocs"
            );

            let reader: Arc<dyn IndexReader> = reader;
            if let Some(parent) = parent {
                CompositeReaderContext::new(
                    parent,
                    reader,
                    ord_in_parent,
                    doc_base_in_parent,
                    children,
                )
            } else {
                CompositeReaderContext::new_top_level(reader, children, leaves)
            }
        })
    }
}

impl IndexReaderContext for CompositeReaderContext {
    fn parent(&self) -> Option<Weak<dyn IndexReaderContext>> {
        self.parent.clone()
    }

    fn is_top_level(&self) -> bool {
        self.is_top_level
    }

    fn doc_base_in_parent(&self) -> i32 {
        self.doc_base_in_parent
    }

    fn ord_in_parent(&self) -> i32 {
        self.ord_in_parent
    }

    fn reader(&self) -> Arc<dyn IndexReader> {
        Arc::clone(&self.reader)
    }

    fn leaves(self: Arc<Self>) -> Vec<Arc<LeafReaderContext>> {
        if !self.is_top_level {
            panic!("This is not a top-level context.");
        }
        self.leaves.iter().map(Arc::clone).collect()
    }

    fn children(&self) -> Option<Vec<Arc<dyn IndexReaderContext>>> {
        Some(self.children.iter().map(Arc::clone).collect())
    }

    fn is_leaf_context(&self) -> bool {
        false
    }

    fn as_leaf(self: Arc<Self>) -> Option<Arc<LeafReaderContext>> {
        None
    }

    fn id(&self) -> usize {
        self.identity
    }
}

fn new_identity() -> usize {
    // A simple counter is sufficient for the contract. The value only needs
    // to be unique for the lifetime of the context.
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(1);
    COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Collects all atomic leaf contexts reachable from `ctx`.
fn collect_leaves(ctx: &Arc<dyn IndexReaderContext>) -> Vec<Arc<LeafReaderContext>> {
    if ctx.is_leaf_context() {
        return vec![ctx.clone().as_leaf().expect("leaf context")];
    }
    let mut leaves = Vec::new();
    if let Some(children) = ctx.children() {
        for child in children {
            leaves.extend(collect_leaves(&child));
        }
    }
    leaves
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{LuceneError, Result};
    use crate::index::index_reader::{
        build_composite_context, CacheHelper, IndexReader, IndexReaderCore,
    };
    use crate::index::leaf_reader::{LeafMetaData, LeafReader};
    use crate::index::{
        BinaryDocValues, ByteVectorValues, FieldInfos, FloatVectorValues, NumericDocValues,
        SortedDocValues, SortedNumericDocValues, SortedSetDocValues,
    };
    use crate::index::{DocValuesSkipper, PointValues};
    use crate::index::{StoredFields, TermVectors};
    use crate::search::knn::KnnCollector;
    use crate::search::AcceptDocs;
    use crate::util::Bits;

    #[derive(Debug)]
    struct DummyLeafReader {
        core: IndexReaderCore,
        max_doc: i32,
        num_docs: i32,
    }

    impl LeafReader for DummyLeafReader {
        fn core(&self) -> &IndexReaderCore {
            &self.core
        }

        fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
            unimplemented!("test stub")
        }

        fn num_docs(&self) -> i32 {
            self.num_docs
        }

        fn max_doc(&self) -> i32 {
            self.max_doc
        }

        fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
            unimplemented!("test stub")
        }

        fn do_close(&self) -> Result<()> {
            Ok(())
        }

        fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }

        fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
            None
        }

        fn terms(&self, _field: &str) -> Result<Option<Box<dyn crate::index::Terms>>> {
            Ok(None)
        }

        fn get_numeric_doc_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }

        fn get_binary_doc_values(&self, _field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
            Ok(None)
        }

        fn get_sorted_doc_values(&self, _field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
            Ok(None)
        }

        fn get_sorted_numeric_doc_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
            Ok(None)
        }

        fn get_sorted_set_doc_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
            Ok(None)
        }

        fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
            Ok(None)
        }

        fn get_doc_values_skipper(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn DocValuesSkipper>>> {
            Ok(None)
        }

        fn get_float_vector_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn FloatVectorValues>>> {
            Ok(None)
        }

        fn get_byte_vector_values(
            &self,
            _field: &str,
        ) -> Result<Option<Box<dyn ByteVectorValues>>> {
            Ok(None)
        }

        fn search_nearest_vectors(
            &self,
            _field: &str,
            _target: &[f32],
            _collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }

        fn search_nearest_vectors_byte(
            &self,
            _field: &str,
            _target: &[u8],
            _collector: &mut dyn KnnCollector,
            _accept_docs: &mut dyn AcceptDocs,
        ) -> Result<()> {
            Ok(())
        }

        fn get_field_infos(&self) -> FieldInfos {
            FieldInfos::empty()
        }

        fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
            None
        }

        fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn PointValues>>> {
            Ok(None)
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn get_meta_data(&self) -> LeafMetaData {
            LeafMetaData::new(10, None, None, false).unwrap()
        }
    }

    fn leaf(max_doc: i32, num_docs: i32) -> Arc<dyn IndexReader> {
        Arc::new(DummyLeafReader {
            core: IndexReaderCore::new(),
            max_doc,
            num_docs,
        }) as Arc<dyn IndexReader>
    }

    #[test]
    fn leaf_context_is_top_level_singleton() {
        let reader = leaf(5, 5);
        let ctx = reader.get_context();
        assert!(ctx.is_top_level());
        assert!(ctx.is_leaf_context());
        let leaves = ctx.leaves();
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].doc_base(), 0);
        assert_eq!(leaves[0].ord(), 0);
        assert!(leaves[0].parent().is_none());
    }

    #[test]
    fn composite_context_builds_parent_child_tree() {
        #[derive(Debug)]
        struct DummyCompositeReader {
            core: IndexReaderCore,
            subs: Vec<Arc<dyn IndexReader>>,
            max_doc: i32,
        }

        impl IndexReader for DummyCompositeReader {
            fn core(&self) -> &IndexReaderCore {
                &self.core
            }

            fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
                Err(LuceneError::UnsupportedOperation("test stub".to_string()))
            }

            fn num_docs(&self) -> i32 {
                self.subs.iter().map(|r| r.num_docs()).sum()
            }

            fn max_doc(&self) -> i32 {
                self.max_doc
            }

            fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
                Err(LuceneError::UnsupportedOperation("test stub".to_string()))
            }

            fn do_close(&self) -> Result<()> {
                Ok(())
            }

            fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
                None
            }

            fn doc_freq(&self, _term: &crate::index::Term) -> Result<i32> {
                Err(LuceneError::UnsupportedOperation("test stub".to_string()))
            }

            fn total_term_freq(&self, _term: &crate::index::Term) -> Result<i64> {
                Err(LuceneError::UnsupportedOperation("test stub".to_string()))
            }

            fn get_sum_doc_freq(&self, _field: &str) -> Result<i64> {
                Err(LuceneError::UnsupportedOperation("test stub".to_string()))
            }

            fn get_doc_count(&self, _field: &str) -> Result<i32> {
                Err(LuceneError::UnsupportedOperation("test stub".to_string()))
            }

            fn get_sum_total_term_freq(&self, _field: &str) -> Result<i64> {
                Err(LuceneError::UnsupportedOperation("test stub".to_string()))
            }

            fn build_context(
                self: Arc<Self>,
                parent: Option<Weak<dyn IndexReaderContext>>,
                ord_in_parent: i32,
                doc_base_in_parent: i32,
                leaf_ord: i32,
                leaf_doc_base: i32,
            ) -> Arc<dyn IndexReaderContext> {
                build_composite_context(
                    self as Arc<dyn CompositeReader>,
                    parent,
                    ord_in_parent,
                    doc_base_in_parent,
                    leaf_ord,
                    leaf_doc_base,
                )
            }
        }

        impl CompositeReader for DummyCompositeReader {
            fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
                self.subs.iter().map(Arc::clone).collect()
            }
        }

        let a = leaf(2, 2);
        let b = leaf(3, 3);
        let c = leaf(4, 4);
        let composite = Arc::new(DummyCompositeReader {
            core: IndexReaderCore::new(),
            subs: vec![Arc::clone(&a), Arc::clone(&b), Arc::clone(&c)],
            max_doc: 9,
        }) as Arc<dyn CompositeReader>;

        let ctx = composite.get_context();
        assert!(ctx.is_top_level());
        assert!(!ctx.is_leaf_context());

        let children = ctx.children().unwrap();
        assert_eq!(children.len(), 3);
        assert!(children[1].parent().is_some());

        let leaves = ctx.leaves();
        assert_eq!(leaves.len(), 3);
        assert_eq!(leaves[0].doc_base(), 0);
        assert_eq!(leaves[1].doc_base(), 2);
        assert_eq!(leaves[2].doc_base(), 5);
        assert_eq!(leaves[0].ord(), 0);
        assert_eq!(leaves[1].ord(), 1);
        assert_eq!(leaves[2].ord(), 2);
    }
}
