# Module Specification: `spatial-extras`

**Spec ID:** SPATIAL-EXTRAS  
**Lucene module:** `lucene/spatial-extras` (Java package `org.apache.lucene.spatial` and sub-packages)  
**Crates:** `rucene-spatial-extras`, with the library subsets `spatial4j` (LocationTech spatial4j 0.8) and `s2-geometry` (s2-geometry-library-java 1.0.0)

## 1. Purpose

Port the spatial-extras module, the home of the original Lucene spatial module. Its principal interface is `SpatialStrategy`, an approach to indexing and searching based on shapes; each strategy has its own features and performance profile.

## 2. Key classes / concepts ported

- Strategies: `RecursivePrefixTreeStrategy`, `TermQueryPrefixTreeStrategy`, `NumberRangePrefixTreeStrategy`, `BBoxStrategy`, `PointVectorStrategy`, `SerializedDVStrategy`, `CompositeSpatialStrategy`.
- Prefix trees (`prefix.tree`): `QuadPrefixTree`, `PackedQuadPrefixTree`, `GeohashPrefixTree`, `S2PrefixTree`, `NumberRangePrefixTree`, `DateRangePrefixTree`; their cell token bytes are the indexed terms (index compatibility).
- `HeatmapFacetCounter`, `PrefixTreeFacetCounter`, the prefix-tree queries, `SpatialArgs` / `SpatialArgsParser` / `SpatialOperation`.
- `spatial4j` bridge: `Geo3dSpatialContextFactory`, `Geo3dShape` and friends over `spatial3d`.
- `spatial4j` crate: the spatial4j 0.8 types Lucene's `spatial-extras` and `benchmark` import and their closure (contexts, shapes, distance, WKT / GeoJSON / Polyshape / legacy IO); with the `test-helpers` feature, spatial4j's test helpers `TestLog`, `RandomizedShapeTest`, `RectIntersectionTestHelper`.
- `s2-geometry` crate: `S2Cell`, `S2CellId`, `S2LatLng`, `S2Point`, `S2Projections` and their closure.

## 3. Design notes

- One Rust module per Java source file, mirroring the packages (crate docs).
- Spatial4j's mutable shared shapes are `Arc<dyn Shape>` with atomic state (`reset` through `&self`); unchecked exceptions are `Result` errors where the Java contract throws on bad input (declared in the `spatial4j` crate docs).
- `DateRangePrefixTree` works on the JDK `Calendar` emulation of `rucene-queryparser`.
- **Declared divergence (fdlibm):** as in `spatial3d`, `java.lang.Math` calls go through `rucene::util::java_math`, bit-exact with a JVM run with `-XX:-UseLibmIntrinsic`.
- **Declared scope (user decision, sprint 17):** spatial4j's JTS integration (`context.jts`, polygons in a GEO context) and JTS itself are not ported; Geo3D shapes provide polygons. Lucene's `TestJtsPolygon` therefore takes Java's own missing-JTS assumption path.

## 4. Acceptance criteria

- The 26 Lucene 10.5.0 test classes of `lucene/spatial-extras/src/test` (87 test method declarations) are ported and bound (`crates/rucene-spatial-extras/tests/spatial_extras_tests.rs`, resources in `tests/test-files`) and pass.
- The sprint-17 Java oracle (`SpatialExtrasOracle.java`: `terms`, `nr`, `queries`, `heatmap`, `wkt`, `decimalFormat`) gives the same dumps as Rucene: indexed prefix-tree terms of quad, geohash, packed-quad, S2, number-range and date-range trees; strategy query matches and distance values; heatmap counts; WKT round trips and `DecimalFormat` HALF_EVEN / FLOOR / CEILING output.
