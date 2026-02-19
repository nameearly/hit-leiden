# HIT-Leiden Explained for Software Developers

HIT-Leiden is an algorithm for efficiently maintaining Leiden communities in large dynamic graphs.

## Why this algorithm exists

Classic Leiden is great for static graphs, but production systems usually deal with **dynamic edges** (arrivals/removals) and need fast updates. HIT-Leiden keeps Leiden’s quality goals while avoiding full recomputation on every change.

If you like mental models: this is **Leiden with incremental deltas + hierarchy-aware maintenance + mode-dependent execution (deterministic vs throughput)**.

## Core concepts (developer-friendly)

- **Graph**: $G=(V,E)$
- **Delta graph**: $\Delta G$ (edge updates to apply)
- **Level-\(p\) supergraph**: $G^p$
- **Community map**: $f^p(\cdot)$
- **Refined community map**: $g^p(\cdot)$
- **Subcommunity map**: $s^p(\cdot)$
- **Resolution**: $\gamma$ (modularity trade-off knob)

In implementation terms, these are mostly arrays/vectors indexed by node or supernode.

## Modularity definitions (what we optimize)

HIT-Leiden uses modularity as its objective during movement/refinement decisions.

### Global modularity

For an undirected weighted graph, modularity is:

$$
Q = \frac{1}{2m}\sum_{i,j}\left(A_{ij} - \gamma\frac{k_i k_j}{2m}\right)\,\delta(c_i,c_j)
$$

Where:
- $A_{ij}$ is edge weight between nodes $i$ and $j$,
- $k_i$ is weighted degree of node $i$,
- $2m = \sum_i k_i$,
- $c_i$ is the community label for node $i$,
- $\delta(c_i,c_j)=1$ if same community, else $0$.

Intuition: higher $Q$ means more internal edge weight than expected under a degree-preserving random baseline.

### Local move gain used in `inc-movement`

For a node $v$ moving from current community $C$ to candidate $C'$, the code evaluates a local gain of the form:

$$
\Delta Q(v: C \to C') = \frac{w(v,C') - w(v,C)}{2m}
+ \gamma\,\frac{k_v\left(d(C) - k_v - d(C')\right)}{(2m)^2}
$$

Where:
- $w(v,C)$ is total edge weight from $v$ to nodes in community $C$,
- $k_v$ is degree of $v$,
- $d(C)=\sum_{u\in C}k_u$ is total degree mass of community $C$.

Moves are applied only when $\Delta Q > 0$ (and in throughput mode, batch guards apply as well).

### Reporting note

The run-level quality score (`compute_modularity`) reports standard modularity from the final partition. Movement/refinement steps still use the configured resolution parameter $\gamma$ for local decisions.

## End-to-end flow

1. Build/load a graph (`GraphInput` -> `InMemoryGraph`).
2. Choose mode:
   - **Deterministic**: stable, single-thread style logic.
   - **Throughput**: parallel movement/refinement with guardrails.
3. For each hierarchy level $p$:
   - Apply updates: $G^p \leftarrow G^p \oplus \Delta G^p$.
   - Run incremental movement.
   - Run incremental refinement.
   - Run incremental aggregation (unless skipped).
4. Run deferred update from top level down.
5. Output final map from refined level-1 assignment.

## Algorithm walkthrough (1–6) in practical terms

### 1) Leiden baseline (Algorithm 1)

At each level:
- move nodes to improve modularity,
- refine communities to preserve connectivity quality,
- aggregate into a coarser graph for the next level.

Then project results back to original nodes.

### 2) Incremental movement (Algorithm 2)

This is the local re-assignment step triggered by changes.

What it does:
1. Build active set $A$ from changed edges in $\Delta G$:
   - positive inter-community changes activate endpoints,
   - negative intra-community changes activate endpoints.
2. Pop active nodes and evaluate best destination community via modularity gain $\Delta Q$.
3. Move only when gain is positive.
4. Record changed nodes ($B$) and refinement-affected nodes ($K$).
5. Re-activate neighbors when local structure changed.

### 3) Incremental refinement (Algorithm 3)

Refinement keeps communities structurally healthy.

What it does:
1. Split disconnected components into new subcommunities.
2. For singleton refined nodes, evaluate merge targets in the same parent community.
3. Use T-filter style check (only consider targets where removing target subcommunity is not quality-improving).
4. Accept best positive-gain merge.

### 4) Incremental aggregation (Algorithm 4)

Converts refinement changes into next-level supergraph deltas.

What it does:
1. Start with lifted edge deltas from node space to subcommunity space.
2. For refined nodes, subtract old subcommunity edge contributions and add new ones.
3. Transfer self-loop contributions when needed.
4. Compress duplicate superedges by summing weights.

### 5) Deferred update (Algorithm 5)

After per-level operations, propagate changes top-down:
- update $f^p$ from $f^{p+1}$ using subcommunity links,
- propagate changed-node sets to lower levels.

### 6) HIT-Leiden orchestration (Algorithm 6)

At each level $p$:
1. Apply graph delta to current supergraph.
2. Run `inc-movement` -> returns changed/affected sets.
3. Run `inc-refinement` -> returns refined set.
4. If not last level, run `inc-aggregation` to produce $\Delta G^{p+1}$.
5. After loop, run deferred updates and finalize output.

## Throughput mode behavior (implemented today)

Throughput mode is not just “run in parallel”; it adds correctness/quality protections.

### T1. Decoupled parallel movement selection

Parallel workers generate candidate moves first, then a global selector keeps only compatible moves.

Rules:
- maintain emitter set $E_{emit}$ and acceptor set $E_{acc}$,
- reject move if it **emits from an acceptor** or **accepts into an emitter**,
- enforce one move per node.

This reduces conflicting batch moves that can harm quality.

### T2. Monotonicity guard

Let selected batch be $\hat{M}$. Compute total gain:
$$
\Sigma\Delta Q(\hat{M}) = \sum_{m\in\hat{M}}\Delta Q(m)
$$
If total gain is non-positive, skip applying that batch.

### T3. Aggregation skip optimization

Skip aggregation when both are true:
- no delta edges at level $p$,
- no refined nodes at level $p$.

Formally:
$$
\operatorname{skip}(p) = (|\Delta G^p|=0) \land (|R^p|=0)
$$

### Throughput safety gate in multilevel movement

The standalone multilevel movement path keeps parallel movement disabled by a safety gate until quality parity is fully acceptable.

## Rust implementation crosswalk (quick reference)

### Core state

| Concept | Rust symbol |
|---|---|
| input graph $G$ | `GraphInput`, `InMemoryGraph` |
| dynamic updates $\Delta G$ | `delta_g`, `delta_graph`, `current_delta` |
| final map $f(\cdot)$ | `state.node_to_comm` |
| per-level map $f^p$ | `state.community_mapping_per_level[p]` |
| per-level refined map $g^p$ | `state.refined_community_mapping_per_level[p]` |
| previous/current subcommunity maps | `previous_subcommunity_mapping_per_level[p]`, `current_subcommunity_mapping_per_level[p]` |

### Main algorithm entry points

| Spec step | Rust function |
|---|---|
| HIT-Leiden loop | `hit_leiden(...)` |
| movement | `inc_movement(...)` |
| refinement | `inc_refinement(...)` |
| aggregation | `inc_aggregation(...)` |
| deferred update | `def_update(...)` |
| multilevel static-style path | `multilevel_leiden(...)` |

### Throughput-specific symbols

| Throughput concept | Rust symbol |
|---|---|
| move candidate record | `MoveCandidate { node, from_comm, to_comm, node_degree, gain }` |
| shard proposal stage | `execute_shard(...)` |
| batch orchestrator | `inc_movement_parallel(...)` |
| decoupling selector | `select_decoupled_moves(...)` |
| selected moves $\hat{M}$ | `selected_moves` |
| emitter / acceptor sets | `emitters`, `acceptors` |
| one-move-per-node guard | `moved_nodes` |
| total selected gain | `total_gain` |
| aggregation skip predicate | `should_skip_aggregation(...)` |

## Practical notes for contributors

- If you change movement semantics, verify both **quality** and **speed**.
- Throughput-mode changes should include tests for:
  - conflict handling,
  - monotonicity guard behavior,
  - aggregation skip correctness.
- Keep docs and symbol crosswalks updated when renaming key structs/functions.

In short: the math spec defines the contract, and this guide tells you how to reason about that contract when writing or reviewing Rust code.
