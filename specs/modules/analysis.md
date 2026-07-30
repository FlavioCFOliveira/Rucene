# Module Specification: `analysis`

**Spec ID:** ANALYSIS  
**Java package:** `org.apache.lucene.analysis`  
**Cargo feature:** `analysis`

## 1. Purpose

Port the Lucene analysis pipeline: tokenizers, token filters, analyzers, and the attribute source model, while keeping the API safe and usable in Rust.

## 2. Key classes / concepts to port

- `Analyzer` — token-stream factory.
- `TokenStream` / `Tokenizer` / `TokenFilter` — pipeline components.
- `Attribute` / `AttributeImpl` / `AttributeSource` — per-token state.
- Standard tokenizer and standard filter set.
- `CharFilter` / `Reader` abstractions adapted to Rust strings/bytes.

## 3. Design notes

- Replace Java reflection-based attribute factories with an explicit plugin registry or generic builders.
- Model `TokenStream` lifecycle (reset → incrementToken → end → close) faithfully.
- Per-thread reusable buffers use OS thread-local storage.

## 4. Acceptance criteria

- Standard tokenizer produces the same token sequence as Lucene 10.5.0 for ASCII and Unicode input.
- Analysis pipeline can be reset and reused.
- Fuzz/property tests verify tokenizer invariants.
