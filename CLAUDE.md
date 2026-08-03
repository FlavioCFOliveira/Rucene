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
    ├── error.rs        # Shared errors (created as needed)
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
5. **Open-source inspired.** For every project component, seek inspiration from open-source projects that implement the same component in an exemplary way. Whenever possible, use multiple (more than one) reference projects so that the positive and negative aspects of each approach can be evaluated. Always use the source code as the absolute source of truth when necessary.

---

## 3. Self-contained development policy

- All development cycles must be **self-contained**: each cycle produces a complete, usable result. Never deliver only part of a task.
- When unforeseen needs arise during a task, resolve them in the same development cycle as quickly as possible (create new tasks and execute them immediately).
- All code must be **full-fledged** (complete and ready to use). Do not create tests with `skip` or placeholder stubs.
- Whenever a pre-existing bug is found, fix it on the spot and then resume the original work.

---

## 4. Production orientation

**Everything** done — development, fixes, evaluations, analyses, audits, and any other actions — must be treated with production-grade rigor.

### 4.1 Exemplary components

Every project component must be an **exemplary** piece for the purpose it serves. Each component must have a clearly and explicitly defined responsibility so that its boundaries of action and responsibility are unambiguous.

To **design, implement, and evaluate** each component, research which open-source projects implement the same functionality in an exemplary way and use that implementation as inspiration for this project. Multiple open-source projects may be used for the same functionality or component.

### 4.2 Perfect architecture

The overall project architecture and the design of its components must be based on the best practices most suited to the project's purpose. Inspiration should also be sought from other open-source projects to ensure that the intended results are achieved assertively and directly.

---

## 5. Task and sprint planning and execution

For operations related to tasks or sprints, use the `roadmap-manager` skill.

- Treat `rmp` (the roadmap management tool available in the environment) as the **single source of truth** for planning and executing project tasks. No other task management method should be used for this purpose.
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

## 12. Token economy policy

### 12.1 Acting principle

**Before executing any operation, always consider its token cost and choose the cheaper alternative that produces the same result.** When two or more ways of obtaining the same information (or the same effect) are available, the most economical one is mandatory.

The choice of the cheaper path **must not affect the result of the operation in any way.** Economy applies **only to the means** used to reach the result, **never to the result itself.** The result obtained by the economical path must be **identical** to what would be obtained by the more expensive path — not "similar enough," not "approximately," not "probably equal": **identical**.

**Mandatory equivalence test.** You may only choose the cheaper alternative when you are certain the result is equivalent. Before choosing, verify:

- Does it return exactly the same information, with the same accuracy and the same level of detail?
- Does it cover exactly the same scope (the same files, the same cases, the same data)?
- Does it produce exactly the same effect on the project?

If the answer to any of these questions is "no" or "I don't know," the economical alternative is **excluded** and you must use the path that guarantees the result. **When in doubt about equivalence, always choose the most reliable path, even if it is more expensive.** Economy is only the tie-breaker between options proven to be equivalent — never a decision criterion for the result itself.

**Never reduce, to save tokens:** the scope of the task, the depth of analysis, the number of files or cases examined when all are relevant, the tests to run, the evidence to gather, the verification against authoritative sources, the validation of acceptance criteria, or the quality of the deliverable. Saving tokens **does not** mean doing less: it means doing the same via a shorter path.

**Limit of this principle (precedence):** token economy **never** justifies compromising correctness, safety, completeness, or evidence gathering. If the cheaper path produces a different, incomplete, or uncertain result, then it is **not** the same operation — in that case, rules 7 (Never guess), 8 (Measure to decide), and 11 (Decision framework) prevail. Saving tokens must never lead to guessing or assuming.

### 12.2 Concrete examples

**External information gathering**

- If it is possible to `git clone` (preferably `git clone --depth 1`) a repository and consult the files locally, **avoid** using `WebFetch` to obtain the same content — especially when several files from the same repository are needed.
- To consult documentation for a dependency, prefer locally available documentation (project files, dependency code, `go doc`, command `--help`) over a web search.
- When a web search is truly necessary, perform **one targeted, specific search** instead of several generic searches followed by reading irrelevant pages.

**Consulting the project itself**

- Consult the **Knowledge Graph first** (rule 6). Reading the graph is cheaper than reading files or walking the code to find the same answer. This is exactly what the graph is for.
- Use targeted searches (`grep`/`glob` with precise patterns) instead of reading whole files to find a reference.
- When reading a large file, read only the range of lines needed instead of the entire file.
- For broad searches (scanning many files or directories), **delegate to a subagent** that returns only the conclusion, instead of bringing the contents of all files into the main context.

**Avoid repeating work already done**

- Do not re-read files already read in this session, nor reconfirm an edit that was applied successfully.
- Do not re-derive facts already established in the conversation, nor reopen decisions already made by the user.
- Do not run the same search twice (for example, delegating a search to a subagent and also running it yourself). Delegate **or** execute, never both.

**Commands and output**

- Limit command output to what is necessary: use `git log --oneline`, `git diff --stat` before the full diff, `git status --short`, `--name-only`, `-q`/`--quiet` flags, or restrict the result (for example with `head`).
- Avoid dumping large files into the context or response. Reference `file_path:line_number` instead of reproducing content.
- Prefer reading text (or the accessibility tree of a page) over capturing images/screenshots, which are substantially more expensive, whenever text is sufficient.

**Tests and validation**

- During iteration, run the specific test or package at stake; reserve the full suite for the final validation of the task.
- Do not run the full suite repeatedly to verify changes that only affect an isolated component.

**Model, effort, and parallelism**

- Adapt the model and effort level to the real difficulty of each operation (see 5.2): mechanical and simple operations do not justify the most expensive model or effort level.
- Group in a single message tool calls that are independent of each other, instead of making them one by one.
- Respect the limit of two concurrent evaluations/audits (see 5.2): excessive parallelism multiplies cost without accelerating the result.

### 12.3 Safeguard

All examples in 12.2 are subject to the equivalence test in 12.1. They are shortcuts in the **path**, not cuts in the **result**.

If, during execution, you verify that the economical path you chose is not yielding the same result — it returned insufficient information, left part of the scope out, or generated doubt — **abandon it immediately and redo the operation via the full path.** The cost already spent is not justification for accepting an inferior result.

---

## 13. Open-source inspiration policy

### 13.1 Acting principle

Before designing or implementing any component, **clearly and objectively identify what that component is intended to do.** Only then, and **always in service of that objective** (first and foremost the macro objective), study how successful or reference open-source projects solved the same problem, and use that knowledge to make more informed decisions for **this** project.

Reference projects are treated as **good practice to analyze**, not as solutions to adopt automatically. What is extracted from them is **understanding** (structure, algorithm, reason for the decision, trade-offs), never code to transcribe.

### 13.2 Protocol

Follow this sequence for each component:

1. **Define the macro objective of the component.** What problem it solves, what role it plays in the project, what guarantees it must offer and under what constraints (correctness, safety, performance, durability, concurrency). Written explicitly and unambiguously.
2. **Define the micro objectives.** The concrete functionalities and behaviors: inputs and outputs, invariants, edge cases, quality and performance requirements, and acceptance criteria (see 5.1).
3. **Register objectives and decisions in the Knowledge Graph** (see 6), so they are consultable and traceable.
4. **Identify reference projects.** Select open-source projects that solve the same class of problem with recognized success. Selection criteria: maturity and real adoption, active maintenance, demonstrable engineering quality, documented design, and production use — **not** isolated popularity. Identification must be verified, never assumed (see 7).
5. **Study the approach in primary sources.** Source code at a concrete version/tag, official documentation, design documents, ADRs, papers, and issue/PR discussions — rather than secondary sources. The goal is to understand **why** the decision was made, not just what it was. To study a repository, apply rule 12 (local clone instead of multiple remote lookups).
6. **Analyze favorable AND unfavorable aspects.** For each approach, explicitly list:
   - what serves this component's objective and why;
   - what does **not** serve it, and what problems it would bring here;
   - what trade-offs the approach assumes;
   - what the reference project's premises and context were (scale, language, concurrency model, durability requirements, runtime environment) and **whether those premises hold for this project**;
   - what the reference project **abandoned** over time and why — negative evidence is often the most valuable.
7. **Decide for this project.** The decision results from the objectives defined in 1 and 2 and the decision framework (see 11: correct → safe → fast). The decision is expected to be an **adaptation or synthesis** — it may combine ideas from several references, or reject them all, as long as it is justified.
8. **Document the decision.** Record the decision made, the alternatives considered, the sources consulted, and the reasoning, in an auditable and revisitable form.
9. **Validate empirically.** When the approach has measurable impact, measure it in this project rather than trusting the reference's claims (see 8).

### 13.3 Prohibition of direct copying

- **It is forbidden to copy directly** code from open-source projects into this project: files, code blocks, or line-by-line transcription/translation into another language.
- The implementation must be **original**, idiomatic for the language and the conventions of this project, and designed for the objectives defined in 13.2.
- **It is also forbidden to copy a decision without understanding it.** Adopting an approach just because a reference project uses it is a form of guessing (see 7). If you cannot explain why it is appropriate for this component, do not adopt it.
- **Licenses and legal obligations.** Inspiration does not exempt respect for the source project's license. Never incorporate third-party code without verifying the license and **without explicit user authorization**. If you conclude that reusing code or adopting a dependency is the best path, **ask the user first** (see 1), presenting the options and identifying the license of each.
- **Attribution.** Record in the Knowledge Graph and documentation which source inspired each decision — for traceability and credit, never as a way to legitimize copying.

### 13.4 Safeguards

- **"This is how project X does it" is never, by itself, justification.** The justification is always this component's objective. Popularity is not suitability.
- **Different context invalidates conclusions.** Compare premises before comparing solutions: an approach excellent in its context may be inadequate here.
- **Approaches evolve.** Study a concrete version and verify whether the approach is still in effect in the reference project.
- If a reference approach conflicts with this project's specification or objectives, **ask the user** how to proceed (see 1), presenting the options.

---

## 14. Project-specific conventions

### 14.1 Code conventions

- Write idiomatic Rust code, leveraging the type system, `Result`, `Option`, iterators, and safe concurrency primitives.
- Prefer names, module boundaries, and APIs close to Lucene Core Java, to make incremental porting and comparison easier.
- Document public functions and modules with `///`, indicating the Lucene Core equivalent when one exists.
- Keep code safe: avoid `unsafe` unless strictly necessary and properly justified.
- Follow standard Rust formatting: run `cargo fmt` and `cargo clippy` before considering any change complete.

### 14.2 Index compatibility

Binary index-file compatibility with Apache Lucene Core 10.5.0 is one of the central goals. This means:

- Respect the file formats, codecs, headers, checksums, and naming conventions of Lucene 10.5.0.
- Test interoperability with indexes generated by the original Java version whenever possible.
- Do not change write formats without updating the corresponding documentation and ensuring backward compatibility.

### 14.3 Portability and parity

Every development effort, whenever justified, must actively seek to guarantee both **functional parity** and **index-file compatibility** with Apache Lucene Core 10.5.0. Porting a feature is not complete until:

- the Rust implementation matches the behavior and contract of the corresponding Lucene Core component (functional parity);
- the produced artifacts are byte-compatible with those generated by the original Java implementation (index compatibility).

To prove this, each feature or module must include **portability tests** that demonstrate parity. These tests should, whenever feasible:

- compare outputs (index files, search results, statistics, or serialized structures) against reference data or behavior from Lucene Core 10.5.0;
- read indexes written by the Java implementation and assert that Rucene interprets them correctly;
- write indexes with Rucene and validate that the Java implementation can read them back correctly;
- cover both happy-path and edge cases that could diverge between the two implementations.

Portability tests are treated as first-class citizens: they must pass before a task is considered complete and must be updated whenever the reference behavior or formats change.

### 14.4 Recommended workflow

1. Before implementing a new feature, identify the corresponding module in Lucene Core 10.5.0.
2. Create or adapt the equivalent Rust module structure.
3. Implement with unit tests; when applicable, include integration tests that verify index compatibility.
4. Add or update **portability tests** that prove functional parity and index-file compatibility with Lucene Core 10.5.0.
5. Run `cargo test`, `cargo fmt`, and `cargo clippy`.
6. Update `README.md` and relevant documentation if there are changes to public behavior.

---

## 15. Assistance notes

- When suggesting code, prefer approaches that preserve functional parity and index compatibility.
- When creating new files, keep consistency with Lucene Core's modular structure.
- If a design decision would compromise compatibility with Lucene 10.5.0, highlight this explicitly and, if possible, propose alternatives.
- Always consider whether a change needs new or updated **portability tests** before marking it complete.

---

## 16. Authoritative sources of truth

Because Rucene is a port of Apache Lucene Core 10.5.0, every implementation detail, behavioral contract, file format, and API design decision must be anchored in **verified, official sources**. The following locations are the **absolute sources of truth** for this project.

Whenever a subagent or this assistant needs to determine how Lucene Core 10.5.0 behaves, what a class or method does, how an index file is structured, or how a feature is supposed to work, these are the **first and primary places to look**. Do not rely on secondary blog posts, forum answers, outdated tutorials, or memory — always verify against the sources below.

### 16.1 Official source code

The canonical source code for Apache Lucene 10.5.0:

- **Official repository (GitHub mirror):** https://github.com/apache/lucene
- **Release tag for 10.5.0:** https://github.com/apache/lucene/releases/tag/releases/lucene/10.5.0
- **Lucene Core source tree (10.5.0):** https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/core
- **Demo / examples source tree (10.5.0):** https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/demo

Use the release tag/branch for the exact 10.5.0 sources. Use the `main` branch only when explicitly comparing against future development, never as the primary reference for the 10.5.0 target.

#### Local reference clone

For all porting and verification work, prefer reading the Lucene Core 10.5.0 sources directly from a local clone instead of issuing web requests. When the clone is not present, obtain it with:

```bash
git clone --branch releases/lucene/10.5.0 --single-branch https://github.com/apache/lucene.git /tmp/lucene1050
```

The canonical local paths are:

- Lucene Core source tree: `/tmp/lucene1050/lucene/core/src/java/org/apache/lucene/`
- Demo source tree: `/tmp/lucene1050/lucene/demo/src/java/org/apache/lucene/demo/`

Consult files directly from this local repository. Do not use `WebFetch` or `WebSearch` for Lucene source lookups when the local clone is available; reading the exact Java files on disk is faster, works offline, and avoids stale or mismatched content.

### 16.2 Official documentation

The canonical documentation and API reference for Apache Lucene 10.5.0:

- **Main documentation page:** https://lucene.apache.org/core/10_5_0/index.html
- **Core API Javadoc:** https://lucene.apache.org/core/10_5_0/core/index.html
- **Demo API Javadoc / examples reference:** https://lucene.apache.org/core/10_5_0/demo/index.html

The main documentation page also links to reference documents such as file formats, query parser syntax, scoring formulas, migration guides, and the changelog. These are the authoritative references for behavior, contracts, and formats.

### 16.3 Official examples

The official examples and demo code are part of the `lucene/demo` module:

- **Demo module source tree:** https://github.com/apache/lucene/tree/releases/lucene/10.5.0/lucene/demo
- **Key demo classes (verified against the 10.5.0 demo Javadoc):**
  - `org.apache.lucene.demo.IndexFiles` — indexing example
  - `org.apache.lucene.demo.SearchFiles` — searching example
  - `org.apache.lucene.demo.SimpleFacetsExample` — facet counting example
  - `org.apache.lucene.demo.ExpressionAggregationFacetsExample` — expression-based aggregation
  - `org.apache.lucene.demo.DynamicRangeFacetsExample` — dynamic range facets
  - `org.apache.lucene.demo.knn.*` — KNN/vector search examples

### 16.4 Empirical verification rule

All work on this project must be empirical:

- Base decisions on evidence from the sources above, not on assumptions, guesses, or recollection.
- When behavior is unclear, read the relevant Lucene Core 10.5.0 source code and/or Javadoc directly.
- When file format details are unclear, consult the File Formats reference linked from the main documentation page and cross-check with the `org.apache.lucene.codecs` source code in the 10.5.0 tree.
- When a subagent needs to understand a Lucene feature or behavior, point it to the exact source file, Javadoc URL, or reference document that defines it.
- Record any non-obvious findings in the Knowledge Graph, citing the exact source URL and/or commit hash so they can be re-verified later.

If a required source does not exist or is ambiguous, ask the user how to proceed rather than guessing.
