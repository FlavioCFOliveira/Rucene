# Module Specification: `util`

**Spec ID:** UTIL  
**Java package:** `org.apache.lucene.util`  
**Cargo feature:** `util`

## 1. Purpose

Provide the low-level utilities on which every other Lucene module depends: byte arrays, numeric encoding/decoding, bit sets, priority queues, and small reusable data structures.

## 2. Key classes / concepts to port

- `BytesRef` — immutable-ish byte slice with offset and length.
- `DataInput` / `DataOutput` — byte-level primitive I/O abstractions.
- `BitSet` / `FixedBitSet` / `SparseFixedBitSet` — doc-id bit sets.
- `PriorityQueue` — heap-based priority queue used by merge and search paths.
- `NumericUtils` — float/double/long byte sorting helpers.
- `ArrayUtil` / `RamUsageEstimator` — array growth and memory estimation.
- `Attribute` / `AttributeImpl` / `AttributeSource` — analysis attribute framework (or split to `analysis` if cycles arise).

## 3. Design notes

- Keep allocations predictable; many structures are used on hot paths.
- Prefer generic functions over trait objects.
- `BytesRef` should be cheap to clone and compare.

## 4. Acceptance criteria

- `util` compiles with `#![deny(unsafe_code)]`.
- Unit tests for all ported structures.
- Property-based tests for serialization round-trips and bit-set operations.
- No dependency on higher-level modules (`store`, `index`, `search`, etc.).
