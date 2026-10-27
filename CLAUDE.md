# Rucene — Claude Code Project Guide

## 1. Project overview

Rucene is a port of **Apache Lucene Core** to **Rust**. It aims to provide a Rust crate that preserves the same functionality and module organization as the original Lucene, while leveraging Rust's advantages: better performance and safer memory management.

**Rucene is a faithful port of the `Apache Lucene` library to Rust.** Every work effort must focus on translating Lucene faithfully and empirically: reproduce what the Lucene source does, and prove it with evidence (see sections 7, 8, 14.5 and 16.4).

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
   - **Boundary between acting and asking:** a decision inside the scope of the task in progress that has one obvious, low-risk answer proceeds immediately. Any work outside that scope (see 3.2), and any decision that changes scope, expected behavior, architecture, or requirements, requires prior user input.
2. **Language.**
   - **English documentation.** All project documentation — the `README.md`, this `CLAUDE.md`, the specifications, and code documentation — is written in correct, professional English, with no spelling, grammar, or syntax errors.
   - **Writing style.** Write every text (documentation, task definitions, `rmp` comments, subagent briefs, and messages) so that it is:
     - **explicit** — it states clearly what is intended;
     - **objective** — it states what is to be executed;
     - **closed** — it defines the scope of the work;
     - **concise** — it uses few words.
   - **Interpretation.** Interpret every instruction the same way: by what it states explicitly, within the scope it defines.
3. **Documentation faithful to the code.** Documentation must be accurate and always reflect the actual state of the code.
4. **Work flow.** Work always follows this order: **Specify → Implement → Test → Document.** When several tasks share one effort (see 5.3), each step covers the whole effort.
5. **Open-source first.** For every component, identify the trustworthy open-source projects that already implement or solve it, and use them as the starting point: copy from them when possible, or study them. Whenever possible, use more than one reference project, so that the positive and negative aspects of each approach can be evaluated. Use the source code as the absolute source of truth. See section 13.
6. **ALWAYS converge — this rule is mandatory and has no exceptions.** Every development effort **MUST** seek convergence with the other open tasks, so that work synergy is maximized. Before starting any work, find every task that touches the same files, subsystem, or tests, and join them into **one** effort. Then:
   - **write the code of all those tasks at once**, in a single pass;
   - **test the code of all those tasks at once**, in a single run;
   - **audit, commit, and document them together**, once.

   Developing converging tasks one by one is a **defect in how the work is done**, not a style choice. See section 5.3.

---

## 3. Scope and completeness

### 3.1 Task completeness

- Execute every task and every piece of work in full. **Never leave a task half-done or partially done.**
- Every development cycle is **self-contained**: it produces a complete, usable result.
- All code must be **full-fledged** (complete and ready to use). Do not create tests with `skip` or placeholder stubs.

### 3.2 No unrequested work

- Act strictly toward the objective of the work in progress.
- **Never start work that was not explicitly requested.** This applies equally to new tasks, fixes, refactorings, and improvements.
- When you identify a need outside the scope of the task in progress — including a pre-existing bug — ask the user how to proceed (see rule 1). Never start that work proactively.
- The scope of a task is its objectives, functional and technical requirements, and acceptance criteria. Work strictly required to meet them is in scope, and is completed in the same cycle (see 3.1).

---

## 4. Perfection orientation

**Everything** done — development, fixes, evaluations, analyses, audits, and any other actions — must be treated with exemplary, production-grade rigor. The goal is not merely "good enough"; every deliverable should be as correct, safe, and well-crafted as possible within the constraints of the task.

### 4.1 Exemplary components

Every project component must be an **exemplary** piece for the purpose it serves. Each component must have a clearly and explicitly defined responsibility so that its boundaries of action and responsibility are unambiguous.

To **design, implement, and evaluate** each component, research which open-source projects implement the same functionality in an exemplary way and use that implementation as the starting point for this project (see section 13). Multiple open-source projects may be used for the same functionality or component.

### 4.2 Perfect architecture

The overall project architecture and the design of its components must be based on the best practices most suited to the project's purpose. Inspiration should also be sought from other open-source projects to ensure that the intended results are achieved assertively and directly.

---

## 5. Task and sprint planning and execution

Use the `roadmap-manager` skill for tasks, sprints, and comments, and for every `rmp` command except `rmp graph` (see 10.2).

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
- **Shared effort:** identify which tasks can share one development effort (see 5.3). Each of them remains a separate task in `rmp`.
- When a task is too large for an AI agent like Claude Code to execute in one go, subdivide it into parts while respecting the principles already defined (especially the self-contained task principle).

### 5.2 Task execution

Execution follows planning. Always use `rmp` and follow this sequence:

1. Check whether any open task is not yet completed, and continue it.
2. Identify the next task, and the tasks that can share its development effort (see 5.3).
3. Understand the objective of each task, based on its description and functional and technical requirements.
4. Delegate execution to the most suitable subagent (see 10.1).
5. Validate each task's acceptance criteria before closing it.
6. Close each task individually, with a brief summary of what was done.
7. After closing the task(s) and before moving on, commit the work through the `gitflow` skill (see 10.2). A commit that records a shared effort names every task it covers.
8. Update the Knowledge Graph through the `knowledge-authority` skill (see 10.2).

Execution notes:

- Whenever possible, adapt the model and effort level to the requirements of each individual operation within the task.
- Sprints are executed **sequentially**. Tasks are executed **sequentially**, in priority order; a shared effort (see 5.3) counts as one unit of execution.
- Subagent concurrency follows 10.1.

### 5.3 Work synergy and convergence

Guiding principle: **make the effort pay — deliver the most with the least work.**

**MANDATORY: developments ALWAYS seek convergence with one another, so that work synergy is maximized (section 2, rule 6).** Write the code of multiple tasks at once, test the code of multiple tasks at once, and likewise audit, commit, and document them together. Never develop converging tasks one by one.

This is the default way of working for all project work, both `rmp` tasks and ad hoc requests. It requires no request from the user, and it is never recorded as an `rmp` task.

- **Convergence.** Always look for tasks whose objectives are complementary, or whose functional or technical proximity can be verified (use the Knowledge Graph). Turn every such convergence into synergy.
- **Synergy between tasks.** Join converging tasks into a single development effort.
- **Synergy within the effort.** Group work of the same type: write all the code at once, then run the tests for all the changed code at once, then write all the documentation at once, instead of writing and testing small pieces one by one. When the scope is too large for one pass, split it into the largest blocks that can each be handled in one pass. Apply this principle to all work.
- **Fewest iterations.** Reach the objectives with the fewest tasks and iterations possible.
- **Quality bar.** Group work only when grouping makes delivery faster without lowering its quality. A shared effort must deliver better work than task-by-task development, never worse.
- **Resources.** Make full use of the internal resources available (subagents, skills, local CLIs, and the Knowledge Graph), so that deliveries are faster and cost the user less.
- **Tasks stay separate.** Grouping joins only the work effort, during development and testing. In `rmp`, each task keeps its own definition, status, acceptance criteria, and closing summary.

### 5.4 Work log — `rmp` comments

Use the `rmp` comment feature to track the progress of the work **as it happens**. Comment **both the sprints and the tasks**, always with the comment type whose meaning fits. Write each entry at the moment it happens, never reconstructed at closing time.

**Comment types and their meaning:**

| Type | Write it when… | The entry contains | Task | Sprint |
|---|---|---|---|---|
| `HYPOTHESIS` | a proposition is about to be tested, **before** the result is known | the proposition, and what would confirm or refute it | ✓ | — |
| `TEST` | a test, benchmark, or verification was run | the exact command or procedure, and what it showed (pass/fail, numbers) | ✓ | — |
| `FINDING` | something was discovered: a behavior, a measurement, a cause, or a **refuted** hypothesis | the observation, concretely, with its evidence | ✓ | ✓ |
| `DECISION` | a choice was made between viable options | the choice, the reasoning, and the **rejected options** | ✓ | ✓ |
| `PROGRESS` | the work advanced materially (a part delivered, a blocker cleared) | what is now done and what remains | ✓ | ✓ |
| `UPDATE` | the task's or sprint's **definition** changed (always with `task edit` / `sprint update`) | **why** it changed, not the new text | ✓ | ✓ |
| `NOTE` | something worth keeping fits none of the above | the thing worth keeping, and why | ✓ | — |

**Rules:**

- **Chain.** Investigative work follows `HYPOTHESIS → TEST → FINDING → DECISION`. Every `HYPOTHESIS` is closed by a later `FINDING`, including when it is refuted.
- **Never fall back on `NOTE`** when a typed entry applies.
- **Task log.** Record, per task: the grouping `DECISION` when an effort starts; `FINDING` and `TEST` entries from each subagent report and each audit; a `DECISION` for each round's scope; `PROGRESS` per round.
- **Sprint log.** Record how the sprint goes: `PROGRESS` when an effort closes, `DECISION` for scope or sequencing changes, `UPDATE` when the sprint's definition changes, and `FINDING` for observations that span several tasks.
- **Before closing a task**, its log holds a `TEST` proving the acceptance criteria and a `DECISION` for every non-obvious choice. The closing summary states the outcome; the log keeps the route.

---

## 6. Knowledge Graph

The Knowledge Graph (KG) is managed exclusively through the `knowledge-authority` skill (see 10.2).

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

### 6.1 The KG coordinates the port

The Knowledge Graph holds a **central responsibility in coordinating the porting work**. It is not a passive record of what happened; it is the instrument that says what to do next.

- **The KG must know, at any moment, what is already ported and what is still missing.** It must therefore model *both* sides in depth — the structure of Apache Lucene Core 10.5.0 and the structure of Rucene — and the mapping between them, so that the port's coverage can be answered by a query rather than by reading code.
- **Coverage must be answerable and defensible.** The set of Lucene elements that are *in scope* for the port must be explicit in the graph; a ratio computed against an implicit or unstated denominator is not an answer. Every component must carry a status that reflects reality.
- The graph must record not only what exists, but what each missing piece **blocks** — so that the cost and the consequence of a gap are both visible.

### 6.2 The KG coordinates development

The KG also holds a **central role in coordinating development tasks**, so that it can always present the **most optimized paths** through the remaining work, and the work proceeds as quickly and efficiently as possible.

**The KG is the right instrument for setting priorities because it holds the widest view of the port** — it is the only place that knows, at the same time, the **current state** of the project and the **target state** the port is trying to reach: the structure of Apache Lucene Core 10.5.0. A priority decided from either half alone is a guess about the other. Seen from the graph, the distance between the two states is a measurable thing, and the ordering of the work follows from it.

- Dependencies between components, and between tasks and the components they need, must be modeled so that the graph can compute what unlocks the most work.
- **Use the graph to plan**, in the sense of rule 5.1: identify foundational tasks, tasks that unblock others, and the shortest route to a solid deliverable. The prioritization stated in 5.1 is meant to be *derived from the graph*, not asserted from memory or from reading files.
- When the graph cannot yet answer such a question, say so plainly and treat closing that gap as work in its own right, rather than substituting an impression for an answer (see rule 7).

---

## 7. Never guess

- All interactions with the project must be based **exclusively on verified knowledge**. Never try to guess the intended answer.
- When available information is insufficient, seek answers in official or authoritative sources: specifications, RFCs, papers, books, or reference authors in the field.
- Use the **Knowledge Graph** as the primary source of information — both for querying and for recording the relationships you discover.

---

## 8. Measure to decide

Whenever performance, completeness, or correctness needs to be evaluated, always gather evidence from the project to determine the facts. Decide empirically.

**Trace, never guess.** Whenever it is needed, use profiling and tracing to determine **exactly and empirically** what must be implemented or fixed. Never fix by trial and error; evaluate directly what needs to be done.

- **Choose the diagnostic technique deliberately.** Whenever you investigate the cause of a problem in the code, first decide which technique, or which combination of techniques, will identify and locate it: profiling, tracing, metrics, logs, differential runs against the Java oracle, or others. Use them alone or together, and keep dissecting the problem until its cause is absolutely clear.
- **Find the cause before the fix.** Instrument the failing path: trace logs, differential traces of Rucene against the Java oracle, and profilers (`perf`, flame graphs, `dhat`) for cost questions.
- **Prove the cause.** A positive control must show the defect with the cause present and its absence with the fix applied.
- **Observed, not inferred.** Every report states, per cause, whether it was **observed** (with the trace or command that shows it) or **inferred**. An inferred cause is traced before it is acted on.
- **Briefs require it.** Every subagent brief for a defect, divergence, or performance issue requires this evidence, and a report whose cause is only inferred is not accepted.

---

## 9. Regression prevention

Whenever a bug is fixed, create the necessary regression tests to ensure the same bug does not recur as a result of future development. When the bug relates to functional parity or index-file compatibility with Lucene Core 10.5.0, the regression test should be a **portability test** that reproduces the divergence and locks in the correct behavior.

---

## 10. Subagents and skills

### 10.1 Subagents

- **Delegate all project work.** The execution of every piece of work is delegated to the subagent specialized in the objectives that work must achieve. Always choose the most suitable subagent (global, user-defined, or project-defined).
- **One subagent at a time.** Run **at most one** subagent alongside the main conversation. Never run more than one concurrently.
- **Serial, not parallel.** Use as many subagents as the objective needs, one after another.
- **Exception.** The user may explicitly authorize more than one subagent in parallel. That authorization applies only to the task for which it was granted, covers at most the number of subagents the user authorized, and is always revoked when that task ends.
- Use the subagents collaboratively and complementarily, each contributing its specialty, so that every task is completed with maximum confidence, effectiveness, and accuracy.

### 10.2 Skills

The following operations must go through these Claude Code skills:

| Skill | Operations |
|---|---|
| `gitflow` | Every git write command, following the gitflow branching model for this repository. |
| `roadmap-manager` | Coordination and management of tasks, sprints, and comments, and every `rmp` command except `rmp graph`. |
| `knowledge-authority` | All management of the project Knowledge Graph (`rmp graph`). No other means may manage it. |
| `tpl` | Operation of the `tpl` CLI, which supports and speeds up development through code generation. Create templates whenever needed; they render against the structure of the application's database. |

---

## 11. Decision framework

When deciding what is expected from the project — whether during evaluations and audits or during code implementation — follow this priority order: **correct → safe → fast.**

1. **Is it correct?** Does the result align with the objective, the project specification, and applicable authoritative sources (RFCs, standards, etc.)?
2. **Is it safe?** Does the decision or task introduce any characteristic or behavior that compromises the safe use of the deliverable?
3. **Is it fast?** Is it as fast as possible without compromising correctness or safety? What can be done to maximize the deliverable's performance?

If conflicts arise between these criteria, or if difficulty arises in following them, ask the user immediately how to proceed, presenting the possible options.

This framework is applied **within** the fidelity principle of 14.5: the port reproduces Lucene Core 10.5.0 unless doing so would be incorrect or unsafe. A faster or more elegant alternative is not, by itself, a reason to diverge.

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

**Prefer the local CLI when available**

- If an operation can be performed locally through a CLI command, **use the CLI** rather than a more expensive alternative such as a web lookup, browser tool, or remote service. The local CLI is systematically the cheapest option when an equivalent command exists.
- Examples:
  - Use `git log`, `git show`, `git diff`, and `git blame` locally instead of browsing the repository web interface;
  - Use `gh issue view`, `gh pr view`, and `gh api` instead of opening the corresponding GitHub web pages;
  - Use `rmp`, through the `roadmap-manager` and `knowledge-authority` skills (see 10.2), for all task, sprint, and Knowledge Graph operations, since it is also the single source of truth;
  - Use `--help`, `man`, or the command's own documentation instead of searching online for the same information;
  - Filter and aggregate data locally (for example with `grep`, `jq`, `sort`, `wc`) instead of pulling the full dataset into the context.
- Reserve more expensive paths (web, browser, remote services) for cases where **no local CLI can produce the same result**.
- This preference is still subject to the equivalence test in 12.1: if the CLI does not return the same information with the same scope and accuracy, use the path that guarantees the result.

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

**Waiting for long-running operations**

- **It is forbidden to poll a long-running operation in a loop.** Repeatedly running `grep`, `tail`, `ps`, or any other status check while waiting for a build, a test run, or a background job to finish is pure waste: each check is a round trip that adds no information and consumes tokens for nothing.
- Arm **one** waiter and then stop issuing calls entirely until it reports. The correct mechanism is a single background command whose body blocks until the condition holds (for example `until <condition>; do sleep N; done`), which notifies exactly once when it exits.
- Never arm two waiters for the same event.
- Between arming the waiter and its notification, only perform work that is genuinely independent of the result. If there is none, say so in one line and wait in silence.

**Tests and validation**

- Write all the code of the effort first, then run the tests covering all the changed code in one run (see 5.3); do not test small pieces one by one. Reserve the full suite for the final validation of the effort.
- Do not run the full suite repeatedly to verify changes that only affect an isolated component.

**Model, effort, and parallelism**

- Adapt the model and effort level to the real difficulty of each operation (see 5.2): mechanical and simple operations do not justify the most expensive model or effort level.
- Group in a single message tool calls that are independent of each other, instead of making them one by one.
- Respect the limit of one subagent at a time (see 10.1).

### 12.3 Safeguard

All examples in 12.2 are subject to the equivalence test in 12.1. They are shortcuts in the **path**, not cuts in the **result**.

If, during execution, you verify that the economical path you chose is not yielding the same result — it returned insufficient information, left part of the scope out, or generated doubt — **abandon it immediately and redo the operation via the full path.** The cost already spent is not justification for accepting an inferior result.

---

## 13. Open-source policy

### 13.1 Acting principle

- At every moment of analysis, exploration, and development, identify whether a reference open-source project already implements or solves what is needed, as a whole or in part.
- **Do not reinvent.** For existing problems, new features, and refactorings alike, do not design a solution from scratch. Base the work first on the knowledge already available on the subject and, wherever possible, on the knowledge, engineering, architecture, and approach of an open-source project that already solves the problem, as a whole or in part.
- Use such a project to **copy from** (fully or partially) when possible, or to **study** as an example.
- Use **only reference projects that bring real utility and trust** to the intended contribution.
- A reference project is only the **starting point**. Never stay bound to its source code: the borrowed code must serve this project, adapted to its objectives, its conventions (see 14.1), and the fidelity principle (see 14.5).
- Before using a reference, **clearly and objectively define what the component is intended to do.** The reference always serves that objective, first and foremost the macro objective.

### 13.2 Protocol

Follow this sequence for each component:

1. **Define the macro objective of the component.** What problem it solves, what role it plays in the project, what guarantees it must offer and under what constraints (correctness, safety, performance, durability, concurrency). Written explicitly and unambiguously.
2. **Define the micro objectives.** The concrete functionalities and behaviors: inputs and outputs, invariants, edge cases, quality and performance requirements, and acceptance criteria (see 5.1).
3. **Register objectives and decisions in the Knowledge Graph** (see 6), so they are consultable and traceable.
4. **Identify reference projects.** Select open-source projects that solve the same class of problem with recognized success, and that bring real utility and trust to the intended contribution. Selection criteria: maturity and real adoption, active maintenance, demonstrable engineering quality, documented design, and production use — **not** isolated popularity. Identification must be verified, never assumed (see 7).
5. **Study the approach in primary sources.** Source code at a concrete version/tag, official documentation, design documents, ADRs, papers, and issue/PR discussions — rather than secondary sources. The goal is to understand **why** the decision was made, not just what it was. To study a repository, apply rule 12 (local clone instead of multiple remote lookups).
6. **Analyze favorable AND unfavorable aspects.** For each approach, explicitly list:
   - what serves this component's objective and why;
   - what does **not** serve it, and what problems it would bring here;
   - what trade-offs the approach assumes;
   - what the reference project's premises and context were (scale, language, concurrency model, durability requirements, runtime environment) and **whether those premises hold for this project**;
   - what the reference project **abandoned** over time and why — negative evidence is often the most valuable.
7. **Decide for this project.** The decision results from the objectives defined in 1 and 2 and the decision framework (see 11: correct → safe → fast). It may copy a reference fully or partially, adapt it, combine ideas from several references, or reject them all, as long as it is justified.
8. **Document the decision.** Record the decision made, what was copied or adapted and from where, the alternatives considered, the sources consulted, and the reasoning, in an auditable and revisitable form.
9. **Validate empirically.** When the approach has measurable impact, measure it in this project rather than trusting the reference's claims (see 8).

### 13.3 Copying and reuse

- **Copying is authorized.** Code from reference open-source projects may be copied, fully or partially: files, code blocks, or line-by-line translation into Rust.
- **Adapt what you copy.** Copied code must become idiomatic Rust, follow this project's conventions (see 14.1), and satisfy the fidelity principle (see 14.5).
- **Understand what you adopt.** Adopt code or a decision only when you can explain why it serves this component's objective (see 7).
- **Licenses.** Copy only from sources whose license allows inclusion in this MIT-licensed crate, and meet that license's obligations, such as preserving copyright and license notices. When a license is incompatible or unclear, ask the user (see 1).
- **Dependencies.** Adding a third-party crate as a dependency is an architecture decision and requires prior user approval (see 1). Present the options and the license of each.
- **Attribution.** Record in the Knowledge Graph and in the code documentation the source of each copied or adapted piece and of each decision it inspired.

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

### 14.5 Fidelity first — minimize divergences

Rucene is a **port**, not a reinterpretation. The governing principle for every design and implementation decision is therefore:

> **Be as faithful as possible to Apache Lucene Core 10.5.0, and keep the number and the size of divergences to a minimum.**

This principle applies to structure, behavior, contracts, and file formats alike:

- **The default answer is "the same as Lucene".** When Lucene's approach can be reproduced in idiomatic Rust, reproduce it. Do not substitute a different algorithm, a different data structure, a different laziness or ownership strategy, or a different error surface simply because it looks cleaner, simpler, or faster in Rust.
- **A divergence is an exception that must be earned.** It is only acceptable when Lucene's approach is genuinely impossible or unsafe in Rust (for example, a construct that Rust's ownership or thread-safety rules forbid), or when it would violate rule 11 (correct → safe → fast) in that order. Convenience, taste, and "it was easier" are never sufficient reasons.
- **Never paper over a divergence with an arbitrary limit.** If the port's own design (rather than Lucene's) creates a hazard, the correct fix is to remove the divergence that caused it, not to add a constant, threshold, or cap that Lucene does not have. Introducing a bound Lucene lacks changes which indexes the crate accepts and is itself a new divergence.
- **A divergence must be minimal in blast radius.** Where one is unavoidable, confine it to the smallest possible surface and keep everything around it faithful.
- **Every divergence must be declared.** Document it at the point of divergence (module or item docs), stating exactly what Lucene does, what this port does instead, and why the Lucene approach could not be used. Record it in the Knowledge Graph as well, citing the Lucene 10.5.0 source file and lines that define the original behavior.
- **Any new divergence requires prior user approval** (see rule 1). Present the options and the reasoning; never introduce one unilaterally.
- **Existing divergences are debt.** When a task touches code that already diverges, re-evaluate whether the divergence is still justified, and propose removing it when it is not.

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
