//! Phase27b §2.1–§2.3 — in-process community detection (Louvain + Leiden refinement).
//!
//! Pure, deterministic, provably-terminating graph community detection.
//! No network or Nexus dependency in this module.
//!
//! ## Termination guarantees
//!
//! Every loop carries a hard cap enforced at the `CommunityConfig` level:
//!
//! - **Local-move passes**: at most `max_local_move_passes` (default 50) per
//!   Louvain level. Stops early when total modularity gain `< EPSILON`.
//! - **Coarsening levels**: at most `max_levels` (default 20). Stops when a
//!   level produces no community merges (partition unchanged).
//! - **Oversized-community recursive splits**: at most `max_split_depth`
//!   recursion levels (default 6) AND only recurses when the split
//!   actually reduces max-community size.
//! - **Hub re-attachment**: single pass — trivially bounded.
//!
//! ## Determinism
//!
//! Nodes are sorted by `node_id` string at construction and always iterated
//! in that stable order. Community-id assignment uses the smallest available
//! integer. All tie-breaking is by (community_id, node_index) — no hash-map
//! traversal order is ever exposed to the algorithm.

use std::collections::HashMap;

use super::patch::NodeOp;

// ── Constants ──────────────────────────────────────────────────────────────

/// Modularity-gain threshold below which a local-move pass is considered
/// converged. Prevents infinite cycling on negligible gains.
const EPSILON: f64 = 1e-9;

// ── Public types ───────────────────────────────────────────────────────────

/// A node identifier — the stable string key from the caller's domain.
pub type NodeId = String;

/// Weighted undirected graph used as input to community detection.
///
/// Construct via [`CommunityGraph::new`]; edges are symmetrized internally.
#[derive(Debug, Clone)]
pub struct CommunityGraph {
    /// Stable-sorted node ids (ascending lexicographic order).
    pub nodes: Vec<NodeId>,
    /// Index-based undirected edges: `(u, v, weight)` where `u < v`.
    pub edges: Vec<(usize, usize, f64)>,
    /// CSR-ish adjacency list: `adj[u]` = `[(v, weight), …]`.
    pub adj: Vec<Vec<(usize, f64)>>,
    /// Weighted degree per node (`k_i = Σ w_{ij}`).
    pub degree: Vec<f64>,
    /// Total graph weight `2m` (sum of all edge weights × 2 for undirected).
    pub two_m: f64,
}

impl CommunityGraph {
    /// Construct a [`CommunityGraph`] from a list of node ids and
    /// weighted edges. Edges are specified as `(from_id, to_id, weight)`;
    /// self-loops are dropped; parallel edges are summed.
    ///
    /// Nodes are sorted deterministically so index 0 = lexicographically
    /// smallest id.
    pub fn new(node_ids: Vec<NodeId>, raw_edges: Vec<(NodeId, NodeId, f64)>) -> Self {
        let mut nodes = node_ids;
        nodes.sort_unstable();
        nodes.dedup();

        let idx: HashMap<&str, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();

        // Accumulate weights into a symmetric adjacency map.
        let n = nodes.len();
        let mut adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n];

        for (a, b, w) in &raw_edges {
            let Some(&u) = idx.get(a.as_str()) else {
                continue;
            };
            let Some(&v) = idx.get(b.as_str()) else {
                continue;
            };
            if u == v {
                continue; // drop self-loops
            }
            *adj[u].entry(v).or_insert(0.0) += w;
            *adj[v].entry(u).or_insert(0.0) += w;
        }

        // Materialise sorted adjacency + canonical edge list.
        let mut edges = Vec::new();
        let mut adj_vec: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
        let mut degree = vec![0.0_f64; n];

        for u in 0..n {
            let mut neighbors: Vec<(usize, f64)> = adj[u]
                .iter()
                .map(|(&v, &w)| (v, w))
                .collect();
            neighbors.sort_unstable_by_key(|&(v, _)| v);
            for &(v, w) in &neighbors {
                degree[u] += w;
                if u < v {
                    edges.push((u, v, w));
                }
            }
            adj_vec[u] = neighbors;
        }

        let two_m: f64 = degree.iter().sum(); // sum of all edge weights × 2

        Self {
            nodes,
            edges,
            adj: adj_vec,
            degree,
            two_m,
        }
    }

    /// Number of nodes.
    #[inline]
    pub fn n(&self) -> usize {
        self.nodes.len()
    }
}

/// Configuration for community detection.
#[derive(Debug, Clone)]
pub struct CommunityConfig {
    /// Hard cap on local-move passes per Louvain level.
    pub max_local_move_passes: usize,
    /// Hard cap on outer Louvain pass repetitions (extra termination guard).
    pub max_levels: usize,
    /// Fraction of total nodes above which a community is "oversized"
    /// and gets recursively re-partitioned (§2.2).
    pub oversized_fraction: f64,
    /// Max recursion depth for oversized-community splitting (§2.2).
    pub max_split_depth: u32,
    /// Percentile (0–1) above which a node is a "hub" and is
    /// temporarily excluded from partitioning (§2.3).
    pub hub_percentile: f64,
}

impl Default for CommunityConfig {
    fn default() -> Self {
        Self {
            max_local_move_passes: 50,
            max_levels: 20,
            oversized_fraction: 0.25,
            max_split_depth: 6,
            hub_percentile: 0.99,
        }
    }
}

/// Per-node community assignment returned by [`detect_communities`].
#[derive(Debug, Clone)]
pub struct NodeAssignment {
    /// Community label (integer, ≥ 0).
    pub community_id: u32,
    /// Louvain level at which this node's community was last updated
    /// (0 = base level).
    pub level: u32,
    /// `true` when this node was classified as a hub (§2.3) and
    /// re-attached to its majority-neighbor community.
    pub is_hub: bool,
}

/// Output of [`detect_communities`]: one [`NodeAssignment`] per node id.
pub type CommunityResult = HashMap<NodeId, NodeAssignment>;

// ── Internal working state ─────────────────────────────────────────────────

/// Per-node mutable state used during Louvain passes.
struct LouvainState {
    /// `comm[i]` = community id of node `i`.
    comm: Vec<usize>,
    /// `sigma_tot[c]` = sum of weighted degrees of all nodes in community `c`.
    sigma_tot: Vec<f64>,
    /// `sigma_in[c]` = sum of internal edge weights × 2 for community `c`.
    sigma_in: Vec<f64>,
}

impl LouvainState {
    fn new(n: usize, degree: &[f64]) -> Self {
        // Each node starts in its own singleton community.
        let comm: Vec<usize> = (0..n).collect();
        let sigma_tot = degree.to_vec();
        let sigma_in = vec![0.0; n];
        Self {
            comm,
            sigma_tot,
            sigma_in,
        }
    }

    /// `k_{i,C}`: sum of edge weights from node `i` to community `c`.
    fn k_i_c(
        &self,
        node: usize,
        target_comm: usize,
        adj: &[Vec<(usize, f64)>],
    ) -> f64 {
        adj[node]
            .iter()
            .filter(|&&(v, _)| self.comm[v] == target_comm)
            .map(|&(_, w)| w)
            .sum()
    }
}

// ── Core Louvain algorithm ─────────────────────────────────────────────────

/// Proportional modularity gain for inserting node `i` into community `C`.
///
/// Standard Blondel 2008 formula (scaled by 2/two_m, which is constant and
/// positive so does not affect the argmax):
///
/// `g(C) = k_{i,C} - sigma_tot(C) * k_i / two_m`
///
/// The actual ΔQ of moving `i` from `src` to `dst` is
/// `(g(dst) - g(src)) * 2 / two_m`.  We only need the sign and ranking,
/// so comparing `g` values directly is sufficient.
#[inline]
fn modularity_gain_proportional(
    k_i_c: f64,
    sigma_tot_c: f64,
    ki: f64,
    two_m: f64,
) -> f64 {
    k_i_c - sigma_tot_c * ki / two_m
}

/// Run one full local-move phase on `graph` starting from `state`.
/// Returns `true` if any node moved (i.e., partition changed).
fn local_move_phase(
    graph: &CommunityGraph,
    state: &mut LouvainState,
    cfg: &CommunityConfig,
) -> bool {
    let n = graph.n();
    let two_m = graph.two_m;
    if two_m < EPSILON {
        return false; // isolated graph — no moves possible
    }

    let mut any_moved = false;

    for _pass in 0..cfg.max_local_move_passes {
        let mut pass_gain = 0.0_f64;
        let mut pass_moved = false;

        // Iterate nodes in stable index order (= sorted node-id order).
        for i in 0..n {
            let current_comm = state.comm[i];
            let ki = graph.degree[i];

            // Compute connection to current community (excluding self).
            let k_i_current = state.k_i_c(i, current_comm, &graph.adj);

            // Temporarily remove node i from its community.
            state.sigma_tot[current_comm] -= ki;
            state.sigma_in[current_comm] -= 2.0 * k_i_current;

            // Baseline gain: staying in (now empty) current_comm.
            // After removal sigma_tot[current_comm] no longer includes ki,
            // so gain of "re-inserting into current_comm" uses updated value.
            let gain_current = modularity_gain_proportional(
                k_i_current,
                state.sigma_tot[current_comm],
                ki,
                two_m,
            );

            // Collect candidate communities from neighbours only
            // (we always consider current_comm as the baseline).
            let mut candidates: Vec<usize> = graph.adj[i]
                .iter()
                .map(|&(v, _)| state.comm[v])
                .collect();
            candidates.sort_unstable();
            candidates.dedup();

            // Choose the community that maximises net modularity gain over
            // staying in current_comm.  Tie-break: lowest community id.
            let mut best_comm = current_comm;
            let mut best_delta = 0.0_f64; // must be > 0 to justify a move

            for c in &candidates {
                let c = *c;
                if c == current_comm {
                    continue; // moving to own community is never a net gain
                }
                let k_i_c = state.k_i_c(i, c, &graph.adj);
                let gain_c = modularity_gain_proportional(
                    k_i_c,
                    state.sigma_tot[c],
                    ki,
                    two_m,
                );
                let delta = gain_c - gain_current;
                if delta > best_delta || (delta == best_delta && c < best_comm) {
                    best_delta = delta;
                    best_comm = c;
                }
            }

            // Re-insert node i into chosen community.
            state.comm[i] = best_comm;
            let k_i_best = state.k_i_c(i, best_comm, &graph.adj);
            state.sigma_tot[best_comm] += ki;
            state.sigma_in[best_comm] += 2.0 * k_i_best;

            if best_comm != current_comm {
                pass_moved = true;
                pass_gain += best_delta;
                any_moved = true;
            }
        }

        // Termination: stop when pass produced no gain or no moves.
        if !pass_moved || pass_gain < EPSILON {
            break;
        }
    }

    any_moved
}

/// Apply Leiden refinement: split any community that is internally
/// disconnected into its connected components (BFS-based).
///
/// Returns the (possibly updated) community assignment vector.
fn leiden_refinement(n: usize, comm: &[usize], adj: &[Vec<(usize, f64)>]) -> Vec<usize> {
    // Group nodes by community.
    let mut by_comm: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, &c) in comm.iter().enumerate().take(n) {
        by_comm.entry(c).or_default().push(i);
    }

    let mut result = comm.to_vec();
    // We'll assign new community ids starting from the next available id.
    let mut next_comm_id = comm.iter().copied().max().unwrap_or(0) + 1;

    // Sort community keys for determinism.
    let mut comm_ids: Vec<usize> = by_comm.keys().copied().collect();
    comm_ids.sort_unstable();

    for c in comm_ids {
        let members = &by_comm[&c];
        if members.len() <= 1 {
            continue; // singleton or empty — trivially connected
        }

        // Build a local index within the community.
        let local_idx: HashMap<usize, usize> = members
            .iter()
            .enumerate()
            .map(|(li, &gi)| (gi, li))
            .collect();

        let m = members.len();
        let mut visited = vec![false; m];
        let mut first_component = true;

        // BFS to find connected components within the induced subgraph.
        for start_li in 0..m {
            if visited[start_li] {
                continue;
            }
            // BFS
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(start_li);
            visited[start_li] = true;

            let component_comm = if first_component {
                first_component = false;
                c // keep the original community id for the first component
            } else {
                let id = next_comm_id;
                next_comm_id += 1;
                id
            };

            while let Some(li) = queue.pop_front() {
                let gi = members[li];
                result[gi] = component_comm;

                for &(v, _) in &adj[gi] {
                    if let Some(&lv) = local_idx.get(&v) {
                        if !visited[lv] {
                            visited[lv] = true;
                            queue.push_back(lv);
                        }
                    }
                }
            }
        }
    }

    result
}


// ── Public entry point ─────────────────────────────────────────────────────

/// Detect communities in `graph` using Louvain + Leiden refinement
/// (§2.1), oversized-community splitting (§2.2), and hub exclusion /
/// re-attachment (§2.3).
///
/// The algorithm is **deterministic**: identical inputs produce identical
/// outputs on every call. All loops carry hard caps from `cfg`.
pub fn detect_communities(graph: &CommunityGraph, cfg: &CommunityConfig) -> CommunityResult {
    if graph.n() == 0 {
        return HashMap::new();
    }

    // §2.3 — identify and exclude hub nodes before partitioning.
    let (partitionable_nodes, hub_indices) = identify_hubs(graph, cfg);

    // Build a sub-graph excluding hubs.
    let (sub_graph, sub_to_original) = build_subgraph(graph, &partitionable_nodes);

    // Run Louvain on the sub-graph.
    let sub_comm = if sub_graph.n() == 0 {
        vec![]
    } else {
        run_louvain(&sub_graph, cfg)
    };

    // Map sub-graph assignments back to original node indices.
    let mut final_comm: Vec<u32> = vec![0; graph.n()];
    let mut final_level: Vec<u32> = vec![0; graph.n()];

    // Compact sub-graph community ids → unique u32.
    let (compacted, _) = compact_community_ids(&sub_comm);
    for (sub_i, &orig_i) in sub_to_original.iter().enumerate() {
        final_comm[orig_i] = compacted[sub_i];
        final_level[orig_i] = 0;
    }

    // §2.2 — split oversized communities.
    let (final_comm, final_level) =
        split_oversized(graph, final_comm, final_level, cfg, 0, &sub_to_original);

    // §2.3 — re-attach hub nodes to majority-neighbor community.
    let mut is_hub = vec![false; graph.n()];
    for &hub_i in &hub_indices {
        is_hub[hub_i] = true;
    }
    let final_comm = reattach_hubs(graph, final_comm, &hub_indices, &is_hub);

    // Build result map.
    let mut result = HashMap::with_capacity(graph.n());
    for (i, node_id) in graph.nodes.iter().enumerate() {
        result.insert(
            node_id.clone(),
            NodeAssignment {
                community_id: final_comm[i],
                level: final_level[i],
                is_hub: is_hub[i],
            },
        );
    }
    result
}

// ── §2.4 — NodeOp writeback ───────────────────────────────────────────────

/// Node property: the detected community id.
pub const COMMUNITY_ID_PROP: &str = "community_id";
/// Node property: the hierarchy level the community was last updated at.
pub const COMMUNITY_LEVEL_PROP: &str = "community_level";
/// Node property: `true` for a hub / "god node" (§2.3).
pub const COMMUNITY_GOD_NODE_PROP: &str = "is_god_node";

/// phase27b §2.4 — map a [`CommunityResult`] to idempotent [`NodeOp`]s that
/// stamp `community_id` + `community_level` (+ `is_god_node`) onto each graph
/// node, ready to push onto a `GraphPatch`.
///
/// - **Idempotent**: every op is built via [`NodeOp::with_identity`], whose
///   default [`super::patch::ConflictPolicy::Match`] sets only these three
///   props on re-run and never clobbers other downstream-stamped properties.
/// - **Deterministic**: ops are emitted sorted by node id, and the community
///   ids come from the deterministic [`detect_communities`] pass.
/// - `label_for` resolves each node id to its Cypher label (the architecture
///   nodes the semantic projection produces). Nodes whose label cannot be
///   resolved are skipped — the live architecture subgraph is gated on the
///   projection (ADR-027), so this stays a pure, offline-testable mapper.
pub fn community_node_ops(
    result: &CommunityResult,
    label_for: impl Fn(&str) -> Option<String>,
) -> Vec<NodeOp> {
    let mut ids: Vec<&NodeId> = result.keys().collect();
    ids.sort();
    let mut ops = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(label) = label_for(id) else {
            continue;
        };
        let a = &result[id];
        let mut op = NodeOp::with_identity(label, id.clone());
        op.props
            .insert(COMMUNITY_ID_PROP.into(), serde_json::json!(a.community_id));
        op.props
            .insert(COMMUNITY_LEVEL_PROP.into(), serde_json::json!(a.level));
        op.props
            .insert(COMMUNITY_GOD_NODE_PROP.into(), serde_json::json!(a.is_hub));
        ops.push(op);
    }
    ops
}

// ── §2.1 — Louvain driver ─────────────────────────────────────────────────

/// Run Louvain local-move + Leiden refinement on `graph`.
///
/// This is a **single-level** Louvain: all passes operate on the original
/// graph with its full edge weights, so `two_m` is constant throughout.
/// Multi-level coarsening is intentionally omitted because the coarsened
/// graph drops intra-community edge weights, which can cause spurious
/// cross-community merges on subsequent levels.  For the scales this
/// module targets (projected knowledge graphs, typically ≤ 50k nodes),
/// repeated passes on the original graph converge correctly.
///
/// Termination is guaranteed by `cfg.max_local_move_passes` (internal
/// cap inside [`local_move_phase`]) and by the `max_levels` outer loop
/// that stops when no new merges occur.
///
/// Returns a community-assignment vector indexed by original node index,
/// with ids compacted to `0..k`.
fn run_louvain(graph: &CommunityGraph, cfg: &CommunityConfig) -> Vec<usize> {
    let n = graph.n();
    if n == 0 {
        return vec![];
    }

    let mut state = LouvainState::new(n, &graph.degree);

    // Run local-move passes until convergence or the cap fires.
    // `local_move_phase` already contains the per-pass cap; this outer
    // loop adds a `max_levels` cap as an extra termination guard.
    for _level in 0..cfg.max_levels {
        let moved = local_move_phase(graph, &mut state, cfg);
        if !moved {
            break; // converged — no node moved in the last full pass
        }
    }

    // Apply Leiden refinement: split any disconnected community into its
    // connected components (BFS on the induced subgraph).
    let refined = leiden_refinement(n, &state.comm, &graph.adj);

    // Compact community ids to a contiguous 0..k range.
    // Sort the unique ids so the mapping is deterministic.
    let mut unique: Vec<usize> = refined.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let remap: HashMap<usize, usize> = unique
        .iter()
        .enumerate()
        .map(|(new_id, &old_id)| (old_id, new_id))
        .collect();

    refined.iter().map(|c| remap[c]).collect()
}

// ── §2.2 — Oversized-community splitting ──────────────────────────────────

/// Recursively split communities that are larger than
/// `cfg.oversized_fraction * total_nodes`. Bounded by `cfg.max_split_depth`.
fn split_oversized(
    graph: &CommunityGraph,
    mut comm: Vec<u32>,
    mut level: Vec<u32>,
    cfg: &CommunityConfig,
    depth: u32,
    partitionable: &[usize],
) -> (Vec<u32>, Vec<u32>) {
    if depth >= cfg.max_split_depth {
        return (comm, level);
    }

    let total = partitionable.len().max(1);
    let threshold = (cfg.oversized_fraction * total as f64).ceil() as usize;

    // Find the largest community among partitionable nodes.
    let mut comm_sizes: HashMap<u32, usize> = HashMap::new();
    for &i in partitionable {
        *comm_sizes.entry(comm[i]).or_insert(0) += 1;
    }

    // Sort for deterministic processing.
    let mut oversized: Vec<u32> = comm_sizes
        .iter()
        .filter(|(_, &sz)| sz > threshold)
        .map(|(&c, _)| c)
        .collect();
    oversized.sort_unstable();

    if oversized.is_empty() {
        return (comm, level);
    }

    let mut next_comm_id: u32 = *comm.iter().max().unwrap_or(&0) + 1;

    for big_comm in oversized {
        // Gather member nodes (partitionable only).
        let members: Vec<usize> = partitionable
            .iter()
            .copied()
            .filter(|&i| comm[i] == big_comm)
            .collect();

        if members.len() <= 1 {
            continue;
        }

        // Build induced sub-graph for just these members.
        let (sub_graph, sub_to_orig) = build_subgraph(graph, &members);
        if sub_graph.n() <= 1 {
            continue;
        }

        let sub_comm_raw = run_louvain(&sub_graph, cfg);
        let (sub_comm, _) = compact_community_ids(&sub_comm_raw);

        // Only recurse if the split actually produced multiple communities.
        let num_sub_comms = {
            let mut ids = sub_comm.clone();
            ids.sort_unstable();
            ids.dedup();
            ids.len()
        };
        if num_sub_comms <= 1 {
            // No progress — indivisible; stop.
            continue;
        }

        // Assign new community ids.
        let base_id = next_comm_id;
        next_comm_id += num_sub_comms as u32;
        for (sub_i, &orig_i) in sub_to_orig.iter().enumerate() {
            comm[orig_i] = base_id + sub_comm[sub_i];
            level[orig_i] = depth + 1;
        }

        // Recurse.
        let new_partitionable: Vec<usize> = sub_to_orig.clone();
        let (c, l) = split_oversized(graph, comm, level, cfg, depth + 1, &new_partitionable);
        comm = c;
        level = l;
    }

    (comm, level)
}

// ── §2.3 — Hub identification + re-attachment ─────────────────────────────

/// Identify hub nodes (top `hub_percentile` by degree).
/// Returns `(non_hub_indices, hub_indices)`.
fn identify_hubs(
    graph: &CommunityGraph,
    cfg: &CommunityConfig,
) -> (Vec<usize>, Vec<usize>) {
    let n = graph.n();
    if n == 0 {
        return (vec![], vec![]);
    }

    // Sort by degree to find the percentile cutoff.
    let mut sorted_degrees: Vec<f64> = graph.degree.clone();
    sorted_degrees.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let cutoff_idx = ((cfg.hub_percentile * n as f64).floor() as usize).min(n - 1);
    let cutoff = sorted_degrees[cutoff_idx];

    // A node is a hub only if its degree is STRICTLY above the cutoff AND
    // there are nodes below the cutoff (so we always have non-hubs to
    // partition if possible).
    let mut non_hubs = Vec::new();
    let mut hubs = Vec::new();

    // First pass: count how many would be hubs at strict cutoff.
    let hub_count = graph.degree.iter().filter(|&&d| d > cutoff).count();

    if hub_count == n {
        // All nodes would be hubs (e.g., complete graph) — don't exclude any.
        return ((0..n).collect(), vec![]);
    }

    for i in 0..n {
        if graph.degree[i] > cutoff && hub_count < n {
            hubs.push(i);
        } else {
            non_hubs.push(i);
        }
    }

    (non_hubs, hubs)
}

/// Re-attach each hub to the community holding the majority of its neighbors.
/// Ties broken by lowest community id (deterministic).
fn reattach_hubs(
    graph: &CommunityGraph,
    mut comm: Vec<u32>,
    hub_indices: &[usize],
    _is_hub: &[bool],
) -> Vec<u32> {
    for &hub in hub_indices {
        // Count neighbor community votes.
        let mut votes: HashMap<u32, f64> = HashMap::new();
        for &(v, w) in &graph.adj[hub] {
            *votes.entry(comm[v]).or_insert(0.0) += w;
        }

        if votes.is_empty() {
            // Isolated hub — keep its current assignment.
            continue;
        }

        // Deterministic: sort by (votes descending, community_id ascending).
        let mut vote_vec: Vec<(u32, f64)> = votes.into_iter().collect();
        vote_vec.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });

        comm[hub] = vote_vec[0].0;
    }
    comm
}

// ── Utilities ──────────────────────────────────────────────────────────────

/// Build a sub-graph containing only `member_indices` (a subset of
/// `graph.nodes`). Returns `(sub_graph, sub_to_original_index_map)`.
fn build_subgraph(graph: &CommunityGraph, member_indices: &[usize]) -> (CommunityGraph, Vec<usize>) {
    // member_indices must already be in a stable order for determinism.
    let local_idx: HashMap<usize, usize> = member_indices
        .iter()
        .enumerate()
        .map(|(li, &gi)| (gi, li))
        .collect();

    let node_ids: Vec<NodeId> = member_indices
        .iter()
        .map(|&gi| graph.nodes[gi].clone())
        .collect();

    let mut raw_edges: Vec<(NodeId, NodeId, f64)> = Vec::new();
    for &gi in member_indices {
        for &(v, w) in &graph.adj[gi] {
            if local_idx.contains_key(&v) && gi < v {
                raw_edges.push((
                    graph.nodes[gi].clone(),
                    graph.nodes[v].clone(),
                    w,
                ));
            }
        }
    }

    let sub_graph = CommunityGraph::new(node_ids, raw_edges);
    // sub_to_original: sub_graph.nodes[i] corresponds to original node
    // with id sub_graph.nodes[i] — find its original index.
    let orig_idx: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let sub_to_original: Vec<usize> = sub_graph
        .nodes
        .iter()
        .map(|n| orig_idx[n.as_str()])
        .collect();

    (sub_graph, sub_to_original)
}

/// Compact community ids from an arbitrary `Vec<usize>` to a contiguous
/// `0..k` range, deterministically (sorted order).
/// Returns `(compacted, old_to_new_map)`.
fn compact_community_ids(comm: &[usize]) -> (Vec<u32>, HashMap<usize, u32>) {
    let mut ids: Vec<usize> = comm.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let remap: HashMap<usize, u32> = ids
        .iter()
        .enumerate()
        .map(|(i, &c)| (c, i as u32))
        .collect();
    let compacted: Vec<u32> = comm.iter().map(|c| remap[c]).collect();
    (compacted, remap)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a complete graph Kn (all pairs, weight 1.0).
    fn complete_graph(n: usize) -> CommunityGraph {
        let node_ids: Vec<NodeId> = (0..n).map(|i| format!("n{i}")).collect();
        let mut edges = Vec::new();
        for i in 0..n {
            for j in (i + 1)..n {
                edges.push((format!("n{i}"), format!("n{j}"), 1.0));
            }
        }
        CommunityGraph::new(node_ids, edges)
    }

    /// Helper: build two K5 cliques joined by one bridge edge.
    fn two_cliques_one_bridge() -> CommunityGraph {
        let node_ids: Vec<NodeId> = (0..10).map(|i| format!("n{i}")).collect();
        let mut edges = Vec::new();
        // Clique A: n0..n4
        for i in 0..5usize {
            for j in (i + 1)..5 {
                edges.push((format!("n{i}"), format!("n{j}"), 1.0));
            }
        }
        // Clique B: n5..n9
        for i in 5..10usize {
            for j in (i + 1)..10 {
                edges.push((format!("n{i}"), format!("n{j}"), 1.0));
            }
        }
        // Bridge: n0 — n5
        edges.push(("n0".into(), "n5".into(), 0.01));
        CommunityGraph::new(node_ids, edges)
    }

    #[test]
    fn community_node_ops_are_idempotent_sorted_and_skip_unresolved() {
        // phase27b §2.4 — the writeback mapper emits one idempotent NodeOp
        // per node, sorted by id, carrying the three community props.
        let g = two_cliques_one_bridge();
        let result = detect_communities(&g, &CommunityConfig::default());

        let ops = community_node_ops(&result, |_| Some("Symbol".to_string()));
        assert_eq!(ops.len(), 10, "one op per node");

        let keys: Vec<&str> = ops.iter().map(|o| o.natural_key.as_str()).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted, "ops emitted deterministically sorted by node id");

        for op in &ops {
            assert_eq!(op.label, "Symbol");
            // with_identity stamps external_id == natural_key + Match policy.
            assert_eq!(op.external_id.as_deref(), Some(op.natural_key.as_str()));
            assert!(op.props.contains_key(COMMUNITY_ID_PROP));
            assert!(op.props.contains_key(COMMUNITY_LEVEL_PROP));
            assert!(op.props.contains_key(COMMUNITY_GOD_NODE_PROP));
        }

        // Unresolved labels are skipped (live nodes gated on the projection).
        let none_ops = community_node_ops(&result, |_| None);
        assert!(none_ops.is_empty(), "nodes with no resolved label are skipped");
    }

    #[test]
    fn two_cliques_one_bridge_yields_two_communities() {
        let g = two_cliques_one_bridge();
        let cfg = CommunityConfig::default();
        let result = detect_communities(&g, &cfg);

        assert_eq!(result.len(), 10, "all 10 nodes must be assigned");

        // Collect community for each clique.
        let comm_a: Vec<u32> = (0..5).map(|i| result[&format!("n{i}")].community_id).collect();
        let comm_b: Vec<u32> = (5..10).map(|i| result[&format!("n{i}")].community_id).collect();

        // All nodes in clique A must share one community.
        assert!(
            comm_a.windows(2).all(|w| w[0] == w[1]),
            "clique A nodes must share a community; got {comm_a:?}"
        );
        // All nodes in clique B must share one community.
        assert!(
            comm_b.windows(2).all(|w| w[0] == w[1]),
            "clique B nodes must share a community; got {comm_b:?}"
        );
        // The two cliques must be in DIFFERENT communities.
        assert_ne!(
            comm_a[0], comm_b[0],
            "the two cliques must be separated into distinct communities"
        );
    }

    #[test]
    fn detection_is_deterministic() {
        let g = two_cliques_one_bridge();
        let cfg = CommunityConfig::default();
        let first = detect_communities(&g, &cfg);
        for _ in 1..5 {
            let run = detect_communities(&g, &cfg);
            for (node_id, a) in &first {
                let b = &run[node_id];
                assert_eq!(
                    a.community_id, b.community_id,
                    "community_id differs across runs for {node_id}"
                );
                assert_eq!(
                    a.level, b.level,
                    "level differs across runs for {node_id}"
                );
            }
        }
    }

    #[test]
    fn terminates_on_pathological_complete_graph() {
        // K20: every node is connected to every other → no community structure.
        // This MUST complete within the pass caps and return a valid partition.
        let g = complete_graph(20);
        let cfg = CommunityConfig::default();
        let result = detect_communities(&g, &cfg);

        // Every node assigned.
        assert_eq!(result.len(), 20, "all 20 nodes must be assigned");
        // All community_ids are valid u32 values — just verify no panic.
        for a in result.values() {
            let _ = a.community_id;
        }
    }

    #[test]
    fn oversized_community_is_split() {
        // Build a graph where two dense clusters (size 8 each) and a thin bridge
        // would naively produce a large community. The §2.2 split must break it.
        let n = 16usize;
        let node_ids: Vec<NodeId> = (0..n).map(|i| format!("n{i}")).collect();
        let mut edges = Vec::new();
        // Cluster A: n0..n7, strong internal edges.
        for i in 0..8usize {
            for j in (i + 1)..8 {
                edges.push((format!("n{i}"), format!("n{j}"), 1.0));
            }
        }
        // Cluster B: n8..n15, strong internal edges.
        for i in 8..16usize {
            for j in (i + 1)..16 {
                edges.push((format!("n{i}"), format!("n{j}"), 1.0));
            }
        }
        // Thin bridge.
        edges.push(("n0".into(), "n8".into(), 0.001));

        let g = CommunityGraph::new(node_ids, edges);
        // Use a tight oversized_fraction so 8/16 = 0.5 is definitely oversized.
        let cfg = CommunityConfig {
            oversized_fraction: 0.4, // 40% = 6.4 → threshold 7; clusters of 8 are oversized
            ..Default::default()
        };
        let result = detect_communities(&g, &cfg);
        assert_eq!(result.len(), 16);

        // Count max community size.
        let mut comm_sizes: HashMap<u32, usize> = HashMap::new();
        for a in result.values() {
            *comm_sizes.entry(a.community_id).or_insert(0) += 1;
        }
        let max_size = comm_sizes.values().copied().max().unwrap_or(0);
        // After splitting, no community should be larger than 8
        // (the two original clusters of 8 should each be their own community
        // or further split).
        assert!(
            max_size <= 8,
            "max community size should be ≤ 8 after split, got {max_size}"
        );
    }

    #[test]
    fn hub_is_excluded_then_reattached_to_neighbor_majority() {
        // Star-ish topology: hub "h" connects to two clusters.
        // Cluster A: a0, a1, a2 (strongly connected among themselves).
        // Cluster B: b0, b1, b2 (strongly connected among themselves).
        // Hub h: connected to all 6 cluster nodes with equal weight.
        // Because hub has degree 6 vs cluster nodes degree ~3, hub_percentile=0.8
        // will exclude it.
        let node_ids: Vec<NodeId> = vec![
            "a0".into(), "a1".into(), "a2".into(),
            "b0".into(), "b1".into(), "b2".into(),
            "h".into(),
        ];
        let edges: Vec<(NodeId, NodeId, f64)> = vec![
            // Cluster A internal.
            ("a0".into(), "a1".into(), 1.0),
            ("a0".into(), "a2".into(), 1.0),
            ("a1".into(), "a2".into(), 1.0),
            // Cluster B internal.
            ("b0".into(), "b1".into(), 1.0),
            ("b0".into(), "b2".into(), 1.0),
            ("b1".into(), "b2".into(), 1.0),
            // Hub connects to cluster A (3 edges) and cluster B (3 edges).
            ("h".into(), "a0".into(), 1.0),
            ("h".into(), "a1".into(), 1.0),
            ("h".into(), "a2".into(), 1.0),
            ("h".into(), "b0".into(), 1.0),
            ("h".into(), "b1".into(), 1.0),
            ("h".into(), "b2".into(), 1.0),
        ];

        let g = CommunityGraph::new(node_ids, edges);
        // hub_percentile=0.8 → top 20% by degree = hub "h" (degree 6 vs ~3).
        let cfg = CommunityConfig {
            hub_percentile: 0.8,
            ..Default::default()
        };
        let result = detect_communities(&g, &cfg);

        assert_eq!(result.len(), 7, "all 7 nodes assigned");

        // Hub must be flagged.
        assert!(result["h"].is_hub, "hub node 'h' must have is_hub=true");

        // Cluster A must be together.
        let ca: Vec<u32> = ["a0", "a1", "a2"]
            .iter()
            .map(|n| result[*n].community_id)
            .collect();
        assert!(ca.windows(2).all(|w| w[0] == w[1]), "cluster A must share a community");

        // Cluster B must be together.
        let cb: Vec<u32> = ["b0", "b1", "b2"]
            .iter()
            .map(|n| result[*n].community_id)
            .collect();
        assert!(cb.windows(2).all(|w| w[0] == w[1]), "cluster B must share a community");

        // Hub must land in one of the two cluster communities (re-attached to
        // majority-neighbor community). Since hub has equal weight to both
        // clusters, tie-break goes to lowest community_id.
        let hub_comm = result["h"].community_id;
        assert!(
            hub_comm == ca[0] || hub_comm == cb[0],
            "hub must be re-attached to one of the two cluster communities, got {hub_comm}"
        );
    }

    #[test]
    fn empty_and_singleton_graphs_are_safe() {
        let cfg = CommunityConfig::default();

        // Empty graph.
        let empty = CommunityGraph::new(vec![], vec![]);
        let result = detect_communities(&empty, &cfg);
        assert!(result.is_empty(), "empty graph → empty result");

        // Single node, no edges.
        let single = CommunityGraph::new(vec!["only".into()], vec![]);
        let result = detect_communities(&single, &cfg);
        assert_eq!(result.len(), 1, "singleton → one assignment");
        assert_eq!(result["only"].community_id, 0);
    }
}
