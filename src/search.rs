//! Search engine ported from `org.apache.lucene.search`.
//!
//! Queries, collectors, scorers, `IndexSearcher`, and sorting live in this
//! module. Functional parity with Java Lucene's search behavior is the goal,
//! even though the public API is async.

#![deny(unsafe_code)]

pub mod abstract_doc_id_set_iterator;
pub mod batch_score_bulk_scorer;
pub mod bit_set_doc_id_stream;
pub mod bit_set_util;
pub mod block_max_conjunction_bulk_scorer;
pub mod block_max_conjunction_scorer;
pub mod boolean_clause;
pub mod boolean_query;
pub mod boolean_scorer;
pub mod boolean_scorer_supplier;
pub mod boolean_weight;
pub mod boost_query;
pub mod bulk_scorer;
pub mod caching_collector;
pub mod collection_terminated_exception;
pub mod collector;
pub mod conjunction_bulk_scorer;
pub mod conjunction_disi;
pub mod conjunction_scorer;
pub mod conjunction_utils;
pub mod constant_score_bulk_scorer;
pub mod constant_score_query;
pub mod constant_score_scorer;
pub mod constant_score_scorer_supplier;
pub mod constant_score_weight;
pub mod dense_conjunction_bulk_scorer;
pub mod disi_priority_queue;
pub mod disi_wrapper;
pub mod disjunction_disi_approximation;
pub mod disjunction_max_bulk_scorer;
pub mod disjunction_max_query;
pub mod disjunction_max_scorer;
pub mod disjunction_score_block_boundary_propagator;
pub mod disjunction_scorer;
pub mod disjunction_sum_scorer;
pub mod doc_id_set;
pub mod doc_id_set_bulk_iterator;
pub mod doc_id_set_iterator;
pub mod doc_id_stream;
pub mod hit_queue;
pub mod impacts_disi;
pub mod index_priority_queue;
pub mod index_searcher;
pub mod knn;
pub mod match_all_docs_query;
pub mod match_no_docs_query;
pub mod matches;
pub mod max_score_accumulator;
pub mod max_score_bulk_scorer;
pub mod max_score_cache;
pub mod multi_collector;
pub mod multi_collector_manager;
pub mod multiset;
pub mod positive_scores_only_collector;
pub mod pruning;
pub mod query;
pub mod query_cache;
pub mod query_visitor;
pub mod reference_manager;
pub mod req_excl_bulk_scorer;
pub mod req_excl_scorer;
pub mod req_opt_sum_scorer;
pub mod scorable;
pub mod score_caching_wrapping_scorer;
pub mod score_doc;
pub mod score_mode;
pub mod scorer;
pub mod scorer_supplier;
pub mod scorer_util;
pub mod segment_cacheable;
pub mod similarities;
pub mod skip_block_range_iterator;
pub mod sort;
pub mod task_executor;
pub mod time_limiting_bulk_scorer;
pub mod top_docs;
pub mod top_docs_collector;
pub mod top_score_doc_collector;
pub mod top_score_doc_collector_manager;
pub mod total_hit_count_collector;
pub mod total_hits;
pub mod two_phase_iterator;
pub mod wand_scorer;
pub mod weight;

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

pub use abstract_doc_id_set_iterator::{AbstractDocIdSetIterator, FilterDocIdSetIterator};
pub use batch_score_bulk_scorer::BatchScoreBulkScorer;
pub use bit_set_doc_id_stream::BitSetDocIdStream;
pub use block_max_conjunction_bulk_scorer::BlockMaxConjunctionBulkScorer;
pub use block_max_conjunction_scorer::BlockMaxConjunctionScorer;
pub use boolean_clause::{BooleanClause, Occur};
pub use boolean_query::{BooleanQuery, Builder as BooleanQueryBuilder};
pub use boolean_scorer::BooleanScorer;
pub use boolean_scorer_supplier::{BooleanScorerSupplier, ClauseSuppliers};
pub use boolean_weight::{BooleanWeight, WeightedBooleanClause};
pub use boost_query::BoostQuery;
pub use bulk_scorer::{BulkScorer, DefaultBulkScorer};
pub use caching_collector::{CachingCollector, NoOpCollector};
pub use collection_terminated_exception::{
    CollectionError, CollectionResult, CollectionTerminatedException, TimeExceededException,
};
pub use collector::{
    Collector, CollectorManager, FilterCollector, FilterLeafCollector, LeafCollector,
    SimpleCollector, SimpleCollectorImpl,
};
pub use conjunction_bulk_scorer::ConjunctionBulkScorer;
pub use conjunction_disi::{create_conjunction, ConjunctionDISI, ConjunctionMember};
pub use conjunction_scorer::ConjunctionScorer;
pub use conjunction_utils::ConjunctionUtils;
pub use constant_score_bulk_scorer::ConstantScoreBulkScorer;
pub use constant_score_query::{ConstantBulkScorer, ConstantScoreQuery};
pub use constant_score_scorer::ConstantScoreScorer;
pub use constant_score_scorer_supplier::{
    ConstantScoreIteratorSupplier, ConstantScoreScorerSupplier, SingleIteratorSupplier,
};
pub use constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
pub use dense_conjunction_bulk_scorer::{
    DenseConjunctionBulkScorer, DENSITY_THRESHOLD_INVERSE, WINDOW_SIZE,
};
pub use disi_priority_queue::{DisiPriorityQueue, DisiPriorityQueue2, DisiPriorityQueueN};
pub use disi_wrapper::DisiWrapper;
pub use disjunction_disi_approximation::DisjunctionDISIApproximation;
pub use disjunction_max_bulk_scorer::DisjunctionMaxBulkScorer;
pub use disjunction_max_query::{DisjunctionMaxQuery, DisjunctionMaxWeight};
pub use disjunction_max_scorer::DisjunctionMaxScorer;
pub use disjunction_score_block_boundary_propagator::{
    DisjunctionScoreBlockBoundaryPropagator, SubScorers,
};
pub use disjunction_scorer::{ByMatchCost, DisjunctionScorer};
pub use disjunction_sum_scorer::DisjunctionSumScorer;
pub use doc_id_set_bulk_iterator::DocIdSetBulkIterator;
pub use doc_id_stream::{CheckedIntConsumer, DocIdStream, RangeDocIdStream};
pub use hit_queue::{HitQueue, HitQueueComparator};
pub use impacts_disi::ImpactsDISI;
pub use index_priority_queue::{IndexOrder, IndexPriorityQueue};
pub use index_searcher::{
    IndexSearcher, LeafReaderContextPartition, LeafSlice, TooManyClauses, TooManyNestedClauses,
};
pub use match_all_docs_query::MatchAllDocsQuery;
pub use match_no_docs_query::MatchNoDocsQuery;
pub use matches::{MatchWithNoTerms, Matches, MatchesIterator, MatchesUtils};
pub use max_score_accumulator::MaxScoreAccumulator;
pub use max_score_bulk_scorer::{MaxScoreBulkScorer, INNER_WINDOW_SIZE};
pub use max_score_cache::MaxScoreCache;
pub use multi_collector::{MinCompetitiveScoreAwareScorable, MultiCollector, MultiLeafCollector};
pub use multi_collector_manager::{
    AnyCollector, ErasedCollectorManager, MultiCollectorHandle, MultiCollectorManager,
};
pub use multiset::Multiset;
pub use positive_scores_only_collector::PositiveScoresOnlyCollector;
pub use pruning::Pruning;
pub use query::{query_to_string, Query, QueryKey};
pub use query_cache::{QueryCache, QueryCachingPolicy};
pub use query_visitor::{EmptyQueryVisitor, QueryVisitor, TermCollectorVisitor};
pub use req_excl_bulk_scorer::ReqExclBulkScorer;
pub use req_excl_scorer::ReqExclScorer;
pub use req_opt_sum_scorer::ReqOptSumScorer;
pub use scorable::{ChildScorable, FilterScorable, Scorable, SimpleScorable};
pub use score_caching_wrapping_scorer::{
    ScoreCachingWrappingLeafCollector, ScoreCachingWrappingScorer,
};
pub use score_doc::ScoreDoc;
pub use score_mode::ScoreMode;
pub use scorer::{
    into_scorer_iterator, FilterScorer, Scorer, ScorerAsIterator, ScorerAsTwoPhaseIterator,
};
pub use scorer_supplier::ScorerSupplier;
pub use scorer_util::{DocAndScoreAccBuffer, ScorerUtil};
pub use segment_cacheable::SegmentCacheable;
pub use skip_block_range_iterator::{Match, SkipBlockRangeIterator};
pub use task_executor::{Executor, TaskExecutor};
pub use time_limiting_bulk_scorer::TimeLimitingBulkScorer;
pub use top_docs::{default_tie_breaker, TieBreaker, TopDocs};
pub use top_docs_collector::{empty_top_docs, TopDocsCollector};
pub use top_score_doc_collector::{DocScoreEncoder, TopScoreDocCollector};
pub use top_score_doc_collector_manager::TopScoreDocCollectorManager;
pub use total_hit_count_collector::{
    EarlyTerminatedMap, TotalHitCountCollector, TotalHitCountCollectorManager,
};
pub use total_hits::{TotalHits, TotalHitsRelation};
pub use two_phase_iterator::{
    ScorerIterator, TwoPhaseIterator, TwoPhaseIteratorAsDocIdSetIterator,
};
pub use wand_scorer::{scale_max_score, scaling_factor, WANDScorer, FLOAT_MANTISSA_BITS};
pub use weight::{DefaultScorerSupplier, FilterWeight, Weight};
