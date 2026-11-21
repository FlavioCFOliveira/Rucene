# Task #269 block 5 — `search` no-production-caller verdicts

Input: `list_369b1ed.jsonl`, rows with `"subsystem": "search"` (497 items in 138 files), checked against the working tree (HEAD `52ed2a1`, src unchanged at `30349af`, plus pass S). Java citations are relative to `/tmp/lucene1050/lucene/core/src/java/org/apache/lucene/search/` (10.5.0 tag) unless a path says otherwise; Rust paths are relative to `src/search/` unless they start with `src/` or another top-level module. Oracle: `scratchpad/oracle/passS/PatienceOracle.java` (`out.txt`).

Final classes as in block 3: **W** link ported now; **W=** the Java link already exists in Rust (through the field, a declared mechanism, or a std call with the same result); **W-blocked** missing link whose Java caller is not ported or that needs a larger change (the Java caller is named); **P** legitimate API, or a faithful member uncalled in Lucene, kept; **D** no Lucene counterpart, removed; **T** test-only, no Lucene counterpart, left.

Positive controls follow block 2: a W wiring is proven by mutating the callee (or reverting the change) and watching a caller-level test fail.

## Counts

Auto (all 497): D? 148, P? 196, W? 153  
Rows written: 497 of 497. Final: D 42, P 351, T 3, W 23, W-blocked 9, W= 69

Re-verified at `225f596` (#435): the 9 W-blocked rows no longer have their stated blocker and are reclassified in place (7 **W**, 2 **W=**). Current: D 42, P 351, T 3, W 30, W-blocked 0, W= 71 (the cross table below is the original pass's).

| auto \ final | D | P | T | W | W-blocked | W= |
|---|---|---|---|---|---|---|
| D? | 41 | 98 | 3 | 1 | 0 | 5 |
| P? | 0 | 196 | 0 | 0 | 0 | 0 |
| W? | 1 | 57 | 0 | 22 | 9 | 64 |

## Items

| item | auto | final | action | Java evidence |
|---|---|---|---|---|
| `search::abstract_doc_id_set_iterator::FilterDocIdSetIterator::into_inner` | D? | **D** | removed | no Lucene member: `FilterDocIdSetIterator` exposes only the `protected final DocIdSetIterator in` field and the overridden iterator methods (FilterDocIdSetIterator.java:25-50); no caller |
| `search::abstract_multi_term_query_constant_score_wrapper::AbstractMultiTermQueryConstantScoreWrapper::get_field` | W? | **W** | wired: `visit` accepts the field through `self.get_field()`, as Java (it read `self.query.get_field()` inline) | AbstractMultiTermQueryConstantScoreWrapper.java:86-88 getField; caller :91-95 visit |
| `search::abstract_multi_term_query_constant_score_wrapper::AbstractMultiTermQueryConstantScoreWrapper::get_query` | D? | **P** | none | AbstractMultiTermQueryConstantScoreWrapper.java:81 getQuery (public, no Lucene caller) |
| `search::automaton_query::AutomatonQuery::get_automaton` | P? | **P** | none | AutomatonQuery.java:155 getAutomaton |
| `search::automaton_query::AutomatonQuery::get_compiled` | P? | **P** | none | AutomatonQuery.java:159 getCompiled |
| `search::automaton_query::AutomatonQuery::is_automaton_binary` | P? | **P** | none | AutomatonQuery.java:164 isAutomatonBinary |
| `search::automaton_query::AutomatonQuery::new` | W? | **P** | none (the two-argument overload; Rust chains to `with_binary` as Java chains to the three-argument constructor) | AutomatonQuery.java:66 AutomatonQuery(Term, Automaton); its only Lucene callers are the overload chain (:79, :93); the subclass constructors call the four-argument form |
| `search::bayesian_score_estimator::BayesianScoreEstimator` | P? | **P** | none | BayesianScoreEstimator.java (class) |
| `search::bayesian_score_estimator::BayesianScoreEstimator::estimate` | W? | **P** | none (Rust `estimate` delegates to `estimate_with`, as Java's two-argument overload to the five-argument one) | BayesianScoreEstimator.java:183-185 estimate(IndexSearcher, String); its only Lucene caller is itself delegating |
| `search::bayesian_score_query::BayesianScoreQuery::get_alpha` | P? | **P** | none | BayesianScoreQuery.java:99 getAlpha |
| `search::bayesian_score_query::BayesianScoreQuery::get_base_rate` | P? | **P** | none | BayesianScoreQuery.java:109 getBaseRate |
| `search::bayesian_score_query::BayesianScoreQuery::get_beta` | P? | **P** | none | BayesianScoreQuery.java:104 getBeta |
| `search::bayesian_score_query::BayesianScoreQuery::get_query` | P? | **P** | none | BayesianScoreQuery.java:94 getQuery |
| `search::bayesian_score_query::BayesianScoreQuery::with_alpha_beta` | D? | **P** | none (renamed overload; doc names it) | BayesianScoreQuery.java:89 BayesianScoreQuery(Query, float, float) |
| `search::binary_sort_field::BinarySortField::comparator` | W? | **W** | wired: `get_index_sorter` builds the `BinarySorter` with `self.comparator()`, as Java (it called the private `binary_comparator` free function) | BinarySortField.java:163-176 comparator(); caller :178-182 getIndexSorter |
| `search::binary_sort_field::BinarySortField::get_index_sorter` | W? | **W** | wired: `SortField::get_index_sorter` dispatches a `Binary` kind through `BinarySortField::from_sort_field(..).get_index_sorter()`, as it already did for `SortedNumeric`/`SortedSet` (it built the sorter through the free function `binary_index_sorter`, now removed) | BinarySortField.java:178-182 getIndexSorter overrides SortField.getIndexSorter; callers IndexWriterConfig.setIndexSort, IndexingChain.maybeSortSegment, MultiSorter.sort, Lucene99SegmentInfoFormat (through SortField dispatch) |
| `search::binary_sort_field::BinarySortField::get_sort_key_doc_values` | W? | **W** | wired: the `BinarySorter`'s values provider calls `get_sort_key_doc_values` (Java `this::getSortKeyDocValues`; it called `DocValues::get_binary` itself). The `getComparator` override's call stays W=: the Rust `TermValComparator` reads `DocValues.getBinary`, the default `getSortKeyDocValues`, and Rust has no subclass to override it | BinarySortField.java:160-162; callers :178-182 getIndexSorter, :185-194 getComparator |
| `search::binary_sort_field::BinarySortField::new` | W? | **P** | none (the two-argument overload; `read_sort_field` calls `with_missing_value`, Java's `Provider.readSortField` the three-argument constructor) | BinarySortField.java BinarySortField(String, boolean); Lucene callers are the overload chain only |
| `search::binary_sort_field::BinarySortField::sort_field` | D? | **P** | doc names the upcast it stands for | `BinarySortField extends SortField` (BinarySortField.java:42): the borrow of the wrapped `SortField` is the upcast the declared `SortFieldKind` divergence (sort.rs) needs |
| `search::blended_term_query::Builder::add_boosted` | D? | **P** | none (renamed overload; doc names it) | BlendedTermQuery.java:85 Builder.add(Term, float) |
| `search::blended_term_query::Builder::add_term` | D? | **P** | none (renamed overload; doc names it) | BlendedTermQuery.java:77 Builder.add(Term) |
| `search::boolean_query::BooleanQuery::get_clauses` | W? | **W=** | none: Java's callers read only the size or emptiness of the view; the Rust callers read `clause_count` (documented as `getClauses(occur).size()`), which counts the same set without the copy `get_clauses` returns | BooleanQuery.java:156; callers :167 isPureDisjunction, :576-578 rewrite, BooleanWeight.java:173-174 count (boolean_query.rs:286, boolean_weight.rs:351) |
| `search::boolean_query::Builder::add_all` | D? | **P** | none (renamed overload; doc names it) | BooleanQuery.java:97 Builder.add(Collection<BooleanClause>) |
| `search::boolean_scorer_supplier::ClauseSuppliers::get` | D? | **D** | removed | no Lucene member: Java's `subs` is an `EnumMap<Occur, Collection<ScorerSupplier>>` and `subs.get(occur)` a JDK call; the Rust code reads the per-occur fields; no caller |
| `search::boolean_weight::BooleanWeight::similarity` | D? | **P** | none | BooleanWeight.java:32 `final Similarity similarity` (assigned at :54, read by no Lucene class) |
| `search::boolean_weight::BooleanWeight::weighted_clauses` | W? | **W=** | none: Java's field, read by BooleanWeight's own methods; the Rust methods read the field | BooleanWeight.java:46; readers :72-297 |
| `search::byte_vector_similarity_query::ByteVectorSimilarityQuery` | P? | **P** | none | ByteVectorSimilarityQuery.java (class) |
| `search::byte_vector_similarity_query::ByteVectorSimilarityQuery::get_search_strategy` | D? | **P** | none (inherited public getter; doc names it) | AbstractVectorSimilarityQuery.getSearchStrategy() (public), no Lucene caller |
| `search::byte_vector_similarity_query::ByteVectorSimilarityQuery::new` | W? | **P** | none (the three-argument overload; Rust chains to `with_filter` as Java chains its constructors) | ByteVectorSimilarityQuery.java ByteVectorSimilarityQuery(String, byte[], float); Lucene callers are the overload chain only |
| `search::caching_collector::CachingCollector` | P? | **P** | none | CachingCollector.java (class) |
| `search::caching_collector::CachingCollector::create_standalone` | D? | **P** | none (renamed overload; doc names it) | CachingCollector.java create(boolean, double) |
| `search::caching_collector::CachingCollector::into_inner` | D? | **D** | removed | no Lucene member: Java's subclasses read the `protected final Collector in` field of `FilterCollector`; there is no unwrapping method; no caller |
| `search::caching_collector::CachingCollector::replay` | P? | **P** | none | CachingCollector.java replay(Collector) |
| `search::collection_terminated_exception::CollectionTerminatedException::fill_in_stack_trace` | P? | **P** | none | CollectionTerminatedException.java fillInStackTrace() |
| `search::collection_terminated_exception::CollectionTerminatedException::new` | W? | **W=** | none: Java's callers `throw new CollectionTerminatedException()`; the Rust callers return `CollectionError::CollectionTerminated`, the variant `From<CollectionTerminatedException>` yields (collection_terminated_exception.rs:139-143); the unit struct needs no constructor call | CollectionTerminatedException.java:27; callers TopFieldCollector.java thresholdCheck (top_field_collector.rs:599), TotalHitCountCollector.getLeafCollector (total_hit_count_collector.rs:104), MultiCollector, SpatialQuery.hasAnyHits |
| `search::collector::FilterLeafCollector` | W? | **W=** | none: Java's four subclasses (`CachingCollector.NoScoreCachingLeafCollector`, `ConstantScoreQuery`'s anonymous wrapper, `PositiveScoresOnlyCollector`'s anonymous leaf, `ScoreCachingWrappingLeafCollector`) are ported as `LeafCollector` impls holding the wrapped collector, overriding the same methods and calling it where Java calls `super`; the methods `FilterLeafCollector` leaves unoverridden are the trait defaults they also keep (caching_collector.rs:462, constant_score_query.rs:98, positive_scores_only_collector.rs:79, score_caching_wrapping_scorer.rs:108) | FilterLeafCollector.java; subclasses CachingCollector, ConstantScoreQuery, PositiveScoresOnlyCollector, ScoreCachingWrappingScorer |
| `search::collector::FilterLeafCollector::into_inner` | D? | **D** | removed | no Lucene member: Java's subclasses read the `protected final LeafCollector in` field (FilterLeafCollector.java); no unwrapping method; no caller |
| `search::collector::FilterLeafCollector::new` | W? | **W=** | none: as `FilterLeafCollector` | FilterLeafCollector.java FilterLeafCollector(LeafCollector); callers the four subclass constructors |
| `search::collector::SimpleCollector::inner` | D? | **D** | removed | no Lucene member: Java's `SimpleCollector` is an abstract class its subclasses extend; there is no wrapped behaviour to expose; no caller |
| `search::collector::SimpleCollector::inner_mut` | D? | **D** | removed | as `SimpleCollector::inner` |
| `search::collector::SimpleCollector::into_inner` | D? | **D** | removed | as `SimpleCollector::inner` |
| `search::combined_field_query::Builder` | P? | **P** | none | CombinedFieldQuery.Builder |
| `search::combined_field_query::Builder::add_field` | W? | **P** | none (Rust chains to `add_field_with_weight` as Java's one-argument overload to the two-argument one) | CombinedFieldQuery.java Builder.addField(String); its only Lucene caller is the overload chain |
| `search::combined_field_query::Builder::build` | P? | **P** | none | CombinedFieldQuery.java Builder.build() |
| `search::combined_field_query::Builder::new` | P? | **P** | none | CombinedFieldQuery.java Builder(String) |
| `search::combined_field_query::CombinedFieldQuery::term` | W? | **W=** | none: Java's field, read by `equals`/`hashCode`/`toString`; the Rust impls read the field | CombinedFieldQuery.java term field |
| `search::comparators::double_comparator::DoubleComparator::base` | D? | **P** | doc names the inheritance it stands for | `DoubleComparator extends NumericComparator` (DoubleComparator.java:30): the borrow of the composed base is the upcast; read by the portability test `comparator_sortable_bytes_to_long_matches_java` |
| `search::comparators::double_comparator::DoubleComparator::missing_value_as_comparable_long` | W? | **W** | wired: `NumericComparator`'s cached `missing_value_as_long` is set from `missing_value_as_comparable_long()`, as Java's constructor calls it (the constructor encoded the value inline) | DoubleComparator.java missingValueAsComparableLong; caller NumericComparator.java:68 (constructor) |
| `search::comparators::float_comparator::FloatComparator::base` | D? | **P** | doc names the inheritance it stands for | as `DoubleComparator::base` (FloatComparator extends NumericComparator) |
| `search::comparators::float_comparator::FloatComparator::missing_value_as_comparable_long` | W? | **W** | wired: as `DoubleComparator::missing_value_as_comparable_long` | FloatComparator.java missingValueAsComparableLong; caller NumericComparator.java:68 |
| `search::comparators::int_comparator::IntComparator::base` | D? | **P** | doc names the inheritance it stands for | as `DoubleComparator::base` (IntComparator extends NumericComparator) |
| `search::comparators::int_comparator::IntComparator::missing_value_as_comparable_long` | W? | **W** | wired: as `DoubleComparator::missing_value_as_comparable_long` | IntComparator.java missingValueAsComparableLong; caller NumericComparator.java:68 |
| `search::comparators::long_comparator::LongComparator::base` | D? | **P** | doc names the inheritance it stands for | as `DoubleComparator::base` (LongComparator extends NumericComparator) |
| `search::comparators::long_comparator::LongComparator::missing_value_as_comparable_long` | W? | **W** | wired: as `DoubleComparator::missing_value_as_comparable_long` | LongComparator.java missingValueAsComparableLong; caller NumericComparator.java:68 |
| `search::comparators::numeric_comparator::NumericComparator::field` | W? | **W=** | none: Java's field, read by the leaf comparator and the competitive-DISI builders; the Rust code reads the field | NumericComparator.java field; readers :367-460 |
| `search::comparators::numeric_comparator::NumericComparator::missing_value_as_comparable_long` | W? | **D** | removed | no Lucene member with this body: Java's `missingValueAsComparableLong()` is abstract in `NumericComparator` (NumericComparator.java:89), implemented by the four subclasses (rows above, now called by their constructors); the cache it returned is the private `missingValueAsLong` field (:53), which the Rust code reads in place (numeric_comparator.rs:537,558) |
| `search::comparators::numeric_comparator::NumericComparator::reverse` | W? | **W=** | none: Java's field, read by `CompetitiveDISIBuilder`; the Rust code reads the field | NumericComparator.java reverse field; readers encodeBottom/encodeTop/isMissingValueCompetitive |
| `search::comparators::numeric_comparator::NumericComparator::sortable_bytes_to_long` | W? | **W=** | none: Java's abstract method, called from the anonymous point visitor; the Rust visitor holds the same function pointer the method calls (numeric_comparator.rs:764,930,954-955), since it cannot borrow the comparator | NumericComparator.java:95; callers :415, :436-437 |
| `search::comparators::numeric_comparator::NumericComparator::top_value_set` | W? | **W=** | none: Java's field, read by `CompetitiveDISIBuilder`; the Rust code reads the field (numeric_comparator.rs:443) | NumericComparator.java topValueSet |
| `search::conjunction_disi::ConjunctionDISI::is_empty` | D? | **P** | none (`len`'s companion; removing it trips clippy `len_without_is_empty` on the kept `len`) | no Lucene member (ConjunctionDISI has no size accessor); kept beside `len` |
| `search::conjunction_disi::add_iterator` | W? | **W** | wired: `ConjunctionUtils::add_iterator` calls `conjunction_disi::add_iterator`, as Java's delegates to `ConjunctionDISI.addIterator` (it pushed the member itself) | ConjunctionDISI.java:54-77; callers ConjunctionUtils.java:94-99 addIterator, ConjunctionDISI.addScorer/addTwoPhaseIterator |
| `search::conjunction_disi::add_scorer` | W? | **W** | wired: `ConjunctionUtils::intersect_scorers` adds each scorer through `conjunction_disi::add_scorer`, as Java's static import (ConjunctionUtils.java:20,41); the port-only `ConjunctionUtils::add_scorer` (Java's ConjunctionUtils has no addScorer) is removed | ConjunctionDISI.java:42-52; caller ConjunctionUtils.java:41 intersectScorers |
| `search::conjunction_disi::add_two_phase_iterator` | W? | **W** | wired: `ConjunctionUtils::add_two_phase_iterator` calls `conjunction_disi::add_two_phase_iterator`, as Java (it pushed the member itself) | ConjunctionDISI.java:79-91; caller ConjunctionUtils.java:83-89 |
| `search::conjunction_utils::ConjunctionUtils::add_two_phase_iterator` | P? | **P** | none | ConjunctionUtils.java:83 addTwoPhaseIterator (public, no Lucene caller) |
| `search::conjunction_utils::ConjunctionUtils::create_conjunction` | W? | **W** | wired: `default_bulk`, `bulk_from_random_scorer_dense` and `bulk_from_random_scorer_sparse` intersect through `ConjunctionUtils::create_conjunction` over the two plain iterators, as Java (they called `intersect_iterators`, which Java's callers do not) | ConjunctionUtils.java:74-77; callers VectorScorer.java:65, :115, :143 |
| `search::conjunction_utils::ConjunctionUtils::intersect_scorers` | W? | **W=** | none: `ConjunctionScorer::new` calls `intersect_scorers_conjunction`, the same construction before its result is erased, because it keeps reaching its clauses' scorers through the conjunction (declared on that method) | ConjunctionUtils.java:34-45 intersectScorers; Java caller ConjunctionScorer.java constructor (`this.disi = ConjunctionUtils.intersectScorers(required)`) |
| `search::constant_score_scorer_supplier::ConstantScoreScorerSupplier::max_doc` | W? | **W=** | none: Java's private field, read by `bulkScorer()`; the Rust `bulk_scorer` reads the field (constant_score_scorer_supplier.rs:189) | ConstantScoreScorerSupplier.java:54,81 |
| `search::constant_score_weight::ConstantScoreWeight::inner` | D? | **D** | removed | no Lucene member: Java's anonymous subclasses are the weight; there is no accessor of a wrapped implementation (ConstantScoreWeight.java:30-69); no caller |
| `search::constant_score_weight::ConstantScoreWeight::score` | W? | **W** | wired: `ConstantScoreWeightImpl::scorer_supplier` receives the weight's `score()`, which every implementation now uses where it kept its own copy of the boost, as Java's anonymous subclasses call `score()` | ConstantScoreWeight.java:39-41 score(); callers the scorerSupplier of every ConstantScoreWeight subclass (TermRangeQuery/MultiTermQueryConstantScoreWrapper, PointRangeQuery, PointInSetQuery, FieldExistsQuery, DocValuesRewriteMethod, MatchAllDocsQuery, IndexSortSortedNumericDocValuesRangeQuery, document.* range queries, ...) |
| `search::controlled_real_time_reopen_thread::ControlledRealTimeReopenThread` | P? | **P** | none | ControlledRealTimeReopenThread.java (class) |
| `search::controlled_real_time_reopen_thread::ControlledRealTimeReopenThread::get_searching_gen` | P? | **P** | none | ControlledRealTimeReopenThread.java getSearchingGen() |
| `search::controlled_real_time_reopen_thread::ControlledRealTimeReopenThread::new` | P? | **P** | none | ControlledRealTimeReopenThread.java constructor |
| `search::controlled_real_time_reopen_thread::ControlledRealTimeReopenThread::start` | D? | **P** | none (stand-in for the inherited `Thread.start()`; declared at the type) | `ControlledRealTimeReopenThread extends Thread` (ControlledRealTimeReopenThread.java:40); `start()` is inherited |
| `search::controlled_real_time_reopen_thread::ControlledRealTimeReopenThread::wait_for_generation` | W? | **P** | none (Rust chains to `wait_for_generation_with_timeout(target, -1)` as Java's one-argument overload) | ControlledRealTimeReopenThread.java waitForGeneration(long); its only Lucene caller is the overload chain |
| `search::disi_wrapper::DisiWrapper::into_scorer` | D? | **D** | removed | no Lucene member: `DisiWrapper` exposes its `scorer`/`scorable` fields (DisiWrapper.java), no unwrapping method; no caller |
| `search::disjunction_disi_approximation::DisjunctionDISIApproximation::is_empty` | D? | **P** | none (`len`'s companion; removing it trips clippy `len_without_is_empty` on the kept `len`) | no Lucene member; kept beside `len` |
| `search::disjunction_disi_approximation::DisjunctionDISIApproximation::of` | P? | **P** | none | DisjunctionDISIApproximation.java of(Collection, long) |
| `search::disjunction_max_query::DisjunctionMaxQuery::get_tie_breaker_multiplier` | P? | **P** | none | DisjunctionMaxQuery.java getTieBreakerMultiplier() |
| `search::disjunction_max_query::DisjunctionMaxWeight::weights` | W? | **W=** | none: Java's field, read by the weight's own methods; the Rust methods read the field | DisjunctionMaxQuery.java DisjunctionMaxWeight.weights |
| `search::disjunction_scorer::DisjunctionScorer::match_cost` | D? | **D** | removed | no Lucene member: Java's `matchCost` is a private field of `DisjunctionScorer` read by its `TwoPhase.matchCost()`, which is ported (disjunction_scorer.rs:399); no caller of the inherent accessor |
| `search::doc_and_score_query::DocAndScoreQuery::visited` | W? | **P** | none | DocAndScoreQuery.java visited() (public; no Lucene caller reads it) |
| `search::doc_id_set::AllDocIdSet::max_doc` | D? | **D** | removed | no Lucene member: `DocIdSet.all(int)` returns an anonymous set with no `maxDoc` accessor (DocIdSet.java:58-78); no caller |
| `search::doc_id_set::all` | P? | **P** | none | DocIdSet.java:58 all(int) |
| `search::doc_id_set::empty` | W? | **W=** | none: `CacheAndCount::empty` builds `EmptyDocIdSet`, the unit value `empty()` returns, as Java's `CacheAndCount.EMPTY` holds `DocIdSet.EMPTY` (lru_query_cache.rs:106-112) | DocIdSet.java:30 EMPTY; caller LRUQueryCache.java:943 |
| `search::doc_id_set_bulk_iterator::DocIdSetBulkIterator` | P? | **P** | none | DocIdSetBulkIterator.java (interface) |
| `search::doc_values_iteration::BinaryDocValuesIterator` | D? | **W=** | none: built by the macro-generated `binary_as_iterator`, which `FieldExistsQuery` calls for a BINARY field (field_exists_query.rs:127) — a KG blind spot for macro bodies | Java upcast `BinaryDocValues extends DocValuesIterator extends DocIdSetIterator`; FieldExistsQuery.java:66 |
| `search::doc_values_iteration::BinaryDocValuesIterator::new` | D? | **W=** | none: as `BinaryDocValuesIterator` | as above |
| `search::doc_values_range_iterator::DocValuesRangeIterator::for_ordinal_set` | W? | **W** | wired: `DocValuesRewriteMethod`'s weight calls it on the singleton branch; `is_singleton` answers Java's `unwrapSingleton` test without consuming the values (doc_values_rewrite_method.rs) | DocValuesRangeIterator.java:178; Java caller DocValuesRewriteMethod.java:165-174 (MultiTermQueryDocValuesWrapper.createWeight scorer) |
| `search::doc_values_range_iterator::DocValuesRangeIterator::for_ordinal_set_bits` | D? | **P** | none (renamed overload; doc names it) | DocValuesRangeIterator.java:216 forOrdinalSet(SortedSetDocValues, DocValuesSkipper, long, long, LongBitSet); no Lucene caller |
| `search::doc_values_rewrite_method::MultiTermQueryDocValuesWrapper::get_field` | D? | **P** | none | DocValuesRewriteMethod.java MultiTermQueryDocValuesWrapper.getField() (public final, no Lucene caller) |
| `search::double_values::ScorerDoubleValues` | D? | **D** | removed (with its `search.rs` export) | duplicate: `double_values_source::ScorerDoubleValues` (double_values_source.rs:1046) is the anonymous `DoubleValues` Java's `DoubleValuesSource.fromScorer` returns, and `from_scorer` builds that one; this copy has no caller |
| `search::double_values::ScorerDoubleValues::new` | D? | **D** | removed | as `ScorerDoubleValues` |
| `search::double_values::with_default` | P? | **P** | none | DoubleValues.java withDefault(DoubleValues, double) |
| `search::double_values_source::DoubleValuesComparatorSource::new` | W? | **W** | wired: `get_sort_field_with_missing` builds the comparator source with `DoubleValuesComparatorSource::new` (it wrote the struct literal), as Java's `DoubleValuesSortField` constructor | DoubleValuesSource.java DoubleValuesComparatorSource(DoubleValuesSource, double); caller DoubleValuesSortField(DoubleValuesSource, boolean, double) |
| `search::double_values_source::as_numeric_doc_values` | W? | **W** | wired: the comparator's `new_leaf` builds its leaf over `as_numeric_doc_values(holder, double_to_long_bits)`, as Java's `DoubleLeafComparator.getNumericDocValues` | DoubleValuesSource.java:615; Java caller DoubleValuesComparatorSource.newComparator (:601) |
| `search::double_values_source::constant` | W? | **W** | wired: `explain` builds its score source with `constant(..)` (it built `ConstantValuesSource` itself), as Java | DoubleValuesSource.java:363 constant; caller :70-79 explain |
| `search::double_values_source::from_double_field` | P? | **P** | none | DoubleValuesSource.java fromDoubleField |
| `search::double_values_source::from_float_field` | P? | **P** | none | DoubleValuesSource.java fromFloatField |
| `search::double_values_source::from_int_field` | P? | **P** | none | DoubleValuesSource.java fromIntField |
| `search::double_values_source::from_query` | P? | **P** | none | DoubleValuesSource.java fromQuery |
| `search::double_values_source::from_scorer` | W? | **W=** | none (declared at `from_scorer`): Rust's exclusive borrow means a caller that must keep the scorable publishes its score through a shared cell, which reads the same number — `ScoreCellDoubleValues` in the comparator sources, `SharedScorerValues` in `RescoreTopNQuery` (rescore_top_n_query.rs:173-178) | DoubleValuesSource.java:440 fromScorer; callers :606 DoubleValuesComparatorSource, LongValuesSource LongValuesComparatorSource.newComparator, RescoreTopNQuery.java:94-100 |
| `search::double_values_source::get_sort_field` | P? | **P** | none | DoubleValuesSource.java getSortField(boolean) |
| `search::double_values_source::scores` | P? | **P** | none | DoubleValuesSource.java SCORES |
| `search::double_values_source::similarity_to_byte_query_vector` | D? | **P** | none (renamed overload; doc names it) | DoubleValuesSource.java:275 similarityToQueryVector(LeafReaderContext, byte[], String); no Lucene caller |
| `search::double_values_source::similarity_to_float_query_vector` | D? | **P** | none (renamed overload; doc names it) | DoubleValuesSource.java:260 similarityToQueryVector(LeafReaderContext, float[], String); no Lucene caller |
| `search::double_values_source::to_sortable_long_double_values_source` | P? | **P** | none | DoubleValuesSource.java toSortableLongDoubleValuesSource() |
| `search::double_values_source_rescorer::DoubleValuesSourceRescorer::combiner` | D? | **D** | removed | no Lucene member: Java's `combine` is an abstract method of the rescorer subclass itself (DoubleValuesSourceRescorer.java:42); there is no accessor of a combining object; no caller |
| `search::double_values_source_rescorer::DoubleValuesSourceRescorer::values_source` | W? | **W=** | none: Java's private field, read by `explain`/`rescore`; the Rust methods read the field (double_values_source_rescorer.rs:86,154) | DoubleValuesSourceRescorer.java:28,47,107 |
| `search::field_comparator::SharedFieldComparator::with` | D? | **D** | removed | no Lucene member: `SharedFieldComparator` is the port's shared-slot adapter (field_comparator.rs); no caller |
| `search::field_comparator::SharedFieldComparator::with_mut` | D? | **D** | removed | as `SharedFieldComparator::with` |
| `search::field_doc::FieldDoc::doc` | W? | **W=** | none: Java's field `ScoreDoc.doc`; the Rust callers read the composed `score_doc.doc` field | ScoreDoc.java doc field (FieldDoc extends ScoreDoc); readers FieldValueHitQueue.fillFields, TopDocs.merge, ... |
| `search::field_doc::FieldDoc::new` | W? | **P** | none (the two-argument constructor; Lucene's callers use the three-argument one, ported as `with_fields` and called by field_value_hit_queue.rs:221 and geo_fields.rs:529) | FieldDoc.java:49 FieldDoc(int, float); Java callers LatLonPoint.java:442 and FieldValueHitQueue.java:212 call FieldDoc(int, float, Object[]) |
| `search::field_doc::FieldDoc::score` | W? | **W=** | none: as `FieldDoc::doc` (field `ScoreDoc.score`) | ScoreDoc.java score field |
| `search::field_doc::FieldDoc::shard_index` | W? | **W=** | none: as `FieldDoc::doc` (field `ScoreDoc.shardIndex`) | ScoreDoc.java shardIndex field |
| `search::field_doc::FieldDoc::with_shard_index` | D? | **P** | none (renamed overload; doc names it) | FieldDoc.java:60 FieldDoc(int, float, Object[], int) |
| `search::field_exists_query::FieldExistsQuery::get_field` | P? | **P** | none | FieldExistsQuery.java getField() |
| `search::field_value_hit_queue::Entry::doc` | D? | **W=** | none: Java's inherited field `ScoreDoc.doc` (`Entry extends ScoreDoc`); the Rust callers read the composed `score_doc.doc` field (field_value_hit_queue.rs:221) | FieldValueHitQueue.java:34 Entry; reader :212 fillFields |
| `search::field_value_hit_queue::FieldValueHitQueue::clear` | D? | **P** | none (inherited public member; no Lucene caller on a FieldValueHitQueue) | util/PriorityQueue.java:275 clear() |
| `search::field_value_hit_queue::FieldValueHitQueue::max_size` | D? | **D** | removed | no Lucene member: `PriorityQueue.maxSize` is a private field with no accessor (util/PriorityQueue.java: public members are add, insertWithOverflow, top, pop, updateTop, size, clear, remove, iterator, addAll); no caller |
| `search::field_value_hit_queue::FieldValueHitQueue::top` | D? | **P** | none (inherited public member; no Lucene caller on a FieldValueHitQueue) | util/PriorityQueue.java:218 top() |
| `search::filter_matches_iterator::FilterMatchesIterator` | P? | **P** | none | FilterMatchesIterator.java (class) |
| `search::filter_matches_iterator::FilterMatchesIterator::into_inner` | D? | **D** | removed | no Lucene member: Java's subclasses read the `protected final MatchesIterator in` field (FilterMatchesIterator.java:26); no unwrapping method; no caller |
| `search::filter_matches_iterator::FilterMatchesIterator::new` | P? | **P** | none | FilterMatchesIterator.java:33 |
| `search::float_vector_similarity_query::FloatVectorSimilarityQuery` | P? | **P** | none | FloatVectorSimilarityQuery.java (class) |
| `search::float_vector_similarity_query::FloatVectorSimilarityQuery::get_search_strategy` | D? | **P** | none (inherited public getter; doc names it) | AbstractVectorSimilarityQuery.getSearchStrategy() |
| `search::float_vector_similarity_query::FloatVectorSimilarityQuery::new` | W? | **P** | none (overload chain to `with_filter`) | FloatVectorSimilarityQuery(String, float[], float); Lucene callers are the overload chain only |
| `search::full_precision_float_vector_similarity_values_source::FullPrecisionFloatVectorSimilarityValuesSource::get_similarity_scores` | P? | **P** | none | FullPrecisionFloatVectorSimilarityValuesSource.getSimilarityScores(LeafReaderContext) |
| `search::full_precision_float_vector_similarity_values_source::FullPrecisionFloatVectorSimilarityValuesSource::new` | W? | **P** | none (the three-argument constructor takes a non-optional function, so the two-argument `with_field_similarity` cannot chain to it; Java's caller `RescoreTopNQuery.createFullPrecisionRescorerQuery` uses the two-argument form, which Rust calls, rescore_top_n_query.rs:86) | FullPrecisionFloatVectorSimilarityValuesSource(float[], String, VectorSimilarityFunction); Java callers RescoreTopNQuery.java:149 (two-argument) |
| `search::fuzzy_query::FuzzyQuery` | P? | **P** | none | FuzzyQuery.java (class) |
| `search::fuzzy_query::FuzzyQuery::float_to_edits` | P? | **P** | none | FuzzyQuery.java floatToEdits |
| `search::fuzzy_query::FuzzyQuery::get_max_edits` | P? | **P** | none | FuzzyQuery.java getMaxEdits |
| `search::fuzzy_query::FuzzyQuery::get_max_expansions` | D? | **D** | removed | no Lucene member: `maxExpansions` is a private field of FuzzyQuery with no getter (FuzzyQuery.java; public getters getMaxEdits, getPrefixLength, getTranspositions, getTerm); no caller |
| `search::fuzzy_query::FuzzyQuery::get_prefix_length` | P? | **P** | none | FuzzyQuery.java getPrefixLength |
| `search::fuzzy_query::FuzzyQuery::get_term` | W? | **W** | wired: `get_terms_enum_with_atts` passes `self.get_term()` to the enum (it read `self.term`), as Java | FuzzyQuery.java:212 getTerm; caller :204-209 getTermsEnum |
| `search::fuzzy_query::FuzzyQuery::get_transpositions` | P? | **P** | none | FuzzyQuery.java getTranspositions |
| `search::fuzzy_query::FuzzyQuery::new` | W? | **P** | none (overload chain to `with_max_edits`) | FuzzyQuery(Term); Lucene callers are the overload chain only |
| `search::fuzzy_terms_enum::FuzzyTermsEnum::get_boost` | P? | **P** | none | FuzzyTermsEnum.java getBoost |
| `search::fuzzy_terms_enum::FuzzyTermsEnum::new` | W? | **P** | none (the five-argument constructor; `FuzzyQuery` calls the attribute-taking one, ported as `with_attributes`, fuzzy_query.rs:408) | FuzzyTermsEnum(Terms, Term, int, int, boolean); Java caller FuzzyQuery.java:208 calls the six-argument form |
| `search::fuzzy_terms_enum::FuzzyTermsEnum::prepare_seek_exact` | W? | **W** | wired: the override moves into `impl TermsEnum for FuzzyTermsEnum`, so callers dispatching through `dyn TermsEnum` reach the wrapped enum's `prepare_seek_exact` as Java (they reached the default, which wraps `seek_exact` and never answers `None`) | FuzzyTermsEnum.java:330-332 prepareSeekExact overrides TermsEnum.prepareSeekExact; callers TermStates.build, FilterTermsEnum.prepareSeekExact, CheckIndex (through TermsEnum dispatch) |
| `search::fuzzy_terms_enum::FuzzyTermsEnum::set_max_non_competitive_boost` | P? | **P** | none | FuzzyTermsEnum.java setMaxNonCompetitiveBoost |
| `search::hit_queue::HitQueue::add` | W? | **P** | none (inherited public member; no Lucene caller adds to a HitQueue) | util/PriorityQueue.java:187 add; Lucene's HitQueue users (AbstractKnnVectorQuery.exactSearch, RescoreTopNQuery, ScalarQuantizer) call top/updateTop/pop/insertWithOverflow |
| `search::hit_queue::HitQueue::clear` | W? | **P** | none (inherited public member; no Lucene caller clears a HitQueue) | util/PriorityQueue.java:275 clear |
| `search::hit_queue::HitQueue::update_top` | W? | **W=** | none: Java's `exactSearch` mutates `top()` in place and calls `updateTop()`; Rust cannot mutate through `top()`, so it calls `update_top_with(ScoreDoc::new(doc, score))`, Java's `updateTop(T)` overload (abstract_knn_vector_query.rs:325), which leaves the same heap | util/PriorityQueue.java:258 updateTop(); caller AbstractKnnVectorQuery.java:332-336 |
| `search::hnsw_queue_saturation_collector::HnswQueueSaturationCollector::next_candidate` | W? | **W=** | none (declared at `next_vectors_block`): Java's `Patience` holds the collector and calls `collector.nextCandidate()`; the Rust `Patience` holds the collector's shared state and advances the same counters (knn.rs:96-102, 398-407) | HnswQueueSaturationCollector.java nextCandidate; caller KnnSearchStrategy.java Patience.nextVectorsBlock |
| `search::hnsw_queue_saturation_collector::HnswQueueSaturationCollector::state` | D? | **D** | removed | no Lucene member: `HnswQueueSaturationCollector` exposes no accessor of its counters; no caller |
| `search::impacts_disi::ImpactsDISI::get_max_score_cache` | W? | **W=** | none: Java stores `impactsDisi.getMaxScoreCache()`; the Rust scorers read the cache through `ImpactsDISI::split_mut` (synonym_query.rs:885-891), which hands it out together with the impacts source it reads, as Rust's borrow rules require | ImpactsDISI.java:47; callers SynonymQuery.java:608, PhraseScorer.java:47 |
| `search::index_or_doc_values_query::IndexOrDocValuesQuery::get_index_query` | P? | **P** | none | IndexOrDocValuesQuery.java getIndexQuery |
| `search::index_or_doc_values_query::IndexOrDocValuesQuery::get_random_access_query` | P? | **P** | none | IndexOrDocValuesQuery.java getRandomAccessQuery |
| `search::index_searcher::IndexSearcher::count` | P? | **P** | none | IndexSearcher.java count(Query) |
| `search::index_searcher::IndexSearcher::get_default_query_cache` | P? | **P** | none | IndexSearcher.java getDefaultQueryCache() |
| `search::index_searcher::IndexSearcher::get_default_query_caching_policy` | P? | **P** | none | IndexSearcher.java getDefaultQueryCachingPolicy() |
| `search::index_searcher::IndexSearcher::get_default_similarity` | W? | **W** | wired: `IndexWriterConfig`'s default similarity is `IndexSearcher::get_default_similarity()`, the shared default instance, as Java's `LiveIndexWriterConfig` constructor (it built a new `BM25Similarity`) | IndexSearcher.java getDefaultSimilarity(); caller LiveIndexWriterConfig.java constructor (`similarity = IndexSearcher.getDefaultSimilarity()`) |
| `search::index_searcher::IndexSearcher::get_query_cache` | P? | **P** | none | IndexSearcher.java getQueryCache() |
| `search::index_searcher::IndexSearcher::get_query_caching_policy` | P? | **P** | none | IndexSearcher.java getQueryCachingPolicy() |
| `search::index_searcher::IndexSearcher::search_partitions` | D? | **P** | none (renamed overload; doc names it) | IndexSearcher.java:814 protected search(LeafReaderContextPartition[], Weight, Collector) |
| `search::index_searcher::IndexSearcher::search_with_collector` | D? | **P** | none (renamed overload; doc names it; used by tests) | IndexSearcher.java:644 search(Query, Collector) |
| `search::index_searcher::IndexSearcher::set_default_query_cache` | P? | **P** | none | IndexSearcher.java setDefaultQueryCache |
| `search::index_searcher::IndexSearcher::set_default_query_caching_policy` | P? | **P** | none | IndexSearcher.java setDefaultQueryCachingPolicy |
| `search::index_searcher::IndexSearcher::set_max_clause_count` | P? | **P** | none | IndexSearcher.java setMaxClauseCount |
| `search::index_searcher::IndexSearcher::set_query_caching_policy` | P? | **P** | none | IndexSearcher.java setQueryCachingPolicy |
| `search::index_searcher::IndexSearcher::set_similarity` | P? | **P** | none | IndexSearcher.java setSimilarity |
| `search::index_searcher::IndexSearcher::set_timeout` | P? | **P** | none | IndexSearcher.java setTimeout |
| `search::index_searcher::IndexSearcher::stored_fields` | P? | **P** | none | IndexSearcher.java storedFields() |
| `search::index_searcher::IndexSearcher::timed_out` | P? | **P** | none | IndexSearcher.java timedOut() |
| `search::index_searcher::LeafReaderContextPartition::max_docs` | W? | **W=** | none: Java's field, read by the `LeafSlice` constructor; the Rust constructor reads the field (index_searcher.rs:348-350) | IndexSearcher.java LeafReaderContextPartition.maxDocs; reader LeafSlice(...) :1042 |
| `search::index_searcher::LeafSlice::get_max_docs` | P? | **P** | none | IndexSearcher.java LeafSlice.getMaxDocs() |
| `search::index_searcher::TooManyClauses::get_max_clause_count` | P? | **P** | none | IndexSearcher.java TooManyClauses.getMaxClauseCount() |
| `search::index_searcher::TooManyNestedClauses::as_too_many_clauses` | D? | **P** | doc names the inheritance it stands for | `TooManyNestedClauses extends TooManyClauses` (IndexSearcher.java): the borrow of the composed parent is the upcast |
| `search::index_sort_sorted_numeric_doc_values_range_query::IndexSortSortedNumericDocValuesRangeQuery::get_fallback_query` | P? | **P** | none | IndexSortSortedNumericDocValuesRangeQuery.java getFallbackQuery() |
| `search::indri_and_query::IndriAndQuery` | W? | **W=** | none: Java's `IndriAndWeight` holds the `IndriAndQuery`; the Rust weight holds the query's `IndriQuery` base and the query itself as `parent_query` (indri_and_weight.rs:26-31; built by indri_and_query.rs:74) | IndriAndQuery.java; dependant IndriAndWeight.java:28 |
| `search::indri_and_query::IndriAndQuery::get_clauses` | P? | **P** | none | IndriQuery.java getClauses() (inherited) |
| `search::indri_and_query::IndriAndQuery::iter` | D? | **P** | none (inherited `Iterable` member; doc names it) | IndriQuery.java iterator() (`IndriQuery implements Iterable<BooleanClause>`) |
| `search::indri_and_query::IndriAndQuery::new` | P? | **P** | none | IndriAndQuery.java constructor |
| `search::indri_disjunction_scorer::IndriDisjunctionScorer::get_sub_matches` | W? | **W=** | none: Java's `score()`/`smoothingScore(int)` pass `getSubMatches()`; the Rust methods split `self` and pass the same `approximation` (indri_disjunction_scorer.rs:105-121), since a call through `get_sub_matches` would borrow all of `self` while `inner` is used | IndriDisjunctionScorer.java:54-70 |
| `search::knn::Hnsw::new` | W? | **W** | wired: `Patience::new` builds its `Hnsw` with `Hnsw::new`, which refuses a threshold outside `[0, 100]` with Java's message, as Java's `super(filteredSearchThreshold)` (it built the struct without the check); `Patience::new` now returns `Result`. `Hnsw::DEFAULT` stays a `const` literal of the valid default (Java's `new Hnsw(0)`) | KnnSearchStrategy.java Hnsw(int); callers :183 Patience constructor, :53 Hnsw.DEFAULT, AbstractVectorSimilarityQuery.java:48 |
| `search::knn::KnnSearchStrategy::next_vectors_block` | W? | **W** | wired: the HNSW searchers (`HnswGraphSearcher::search_level` for both graph kinds, `FilteredHnswGraphSearcher::search_level`) call `results.get_search_strategy().next_vectors_block()` after each candidate, as Java (they never called it, so the patience counter never advanced and `PatienceKnnVectorQuery` never stopped early) | KnnSearchStrategy.java:45 nextVectorsBlock; callers HnswGraphSearcher.java:363-365, FilteredHnswGraphSearcher.java:214-216, Seeded.nextVectorsBlock (:168) |
| `search::knn::Patience::state` | D? | **D** | removed | no Lucene member: `KnnSearchStrategy.Patience` exposes no accessor of its collector; no caller |
| `search::knn::Patience::use_filtered_search` | D? | **P** | none (inherited final member; doc names it) | KnnSearchStrategy.java Hnsw.useFilteredSearch(float) (`Patience extends Hnsw`) |
| `search::knn::TopKnnCollector::nodes` | D? | **T** | none: `#[cfg(test)]` helper of the HNSW tests | no Lucene member |
| `search::knn::TopKnnCollector::top_docs_with_scores` | D? | **T** | none: used only by codec and merger unit tests; the divergence is declared at the item | no Lucene member |
| `search::knn::multi_leaf_knn_collector::MultiLeafKnnCollector` | P? | **P** | none | MultiLeafKnnCollector.java (class) |
| `search::knn::multi_leaf_knn_collector::MultiLeafKnnCollector::new` | W? | **P** | none (overload chain to `with_greediness`) | MultiLeafKnnCollector(int, BlockingFloatHeap, AbstractKnnCollector); Lucene callers are the overload chain only |
| `search::knn::top_knn_collector_manager::TopKnnCollectorManager::k` | W? | **W=** | none: Java's field, read by `newCollector`; the Rust `new_collector` reads the field | TopKnnCollectorManager.java k |
| `search::knn::top_knn_collector_manager::TopKnnCollectorManager::with_k` | D? | **D** | removed (with its declared divergence) | no Lucene member: the only constructor is `TopKnnCollectorManager(int, IndexSearcher)` (TopKnnCollectorManager.java); no caller |
| `search::knn_byte_vector_query::KnnByteVectorQuery::get_field` | W? | **W=** | none: Java's `PatienceKnnVectorQuery`/`SeededKnnVectorQuery.getField()` call `delegate.getField()`; the Rust wrappers read `delegate.knn_base().get_field()` (patience_knn_vector_query.rs:162-164), the shared `AbstractKnnVectorQuery` state this getter also returns | AbstractKnnVectorQuery.getField(); callers PatienceKnnVectorQuery.java:240-242, SeededKnnVectorQuery.getField |
| `search::knn_byte_vector_query::KnnByteVectorQuery::get_filter` | W? | **W=** | none: as `get_field` (`get_filter`) | AbstractKnnVectorQuery.getFilter(); callers PatienceKnnVectorQuery.java:250-252, SeededKnnVectorQuery.getFilter |
| `search::knn_byte_vector_query::KnnByteVectorQuery::get_k` | W? | **W=** | none: as `get_field` (`get_k`) | AbstractKnnVectorQuery.getK(); callers PatienceKnnVectorQuery.java:245-247, SeededKnnVectorQuery.getK |
| `search::knn_byte_vector_query::KnnByteVectorQuery::get_search_strategy` | D? | **P** | none (inherited public getter; doc names it) | AbstractKnnVectorQuery.getSearchStrategy() |
| `search::knn_byte_vector_query::KnnByteVectorQuery::get_target_copy` | P? | **P** | none | KnnByteVectorQuery.java getTargetCopy() |
| `search::knn_float_vector_query::KnnFloatVectorQuery::get_field` | W? | **W=** | none: as `KnnByteVectorQuery::get_field` | as above |
| `search::knn_float_vector_query::KnnFloatVectorQuery::get_filter` | W? | **W=** | none: as `KnnByteVectorQuery::get_filter` | as above |
| `search::knn_float_vector_query::KnnFloatVectorQuery::get_k` | W? | **W=** | none: as `KnnByteVectorQuery::get_k` | as above |
| `search::knn_float_vector_query::KnnFloatVectorQuery::get_search_strategy` | D? | **P** | none (inherited public getter; doc names it) | AbstractKnnVectorQuery.getSearchStrategy() |
| `search::knn_float_vector_query::KnnFloatVectorQuery::get_target_copy` | P? | **P** | none | KnnFloatVectorQuery.java getTargetCopy() |
| `search::late_interaction_float_values_source::LateInteractionFloatValuesSource::new` | W? | **P** | none (overload chain to `with_score_function`) | LateInteractionFloatValuesSource(String, float[][]); Lucene callers are the overload chain only |
| `search::late_interaction_float_values_source::LateInteractionFloatValuesSource::query_vector` | W? | **W=** | none: Java's field, read by `equals`/`getValues`/`hashCode`/`toString`; the Rust impls read the field | LateInteractionFloatValuesSource.java queryVector |
| `search::late_interaction_rescorer::LateInteractionRescorer` | P? | **P** | none | LateInteractionRescorer.java (class) |
| `search::late_interaction_rescorer::LateInteractionRescorer::create` | W? | **P** | none (overload chain to `create_with_similarity`) | LateInteractionRescorer.create(String, float[][]); its only Lucene caller is itself delegating |
| `search::late_interaction_rescorer::LateInteractionRescorer::with_fallback_to_first_pass_score` | P? | **P** | none | LateInteractionRescorer.withFallbackToFirstPassScore |
| `search::live_field_values::LiveFieldValues` | P? | **P** | none | LiveFieldValues.java (class) |
| `search::live_field_values::LiveFieldValues::add` | P? | **P** | none | LiveFieldValues.java add |
| `search::live_field_values::LiveFieldValues::close` | P? | **P** | none | LiveFieldValues.java close |
| `search::live_field_values::LiveFieldValues::delete` | P? | **P** | none | LiveFieldValues.java delete |
| `search::live_field_values::LiveFieldValues::get` | P? | **P** | none | LiveFieldValues.java get |
| `search::live_field_values::LiveFieldValues::new` | P? | **P** | none | LiveFieldValues.java constructor |
| `search::log_odds_fusion_query::LogOddsFusionQuery::get_alpha` | P? | **P** | none | LogOddsFusionQuery.java getAlpha |
| `search::log_odds_fusion_query::LogOddsFusionQuery::get_weights` | P? | **P** | none | LogOddsFusionQuery.java getWeights |
| `search::log_odds_fusion_query::LogOddsFusionQuery::with_alpha` | D? | **P** | none (renamed overload; doc names it) | LogOddsFusionQuery.java:143 LogOddsFusionQuery(Collection, float) |
| `search::log_odds_fusion_query::LogOddsFusionQuery::with_clauses` | D? | **P** | none (renamed overload; doc names it; used by a portability test) | LogOddsFusionQuery.java:153 LogOddsFusionQuery(Collection) |
| `search::log_odds_fusion_query::LogOddsFusionQuery::with_weights` | D? | **P** | none (renamed overload; doc names it) | LogOddsFusionQuery.java:132 LogOddsFusionQuery(Collection, float, float[]) |
| `search::long_values_source::ConstantLongValuesSource::get_value` | P? | **P** | none | LongValuesSource.java ConstantLongValuesSource.getValue() |
| `search::long_values_source::LongValuesComparatorSource::new` | W? | **W** | wired: `get_sort_field_with_missing` builds the comparator source with `LongValuesComparatorSource::new` (it wrote the struct literal), as Java's `LongValuesSortField` constructor | LongValuesSource.java LongValuesComparatorSource(LongValuesSource, long); caller LongValuesSortField(LongValuesSource, boolean, long) |
| `search::long_values_source::as_numeric_doc_values` | W? | **W** | wired: the comparator's `new_leaf` builds its leaf over `as_numeric_doc_values(holder)`, as Java's `LongLeafComparator.getNumericDocValues` | LongValuesSource.java asNumericDocValues(LongValuesHolder); Java caller LongValuesComparatorSource.newComparator |
| `search::long_values_source::constant` | P? | **P** | none | LongValuesSource.java constant(long) |
| `search::long_values_source::from_int_field` | P? | **P** | none | LongValuesSource.java fromIntField |
| `search::long_values_source::get_sort_field` | P? | **P** | none | LongValuesSource.java getSortField(boolean) |
| `search::lru_query_cache::LRUQueryCache::assert_consistent` | D? | **P** | none (package-private test hook, faithful) | LRUQueryCache.java assertConsistent() (package-private; called by Lucene's tests only) |
| `search::lru_query_cache::LRUQueryCache::cached_queries` | D? | **P** | none (package-private test hook, faithful) | LRUQueryCache.java cachedQueries() (package-private; called by Lucene's tests only) |
| `search::lru_query_cache::LRUQueryCache::clear` | P? | **P** | none | LRUQueryCache.java clear() |
| `search::lru_query_cache::LRUQueryCache::clear_core_cache_key` | W? | **W=** | none: Java's `putIfAbsent` registers `this::clearCoreCacheKey` as the core's closed listener; the Rust listener calls the free function `clear_core_cache_key(&state, key)` this method wraps (lru_query_cache.rs:578,584), since it holds the shared state rather than the cache | LRUQueryCache.java clearCoreCacheKey(Object); caller putIfAbsent (closed listener) |
| `search::lru_query_cache::LRUQueryCache::clear_query` | P? | **P** | none | LRUQueryCache.java clearQuery(Query) |
| `search::lru_query_cache::LRUQueryCache::get_eviction_count` | P? | **P** | none | LRUQueryCache.java getEvictionCount() |
| `search::lru_query_cache::LRUQueryCache::get_skip_cache_factor` | P? | **P** | none | LRUQueryCache.java getSkipCacheFactor() |
| `search::lru_query_cache::LRUQueryCache::get_total_count` | P? | **P** | none | LRUQueryCache.java getTotalCount() |
| `search::lru_query_cache::LRUQueryCache::set_skip_cache_factor` | P? | **P** | none | LRUQueryCache.java setSkipCacheFactor(float) |
| `search::match_no_docs_query::MatchNoDocsQuery::reason` | W? | **W=** | none: Java's private field, read by `toString` and the weight's explanation; the Rust code reads the field | MatchNoDocsQuery.java reason |
| `search::matches::MatchesUtils::disjunction` | P? | **P** | none | MatchesUtils.java disjunction(List<MatchesIterator>) |
| `search::matches::MatchesUtils::disjunction_from_terms` | D? | **P** | none (renamed overload; doc names it) | MatchesUtils.java disjunction(LeafReaderContext, int, Query, String, BytesRefIterator) |
| `search::max_score_accumulator::MaxScoreAccumulator::set_mod_interval` | D? | **P** | none (writes Java's package-private, non-final field; Lucene sets it only from its tests) | MaxScoreAccumulator.java:31 `long modInterval` |
| `search::multi_collector::MultiCollector::get_collectors` | W? | **W=** | none (declared at `AnyCollector`): Java's `reduce` reads `getCollectors()[i]`; the Rust `reduce` takes them back by value (`MultiCollectorHandle::into_collectors`), since it hands them to the sub-managers' `reduce`, which consumes them | MultiCollector.java getCollectors(); caller MultiCollectorManager.java:65-68 reduce |
| `search::multi_collector::MultiCollector::wrap` | W? | **W=** | none (declared at `AnyCollector`): Java's `newCollector` returns `MultiCollector.wrap(collectors)`; the Rust manager applies wrap's single-collector rule on its typed handle (multi_collector_manager.rs:248-257), since `wrap` erases the collectors that `reduce` must recover | MultiCollector.java wrap(Collector...); caller MultiCollectorManager.java:55 |
| `search::multi_collector_manager::MultiCollectorManager` | P? | **P** | none | MultiCollectorManager.java (class) |
| `search::multi_collector_manager::MultiCollectorManager::new` | P? | **P** | none | MultiCollectorManager.java constructor |
| `search::multi_phrase_query::Builder::add` | W? | **P** | none (overload chain to `add_terms`; `QueryBuilder::analyze_multi_phrase` calls `add_terms`/`add_at`, Java's `add(Term[])`/`add(Term[], int)`, util/query_builder.rs:626-640) | MultiPhraseQuery.java:104 Builder.add(Term); Java callers are the overload chain; QueryBuilder.java:481-494 calls add(Term[]) and add(Term[], int) |
| `search::multi_phrase_query::Builder::from_query` | D? | **P** | none (renamed copy constructor; doc names it) | MultiPhraseQuery.java:73 Builder(MultiPhraseQuery) |
| `search::multi_phrase_query::MultiPhraseQuery::get_field` | D? | **D** | removed | no Lucene member: `field` is a private field of MultiPhraseQuery with no getter (public members getSlop, getTermArrays, getPositions; MultiPhraseQuery.java:166-176); no caller |
| `search::multi_phrase_query::UnionPostingsEnum::pos_queue_doc` | W? | **W=** | none: Java's field, read by `freq()`; the Rust `freq` reads the field (multi_phrase_query.rs:765) | MultiPhraseQuery.java:424 posQueueDoc; readers :443, :610 |
| `search::multi_phrase_query::UnionPostingsEnum::set_pos_queue_doc` | D? | **D** | removed | no Lucene member: Java writes the field in place (MultiPhraseQuery.java:454), as the Rust code does (multi_phrase_query.rs:779); no caller |
| `search::multi_term_query::TopTermsBoostOnlyBooleanQueryRewrite` | P? | **P** | none | MultiTermQuery.java TopTermsBoostOnlyBooleanQueryRewrite |
| `search::multi_term_query::TopTermsBoostOnlyBooleanQueryRewrite::new` | P? | **P** | none | MultiTermQuery.java TopTermsBoostOnlyBooleanQueryRewrite(int) |
| `search::multi_term_query::TopTermsScoringBooleanQueryRewrite` | P? | **P** | none | MultiTermQuery.java TopTermsScoringBooleanQueryRewrite |
| `search::multi_term_query::TopTermsScoringBooleanQueryRewrite::new` | P? | **P** | none | MultiTermQuery.java TopTermsScoringBooleanQueryRewrite(int) |
| `search::multi_term_query::constant_score_boolean_rewrite` | P? | **P** | none | MultiTermQuery.java CONSTANT_SCORE_BOOLEAN_REWRITE |
| `search::multi_term_query::constant_score_rewrite` | P? | **P** | none | MultiTermQuery.java CONSTANT_SCORE_REWRITE |
| `search::multi_term_query::scoring_boolean_rewrite` | P? | **P** | none | MultiTermQuery.java SCORING_BOOLEAN_REWRITE |
| `search::multi_term_query_constant_score_blended_wrapper::MultiTermQueryConstantScoreBlendedWrapper::get_field` | D? | **P** | none (inherited public getter; doc names it) | AbstractMultiTermQueryConstantScoreWrapper.java:86 getField() (`MultiTermQueryConstantScoreBlendedWrapper extends AbstractMultiTermQueryConstantScoreWrapper`) |
| `search::multi_term_query_constant_score_blended_wrapper::MultiTermQueryConstantScoreBlendedWrapper::get_query` | D? | **P** | none (inherited public getter; doc names it) | AbstractMultiTermQueryConstantScoreWrapper.java:81 getQuery() |
| `search::multi_term_query_constant_score_wrapper::MultiTermQueryConstantScoreWrapper::get_field` | D? | **P** | none (inherited public getter; doc names it) | AbstractMultiTermQueryConstantScoreWrapper.java:86 getField() (`MultiTermQueryConstantScoreWrapper extends AbstractMultiTermQueryConstantScoreWrapper`) |
| `search::multi_term_query_constant_score_wrapper::MultiTermQueryConstantScoreWrapper::get_query` | D? | **P** | none (inherited public getter; doc names it) | AbstractMultiTermQueryConstantScoreWrapper.java:81 getQuery() |
| `search::multiset::Multiset::clear` | P? | **P** | none | Multiset.java clear() |
| `search::multiset::Multiset::count` | D? | **D** | removed | no Lucene member: Multiset's backing `Map<T, Integer>` is private and has no accessor (Multiset.java); no caller |
| `search::multiset::Multiset::entries` | D? | **D** | removed | as `Multiset::count` |
| `search::multiset::Multiset::remove` | P? | **P** | none | Multiset.java remove(Object) |
| `search::named_matches::NamedMatches::find_named_matches` | P? | **P** | none | NamedMatches.java findNamedMatches |
| `search::named_matches::NamedMatches::get_name` | P? | **P** | none | NamedMatches.java getName |
| `search::named_matches::wrap_query` | P? | **P** | none | NamedMatches.java wrapQuery |
| `search::ngram_phrase_query::NGramPhraseQuery` | P? | **P** | none | NGramPhraseQuery.java (class) |
| `search::ngram_phrase_query::NGramPhraseQuery::get_n` | P? | **P** | none | NGramPhraseQuery.java:100 getN |
| `search::ngram_phrase_query::NGramPhraseQuery::get_phrase_query` | D? | **D** | removed | no Lucene member: `phraseQuery` is a private field of NGramPhraseQuery with no getter (public getN, getTerms, getPositions; NGramPhraseQuery.java:100-112); no caller |
| `search::ngram_phrase_query::NGramPhraseQuery::get_positions` | P? | **P** | none | NGramPhraseQuery.java:110 getPositions |
| `search::ngram_phrase_query::NGramPhraseQuery::get_terms` | P? | **P** | none | NGramPhraseQuery.java:105 getTerms |
| `search::ngram_phrase_query::NGramPhraseQuery::new` | P? | **P** | none | NGramPhraseQuery.java:39 constructor |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery` | P? | **P** | none | PatienceKnnVectorQuery.java (class) |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::delegate` | W? | **W=** | none: Java's private field, read by `approximateSearch`/`createVectorScorer`/...; the Rust impl reads the field | PatienceKnnVectorQuery.java delegate |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::from_byte_query` | P? | **P** | none | PatienceKnnVectorQuery.java fromByteQuery(KnnByteVectorQuery, double, int) |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::from_float_query` | P? | **P** | none | PatienceKnnVectorQuery.java fromFloatQuery(KnnFloatVectorQuery, double, int) |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::from_query` | D? | **P** | none (renamed overloads; doc names them) | PatienceKnnVectorQuery.java fromFloatQuery(KnnFloatVectorQuery), fromByteQuery(KnnByteVectorQuery), fromSeededQuery(SeededKnnVectorQuery) |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::from_seeded_query` | P? | **P** | none | PatienceKnnVectorQuery.java fromSeededQuery(SeededKnnVectorQuery, double, int) |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::get_field` | W? | **W=** | none: as `KnnByteVectorQuery::get_field` (a wrapping `SeededKnnVectorQuery` reads `delegate.knn_base()`) | PatienceKnnVectorQuery.java:240-242; callers itself (delegating), SeededKnnVectorQuery.getField |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::get_filter` | W? | **W=** | none: as `get_field` (`get_filter`) | PatienceKnnVectorQuery.java:250-252 |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::get_k` | W? | **W=** | none: as `get_field` (`get_k`) | PatienceKnnVectorQuery.java:245-247 |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::patience` | W? | **W=** | none: Java's field, read by `equals`/`hashCode`/`toString`/`PatienceCollectorManager.newCollector`; the Rust code reads the field | PatienceKnnVectorQuery.java patience |
| `search::patience_knn_vector_query::PatienceKnnVectorQuery::saturation_threshold` | W? | **W=** | none: as `patience` (`saturationThreshold`) | PatienceKnnVectorQuery.java saturationThreshold |
| `search::phrase_matcher::IteratorWithImpacts::iterator_mut` | D? | **D** | removed | no Lucene member: `IteratorWithImpacts` is the port's pairing of an iterator with its impacts source; no caller of the mutable accessor |
| `search::phrase_query::Builder::add` | W? | **P** | none (overload chain to `add_at`; `QueryBuilder::analyze_phrase` and `NGramPhraseQuery::rewrite` call `add_at`, Java's `add(Term, int)`) | PhraseQuery.java Builder.add(Term); Java callers NGramPhraseQuery.rewrite and QueryBuilder.analyzePhrase call add(Term, int) |
| `search::phrase_query::Builder::set_max_terms` | P? | **P** | none | PhraseQuery.java Builder.setMaxTerms |
| `search::phrase_query::PhraseQuery::from_bytes` | D? | **P** | none (renamed overload; doc names it) | PhraseQuery.java PhraseQuery(String, BytesRef...) |
| `search::phrase_query::PhraseQuery::from_strings` | D? | **P** | none (renamed overload; doc names it) | PhraseQuery.java PhraseQuery(String, String...) |
| `search::phrase_query::PhraseQuery::get_field` | P? | **P** | none | PhraseQuery.java getField() |
| `search::phrase_query::PhraseQuery::term_not_in_reader` | W? | **W** | wired: the phrase matcher's "term does not exist in this segment" branch runs `debug_assert!(term_not_in_reader(..), "no termstate found but term exists in reader")`, Java's `assert` (Rust's debug assertions stand for `-ea`); the helper now takes the `LeafReader`, as Java | PhraseQuery.java:558-560 termNotInReader; caller :526 (assert in getPhraseMatcher) |
| `search::phrase_query::PhraseQuery::to_terms_from_bytes` | D? | **W** | wired: `with_slop_bytes` builds its terms through `to_terms_from_bytes`, as Java's constructor calls `toTerms(field, terms)` (it mapped them inline) | PhraseQuery.java toTerms(String, BytesRef...); caller :256-258 PhraseQuery(int, String, BytesRef...) |
| `search::phrase_query::PostingsAndFreq::n_terms` | W? | **W=** | none: Java's field, read by the `PostingsAndFreq` constructor, `compareTo`, `hashCode`, `equals`; the Rust code reads the field | PhraseQuery.java PostingsAndFreq.nTerms |
| `search::phrase_queue::PhraseQueue::size` | W? | **P** | none (inherited public member; `SloppyPhraseMatcher` never asks the queue's size) | util/PriorityQueue.java:270 size(); SloppyPhraseMatcher.java uses pop/top/add/clear only |
| `search::phrase_queue::PhraseQueue::update_top` | W? | **P** | none (inherited public member; `SloppyPhraseMatcher` never calls it) | util/PriorityQueue.java:258 updateTop(); SloppyPhraseMatcher.java uses pop/top/add/clear only |
| `search::phrase_weight::PhraseWeight::field` | W? | **W=** | none: Java's field, read by the weight's own methods; the Rust methods read the field (phrase_weight.rs:157,183) | PhraseWeight.java field |
| `search::phrase_weight::PhraseWeight::score_mode` | W? | **W=** | none: as `field` (`scoreMode`; phrase_weight.rs:156,161) | PhraseWeight.java scoreMode |
| `search::phrase_weight::PhraseWeight::stats` | W? | **W=** | none: as `field` (`stats`; phrase_weight.rs:153,193,207) | PhraseWeight.java stats |
| `search::point_in_set_query::PointInSetQuery::get_bytes_per_dim` | P? | **P** | none | PointInSetQuery.java getBytesPerDim |
| `search::point_in_set_query::PointInSetQuery::get_field` | P? | **P** | none | PointInSetQuery.java getField |
| `search::point_in_set_query::PointInSetQuery::get_num_dims` | P? | **P** | none | PointInSetQuery.java getNumDims |
| `search::point_in_set_query::PointInSetQuery::get_packed_points` | P? | **P** | none | PointInSetQuery.java getPackedPoints |
| `search::point_range_query::PointRangeQuery::get_bytes_per_dim` | P? | **P** | none | PointRangeQuery.java getBytesPerDim |
| `search::point_range_query::PointRangeQuery::get_lower_point` | P? | **P** | none | PointRangeQuery.java getLowerPoint |
| `search::point_range_query::PointRangeQuery::get_num_dims` | P? | **P** | none | PointRangeQuery.java getNumDims |
| `search::point_range_query::PointRangeQuery::get_upper_point` | P? | **P** | none | PointRangeQuery.java getUpperPoint |
| `search::positive_scores_only_collector::PositiveScoresOnlyCollector` | P? | **P** | none | PositiveScoresOnlyCollector.java (class) |
| `search::positive_scores_only_collector::PositiveScoresOnlyCollector::into_inner` | D? | **D** | removed | no Lucene member: as `CachingCollector::into_inner` (FilterCollector's protected `in` field); no caller |
| `search::positive_scores_only_collector::PositiveScoresOnlyCollector::new` | P? | **P** | none | PositiveScoresOnlyCollector.java constructor |
| `search::prefix_query::PrefixQuery` | W? | **P** | none (DateTools only names it in javadoc: `{@link PrefixQuery}`, DateTools.java:25,34) | PrefixQuery.java (class) |
| `search::prefix_query::PrefixQuery::automaton_query` | D? | **P** | none (the declared upcast stand-in; doc states it) | `PrefixQuery extends AutomatonQuery` (PrefixQuery.java) |
| `search::prefix_query::PrefixQuery::get_prefix` | P? | **P** | none | PrefixQuery.java getPrefix() |
| `search::prefix_query::PrefixQuery::new` | W? | **P** | none (overload chain to `with_rewrite_method`) | PrefixQuery(Term); its only Lucene caller is the overload chain |
| `search::query_rescorer::QueryRescorer::query` | W? | **W=** | none: Java's field, read by `explain`/`rescore`; the Rust methods read the field | QueryRescorer.java query |
| `search::query_rescorer::rescore` | W? | **P** | none | QueryRescorer.java static rescore(IndexSearcher, TopDocs, Query, double, int) (public, no Lucene caller) |
| `search::query_visitor::term_collector` | P? | **P** | none | QueryVisitor.java termCollector(Set<Term>) |
| `search::reference_manager::ReferenceManager::close` | P? | **P** | none | ReferenceManager.java close() |
| `search::reference_manager::ReferenceManager::is_closed` | D? | **T** | none: used only by `index::reader_manager` tests | no Lucene member: ReferenceManager has no `isClosed` (its closed state is `current == null`, checked by the private `ensureOpen`) |
| `search::reference_manager::ReferenceManager::maybe_refresh` | P? | **P** | none | ReferenceManager.java maybeRefresh() |
| `search::reference_manager::ReferenceManager::source` | D? | **D** | removed | no Lucene member: the refresh hooks are ReferenceManager's own abstract methods, with no accessor of a source object; no caller |
| `search::regexp_query::RegexpQuery` | P? | **P** | none | RegexpQuery.java (class) |
| `search::regexp_query::RegexpQuery::automaton_query` | D? | **P** | none (the declared upcast stand-in; doc states it) | `RegexpQuery extends AutomatonQuery` (RegexpQuery.java) |
| `search::regexp_query::RegexpQuery::get_regexp` | P? | **P** | none | RegexpQuery.java getRegexp() |
| `search::regexp_query::RegexpQuery::new` | W? | **P** | none (overload chain to `with_syntax_flags`) | RegexpQuery(Term); its only Lucene caller is the overload chain |
| `search::regexp_query::RegexpQuery::with_match_flags` | D? | **P** | none (renamed overload; doc names it) | RegexpQuery.java RegexpQuery(Term, int, int, int) |
| `search::regexp_query::RegexpQuery::with_work_limit` | D? | **P** | none (renamed overload; doc names it) | RegexpQuery.java RegexpQuery(Term, int, int) |
| `search::req_excl_bulk_scorer::ReqExclBulkScorer::from_iterator` | D? | **P** | none (renamed overload; doc names it) | ReqExclBulkScorer.java:38 ReqExclBulkScorer(BulkScorer, DocIdSetIterator) (package-private, no Lucene caller) |
| `search::req_excl_bulk_scorer::ReqExclBulkScorer::from_two_phase` | D? | **P** | none (renamed overload; doc names it) | ReqExclBulkScorer.java:44 ReqExclBulkScorer(BulkScorer, TwoPhaseIterator) (package-private, no Lucene caller) |
| `search::rescore_top_n_query::RescoreTopNQuery` | P? | **P** | none | RescoreTopNQuery.java (class) |
| `search::rescore_top_n_query::RescoreTopNQuery::create_full_precision_rescorer_query` | P? | **P** | none | RescoreTopNQuery.java createFullPrecisionRescorerQuery |
| `search::rescore_top_n_query::RescoreTopNQuery::create_late_interaction_query` | P? | **P** | none | RescoreTopNQuery.java createLateInteractionQuery |
| `search::rescore_top_n_query::RescoreTopNQuery::n` | W? | **W=** | none: Java's field, read by `equals`/`hashCode`/`rewrite`/`toString`; the Rust impls read the field | RescoreTopNQuery.java n |
| `search::rescore_top_n_query::RescoreTopNQuery::query` | W? | **W=** | none: as `n` (`query`) | RescoreTopNQuery.java query |
| `search::rescore_top_n_query::RescoreTopNQuery::values_source` | W? | **W=** | none: as `n` (`valuesSource`) | RescoreTopNQuery.java valuesSource |
| `search::score_caching_wrapping_scorer::ScoreCachingWrappingLeafCollector::into_inner` | D? | **D** | removed | no Lucene member: `ScoreCachingWrappingLeafCollector extends FilterLeafCollector`, whose `in` field is protected; no unwrapping method; no caller |
| `search::scorer::FilterScorer::into_inner` | D? | **P** | none (Java's public `unwrap()`; the consuming form is declared in its doc) | FilterScorer.java:68 unwrap() (`FilterScorer implements Unwrappable<Scorer>`); no Lucene caller unwraps a scorer |
| `search::scorer::ScorerAsIterator::into_scorer` | D? | **D** | removed | no Lucene member: the iterator view of a scorer has no unwrapping method; no caller |
| `search::scorer::ScorerAsTwoPhaseIterator::into_scorer` | D? | **D** | removed | as `ScorerAsIterator::into_scorer` |
| `search::scorer_util::ScorerUtil::apply_optional_clause` | W? | **W** | wired: `MaxScoreBulkScorer` calls it (max_score_bulk_scorer.rs) | ScorerUtil.java applyOptionalClause; Java caller MaxScoreBulkScorer.scoreNonEssentialClauses |
| `search::scorer_util::ScorerUtil::apply_required_clause` | W? | **W** | wired: `MaxScoreBulkScorer` and `BlockMaxConjunctionBulkScorer` call it | ScorerUtil.java applyRequiredClause; Java callers MaxScoreBulkScorer.scoreNonEssentialClauses, BlockMaxConjunctionBulkScorer.scoreWindowScoreFirst |
| `search::scorer_util::ScorerUtil::likely_impacts_enum` | W? | **W=** | none (declared at the item): the port's `likely_impacts_enum` is the identity, since the HotSpot receiver-shaping it performs has no Rust counterpart; the callers' iterators are what a call would return | ScorerUtil.java likelyImpactsEnum; callers BlockMaxConjunctionBulkScorer, BlockMaxConjunctionScorer, DisiWrapper constructors |
| `search::searcher_lifetime_manager::PruneByAge` | P? | **P** | none | SearcherLifetimeManager.java PruneByAge |
| `search::searcher_lifetime_manager::PruneByAge::new` | P? | **P** | none | SearcherLifetimeManager.java PruneByAge(double) |
| `search::searcher_lifetime_manager::SearcherLifetimeManager` | P? | **P** | none | SearcherLifetimeManager.java (class) |
| `search::searcher_lifetime_manager::SearcherLifetimeManager::acquire` | P? | **P** | none | SearcherLifetimeManager.java acquire(long) |
| `search::searcher_lifetime_manager::SearcherLifetimeManager::close` | P? | **P** | none | SearcherLifetimeManager.java close() |
| `search::searcher_lifetime_manager::SearcherLifetimeManager::prune` | P? | **P** | none | SearcherLifetimeManager.java prune(Pruner) |
| `search::searcher_lifetime_manager::SearcherLifetimeManager::record` | P? | **P** | none | SearcherLifetimeManager.java record(IndexSearcher) |
| `search::searcher_lifetime_manager::SearcherLifetimeManager::release` | P? | **P** | none | SearcherLifetimeManager.java release(IndexSearcher) |
| `search::searcher_manager::ManagedSearcher::searcher` | D? | **P** | none (the managed reference's searcher: Java's `acquire()` hands out the `IndexSearcher` itself; doc names it) | SearcherManager.java (ReferenceManager<IndexSearcher>) |
| `search::searcher_manager::SearcherManager` | W? | **P** | none (`DirectoryReader` and `ReaderManager` only name it in javadoc) | SearcherManager.java (class) |
| `search::searcher_manager::SearcherManager::from_directory` | D? | **P** | none (renamed constructor; doc names it) | SearcherManager.java SearcherManager(Directory, SearcherFactory) |
| `search::searcher_manager::SearcherManager::from_reader` | D? | **P** | none (renamed constructor; doc names it) | SearcherManager.java SearcherManager(DirectoryReader, SearcherFactory[, RefreshCommitSupplier]) |
| `search::searcher_manager::SearcherManager::from_writer` | D? | **P** | none (renamed constructor; doc names it) | SearcherManager.java SearcherManager(IndexWriter, boolean, boolean, SearcherFactory) |
| `search::searcher_manager::SearcherManager::get_searcher_commit_generation` | D? | **P** | none (package-private test hook, faithful) | SearcherManager.java getSearcherCommitGeneration() (called by Lucene's tests only) |
| `search::searcher_manager::SearcherManager::is_searcher_current` | D? | **P** | none (Java's public `isSearcherCurrent()`) | SearcherManager.java isSearcherCurrent() |
| `search::seeded_knn_vector_query::MappedDISI` | D? | **P** | none | SeededKnnVectorQuery.java MappedDISI |
| `search::seeded_knn_vector_query::MappedDISI::new` | W? | **W=** | none (declared at `mapped_seed_ordinals`): Java hands a lazy `MappedDISI` to `KnnSearchStrategy.Seeded`; the Rust strategy must be `Send + Sync`, so `mapped_seed_ordinals` drains the same mapping eagerly over the scorer's lent `DocIndexIterator` (seeded_knn_vector_query.rs:451-488) and both Java callers' ports call it (abstract_knn_vector_query.rs:799, seeded_knn_vector_query.rs:430) | SeededKnnVectorQuery.java MappedDISI(DocIndexIterator, DocIdSetIterator); callers :351 SeededCollectorManager.newCollector, AbstractKnnVectorQuery ReentrantKnnCollectorManager.newCollector |
| `search::seeded_knn_vector_query::SeededKnnVectorQuery::from_byte_query` | P? | **P** | none | SeededKnnVectorQuery.java fromByteQuery(KnnByteVectorQuery, Query) |
| `search::seeded_knn_vector_query::SeededKnnVectorQuery::from_query` | D? | **P** | none (renamed overloads; doc names them) | SeededKnnVectorQuery.java constructors (KnnFloatVectorQuery|KnnByteVectorQuery, Query, Weight), fromFloatQuery |
| `search::seeded_knn_vector_query::SeededKnnVectorQuery::get_field` | W? | **W=** | none: as `PatienceKnnVectorQuery::get_field` | SeededKnnVectorQuery.java getField() |
| `search::seeded_knn_vector_query::SeededKnnVectorQuery::get_filter` | W? | **W=** | none: as `PatienceKnnVectorQuery::get_filter` | SeededKnnVectorQuery.java getFilter() |
| `search::seeded_knn_vector_query::SeededKnnVectorQuery::get_k` | W? | **W=** | none: as `PatienceKnnVectorQuery::get_k` | SeededKnnVectorQuery.java getK() |
| `search::seeded_knn_vector_query::SeededKnnVectorQuery::seed_weight` | W? | **W=** | none: Java's field, read by `equals`/`hashCode`/`rewrite`/`toString`/`createWeight`; the Rust impls read the field | SeededKnnVectorQuery.java seedWeight |
| `search::sim_scorer_source::SimScorerSource::boost` | D? | **D** | removed | no Lucene member: `SimScorerSource` is the port's owned-scorer adapter, declared as having no counterpart (sim_scorer_source.rs module docs); no caller |
| `search::sim_scorer_source::SimScorerSource::collection_stats` | D? | **D** | removed | as `SimScorerSource::boost` |
| `search::sim_scorer_source::SimScorerSource::similarity` | D? | **D** | removed | as `SimScorerSource::boost` |
| `search::sim_scorer_source::SimScorerSource::term_stats` | D? | **D** | removed | as `SimScorerSource::boost` |
| `search::sim_scorer_source::SimScorerSource::with_bulk` | D? | **D** | removed | as `SimScorerSource::boost` (`asBulkSimScorer` is reached through the stored scorer) |
| `search::similarities::BM25Similarity::b` | W? | **P** | none (Java's public `getB()`; its other Java readers read the field `b`, as the Rust `scorer`/`Display` do) | BM25Similarity.java getB() |
| `search::similarities::BM25Similarity::k1` | W? | **P** | none (Java's public `getK1()`; as `b`) | BM25Similarity.java getK1() |
| `search::similarities::BM25Similarity::with_discount_overlaps` | D? | **P** | none (renamed constructor; doc names it) | BM25Similarity.java:84 BM25Similarity(boolean) |
| `search::similarities::BM25Similarity::with_parameters` | D? | **P** | none (renamed constructor; doc names it) | BM25Similarity.java:67 BM25Similarity(float, float) |
| `search::similarities::after_effect::AfterEffectB` | P? | **P** | none | AfterEffectB.java (class) |
| `search::similarities::after_effect::AfterEffectB::new` | P? | **P** | none | AfterEffectB() |
| `search::similarities::after_effect::AfterEffectL` | P? | **P** | none | AfterEffectL.java (class) |
| `search::similarities::after_effect::AfterEffectL::new` | P? | **P** | none | AfterEffectL() |
| `search::similarities::axiomatic::AxiomaticF1EXP::with_s` | D? | **P** | none (renamed constructor; doc names it) | AxiomaticF1EXP(float) |
| `search::similarities::axiomatic::AxiomaticF1LOG::with_s` | D? | **P** | none (renamed constructor; doc names it) | AxiomaticF1LOG(float) |
| `search::similarities::axiomatic::AxiomaticF2EXP::with_s` | D? | **P** | none (renamed constructor; doc names it) | AxiomaticF2EXP(float) |
| `search::similarities::axiomatic::AxiomaticF2LOG::with_s` | D? | **P** | none (renamed constructor; doc names it) | AxiomaticF2LOG(float) |
| `search::similarities::axiomatic::AxiomaticF3EXP::with_s_and_query_len` | D? | **P** | none (renamed constructor; doc names it) | AxiomaticF3EXP(float, int) |
| `search::similarities::axiomatic::AxiomaticF3LOG::with_s_and_query_len` | D? | **P** | none (renamed constructor; doc names it) | AxiomaticF3LOG(float, int) |
| `search::similarities::axiomatic::axiomatic_explain` | D? | **W=** | none: called by the macro-generated `SimilarityBase` impls of the six Axiomatic types (`axiomatic_boilerplate!`, similarities/axiomatic.rs:462-505) — a KG blind spot for macro bodies | Axiomatic.java:100-126 explain(BasicStats, Explanation, double), inherited by AxiomaticF1EXP..F3LOG |
| `search::similarities::axiomatic::axiomatic_score` | D? | **W=** | none: as `axiomatic_explain` (similarities/axiomatic.rs:491) | Axiomatic.java:88-98 score(BasicStats, double, double) |
| `search::similarities::basic_model::BasicModelG` | P? | **P** | none | BasicModelG (similarities) |
| `search::similarities::basic_model::BasicModelG::new` | P? | **P** | none | BasicModelG() (similarities) |
| `search::similarities::basic_model::BasicModelIF` | P? | **P** | none | BasicModelIF (similarities) |
| `search::similarities::basic_model::BasicModelIF::new` | P? | **P** | none | BasicModelIF() (similarities) |
| `search::similarities::basic_model::BasicModelIn` | P? | **P** | none | BasicModelIn (similarities) |
| `search::similarities::basic_model::BasicModelIn::new` | P? | **P** | none | BasicModelIn() (similarities) |
| `search::similarities::basic_model::BasicModelIne` | P? | **P** | none | BasicModelIne (similarities) |
| `search::similarities::basic_model::BasicModelIne::new` | P? | **P** | none | BasicModelIne() (similarities) |
| `search::similarities::basic_stats::BasicStats::field` | D? | **P** | none | BasicStats.java:27 `final String field` (package-private; read by no Lucene class) |
| `search::similarities::boolean_similarity::BooleanSimilarity` | P? | **P** | none | BooleanSimilarity.java (class) |
| `search::similarities::boolean_similarity::BooleanSimilarity::new` | P? | **P** | none | BooleanSimilarity() |
| `search::similarities::dfi_similarity::DFISimilarity` | P? | **P** | none | DFISimilarity.java (class) |
| `search::similarities::dfi_similarity::DFISimilarity::independence` | W? | **P** | none (Java's public `getIndependence()`; its other Java readers read the field, as the Rust `score`/`explain` do) | DFISimilarity.java getIndependence() |
| `search::similarities::dfi_similarity::DFISimilarity::new` | W? | **P** | none (overload chain to the two-argument constructor) | DFISimilarity(Independence); its only Lucene caller is the overload chain |
| `search::similarities::dfr_similarity::DFRSimilarity` | W? | **P** | none (`CombinedFieldQuery` only names it in javadoc, CombinedFieldQuery.java:37,64) | DFRSimilarity.java (class) |
| `search::similarities::dfr_similarity::DFRSimilarity::after_effect` | W? | **P** | none (Java's public `getAfterEffect()`; its other Java readers read the field, as the Rust `score`/`explain` do) | DFRSimilarity.java getAfterEffect() |
| `search::similarities::dfr_similarity::DFRSimilarity::basic_model` | W? | **P** | none (Java's public `getBasicModel()`; as `after_effect`) | DFRSimilarity.java getBasicModel() |
| `search::similarities::dfr_similarity::DFRSimilarity::new` | W? | **P** | none (overload chain to the four-argument constructor) | DFRSimilarity(BasicModel, AfterEffect, Normalization); its only Lucene caller is the overload chain |
| `search::similarities::dfr_similarity::DFRSimilarity::normalization` | W? | **P** | none (Java's public `getNormalization()`; as `after_effect`) | DFRSimilarity.java getNormalization() |
| `search::similarities::distribution::DistributionLL` | P? | **P** | none | DistributionLL.java (class) |
| `search::similarities::distribution::DistributionLL::new` | P? | **P** | none | DistributionLL() |
| `search::similarities::distribution::DistributionSPL` | P? | **P** | none | DistributionSPL.java (class) |
| `search::similarities::distribution::DistributionSPL::new` | P? | **P** | none | DistributionSPL() |
| `search::similarities::explanation::Explanation::description` | W? | **P** | none (Java's public `getDescription()`; its other Java readers read the field, as the Rust `PartialEq`/`Hash`/`Display` do) | Explanation.java getDescription() |
| `search::similarities::explanation::Explanation::details` | W? | **P** | none (Java's public `getDetails()`; as `description`) | Explanation.java getDetails() |
| `search::similarities::ib_similarity::IBSimilarity` | P? | **P** | none | IBSimilarity.java (class) |
| `search::similarities::ib_similarity::IBSimilarity::distribution` | W? | **P** | none (Java's public `getDistribution()`; its other Java readers read the field, as the Rust `score`/`explain` do) | IBSimilarity.java getDistribution() |
| `search::similarities::ib_similarity::IBSimilarity::lambda` | W? | **P** | none (Java's public `getLambda()`; as `distribution`) | IBSimilarity.java getLambda() |
| `search::similarities::ib_similarity::IBSimilarity::new` | W? | **P** | none (overload chain to the four-argument constructor) | IBSimilarity(Distribution, Lambda, Normalization); its only Lucene caller is the overload chain |
| `search::similarities::ib_similarity::IBSimilarity::normalization` | W? | **P** | none (Java's public `getNormalization()`; as `distribution`) | IBSimilarity.java getNormalization() |
| `search::similarities::independence::IndependenceChiSquared` | P? | **P** | none | IndependenceChiSquared (similarities) |
| `search::similarities::independence::IndependenceChiSquared::new` | P? | **P** | none | IndependenceChiSquared() (similarities) |
| `search::similarities::independence::IndependenceSaturated` | P? | **P** | none | IndependenceSaturated (similarities) |
| `search::similarities::independence::IndependenceSaturated::new` | P? | **P** | none | IndependenceSaturated() (similarities) |
| `search::similarities::independence::IndependenceStandardized` | P? | **P** | none | IndependenceStandardized (similarities) |
| `search::similarities::independence::IndependenceStandardized::new` | P? | **P** | none | IndependenceStandardized() (similarities) |
| `search::similarities::lambda::LambdaDF` | P? | **P** | none | LambdaDF (similarities) |
| `search::similarities::lambda::LambdaDF::new` | P? | **P** | none | LambdaDF() (similarities) |
| `search::similarities::lambda::LambdaTTF` | P? | **P** | none | LambdaTTF (similarities) |
| `search::similarities::lambda::LambdaTTF::new` | P? | **P** | none | LambdaTTF() (similarities) |
| `search::similarities::lm_similarity::IndriDirichletSimilarity` | P? | **P** | none | IndriDirichletSimilarity (similarities) |
| `search::similarities::lm_similarity::IndriDirichletSimilarity::mu` | W? | **P** | none (Java's public `getMu()`; its other Java readers read the field, as the Rust `score`/`explain` do) | IndriDirichletSimilarity.java getMu() |
| `search::similarities::lm_similarity::IndriDirichletSimilarity::with_collection_model` | D? | **P** | none (renamed constructor; doc names it) | IndriDirichletSimilarity(CollectionModel) |
| `search::similarities::lm_similarity::IndriDirichletSimilarity::with_collection_model_and_mu` | D? | **P** | none (renamed constructor; doc names it) | IndriDirichletSimilarity(CollectionModel, float) |
| `search::similarities::lm_similarity::IndriDirichletSimilarity::with_mu` | D? | **P** | none (renamed constructor; doc names it) | IndriDirichletSimilarity(float) |
| `search::similarities::lm_similarity::IndriDirichletSimilarity::with_parameters` | D? | **P** | none (renamed constructor; doc names it) | IndriDirichletSimilarity(CollectionModel, boolean, float) |
| `search::similarities::lm_similarity::LMDirichletSimilarity` | P? | **P** | none | LMDirichletSimilarity (class) |
| `search::similarities::lm_similarity::LMDirichletSimilarity::mu` | W? | **P** | none (Java's public `getMu()`; as `IndriDirichletSimilarity::mu`) | LMDirichletSimilarity.java getMu() |
| `search::similarities::lm_similarity::LMDirichletSimilarity::with_collection_model` | D? | **P** | none (renamed constructor; doc names it) | LMDirichletSimilarity(CollectionModel) |
| `search::similarities::lm_similarity::LMDirichletSimilarity::with_collection_model_and_mu` | D? | **P** | none (renamed constructor; doc names it) | LMDirichletSimilarity(CollectionModel, float) |
| `search::similarities::lm_similarity::LMDirichletSimilarity::with_mu` | D? | **P** | none (renamed constructor; doc names it) | LMDirichletSimilarity(float) |
| `search::similarities::lm_similarity::LMJelinekMercerSimilarity` | P? | **P** | none | LMJelinekMercerSimilarity (class) |
| `search::similarities::lm_similarity::LMJelinekMercerSimilarity::lambda` | W? | **P** | none (Java's public `getLambda()`; as `IndriDirichletSimilarity::mu`) | LMJelinekMercerSimilarity.java getLambda() |
| `search::similarities::lm_similarity::LMJelinekMercerSimilarity::with_collection_model_and_lambda` | D? | **P** | none (renamed constructor; doc names it) | LMJelinekMercerSimilarity(CollectionModel, float) |
| `search::similarities::lm_similarity::LMJelinekMercerSimilarity::with_lambda` | D? | **P** | none (renamed constructor; doc names it) | LMJelinekMercerSimilarity(float) |
| `search::similarities::multi_similarity::MultiSimilarity` | W? | **W=** | none: Java's `SimilarityBase.scorer` builds `MultiSimilarity.MultiSimScorer` for several terms; the Rust `similarity_base_scorer` builds the same nested type (similarities/similarity_base.rs:254) | SimilarityBase.java:66; MultiSimilarity.MultiSimScorer |
| `search::similarities::multi_similarity::MultiSimilarity::new` | P? | **P** | none | MultiSimilarity(Similarity[]) |
| `search::similarities::multi_similarity::MultiSimilarity::sims` | W? | **W=** | none: Java's protected field, read by `computeNorm`/`scorer`; the Rust methods read the field (similarities/multi_similarity.rs:66) | MultiSimilarity.java:34 sims; readers :43, :49-51 |
| `search::similarities::normalization::NoNormalization` | P? | **P** | none | Normalization.NoNormalization |
| `search::similarities::normalization::NoNormalization::new` | P? | **P** | none | Normalization.NoNormalization() |
| `search::similarities::normalization::NormalizationH1` | P? | **P** | none | NormalizationH1 (class) |
| `search::similarities::normalization::NormalizationH1::c` | W? | **P** | none (Java's public `getC()`; its other Java readers read the field, as the Rust `tfn`/`explain` do) | NormalizationH1.java getC() |
| `search::similarities::normalization::NormalizationH1::with_c` | D? | **P** | none (renamed constructor; doc names it) | NormalizationH1(float) |
| `search::similarities::normalization::NormalizationH2` | P? | **P** | none | NormalizationH2 (class) |
| `search::similarities::normalization::NormalizationH2::c` | W? | **P** | none (Java's public `getC()`; as `NormalizationH1::c`) | NormalizationH2.java getC() |
| `search::similarities::normalization::NormalizationH2::with_c` | D? | **P** | none (renamed constructor; doc names it) | NormalizationH2(float) |
| `search::similarities::normalization::NormalizationH3` | P? | **P** | none | NormalizationH3 (class) |
| `search::similarities::normalization::NormalizationH3::mu` | W? | **P** | none (Java's public `getMu()`; as `NormalizationH1::c`) | NormalizationH3.java getMu() |
| `search::similarities::normalization::NormalizationH3::with_mu` | D? | **P** | none (renamed constructor; doc names it) | NormalizationH3(float) |
| `search::similarities::normalization::NormalizationZ` | P? | **P** | none | NormalizationZ (class) |
| `search::similarities::normalization::NormalizationZ::with_z` | D? | **P** | none (renamed constructor; doc names it) | NormalizationZ(float) |
| `search::similarities::normalization::NormalizationZ::z` | W? | **P** | none (Java's public `getZ()`; as `NormalizationH1::c`) | NormalizationZ.java getZ() |
| `search::similarities::per_field_similarity_wrapper::per_field_compute_norm` | D? | **P** | none (body of a final method; lucene-core has no PerFieldSimilarityWrapper subclass to forward to it) | PerFieldSimilarityWrapper.java:36-39 computeNorm (final) |
| `search::similarities::per_field_similarity_wrapper::per_field_scorer` | D? | **P** | none (as `per_field_compute_norm`) | PerFieldSimilarityWrapper.java:41-45 scorer (final) |
| `search::similarities::raw_tf_similarity::RawTFSimilarity` | P? | **P** | none | RawTFSimilarity (class) |
| `search::similarities::raw_tf_similarity::RawTFSimilarity::with_discount_overlaps` | D? | **P** | none (renamed constructor; doc names it) | RawTFSimilarity(boolean) |
| `search::similarities::statistics::TermStatistics::term` | D? | **P** | none | TermStatistics.java (record) `term()` accessor |
| `search::similarities::tfidf_similarity::ClassicSimilarity` | P? | **P** | none | ClassicSimilarity.java (class) |
| `search::similarities::tfidf_similarity::ClassicSimilarity::with_discount_overlaps` | D? | **P** | none (renamed constructor; doc names it) | ClassicSimilarity.java:33 ClassicSimilarity(boolean) |
| `search::sort::Sort::rewrite` | W? | **W** | wired: `IndexSearcher::super_search_after_with_sort_and_scores`, the port of the private `searchAfter(FieldDoc, Query, int, Sort, boolean)`, rewrites the sort with it | Sort.java rewrite(IndexSearcher); Java caller IndexSearcher.java:736 |
| `search::sort::SortField::comparator_source` | W? | **W=** | none: the `DoubleValuesSortField`/`LongValuesSortField` overrides are ported inside `SortField` (`needs_scores`, `set_missing_value`, `rewrite`, `Display`), recognised by their comparator source, and read the field `comparator_source` directly | SortField.java getComparatorSource(); Java callers DoubleValuesSource.DoubleValuesSortField.rewrite/setMissingValue, LongValuesSource.LongValuesSortField.rewrite |
| `search::sort::SortField::get_optimize_sort_with_points` | P? | **P** | none | SortField.java getOptimizeSortWithPoints() (deprecated) |
| `search::sort::SortField::new` | W? | **W=** | none: Java's subclass constructors call `SortField(String, Type)`; their ports build the sort field with `SortField::with_kind`, which also records the subclass kind the declared `SortFieldKind` divergence needs (e.g. feature_field.rs:701) | SortField.java SortField(String, Type); callers FeatureSortField, LatLonPointSortField, XYPointSortField, ... constructors |
| `search::sort::SortField::new_reverse` | D? | **P** | none (renamed overload; used by tests) | SortField.java SortField(String, Type, boolean) |
| `search::sort::SortField::set_optimize_sort_with_points` | P? | **P** | none | SortField.java setOptimizeSortWithPoints(boolean) (deprecated) |
| `search::sort_rescorer::SortRescorer` | P? | **P** | none | SortRescorer.java (class) |
| `search::sort_rescorer::SortRescorer::new` | P? | **P** | none | SortRescorer.java SortRescorer(Sort) |
| `search::sorted_numeric_sort_field::SortedNumericSortField::new` | W? | **P** | none (the two-argument overload; the document fields' `newSortField` call the four-argument one, ported as `with_selector`) | SortedNumericSortField(String, SortField.Type); Java callers DoubleField/FloatField/IntField/LongField.newSortField use the four-argument form |
| `search::sorted_numeric_sort_field::SortedNumericSortField::sort_field` | D? | **P** | doc names the upcast it stands for | `SortedNumericSortField extends SortField`: as `BinarySortField::sort_field` |
| `search::sorted_set_sort_field::SortedSetSortField::new` | W? | **P** | none (the two-argument overload; `KeywordField.newSortField` and `SortedSetSortField.Provider` call the selector forms, ported as `with_selector`/`with_missing_value`) | SortedSetSortField(String, boolean); Java callers KeywordField.newSortField use the selector forms |
| `search::sorted_set_sort_field::SortedSetSortField::sort_field` | D? | **P** | doc names the upcast it stands for | `SortedSetSortField extends SortField`: as `BinarySortField::sort_field` |
| `search::synonym_query::Builder::add_term` | W? | **P** | none (overload chain to `add_term_with_boost`; `QueryBuilder::new_synonym_query` calls `add_bytes`, Java's `addTerm(BytesRef, float)`, util/query_builder.rs:768) | SynonymQuery.java Builder.addTerm(Term); Java caller QueryBuilder.java:606 calls addTerm(BytesRef, float) |
| `search::term_in_set_query::TermInSetQuery::get_bytes_ref_iterator` | P? | **P** | none | TermInSetQuery.java getBytesRefIterator() |
| `search::term_in_set_query::TermInSetQuery::new` | W? | **P** | none (the two-argument constructor; Java's other callers, `SortedDocValuesField`/`SortedSetDocValuesField.newSlowSetQuery`, use the rewrite-method form) | TermInSetQuery(String, Collection<BytesRef>); its Lucene callers are the overload chain and the three-argument form |
| `search::term_in_set_query::TermInSetQuery::term_data` | W? | **W=** | none: Java's private field, read by the constructor, `asByteRunAutomaton`, `equalsTo`, ...; the Rust code reads the field | TermInSetQuery.java termData |
| `search::term_matches_iterator::TermMatchesIterator::into_postings` | D? | **D** | removed (with its declared divergence) | no Lucene member: TermMatchesIterator holds `pe` with no accessor (TermMatchesIterator.java); no caller recovers it |
| `search::term_query::empty_term_scorer` | D? | **D** | removed | no Lucene member: Java's empty-scorer branch is inline in TermWeight's scorer supplier `get(long)`; the Rust function had no caller (its own doc records the branch as unreachable) |
| `search::term_range_query::TermRangeQuery` | W? | **P** | none (DateTools only names it in javadoc) | TermRangeQuery.java (class) |
| `search::term_range_query::TermRangeQuery::automaton_query` | D? | **P** | none (the declared upcast stand-in; doc states it) | `TermRangeQuery extends AutomatonQuery` |
| `search::term_range_query::TermRangeQuery::get_lower_term` | P? | **P** | none | TermRangeQuery.java getLowerTerm() |
| `search::term_range_query::TermRangeQuery::get_upper_term` | P? | **P** | none | TermRangeQuery.java getUpperTerm() |
| `search::term_range_query::TermRangeQuery::includes_lower` | P? | **P** | none | TermRangeQuery.java includesLower() |
| `search::term_range_query::TermRangeQuery::includes_upper` | P? | **P** | none | TermRangeQuery.java includesUpper() |
| `search::term_range_query::TermRangeQuery::new` | W? | **P** | none (the five-argument constructor; Java's `newStringRange` reaches the six-argument one, as the Rust `new_string_range` reaches `with_rewrite_method`) | TermRangeQuery(String, BytesRef, BytesRef, boolean, boolean); Lucene callers are the overload chain |
| `search::term_range_query::TermRangeQuery::new_string_range` | W? | **P** | none (overload chain to `new_string_range_with_rewrite_method`) | TermRangeQuery.newStringRange(String, String, String, boolean, boolean); its only Lucene caller is the overload chain |
| `search::top_docs::TopDocs::merge_sorted` | D? | **P** | none (renamed overload; doc names it) | TopDocs.java merge(Sort, int, TopFieldDocs[]) |
| `search::top_docs::TopDocs::rrf` | P? | **P** | none | TopDocs.java rrf(int, int, TopDocs[]) |
| `search::top_field_collector::TopFieldCollector::is_early_terminated` | P? | **P** | none | TopFieldCollector.java isEarlyTerminated() |
| `search::top_field_collector::TopFieldCollector::populate_scores` | W? | **W** | wired: `IndexSearcher::super_search_after_with_sort_and_scores` calls it when `do_doc_scores` is set | TopFieldCollector.java populateScores; Java caller IndexSearcher.java:742-744 |
| `search::top_field_collector::TopFieldCollector::total_hits_threshold` | W? | **W=** | none: Java's package-private field, read by `updateMinCompetitiveScore`/`countHit`/`thresholdCheck`; the Rust code reads the field | TopFieldCollector.java totalHitsThreshold |
| `search::top_field_collector_manager::TopFieldCollectorManager::get_collectors` | P? | **P** | none | TopFieldCollectorManager.java getCollectors() (deprecated) |
| `search::top_field_collector_manager::TopFieldCollectorManager::with_num_hits` | D? | **P** | none (renamed overload; doc names it) | TopFieldCollectorManager.java TopFieldCollectorManager(Sort, int, int) |
| `search::top_score_doc_collector::TopScoreDocCollector::min_score_acc` | W? | **W=** | none: Java's package-private field, read by `getLeafCollector`; the Rust code reads the field | TopScoreDocCollector.java minScoreAcc |
| `search::top_score_doc_collector::TopScoreDocCollector::total_hits_threshold` | W? | **W=** | none: as `min_score_acc` (`totalHitsThreshold`, read by `getLeafCollector`/`scoreMode`) | TopScoreDocCollector.java totalHitsThreshold |
| `search::top_score_doc_collector_manager::TopScoreDocCollectorManager::with_num_hits` | D? | **P** | none (renamed overload; doc names it) | TopScoreDocCollectorManager.java TopScoreDocCollectorManager(int, int) |
| `search::two_phase_iterator::TwoPhaseIteratorAsDocIdSetIterator::into_two_phase` | D? | **D** | removed | no Lucene member: `TwoPhaseIteratorAsDocIdSetIterator` exposes no unwrapping method beyond the field `TwoPhaseIterator.unwrap` reads; no caller of the consuming form |
| `search::two_phase_iterator::TwoPhaseIteratorAsDocIdSetIterator::two_phase` | D? | **P** | none (the field read Java's static `TwoPhaseIterator.unwrap(DocIdSetIterator)` performs after its `instanceof` test; doc names it) | TwoPhaseIterator.java unwrap(DocIdSetIterator) |
| `search::vector_scorer::default_bulk` | D? | **P** | none (the body of Java's default `bulk`; the crate's only production `VectorScorer`, `OffHeapVectorScorer`, overrides it as Java's off-heap scorers do) | VectorScorer.java:62-80 default bulk(DocIdSetIterator) |
| `search::vector_similarity_collector::VectorSimilarityCollector::new` | W? | **P** | none (overload chain to `with_strategy`; `AbstractVectorSimilarityQuery` calls `with_strategy`, Java's four-argument form, abstract_vector_similarity_query.rs:283) | VectorSimilarityCollector(float, float, long); Java caller AbstractVectorSimilarityQuery.java:126 uses the four-argument form |
| `search::wildcard_query::WildcardQuery` | P? | **P** | none | WildcardQuery.java (class) |
| `search::wildcard_query::WildcardQuery::automaton_query` | D? | **P** | none (the declared upcast stand-in; doc states it) | `WildcardQuery extends AutomatonQuery` |
| `search::wildcard_query::WildcardQuery::get_term` | P? | **P** | none | WildcardQuery.java getTerm() |
| `search::wildcard_query::WildcardQuery::new` | W? | **P** | none (overload chain to `with_work_limit`) | WildcardQuery(Term); its only Lucene caller is the overload chain |

## W-blocked items and their Java callers

At `225f596` (#435) none of the nine items below is blocked: each Java link now exists in Rust (rows above).

| item | Java caller | current Rust link |
|---|---|---|
| `search::conjunction_utils::ConjunctionUtils::intersect_scorers` | ConjunctionScorer constructor (`ConjunctionUtils.intersectScorers(required)`) | `ConjunctionScorer::new` calls `intersect_scorers_conjunction`, the unerased form (**W=**) |
| `search::doc_values_range_iterator::DocValuesRangeIterator::for_ordinal_set` | DocValuesRewriteMethod.java:165-174 (`unwrapSingleton` branch) | `DocValuesRewriteMethod`'s singleton branch, selected by `is_singleton` |
| `search::double_values_source::as_numeric_doc_values` | DoubleValuesSource.DoubleValuesComparatorSource.newComparator (:601) | the comparator's `new_leaf` |
| `search::long_values_source::as_numeric_doc_values` | LongValuesSource.LongValuesComparatorSource.newComparator | the comparator's `new_leaf` |
| `search::scorer_util::ScorerUtil::apply_optional_clause` | MaxScoreBulkScorer.scoreNonEssentialClauses | `MaxScoreBulkScorer` |
| `search::scorer_util::ScorerUtil::apply_required_clause` | MaxScoreBulkScorer.scoreNonEssentialClauses, BlockMaxConjunctionBulkScorer.scoreWindowScoreFirst | `MaxScoreBulkScorer`, `BlockMaxConjunctionBulkScorer` |
| `search::sort::Sort::rewrite` | IndexSearcher.java:736 private `searchAfter(FieldDoc, Query, int, Sort, boolean)` | `IndexSearcher::super_search_after_with_sort_and_scores` |
| `search::sort::SortField::comparator_source` | DoubleValuesSortField.rewrite/setMissingValue, LongValuesSortField.rewrite | the overrides ported inside `SortField` read the field directly (**W=**) |
| `search::top_field_collector::TopFieldCollector::populate_scores` | IndexSearcher.java:742-744 (`doDocScores`) | `IndexSearcher::super_search_after_with_sort_and_scores` |

## Removed as a consequence (not in the input list)

- `ConjunctionUtils::add_scorer`: port-only (Java's `ConjunctionUtils` statically imports `ConjunctionDISI.addScorer`, ConjunctionUtils.java:20); its only caller now calls `conjunction_disi::add_scorer`.
- `binary_sort_field::binary_index_sorter` (crate-private free function shared by the two index-sorter paths, now both `BinarySortField::get_index_sorter`).
- The `score` copies of the eleven `ConstantScoreWeightImpl` implementations (`PointRangeWeight`, `PointInSetWeight`, `FieldExistsWeight`, `MatchAllDocsWeight`, `ConstantScoreQueryWeight`, `IndexSortRangeWeight`, search's and document's `DocValuesWeightImpl`, `BinaryRangeWeight`, `RangeFieldWeight`, `SpatialWeight`): they now score with the weight's `score()`.
- The `ScoreCachingWrappingLeafCollector` inherent impl block (it held only `into_inner`); the `search.rs` re-export of `empty_term_scorer`; `search::ScorerDoubleValues` now re-exports `double_values_source::ScorerDoubleValues`, the type `from_scorer` returns; imports left unused (`term_query`, `sim_scorer_source`, `double_values`).

## Signature changes on the way

- `NumericComparator::new(field, reverse, pruning, bytes_count, sortable_bytes_to_long)`: the encoded missing value is no longer a parameter; each concrete constructor stores its override's result in the `pub(super)` field `missing_value_as_long`.
- `ConstantScoreWeightImpl::scorer_supplier(&self, context, score)` and `matches(&self, context, doc, score)`: `score` is the weight's `score()`; `SpatialWeight::new(matcher, score_mode)`.
- `Patience::new(state, threshold) -> Result<Patience>`.
- `PhraseQuery::term_not_in_reader(&Arc<dyn LeafReader>, &Term)` (Java takes the `LeafReader`).
- `FuzzyTermsEnum::prepare_seek_exact` is the `TermsEnum` override (no inherent method).

## Java oracle (pass S)

`scratchpad/oracle/passS/PatienceOracle.java` (run with `java -cp lucene-core-10.5.0.jar PatienceOracle.java`, output `out.txt`): 2000 deterministic 8-dimensional vectors, one segment, k = 50; plain `KnnFloatVectorQuery` 50 hits, 383 visited; `PatienceKnnVectorQuery.fromFloatQuery(q, 0.5, 2)` 43 hits, 71 visited, doc order pinned by `a_patience_query_stops_once_the_queue_saturates` (Rucene matches all three, observed).

## Declared divergences

Removed: `TopKnnCollectorManager::with_k` (with the item); `TermMatchesIterator::into_postings` (with the item); `NumericComparator::missing_value_as_comparable_long`'s "this port takes the encoded value at construction" (the constructors now call the override, as Java's).
Re-worded (each at its item): `NumericComparator.missing_value_as_long` (the shared constructor cannot call the override; the concrete constructor stores its result); `ConstantScoreWeightImpl::scorer_supplier` (the weight passes its `score()` because the implementation is a separate object); `PhraseQuery` matcher (Java's `assert termNotInReader` as `debug_assert!`, Rust's debug assertions standing for `-ea`).
New: none.

## Noticed, not acted on (outside scope)

- The Rust `IndexSearcher` has no sorted search (`search(Query, int, Sort[, boolean])`, `searchAfter(ScoreDoc, Query, int, Sort[, boolean])`); this blocks `Sort::rewrite` and `TopFieldCollector::populate_scores`.
- `DoubleValuesComparatorSource`/`LongValuesComparatorSource` build their own comparators where Java returns a `DoubleComparator`/`LongComparator` over `asNumericDocValues` (same values; the comparator choice is not declared), and the plain `CUSTOM` sort field never rewrites its producer (`SortField::rewrite` returns a clone), so a sort on `DoubleValuesSource.fromQuery` would read an unrewritten source.
- `SortedDocValuesField`/`SortedSetDocValuesField.newSlowSetQuery` (Java callers of the rewrite-method `TermInSetQuery` constructor) have no port.
