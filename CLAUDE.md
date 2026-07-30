# Rucene — Claude Code Project Guide

## 1. Project overview

Rucene is a port of **Apache Lucene Core** to **Rust**. It aims to provide a Rust crate that preserves the same functionality and module organization as the original Lucene, while leveraging Rust's advantages: better performance and safer memory management.

- **Reference source:** Apache Lucene Core `10.5.0`
- **Project type:** Rust library crate
- **Reference site/organization:** [Apache Lucene](https://lucene.apache.org/)

### Port goals

The project pursues two dimensions of parity with Apache Lucene Core 10.5.0, whenever feasible:

1. **Functional parity** — same functionality and same modular organization, only in a different language (Rust).
2. **100% index compatibility** — the crate must be able to **read and write index files** that are 100% compatible with Apache Lucene Core 10.5.0.

### Expected project structure

Because the project is still in an early phase, the structure should follow the usual conventions of a Rust crate:

```text
Rucene/
├── Cargo.toml          # Crate definition, dependencies, and metadata
├── README.md           # Entry-point documentation (already exists)
├── LICENSE             # Project license (already exists)
├── CLAUDE.md           # This file
└── src/
    ├── lib.rs          # Crate entry point
├── error.rs            # Shared errors (created as needed)
    └── ...             # Modules mirroring Lucene Core's organization
```

Internal module organization should mirror the Lucene Core Java structure as closely as possible (for example: `store`, `index`, `search`, `analysis`, `document`, `util`, `codecs`), making it easier to navigate between the two codebases and to maintain functional parity.

---

## 2. Base rules

1. **Do not make decisions alone.** Whenever instructions are insufficient, unclear, unspecific, or concrete, or when contradictions or ambiguities exist, **always ask the user** how to proceed.
   - When asking, always provide multiple options (a, b, c, ...) and indicate which one is recommended.
   - When several questions need clarification, ask them one at a time (sequentially), not all at once.
   - **Boundary between acting and asking:** obvious, low-risk fixes (for example, a pre-existing bug with an unambiguous solution) proceed immediately; any decision that changes scope, expected behavior, architecture, or requirements requires prior user input.
2. **Documentation in English.** All project documentation (including this `CLAUDE.md`) must be written in correct English, without spelling, grammar, or syntax errors. Use clear, simple, unambiguous technical language intended for human readers.
3. **Documentation faithful to the code.** Documentation must be accurate and always reflect the actual state of the code.
4. **Work flow.** Work always follows this order: **Specify → Implement → Test → Document.**

---

## 3. Self-contained development policy

- All development cycles must be **self-contained**: each cycle produces a complete, usable result. Never deliver only part of a task.
- When unforeseen needs arise during a task, resolve them in the same development cycle as quickly as possible (create new tasks and execute them immediately).
- All code must be **full-fledged** (complete and ready to use). Do not create tests with `skip` or placeholder stubs.
- Whenever a pre-existing bug is found, fix it on the spot and then resume the original work.

---

## 4. Production orientation

**Everything** done — development, fixes, evaluations, analyses, audits, and any other actions — must be treated with production-grade rigor.

---

## 5. Task and sprint planning and execution

For operations related to tasks or sprints, use the `roadmap-manager` skill.

- Use the `rmp` CLI (the roadmap management tool available in the environment) to plan and coordinate task execution.
- Treat `rmp` as the **single source of truth** for planning and executing project tasks. No other task management method should be used for this purpose.
- Use the **Knowledge Graph** to understand the project, its components, and the relationships between them, so that the scope and impact of each task can be identified more easily.

### 5.1 Planning

- Analyze the scope of work proposed by the user and determine whether it justifies being split into multiple development phases. Each phase must correspond to a solid deliverable.
- Every task must have a clear, objective definition of:
  - objectives;
  - functional requirements;
  - technical requirements;
  - acceptance criteria (the conditions that confirm the task is complete).
- Phases map to **Sprints** in `rmp` and are used to group tasks.
- When work requires several phases, planning happens in two distinct steps:
  1. define which phases (sprints) are needed and the scope/objective of each;
  2. only afterwards, sprint by sprint, define the tasks inside each sprint.

  In both steps, use `rmp` as the single source of truth.
- Use the **Knowledge Graph** to identify tasks with the highest gain or impact, foundational tasks, and tasks that unlock other tasks or features, in order to optimize execution order.
- **Prioritization:** by default, always work from highest-gain/highest-impact tasks to least essential. Foundational tasks and tasks that unblock others are always prioritized.
- When a task is too large for an AI agent like Claude Code to execute in one go, subdivide it into parts while respecting the principles already defined (especially the self-contained task principle).

### 5.2 Task execution

Execution follows planning. Always use `rmp` and follow this sequence:

1. Check whether any open task is not yet completed, and continue it.
2. Identify the next task.
3. Understand the objective of the task to be started, based on its description and functional and technical requirements.
4. Determine the most appropriate subagent and delegate execution to it.
5. Always validate the acceptance criteria before closing the task.
6. Close the task with a brief summary of what was done.
7. After closing the task and before moving on, make a `git commit` following best practices, explaining what was done.
8. Update the Knowledge Graph.

Execution notes:

- Whenever possible, adapt the model and effort level to the requirements of each individual operation within the task.
- Task and sprint execution is **sequential**.
- Subagent-based work (including evaluations and audits) may run in parallel **only when the user has explicitly authorized it for that specific operation**.
  - Any authorization is strictly scope-bound and one-time: it applies only to the operation for which it was granted and never establishes a standing exception.
  - **Even when authorized, never run more than two subagent operations concurrently.** Plan all required parallel work, but execute at most two operations at a time: as one finishes, start the next, always keeping the limit of two concurrent runs.

---

## 6. Knowledge Graph

The Knowledge Graph (KG) must be managed with the help of the `knowledge-authority` skill.

- Use the "Graph" features of `rmp` (Groadmap) to create, maintain (update), and query a project knowledge graph.
- This graph **must contain everything** useful to know about the project. Examples:
  - which features exist and where they are specified and implemented;
  - which tests exist and what they test;
  - which components exist, how they relate, and what dependencies exist between them;
  - in which `git commit` each feature was specified, implemented, and tested;
  - `rmp` tasks and their links to components.
- The graph **must be updated on every `git commit`**, recording changes to graph objects. Each update of nodes and edges must identify the corresponding commit and its date.
- **This graph is the single source of truth about the project.** Keep it as up to date as possible, so that before reading files you can consult the graph and obtain what you need.
- Create the nodes and edges that make the most sense for the project. Use the graph together with tasks and sprints to coordinate work.

---

## 7. Never guess

- All interactions with the project must be based **exclusively on verified knowledge**. Never try to guess the intended answer.
- When available information is insufficient, seek answers in official or authoritative sources: specifications, RFCs, papers, books, or reference authors in the field.
- Use the **Knowledge Graph** as the primary source of information — both for querying and for recording the relationships you discover.

---

## 8. Measure to decide

Whenever performance, completeness, or correctness needs to be evaluated, always gather evidence from the project to determine the facts. Decide empirically.

---

## 9. Regression prevention

Whenever a bug is identified, create the necessary regression tests to ensure the same bug does not recur as a result of future development. When the bug relates to functional parity or index-file compatibility with Lucene Core 10.5.0, the regression test should be a **portability test** that reproduces the divergence and locks in the correct behavior.

---

## 10. Subagent team

- A team composed of all available subagents (global, user-defined, or project-defined) is available.
- Use them collaboratively and complementarily so that every task is completed with maximum confidence, effectiveness, and accuracy.
- Each subagent should contribute proactively with its specialty.
- **Subagents must never be run in parallel unless the user has explicitly approved parallel execution for that specific operation.**
- Any approval for parallel subagent execution is **strictly scope-bound and one-time**: it applies only to the operation for which it was granted and never establishes a standing exception.
- **Even when parallel execution is authorized, never run more than two subagent operations concurrently.**

---

## 11. Decision framework

When deciding what is expected from the project — whether during evaluations and audits or during code implementation — follow this priority order: **correct → safe → fast.**

1. **Is it correct?** Does the result align with the objective, the project specification, and applicable authoritative sources (RFCs, standards, etc.)?
2. **Is it safe?** Does the decision or task introduce any characteristic or behavior that compromises the safe use of the deliverable?
3. **Is it fast?** Is it as fast as possible without compromising correctness or safety? What can be done to maximize the deliverable's performance?

If conflicts arise between these criteria, or if difficulty arises in following them, ask the user immediately how to proceed, presenting the possible options.

---

## 12. Project-specific conventions

### 12.1 Code conventions

- Write idiomatic Rust code, leveraging the type system, `Result`, `Option`, iterators, and safe concurrency primitives.
- Prefer names, module boundaries, and APIs close to Lucene Core Java, to make incremental porting and comparison easier.
- Document public functions and modules with `///`, indicating the Lucene Core equivalent when one exists.
- Keep code safe: avoid `unsafe` unless strictly necessary and properly justified.
- Follow standard Rust formatting: run `cargo fmt` and `cargo clippy` before considering any change complete.

### 12.2 Index compatibility

Binary index-file compatibility with Apache Lucene Core 10.5.0 is one of the central goals. This means:

- Respect the file formats, codecs, headers, checksums, and naming conventions of Lucene 10.5.0.
- Test interoperability with indexes generated by the original Java version whenever possible.
- Do not change write formats without updating the corresponding documentation and ensuring backward compatibility.

### 12.3 Portability and parity

Every development effort, whenever justified, must actively seek to guarantee both **functional parity** and **index-file compatibility** with Apache Lucene Core 10.5.0. Porting a feature is not complete until:

- the Rust implementation matches the behavior and contract of the corresponding Lucene Core component (functional parity);
- the produced artifacts are byte-compatible with those generated by the original Java implementation (index compatibility).

To prove this, each feature or module must include **portability tests** that demonstrate parity. These tests should, whenever feasible:

- compare outputs (index files, search results, statistics, or serialized structures) against reference data or behavior from Lucene Core 10.5.0;
- read indexes written by the Java implementation and assert that Rucene interprets them correctly;
- write indexes with Rucene and validate that the Java implementation can read them back correctly;
- cover both happy-path and edge cases that could diverge between the two implementations.

Portability tests are treated as first-class citizens: they must pass before a task is considered complete and must be updated whenever the reference behavior or formats change.

### 12.4 Recommended workflow

1. Before implementing a new feature, identify the corresponding module in Lucene Core 10.5.0.
2. Create or adapt the equivalent Rust module structure.
3. Implement with unit tests; when applicable, include integration tests that verify index compatibility.
4. Add or update **portability tests** that prove functional parity and index-file compatibility with Lucene Core 10.5.0.
5. Run `cargo test`, `cargo fmt`, and `cargo clippy`.
6. Update `README.md` and relevant documentation if there are changes to public behavior.

---

## 13. Assistance notes

- When suggesting code, prefer approaches that preserve functional parity and index compatibility.
- When creating new files, keep consistency with Lucene Core's modular structure.
- If a design decision would compromise compatibility with Lucene 10.5.0, highlight this explicitly and, if possible, propose alternatives.
- Always consider whether a change needs new or updated **portability tests** before marking it complete.

---

## 14. Authoritative sources of truth

Because Rucene is a port of Apache Lucene Core 10.5.0, every implementation detail, behavioral contract, file format, and API design decision must be anchored in **verified, official sources**. The following locations are the **absolute sources of truth** for this project.

Whenever a subagent or this assistant needs to determine how Lucene Core 10.5.0 behaves, what a class or method does, how an index file is structured, or how a feature is supposed to work, these are the **first and primary places to look**. Do not rely on secondary blog posts, forum answers, outdated tutorials, or memory — always verify against the sources below.

### 14.1 Official source code

The canonical source code for Apache Lucene 10.5.0:

- **Official repository (GitHub mirror):** https://github.com/apache/lucene
- **Release tag for 10.5.0:** https://github.com/apache/lucene/releases/tag/releases/lucene/10.5.0
- **Lucene Core source tree (10.5.0):** https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/core
- **Demo / examples source tree (10.5.0):** https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/demo

Use the release tag/branch for the exact 10.5.0 sources. Use the `main` branch only when explicitly comparing against future development, never as the primary reference for the 10.5.0 target.

### 14.2 Official documentation

The canonical documentation and API reference for Apache Lucene 10.5.0:

- **Main documentation page:** https://lucene.apache.org/core/10_5_0/index.html
- **Core API Javadoc:** https://lucene.apache.org/core/10_5_0/core/index.html
- **Demo API Javadoc / examples reference:** https://lucene.apache.org/core/10_5_0/demo/index.html

The main documentation page also links to reference documents such as file formats, query parser syntax, scoring formulas, migration guides, and the changelog. These are the authoritative references for behavior, contracts, and formats.

### 14.3 Official examples

The official examples and demo code are part of the `lucene/demo` module:

- **Demo module source tree:** https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/demo
- **Key demo classes (verified against the 10.5.0 demo Javadoc):**
  - `org.apache.lucene.demo.IndexFiles` — indexing example
  - `org.apache.lucene.demo.SearchFiles` — searching example
  - `org.apache.lucene.demo.SimpleFacetsExample` — facet counting example
  - `org.apache.lucene.demo.ExpressionAggregationFacetsExample` — expression-based aggregation
  - `org.apache.lucene.demo.DynamicRangeFacetsExample` — dynamic range facets
  - `org.apache.lucene.demo.knn.*` — KNN/vector search examples

### 14.4 Empirical verification rule

All work on this project must be empirical:

- Base decisions on evidence from the sources above, not on assumptions, guesses, or recollection.
- When behavior is unclear, read the relevant Lucene Core 10.5.0 source code and/or Javadoc directly.
- When file format details are unclear, consult the File Formats reference linked from the main documentation page and cross-check with the `org.apache.lucene.codecs` source code in the 10.5.0 tree.
- When a subagent needs to understand a Lucene feature or behavior, point it to the exact source file, Javadoc URL, or reference document that defines it.
- Record any non-obvious findings in the Knowledge Graph, citing the exact source URL and/or commit hash so they can be re-verified later.

If a required source does not exist or is ambiguous, ask the user how to proceed rather than guessing.
