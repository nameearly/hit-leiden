# HIT-Leiden Mathematical Specification

Objective: Efficiently maintain Leiden communities in large dynamic graphs.

## Table 1: Notation

| Notation | Meaning |
|---|---|
| $G = (V, E)$ | A graph with vertex set $V$ and edge set $E$ |
| $N(v), N_2(v)$ | The vertex $v$'s 1- and 2-hop neighbor sets, resp. |
| $w(v_i, v_j)$ | The weight of edge between $v_i$ and $v_j$ |
| $d(v)$ | The weighted degree of vertex $v$ |
| $m$ | The total weight of all edges in $G$ |
| $\mathbb{C}$ | A set of communities forming a partition of $G$ |
| $Q$ | The modularity of the graph $G$ with partition $\mathbb{C}$ |
| $G^p = (V^p, E^p)$ | The supergraph in the $p$-th iteration of Leiden |
| $\Delta Q(v \to C', \gamma)$ | Modularity gain by moving $v$ from $C$ to $C'$ with $\gamma$ |
| $f(\cdot): V \to \mathbb{C}$ | A mapping from vertices to communities |
| $f^p(\cdot): V^p \to \mathbb{C}$ | A mapping from supervertices to communities |
| $s^p(\cdot): V^p \to V^{p+1}$ | A mapping from supervertices in $p$-th level to supervertices in $(p+1)$-th level (sub-communities) |
| $\Delta G$ | The set of changed edges in the dynamic graph |
| $P$ | Number of hierarchical levels |
| $\Psi$ | Connected Component (CC) indices |
| $\gamma$ | Resolution parameter |
| $M$ | Candidate move set collected in parallel |
| $\hat{M}$ | Decoupled move subset selected from $M$ |
| $E_{emit}, E_{acc}$ | Emitter and acceptor community sets in decoupling |
| $\Sigma\Delta Q(\hat{M})$ | Sum of modularity gains of selected moves |

## Modularity Definitions

### Global modularity objective

For an undirected weighted graph, the partition quality is measured by:

$$
Q = \frac{1}{2m}\sum_{i,j}\left(A_{ij} - \gamma\frac{k_i k_j}{2m}\right)\,\delta(c_i,c_j)
$$

where $A_{ij}$ is edge weight, $k_i$ is weighted degree of node $i$,
$2m=\sum_i k_i$, and $\delta(c_i,c_j)=1$ iff $c_i=c_j$ (else $0$).

### Local move-gain form used in Algorithm 2

For moving node $v$ from current community $C$ to candidate community $C'$, use:

$$
\Delta Q(v: C \to C', \gamma)
= \frac{w(v,C') - w(v,C)}{2m}
+ \gamma\,\frac{k_v\left(d(C) - k_v - d(C')\right)}{(2m)^2}
$$

where $w(v,C)$ is total edge weight from $v$ to nodes in $C$,
$k_v=d(v)$, and $d(C)=\sum_{u\in C} d(u)$.

## Algorithm 1: Leiden Algorithm

**Input**: $G$, $f(\cdot)$, $P$, $\gamma$
**Output**: Updated $f(\cdot)$

1. $G^1 \leftarrow G$, $f^1(\cdot) \leftarrow f(\cdot)$;
2. **for** $p = 1$ to $P$ **do**
3. $\quad f^p(\cdot) \leftarrow \text{Move}(G^p, f^p(\cdot), \gamma)$;
4. $\quad s^p(\cdot) \leftarrow \text{Refine}(G^p, f^p(\cdot), \gamma)$;
5. $\quad$ **if** $p < P$ **then**
6. $\quad\quad G^{p+1}, f^{p+1}(\cdot) \leftarrow \text{Aggregate}(G^p, f^p(\cdot), s^p(\cdot))$;
7. Update $f(\cdot)$ using $s^1(\cdot), \cdots, s^P(\cdot)$;
8. **return** $f(\cdot)$;

## Algorithm 2: Inc-movement

**Input**: $G$, $\Delta G$, $f(\cdot)$, $s(\cdot)$, $\Psi$, $\gamma$
**Output**: Updated $f(\cdot)$, $\Psi$, $B$, $K$

1. $A \leftarrow \emptyset$, $B \leftarrow \emptyset$, $K \leftarrow \emptyset$;
2. **for** $(v_i, v_j, \alpha) \in \Delta G$ **do**
3. $\quad$ **if** $\alpha > 0$ and $f(v_i) \neq f(v_j)$ **then**
4. $\quad\quad A.add(v_i)$; $A.add(v_j)$;
5. $\quad$ **if** $\alpha < 0$ and $f(v_i) = f(v_j)$ **then**
6. $\quad\quad A.add(v_i)$; $A.add(v_j)$;
7. $\quad$ **if** $s(v_i) = s(v_j)$ and $\text{update\_edge}(G_\Psi, (v_i, v_j, \alpha))$ **then**
8. $\quad\quad K.add(v_i)$; $K.add(v_j)$;
9. **for** $A \neq \emptyset$ **do**
10. $\quad v_i \leftarrow A.pop()$;
11. $\quad C^* \leftarrow \text{argmax}_{C \in \mathbb{C} \cup \emptyset} \Delta Q(v_i \to C, \gamma)$;
12. $\quad$ **if** $\Delta Q(v_i \to C^*, \gamma) > 0$ **then**
13. $\quad\quad f(v_i) \leftarrow C^*$; $B.add(v_i)$;
14. $\quad\quad$ **for** $v_j \in N(v_i)$ **do**
15. $\quad\quad\quad$ **if** $f(v_j) \neq C^*$ **then**
16. $\quad\quad\quad\quad A.add(v_j)$;
17. $\quad\quad$ **for** $v_j \in N(v_i) \wedge s(v_i) = s(v_j)$ **do**
18. $\quad\quad\quad$ **if** $\text{update\_edge}(G_\Psi, (v_i, v_j, -w(v_i, v_j)))$ **then**
19. $\quad\quad\quad\quad K.add(v_i)$; $K.add(v_j)$;
20. **return** $f(\cdot)$, $\Psi$, $B$, $K$;

## Algorithm 3: Inc-refinement

**Input**: $G$, $f(\cdot)$, $s(\cdot)$, $\Psi$, $K$, $\gamma$
**Output**: Updated $s(\cdot)$, $\Psi$, $R$

1. $R \leftarrow \emptyset$;
2. **for** $v_i \in K$ **do**
3. $\quad$ **if** $v_i$ is not in the largest connected component of $s(v)$ **then**
4. $\quad\quad$ Map all vertices in the connected component into a new sub-community and add them into $R$;
5. **for** $v_i \in R$ **do**
6. $\quad$ **if** $v_i$ is in singleton sub-community **then**
7. $\quad\quad \mathcal{T} \leftarrow \{s(v) | v \in N(v_i) \cap f(v_i), \Delta Q(s(v) \to \emptyset, \gamma) \leq 0\}$;
8. $\quad\quad S^* \leftarrow \text{argmax}_{S \in \mathcal{T}} \Delta M(v_i \to S, \gamma)$;
9. $\quad\quad$ **if** $\Delta M(v_i \to S^*, \gamma) > 0$ **then**
10. $\quad\quad\quad s(v_i) \leftarrow S^*$;
11. $\quad\quad\quad$ **for** $v_j \in N(v_i)$ **do**
12. $\quad\quad\quad\quad$ **if** $s(v_i) = s(v_j)$ **then**
13. $\quad\quad\quad\quad\quad \text{update\_edge}(G_\Psi, (v_i, v_j, w(v_i, v_j)))$;
14. **return** $s(\cdot)$, $\Psi$, $R$;

## Algorithm 4: Inc-aggregation

**Input**: $G$, $\Delta G$, $s_{pre}(\cdot)$, $s_{cur}(\cdot)$, $R$
**Output**: $\Delta H$, $s_{pre}(\cdot)$

1. $\Delta H \leftarrow \emptyset$;
2. **for** $(v_i, v_j, \alpha) \in \Delta G$ **do**
3. $\quad r_i \leftarrow s_{pre}(v_i)$, $r_j \leftarrow s_{pre}(v_j)$;
4. $\quad \Delta H.add((s_i, s_j, \alpha))$;
5. **for** $v_i \in R$ **do**
6. $\quad$ **for** $v_j \in N(v_j)$ **do**
7. $\quad\quad$ **if** $s_{cur}(v_j) = s_{pre}(v_j)$ or $i < j$ **then**
8. $\quad\quad\quad \Delta H.add((s_{pre}(v_i), s_{pre}(v_j), -w(v_i, v_j)))$;
9. $\quad\quad\quad \Delta H.add((s_{cur}(v_i), s_{cur}(v_j), w(v_i, v_j)))$;
10. $\quad \Delta H.add((s_{pre}(v_i), s_{pre}(v_i), -w(v_i, v_i)))$;
11. $\quad \Delta H.add((s_{cur}(v_i), s_{cur}(v_i), w(v_i, v_i)))$;
12. **for** $v_i \in R$ **do**
13. $\quad s_{pre}(v_i) \leftarrow s_{cur}(v_i)$;
14. $\text{Compress}(\Delta H)$;
15. **return** $\Delta H$, $s_{pre}(\cdot)$;

## Algorithm 5: Def-update

**Input**: $\{f^P(\cdot)\}$, $\{s^P(\cdot)\}$, $\{B^P\}$, $P$
**Output**: Updated $\{f^P(\cdot)\}$

1. **for** $p$ from $P$ to 1 **do**
2. $\quad$ **if** $p \neq P$ **then**
3. $\quad\quad$ **for** $v_i^p \in B^p$ **do**
4. $\quad\quad\quad f^p(v_i^p) = f^{p+1}(s^p(v_i^p))$;
5. $\quad$ **if** $p \neq 1$ **then**
6. $\quad\quad$ **for** $v_i^p \in B^p$ **do**
7. $\quad\quad\quad B^{p-1}.add(s^{-p}(v_i^p))$;
8. **return** $\{f^P(\cdot)\}$;

## Algorithm 6: HIT-Leiden

**Input**: $\{G^P\}$, $\Delta G$, $\{f^P(\cdot)\}$, $\{g^P(\cdot)\}$, $\{s_{pre}^P(\cdot)\}$, $\{s_{cur}^P(\cdot)\}$, $\{\Psi^P\}$, $P$, $\gamma$
**Output**: $f(\cdot)$, $\{G^P\}$, $\{f^P(\cdot)\}$, $\{g^P(\cdot)\}$, $\{s_{pre}^P(\cdot)\}$, $\{s_{cur}^P(\cdot)\}$, $\{\Psi^P\}$

1. $\Delta G^1 \leftarrow \Delta G$;
2. **for** $p$ from 1 to $P$ **do**
3. $\quad G^p \leftarrow G^p \oplus \Delta G^p$;
4. $\quad f^p(\cdot), \Psi, B^p, K \leftarrow \text{inc-movement}(G^p, \Delta G^p, f^p(\cdot), s_{cur}^p(\cdot), \Psi, \gamma)$;
5. $\quad s_{cur}^p(\cdot), \Psi, R^p \leftarrow \text{inc-refinement}(G^p, f^p(\cdot), s_{cur}^p(\cdot), \Psi, K, \gamma)$;
6. $\quad$ **if** $p < P$ **then**
7. $\quad\quad \Delta G^{p+1}, s_{pre}^p(\cdot) \leftarrow \text{inc-aggregation}(G^p, \Delta G^p, s_{pre}^p(\cdot), s_{cur}^p(\cdot), R^p)$;
8. $\{f^P(\cdot)\} \leftarrow \text{def-update}(\{f^P(\cdot)\}, \{s_{cur}^P(\cdot)\}, \{B^P\}, P)$;
9. $\{g^P(\cdot)\} \leftarrow \text{def-update}(\{g^P(\cdot)\}, \{s_{cur}^P(\cdot)\}, \{R^P\}, P)$;
10. $f(\cdot) \leftarrow g^1(\cdot)$;
11. **return** $f(\cdot)$, $\{G^P\}$, $\{f^P(\cdot)\}$, $\{g^P(\cdot)\}$, $\{s_{pre}^P(\cdot)\}$, $\{s_{cur}^P(\cdot)\}$, $\{\Psi^P\}$;

## Throughput Mode Extensions (Implemented)

The current throughput implementation adds three safeguards/extensions to the baseline pseudocode above.

### T1. Decoupled parallel move application for Algorithm 2

In throughput mode, line 11 of Algorithm 2 is split into **candidate generation** and **decoupled selection**:

1. Build candidate moves in parallel as records equivalent to Rust
   `MoveCandidate { node, from_comm, to_comm, node_degree, gain }`:
	$$
	M = \{(v, C_{from}, C_{to}, d(v), \Delta Q(v\to C_{to},\gamma))\}
	$$
2. Sort $M$ by descending $\Delta Q$.
3. Greedily select decoupled moves into $\hat{M}$ while tracking:
	- $E_{emit}$: communities that already emit,
	- $E_{acc}$: communities that already accept.
4. Accept a candidate $(v, C_{from}, C_{to}, \dots)$ **iff**:
	$$
	C_{from} \notin E_{acc} \quad \land \quad C_{to} \notin E_{emit}
	$$
	then update:
	$$
	E_{emit} \leftarrow E_{emit}\cup\{C_{from}\},\quad
	E_{acc} \leftarrow E_{acc}\cup\{C_{to}\}
	$$

In code, this corresponds to `select_decoupled_moves(...)`, which also enforces one move per node (`moved_nodes`).

This matches the decoupling rule from the paper (no move leaves an acceptor or enters an emitter).

### T2. Monotonicity guard in throughput movement

Before applying selected moves, throughput computes `total_gain`:
$$
\Sigma\Delta Q(\hat{M}) = \sum_{m\in\hat{M}} \Delta Q(m)
$$
If:
$$
\Sigma\Delta Q(\hat{M}) \le 0
$$
then no move from this batch is applied.

### T3. Aggregation skip in Algorithm 6

In line 6-7 of Algorithm 6, throughput now skips `inc-aggregation` when both are true:
1. $\Delta G^p$ is empty, and
2. $R^p$ is empty.

Formally, define:
$$
\operatorname{skip}(p) = (|\Delta G^p| = 0) \land (|R^p| = 0)
$$
If $\text{skip}(p)$ and $p<P$, set $\Delta G^{p+1}=\emptyset$ and continue.

In code this predicate is implemented by:
`should_skip_aggregation(delta_graph, refined_nodes)`.

### Throughput multilevel safety gate (current runtime policy)

For the standalone multilevel movement path (outside Algorithm 6 incremental updates), parallel movement remains safety-gated off in throughput mode until full quality parity is guaranteed. Incremental throughput in Algorithm 6 remains enabled with T1-T3.

## Appendix A: Rust Symbol Supplement (Math \(\leftrightarrow\) Implementation)

This appendix maps mathematical symbols and algorithm operators to concrete Rust symbols in the current implementation.

### A1. Core state symbols

| Math symbol | Rust symbol(s) | Location |
|---|---|---|
| $G$ | `GraphInput`, `InMemoryGraph` | `src/core/types.rs`, `src/core/graph/in_memory.rs` |
| $\Delta G$ | `delta_g`, `delta_graph`, `current_delta` | `src/core/algorithm/hit_leiden.rs` |
| $f(\cdot)$ | `state.node_to_comm` | `src/core/algorithm/hit_leiden.rs` |
| $f^p(\cdot)$ | `state.community_mapping_per_level[p]` | `src/core/algorithm/hit_leiden.rs` |
| $g^p(\cdot)$ | `state.refined_community_mapping_per_level[p]` | `src/core/algorithm/hit_leiden.rs` |
| $s_{pre}^p(\cdot)$ | `state.previous_subcommunity_mapping_per_level[p]` | `src/core/algorithm/hit_leiden.rs` |
| $s_{cur}^p(\cdot)$ | `state.current_subcommunity_mapping_per_level[p]` | `src/core/algorithm/hit_leiden.rs` |
| $P$ | `state.levels`, `p_max` | `src/core/algorithm/hit_leiden.rs` |
| $\gamma$ | `gamma`, `resolution_parameter` | `src/core/algorithm/hit_leiden.rs`, `throughput.rs` |
| $d(v)$ | `node_degrees[node]`, `current_node_degree` | `hit_leiden.rs`, `throughput.rs`, `parallel_frontier.rs` |
| $2m$ | `twice_total_weight` | `hit_leiden.rs`, `throughput.rs`, `parallel_frontier.rs` |

### A2. Algorithm-level operator mapping

| Spec operator | Rust function | Location |
|---|---|---|
| `Move` / `inc-movement` | `inc_movement(...)` | `src/core/algorithm/hit_leiden.rs` |
| parallel `inc-movement` shard eval | `execute_shard(...)` | `src/core/algorithm/parallel_frontier.rs` |
| parallel `inc-movement` orchestrator | `inc_movement_parallel(...)` | `src/core/algorithm/throughput.rs` |
| `Refine` / `inc-refinement` | `inc_refinement(...)` | `src/core/algorithm/hit_leiden.rs` |
| parallel `inc-refinement` | `inc_refinement_parallel(...)` | `src/core/algorithm/throughput.rs` |
| `Aggregate` / `inc-aggregation` | `inc_aggregation(...)` | `src/core/algorithm/hit_leiden.rs` |
| `def-update` | `def_update(...)` | `src/core/algorithm/hit_leiden.rs` |
| HIT-Leiden loop | `hit_leiden(...)` | `src/core/algorithm/hit_leiden.rs` |
| multilevel Leiden path | `multilevel_leiden(...)` | `src/core/algorithm/hit_leiden.rs` |

### A3. Throughput decoupling symbols

| Math symbol | Rust symbol(s) | Location |
|---|---|---|
| $M$ | `Vec<MoveCandidate> all_candidates` | `src/core/algorithm/throughput.rs` |
| candidate record | `MoveCandidate { node, from_comm, to_comm, node_degree, gain }` | `src/core/algorithm/parallel_frontier.rs` |
| $\hat{M}$ | `selected_moves` | `src/core/algorithm/throughput.rs` |
| decoupling selector | `select_decoupled_moves(...)` | `src/core/algorithm/throughput.rs` |
| $E_{emit}$ | `emitters: HashSet<usize>` | `src/core/algorithm/throughput.rs` |
| $E_{acc}$ | `acceptors: HashSet<usize>` | `src/core/algorithm/throughput.rs` |
| one-move-per-node constraint | `moved_nodes: HashSet<usize>` | `src/core/algorithm/throughput.rs` |
| $\Sigma\Delta Q(\hat{M})$ | `total_gain: f64` | `src/core/algorithm/throughput.rs` |
| changed node set $B$ | `shared_changed` / `changed_nodes` | `throughput.rs`, `hit_leiden.rs` |
| affected refinement set $K$ | `shared_affected` / `affected_nodes_for_refinement` | `throughput.rs`, `hit_leiden.rs` |

### A4. Throughput control/safety symbols

| Spec concept | Rust symbol(s) | Location |
|---|---|---|
| skip aggregation when no delta and no refinement | `should_skip_aggregation(delta_graph, refined_nodes)` | `src/core/algorithm/hit_leiden.rs` |
| multilevel throughput safety gate | `const USE_PARALLEL_MULTILEVEL_MOVEMENT: bool = false` | `src/core/algorithm/hit_leiden.rs` |
| max movement iterations (throughput) | `const MAX_MOVEMENT_ITERATIONS: usize = 20` | `src/core/algorithm/hit_leiden.rs` |
| lock-free bitsets for parallel writes | `SharedBitVec` | `src/core/algorithm/parallel_frontier.rs` |
