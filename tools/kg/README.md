# KG extraction tools

Scripts used to populate the `rmp` Knowledge Graph with the structure of the
reference Apache Lucene Core 10.5.0 source tree.

- `extract_lucene_kg.py` — regex-based extractor of packages, source files,
  top-level types, imports, `extends` and `implements` relationships.
- `run_kg_batches.py` — feeds the generated Cypher files into
  `rmp graph create` / `rmp graph update` in small batches.

Both are written for one-off survey work; they are not part of the Rucene crate.
