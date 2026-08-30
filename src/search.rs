//! Search engine ported from `org.apache.lucene.search`.
//!
//! Queries, collectors, scorers, `IndexSearcher`, and sorting live in this
//! module. Functional parity with Java Lucene's search behavior is the goal,
//! even though the public API is async.

#![deny(unsafe_code)]

pub mod doc_id_set;
pub mod doc_id_set_iterator;
pub mod knn;
pub mod reference_manager;
pub mod similarities;
pub mod sort;

pub use doc_id_set_iterator::{
    all, empty, from_iterator_supplier, from_live_docs, range, AcceptDocs, AllDocIdSetIterator,
    BitsAcceptDocs, DocIdSetIterator, DocIdSetIteratorSupplier, EmptyDocIdSetIterator,
    IteratorAcceptDocs, RangeDocIdSetIterator, NO_MORE_DOCS,
};
pub use reference_manager::{ManagedReference, ReferenceManager, RefreshListener, RefreshSource};
pub use similarities::{
    axiomatic_explain, axiomatic_explain_details, axiomatic_score, compute_default_norm,
    fill_basic_stats, lm_explain_details, lm_fill_basic_stats, lm_new_stats, lm_to_string, log2,
    per_field_compute_norm, per_field_scorer, similarity_base_scorer, tfidf_scorer, AfterEffect,
    AfterEffectB, AfterEffectL, Axiomatic, AxiomaticF1EXP, AxiomaticF1LOG, AxiomaticF2EXP,
    AxiomaticF2LOG, AxiomaticF3EXP, AxiomaticF3LOG, BM25Similarity, BasicModel, BasicModelG,
    BasicModelIF, BasicModelIn, BasicModelIne, BasicStats, BooleanSimilarity, BulkSimScorer,
    ClassicSimilarity, CollectionModel, CollectionStatistics, DFISimilarity, DFRSimilarity,
    DefaultCollectionModel, Distribution, DistributionLL, DistributionSPL, Explanation,
    ExplanationValue, IBSimilarity, Independence, IndependenceChiSquared, IndependenceSaturated,
    IndependenceStandardized, IndriCollectionModel, IndriDirichletSimilarity,
    LMDirichletSimilarity, LMJelinekMercerSimilarity, LMSimilarity, LMStats, Lambda, LambdaDF,
    LambdaTTF, MultiSimilarity, NoNormalization, Normalization, NormalizationH1, NormalizationH2,
    NormalizationH3, NormalizationZ, PerFieldSimilarityWrapper, RawTFSimilarity, SimScorer,
    Similarity, SimilarityBase, TFIDFSimilarity, TermStatistics,
};
pub use sort::{read_sort, write_sort, MissingValue, Sort, SortField, SortFieldType};
