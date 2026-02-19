# hit-leiden Constitution

This project provides fast, incremental community detection for GraphRAG systems
running on modest hardware. See the [project README](../../README.md) for the
full project goals and priorities. This constitution governs how the project is developed.

## Core Principles

### I. Incremental Updates Are Non-Negotiable
When the knowledge graph changes, only affected communities MUST be reprocessed.
No design decision, optimisation, or architectural change may introduce a
requirement for full re-clustering. A change that breaks incrementality is a
breaking change, regardless of any other benefit it provides.

Rationale: the entire value of this project depends on avoiding full re-clustering
on every graph update. Without incrementality, community detection cannot keep pace
with document ingestion on modest hardware.

### II. Hierarchical Communities
Communities MUST be organised hierarchically. Larger communities MUST be composed
of smaller sub-communities so that each level can be summarised within LLM context
window limits. Changes that flatten the hierarchy or prevent recursive
summarisation MUST NOT be accepted without an equivalent alternative.

Rationale: LLM context windows impose hard limits on summarisation input size.
Hierarchical structure enables recursive summarisation from leaves upward.

### III. Incremental Output
The system MUST support emitting stabilised communities as soon as they are ready,
rather than requiring the entire graph to converge before producing output. Changes
that force batch-only output MUST justify the regression and provide a remediation
path.

Rationale: incremental output enables pipelined workflows where downstream LLM
summarisation begins immediately, keeping end-to-end ingestion latency low.

### IV. Modest Hardware and Scale-Out
The system MUST run correctly on consumer-grade machines. The architecture MUST
support distributing work across multiple modest nodes. Changes that introduce
hard dependencies on GPU clusters, high-memory servers, or single-machine-only
execution MUST NOT be accepted for core paths.

Rationale: the project targets RAG deployments on laptops, small servers, and
commodity cloud instances, with the ability to scale out by adding nodes.


## Technical Standards

- Implementation language MUST be stable Rust.
- Dependencies MUST be justified by measurable capability or maintenance value.
- Core paths MUST avoid unnecessary allocation, copying, and dynamic dispatch
  where these affect measured performance goals.
- Numerical assumptions, convergence criteria, and data format expectations MUST
  be documented in feature specs and developer-facing docs.
- Pull requests touching hot paths, data layouts, parallelism, or allocation
  behaviour MUST include measurable performance evidence.

## Development Workflow

- Feature specs MUST define measurable success criteria for correctness and
  performance.
- Implementation plans MUST pass Constitution Check gates before design execution.
- Task lists MUST include verification tasks for tests and benchmarks when
  relevant.
- Code review MUST include constitution compliance verification before merge.

## Governance

This constitution overrides conflicting local conventions for this repository.

Amendments require:
1. A documented proposal describing the rule change and rationale.
2. Explicit updates to affected templates and workflow guidance in the same change.
3. Reviewer approval from at least one maintainer.

Versioning policy:
- MAJOR: incompatible governance or principle removals/redefinitions.
- MINOR: new principle/section or materially expanded guidance.
- PATCH: clarifications, wording improvements, and non-semantic refinements.

Compliance review expectations:
- Every pull request MUST state constitution compliance or justified exceptions.
- Exceptions MUST include scope, risk, and follow-up remediation.

**Version**: 1.0.0 | **Ratified**: 2026-02-22
