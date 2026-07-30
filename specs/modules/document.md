# Module Specification: `document`

**Spec ID:** DOC  
**Java package:** `org.apache.lucene.document`  
**Cargo feature:** `document`

## 1. Purpose

Model Lucene documents, fields, and field types in Rust, mirroring the Java API so that document construction and field configuration remain familiar.

## 2. Key classes / concepts to port

- `Document` — collection of fields.
- `Field` — base field abstraction.
- `FieldType` — indexed/stored/tokenized/doc-values configuration.
- Concrete field types: `TextField`, `StringField`, `IntField`, `LongField`, `FloatField`, `DoubleField`, `StoredField`, etc.
- `IndexableField` / `StorableField` traits.

## 3. Design notes

- Keep field construction ergonomic while preserving Java names.
- Field values borrow or own data as appropriate for the use case.
- No dependency on `index` or `search`; only on `analysis` and `util`.

## 4. Acceptance criteria

- All field types compile and are documented with their Lucene equivalents.
- A `Document` can be built programmatically and serialized to a byte buffer.
- Unit tests cover field type validation and value extraction.
