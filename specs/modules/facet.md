# Module Specification: `facet`

**Spec ID:** FACET  
**Java module:** `lucene/facet` (package `org.apache.lucene.facet` and subpackages)  
**Crate:** `crates/rucene-facet`

## 1. Purpose

Provide faceted indexing and search, as Lucene 10.5.0 `lucene/facet` does: facet counts and value aggregations over a search's hits, and drill-down and drill-sideways navigation.

## 2. Key classes / concepts ported

- Taxonomy facets (`taxonomy`, `taxonomy.directory`, `taxonomy.writercache`): `DirectoryTaxonomyWriter`/`DirectoryTaxonomyReader` (the separate taxonomy index mapping hierarchical `FacetLabel` paths to global int ordinals), `FastTaxonomyFacetCounts`, `TaxonomyFacetIntAssociations`/`TaxonomyFacetFloatAssociations`, `AssociationFacetField`, `SearcherTaxonomyManager`, `OrdinalMappingLeafReader`, the writer caches (`LruTaxonomyWriterCache`, `UTF8TaxonomyWriterCache`, `CharBlockArray`).
- Sorted-set doc values facets (`sortedset`): `SortedSetDocValuesFacetField`, `SortedSetDocValuesFacetCounts`, `ConcurrentSortedSetDocValuesFacetCounts`, `DefaultSortedSetDocValuesReaderState`.
- Numeric facets: `range` (`LongRangeFacetCounts`, `DoubleRangeFacetCounts`, `DynamicRangeUtil`), `rangeonrange`, `facetset` (`MatchingFacetSetsCounts`, exact and range matchers), `LongValueFacetCounts`, `StringValueFacetCounts`.
- Search-time API: `FacetsConfig`, `FacetField`, `FacetsCollector`/`FacetsCollectorManager`, `RandomSamplingFacetsCollector`, `Facets`, `FacetResult`, `DrillDownQuery`, `DrillSideways`.

## 3. Design notes

- One crate module per Java source file; the types of `org.apache.lucene.facet` sit at the crate root, each subpackage is a module (`taxonomy::writercache`).
- The taxonomy index is written and read in Lucene 10.5.0's format (a Lucene index with the ordinal payload/doc values layout of `DirectoryTaxonomyWriter`), so a taxonomy written by either side is readable by the other.
- Package-private members that Lucene's tests use are public, documented as such.

## 4. Acceptance criteria

- The 38 Lucene 10.5.0 test classes under `lucene/facet/src/test` are ported in `crates/rucene-facet/tests/` and pass (rmp #630).
- The facet Java oracle (`Module::Facet`, `FacetOracle.java`) gives equal taxonomy and sorted-set counts, range facets, drill-down/drill-sideways results and taxonomy index bytes on seeded corpora.
