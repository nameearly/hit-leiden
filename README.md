# hit-leiden

Fast, incremental community detection for GraphRAG systems running on modest
hardware. When a user adds a document to a knowledge graph, communities and their
summaries update without reprocessing the entire graph.

## ⚠️ Validation warning

Do not use clustering output from this project in production decision-making
without independent validation on your own datasets and acceptance criteria.
Always verify invariants, quality metrics, and downstream impact before relying on results.

## Background

GraphRAG extends retrieval-augmented generation by organising a knowledge graph
into communities and summarising each one. These community summaries let an LLM
answer broad, thematic questions that span many documents — queries that
traditional vector search handles poorly. The quality of those summaries depends
directly on the quality and timeliness of the underlying community structure.

Standard community detection algorithms like Leiden run from scratch every time
the graph changes. For a RAG system where users regularly submit new documents,
this means re-clustering the entire graph on every ingestion — a process that
becomes prohibitively slow on consumer hardware as the knowledge graph grows.

This project takes an incremental approach: when edges are added or removed, only
the affected parts of the community hierarchy are reprocessed. This makes
continuous community maintenance feasible on modest machines and enables
downstream summarisation to keep pace with document ingestion.

## Quick Start

```sh
# Build
cargo build --release

# Run on an edge list (one "src dst [weight]" per line)
cargo run --release -- run --source file --path graph.txt

# Run benchmarks
cargo bench
```

## Goals

These are the current goals of the project:

### 1. Incremental community updates

The non-negotiable core of the project. When the knowledge graph changes, only
affected communities are reprocessed. Every architectural and algorithmic decision
must preserve this property. A design that requires full re-clustering on update
is a failure, regardless of how fast it is.

### 2. Hierarchical communities

Communities must be organised hierarchically. LLM context windows impose an upper
bound on how much of a community can be sent for summarisation in a single pass.
Larger communities must therefore be composed of smaller sub-communities, enabling
recursive summarisation: summarise the leaves first, then summarise the summaries,
up through the hierarchy. This directly supports multi-resolution understanding of
the knowledge graph.

### 3. Incremental community output

Emit stabilised communities as soon as they are ready, rather than waiting for the
entire graph to converge. In a pipeline, this means downstream LLM summarisation
can begin as soon as a community is established or updated, rather than blocking
on the full clustering pass. This is essential for keeping end-to-end ingestion
latency low.

### 4. Modest hardware viability

Community detection must run on consumer-grade machines — laptops, small servers,
commodity cloud instances. The project must not assume access to GPU clusters or
high-memory hardware for correct operation. Resource efficiency is a first-class
concern.


## Current Approach

HIT-Leiden was chosen as the starting point because the paper directly addresses
incremental community detection for dynamic graphs using the Leiden method, which
aligns with the core goals above. The algorithm maintains a hierarchical community
structure and updates it incrementally through movement, refinement, and
aggregation phases. This may be supplemented or replaced by other approaches as
the project evolves.

#### Preliminary Results

### Batch results using ca-HepTh dataset (https://snap.stanford.edu/data/ca-HepTh.html)

| Batch | Total edges | HIT (ms) | igraph (ms) | Speedup (x) | Cum. Speedup (x) | HIT Q | igraph Q |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 21778 | 11.06 | 1511.20 | 136.61x | 136.61x | 0.78226 | 0.79010 |
| 2 | 22778 | 10.68 | 1511.20 | 141.46x | 138.99x | 0.78145 | 0.79010 |
| 3 | 23778 | 10.60 | 1511.20 | 142.53x | 140.15x | 0.77923 | 0.79010 |
| 4 | 24778 | 11.39 | 1511.20 | 132.63x | 138.19x | 0.77404 | 0.79010 |
| 5 | 25778 | 11.24 | 708.90 | 134.49x | 122.84x | 0.77366 | 0.78023 |

### Final batch fresh comparison

| Metric | Value |
|---|---:|
| Batch | 5 |
| HIT time (ms) | 11.24 |
| Fresh igraph time (ms) | 708.90 |
| Speedup vs fresh igraph (x) | 63.09x |
| Modularity HIT | 0.77366 |
| Modularity igraph | 0.78023 |
| Modularity Δ (HIT−igraph) | -0.00657 |
| NMI (HIT vs igraph) | 0.7715 |
| HIT → igraph purity | 0.6651 |
| igraph → HIT purity | 0.6744 |
| Largest-community Jaccard | 0.5057 |
| igraph community count | 483 |


## References

Lin, Chunxu, Yumao Xie, Yixiang Fang, Yongmin Hu, Yingqian Hu, and Chen Cheng. "Efficient Maintenance of Leiden Communities in Large Dynamic Graphs." arXiv preprint [arXiv:2601.08554](https://arxiv.org/abs/2601.08554) (2026).

Bokov, Grigoriy, Aleksandr Konovalov, Anna Uporova, Stanislav Moiseev, Ivan Safonov, and Alexander Radionov. "A Parallel Hierarchical Approach for Community Detection on Large-scale Dynamic Networks." arXiv preprint [arXiv:2502.18497](https://arxiv.org/abs/2502.18497) (2025).

## Documentation

- [Developer guide](docs/guide/hit_leiden_explained.md)
- [Mathematical specification](docs/math/hit_leiden_spec.md)

## License

This project is dual-licensed under:

- [MIT](LICENSE-MIT)
- [Apache-2.0](LICENSE-APACHE)

You may choose either license.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
