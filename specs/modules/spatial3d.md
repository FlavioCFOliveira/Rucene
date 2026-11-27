# Module Specification: `spatial3d`

**Spec ID:** SPATIAL3D  
**Lucene module:** `lucene/spatial3d` (Java packages `org.apache.lucene.spatial3d`, `org.apache.lucene.spatial3d.geom`)  
**Crate:** `rucene-spatial3d`  
**Test fixtures:** `lucene/spatial-test-fixtures` (`org.apache.lucene.spatial3d.tests`), crate `rucene-spatial-test-fixtures`

## 1. Purpose

Port the Spatial3D module: APIs for planar spatial3d math, mostly the computational geometry of the `geom` package (AKA "Geo3D"), plus the `Geo3DPoint` field, its doc values, sorts and queries.

## 2. Key classes / concepts ported

- `geom`: `PlanetModel` (`SPHERE`, `WGS84`, `CLARKE_1866`), `GeoPoint`, `Vector`, `Plane`, `SidedPlane`; shapes built by `GeoCircleFactory`, `GeoBBoxFactory`, `GeoPolygonFactory`, `GeoPathFactory`, `GeoAreaFactory`, `XYZSolidFactory`; relationships (`GeoArea.getRelationship`); `SerializableObject` binary codec.
- Package root: `Geo3DPoint`, `Geo3DDocValuesField`, `PointInGeo3DShapeQuery`, `Geo3DUtil`, the distance comparators and sort fields.
- Test fixtures: `RandomGeo3dShapeGenerator`, the random shapes the spatial tests draw.

## 3. Design notes

- One Rust module per Java source file, mirroring the packages (crate docs).
- **Declared divergence (fdlibm):** every `java.lang.Math` call goes through `rucene::util::java_math`, an fdlibm port bit-exact with JDK 25 `StrictMath`. HotSpot's x86_64 LIBM intrinsics for `Math` may differ in the last bit, so the port is bit-exact with a JVM run with `-XX:-UseLibmIntrinsic`, the flag the Java oracle uses (rmp #628).
- `SerializableObject.readObject` resolves classes from a closed table of the package's serializable classes instead of reflective class loading, with Java's `ClassNotFoundException` / `ClassCastException` outcomes.

## 4. Acceptance criteria

- The 19 Lucene 10.5.0 test classes of `lucene/spatial3d/src/test` are ported and pass (`crates/rucene-spatial3d/tests/`).
- The sprint-17 Java oracle (`Spatial3dOracle.java`) gives the same geometry results and query matches as Rucene on seeded inputs.
