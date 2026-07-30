# Module Specification: `search`

**Spec ID:** SEARCH  
**Java package:** `org.apache.lucene.search`  
**Cargo feature:** `search`

## 1. Purpose

Implement the search engine: query execution, scoring, collectors, sorting, and the top-level `IndexSearcher`.

## 2. Key classes / concepts to port

- `IndexSearcher` — search entry point.
- `Query` / `Weight` / `Scorer` / `Collector` / `LeafCollector`.
- `TopDocs` / `ScoreDoc` / `TotalHits`.
- Boolean, term, phrase, range, and prefix queries.
- `Similarity` (`BM25Similarity`, `ClassicSimilarity`).
- `Sort` / `SortField` / `FieldComparator`.
- `QueryCache` / `QueryCachingPolicy`.

## 3. Design notes

- `IndexSearcher` exposes an async API; queries run as `tokio` tasks.
- Scorers and collectors are generic where possible, trait objects only where query types are dynamic.
- Per-thread scorer contexts use OS thread-local storage.

## 4. Acceptance criteria

- A Rucene-built index yields the same top-N results and scores as Java Lucene 10.5.0 for the same query.
- Boolean, term, and range queries pass parity tests.
- `BM25Similarity` and `ClassicSimilarity` produce identical scores for identical statistics.
