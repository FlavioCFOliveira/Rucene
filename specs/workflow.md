# Rucene — Development Workflow Specification

**Spec ID:** WORKFLOW  
**Scope:** Specification versioning, task lifecycle, acceptance criteria, reviews, and traceability.

## 1. Specification documents

- All specifications live under `specs/`.
- Each major area has a top-level spec (`overview.md`, `architecture.md`, `compatibility.md`, `security.md`, `testing.md`, `workflow.md`).
- Each Lucene module has a dedicated spec under `specs/modules/`.
- Specifications are **versioned**: when a requirement changes, a new version of the spec is created with a changelog entry; the old version is retained for traceability.

## 2. Traceability

- Every section in a spec has a unique ID (e.g., `ARCH-3.2`, `COMPAT-5.1`).
- Code comments reference the spec ID they implement.
- Test names reference the spec ID they verify.
- `rmp` tasks reference the spec IDs they deliver.
- The Knowledge Graph links features, specifications, tests, and commits.

## 3. Task granularity

- Tasks are created **per class/interface** from the Lucene Core 10.5.0 source tree.
- A task may include its direct inner types, methods, and fields.
- Tasks are grouped into sprints by module dependency order.

## 4. Task lifecycle

Tasks move through the `rmp` states:

```
BACKLOG → SPRINT → DOING → TESTING → COMPLETED
```

- A task is moved to `DOING` only when its dependencies are complete or in progress.
- A task moves to `TESTING` when implementation and unit tests are done.
- A task moves to `COMPLETED` after acceptance criteria are verified and a review is done.

## 5. Acceptance criteria

Every task must satisfy:

1. Functional parity with the Lucene Core 10.5.0 equivalent.
2. At least one portability or behavioral test, when applicable.
3. Unit tests for non-trivial logic.
4. Public `rustdoc` comments with the Lucene equivalent.
5. `cargo test`, `cargo fmt`, and `cargo clippy` pass.
6. Knowledge Graph updated with the commit hash.

## 6. Specification vs. implementation divergence

If implementation discovers a better approach than what the spec mandates:

- The code is adapted to match the spec **unless** the spec is demonstrably wrong or unsafe.
- A spec change follows the versioning process and is reviewed before implementation is accepted.

## 7. Knowledge Graph maintenance

- The KG is updated on every commit.
- Nodes and edges record the commit hash and date.
- New features, tests, and specs are linked to the corresponding Lucene Core 10.5.0 source nodes.
