//! RES-2659: Graph algorithm builtins.
//!
//! All functions use an adjacency-list representation:
//!
//! * **Unweighted graph**: `Array<Array<int>>` where `adj[i]` is the list
//!   of neighbour indices reachable from node `i`. Indices must be in
//!   `[0, len(adj))`.
//!
//! * **Weighted graph** (`dijkstra`): `Array<Array<Array<int>>>` where
//!   `adj[i]` is a list of `[neighbour, weight]` pairs. Weights must be
//!   non-negative integers.
//!
//! Builtins:
//! * `graph_bfs(adj, start)` — BFS visit order (Array<int>)
//! * `graph_dfs(adj, start)` — DFS visit order (Array<int>)
//! * `graph_has_path(adj, src, dst)` — BFS reachability (bool)
//! * `graph_topological_sort(adj)` — Kahn's algorithm (Array<int> or error)
//! * `graph_connected_components(adj)` — component label per node (Array<int>)
//! * `graph_dijkstra(adj, start)` — shortest distances (Array<int>, -1=unreachable)

use crate::Value;
use std::collections::VecDeque;

type WeightedAdj = Vec<Vec<(usize, i64)>>;

type RResult<T> = Result<T, String>;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Extract adjacency list as `Vec<Vec<usize>>` from `Array<Array<int>>`.
fn extract_adj(name: &str, v: &Value) -> RResult<Vec<Vec<usize>>> {
    let outer = match v {
        Value::Array(a) => a,
        other => {
            return Err(format!(
                "{name}: first argument must be Array<Array<int>> (adjacency list), got {other}"
            ));
        }
    };
    let n = outer.len();
    let mut adj: Vec<Vec<usize>> = Vec::with_capacity(n);
    for (i, row) in outer.iter().enumerate() {
        let inner = match row {
            Value::Array(a) => a,
            other => return Err(format!("{name}: adj[{i}] must be Array<int>, got {other}")),
        };
        let mut neighbours = Vec::with_capacity(inner.len());
        for (j, nb) in inner.iter().enumerate() {
            let idx = match nb {
                Value::Int(k) => *k,
                other => return Err(format!("{name}: adj[{i}][{j}] must be int, got {other}")),
            };
            if idx < 0 || idx as usize >= n {
                return Err(format!(
                    "{name}: neighbour index {idx} at adj[{i}][{j}] out of range [0, {n})"
                ));
            }
            neighbours.push(idx as usize);
        }
        adj.push(neighbours);
    }
    Ok(adj)
}

/// Extract starting node index.
fn extract_node(name: &str, v: &Value, n: usize) -> RResult<usize> {
    match v {
        Value::Int(k) => {
            if *k < 0 || *k as usize >= n {
                Err(format!("{name}: start node {k} out of range [0, {n})"))
            } else {
                Ok(*k as usize)
            }
        }
        other => Err(format!("{name}: node index must be int, got {other}")),
    }
}

// ── graph_bfs ────────────────────────────────────────────────────────────────

/// `graph_bfs(adj, start) -> Array<int>`
///
/// Returns the order in which nodes are visited in breadth-first search
/// starting from `start`. Unreachable nodes are not included.
///
/// ```text
/// let adj = [[1, 2], [3], [3], []];   // 0→1,2  1→3  2→3
/// graph_bfs(adj, 0)  // == [0, 1, 2, 3]
/// ```
pub(crate) fn builtin_graph_bfs(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val, start_val] => {
            let adj = extract_adj("graph_bfs", adj_val)?;
            let n = adj.len();
            let start = extract_node("graph_bfs", start_val, n)?;

            let mut visited = vec![false; n];
            let mut order = Vec::with_capacity(n);
            // RES-1944: queue holds at most n nodes (each enqueued at
            // most once); pre-size to skip the default 0→4→8→… grow.
            let mut queue = VecDeque::with_capacity(n);

            visited[start] = true;
            queue.push_back(start);

            while let Some(node) = queue.pop_front() {
                order.push(Value::Int(node as i64));
                for &nb in &adj[node] {
                    if !visited[nb] {
                        visited[nb] = true;
                        queue.push_back(nb);
                    }
                }
            }
            Ok(Value::Array(order))
        }
        _ => Err(format!(
            "graph_bfs: expected 2 arguments (adj, start), got {}",
            args.len()
        )),
    }
}

// ── graph_dfs ────────────────────────────────────────────────────────────────

/// `graph_dfs(adj, start) -> Array<int>`
///
/// Returns the order in which nodes are first visited in depth-first search
/// starting from `start`. Unreachable nodes are not included.
///
/// ```text
/// let adj = [[1, 2], [3], [3], []];
/// graph_dfs(adj, 0)  // == [0, 1, 3, 2]
/// ```
pub(crate) fn builtin_graph_dfs(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val, start_val] => {
            let adj = extract_adj("graph_dfs", adj_val)?;
            let n = adj.len();
            let start = extract_node("graph_dfs", start_val, n)?;

            let mut visited = vec![false; n];
            let mut order = Vec::with_capacity(n);
            dfs_visit(&adj, start, &mut visited, &mut order);
            Ok(Value::Array(order))
        }
        _ => Err(format!(
            "graph_dfs: expected 2 arguments (adj, start), got {}",
            args.len()
        )),
    }
}

fn dfs_visit(adj: &[Vec<usize>], node: usize, visited: &mut Vec<bool>, order: &mut Vec<Value>) {
    visited[node] = true;
    order.push(Value::Int(node as i64));
    for &nb in &adj[node] {
        if !visited[nb] {
            dfs_visit(adj, nb, visited, order);
        }
    }
}

// ── graph_has_path ───────────────────────────────────────────────────────────

/// `graph_has_path(adj, src, dst) -> bool`
///
/// Returns `true` if there is a directed path from `src` to `dst`.
///
/// ```text
/// let adj = [[1], [2], [], [0]];
/// graph_has_path(adj, 0, 2)  // == true
/// graph_has_path(adj, 0, 3)  // == false
/// ```
pub(crate) fn builtin_graph_has_path(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val, src_val, dst_val] => {
            let adj = extract_adj("graph_has_path", adj_val)?;
            let n = adj.len();
            let src = extract_node("graph_has_path", src_val, n)?;
            let dst = extract_node("graph_has_path", dst_val, n)?;

            if src == dst {
                return Ok(Value::Bool(true));
            }

            let mut visited = vec![false; n];
            // RES-1944: queue holds at most n nodes (each enqueued at
            // most once); pre-size to skip the default 0→4→8→… grow.
            let mut queue = VecDeque::with_capacity(n);
            visited[src] = true;
            queue.push_back(src);

            while let Some(node) = queue.pop_front() {
                for &nb in &adj[node] {
                    if nb == dst {
                        return Ok(Value::Bool(true));
                    }
                    if !visited[nb] {
                        visited[nb] = true;
                        queue.push_back(nb);
                    }
                }
            }
            Ok(Value::Bool(false))
        }
        _ => Err(format!(
            "graph_has_path: expected 3 arguments (adj, src, dst), got {}",
            args.len()
        )),
    }
}

// ── graph_topological_sort ───────────────────────────────────────────────────

/// `graph_topological_sort(adj) -> Array<int>`
///
/// Returns a topological ordering of nodes using Kahn's algorithm.
/// Returns an error if the graph has a cycle.
///
/// ```text
/// let adj = [[], [0], [0], [1, 2]];  // 3→1,2  1→0  2→0
/// graph_topological_sort(adj)  // == [3, 1, 2, 0] (or [3, 2, 1, 0])
/// ```
pub(crate) fn builtin_graph_topological_sort(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val] => {
            let adj = extract_adj("graph_topological_sort", adj_val)?;
            let n = adj.len();

            let mut in_degree = vec![0usize; n];
            for neighbours in &adj {
                for &nb in neighbours {
                    in_degree[nb] += 1;
                }
            }

            let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
            let mut order = Vec::with_capacity(n);

            while let Some(node) = queue.pop_front() {
                order.push(Value::Int(node as i64));
                for &nb in &adj[node] {
                    in_degree[nb] -= 1;
                    if in_degree[nb] == 0 {
                        queue.push_back(nb);
                    }
                }
            }

            if order.len() != n {
                return Err(format!(
                    "graph_topological_sort: graph contains a cycle ({} of {n} nodes processed)",
                    order.len()
                ));
            }
            Ok(Value::Array(order))
        }
        _ => Err(format!(
            "graph_topological_sort: expected 1 argument (adj), got {}",
            args.len()
        )),
    }
}

// ── graph_connected_components ───────────────────────────────────────────────

/// `graph_connected_components(adj) -> Array<int>`
///
/// Treats the graph as **undirected** and labels each node with its
/// component index (0-based). Returns an array of length `n` where
/// `result[i]` is the component id of node `i`.
///
/// ```text
/// let adj = [[1], [0], [3], [2], []];
/// graph_connected_components(adj)  // == [0, 0, 1, 1, 2]
/// ```
pub(crate) fn builtin_graph_connected_components(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val] => {
            let adj = extract_adj("graph_connected_components", adj_val)?;
            let n = adj.len();

            let mut component = vec![usize::MAX; n];
            let mut comp_id = 0usize;

            // RES-1944: lift queue out of the per-component loop and
            // clear() between components. `VecDeque::clear` retains
            // capacity, so subsequent components reuse the buffer.
            // Pre-size to n — each component's queue holds at most n
            // entries (a component cannot exceed the whole graph).
            let mut queue: VecDeque<usize> = VecDeque::with_capacity(n);
            for start in 0..n {
                if component[start] != usize::MAX {
                    continue;
                }
                // BFS treating edges as undirected
                queue.clear();
                queue.push_back(start);
                component[start] = comp_id;
                while let Some(node) = queue.pop_front() {
                    // Forward edges
                    for &nb in &adj[node] {
                        if component[nb] == usize::MAX {
                            component[nb] = comp_id;
                            queue.push_back(nb);
                        }
                    }
                    // Reverse edges: any node whose adj list contains `node`
                    for (other, neighbours) in adj.iter().enumerate() {
                        if component[other] == usize::MAX && neighbours.contains(&node) {
                            component[other] = comp_id;
                            queue.push_back(other);
                        }
                    }
                }
                comp_id += 1;
            }

            let result: Vec<Value> = component.iter().map(|&c| Value::Int(c as i64)).collect();
            Ok(Value::Array(result))
        }
        _ => Err(format!(
            "graph_connected_components: expected 1 argument (adj), got {}",
            args.len()
        )),
    }
}

// ── graph_dijkstra ───────────────────────────────────────────────────────────

/// `graph_dijkstra(adj, start) -> Array<int>`
///
/// Dijkstra's shortest-path algorithm on a weighted directed graph.
/// `adj` is `Array<Array<Array<int>>>` where `adj[i]` is a list of
/// `[neighbour, weight]` pairs. Weights must be non-negative.
///
/// Returns an array of length `n` where `result[i]` is the shortest
/// distance from `start` to node `i`, or `-1` if unreachable.
///
/// ```text
/// let adj = [[[1, 4], [2, 1]], [[3, 1]], [[1, 2], [3, 5]], [[]]];
/// graph_dijkstra(adj, 0)  // == [0, 3, 1, 4]
/// ```
pub(crate) fn builtin_graph_dijkstra(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val, start_val] => {
            let (adj_w, n) = extract_weighted_adj("graph_dijkstra", adj_val)?;
            let start = extract_node("graph_dijkstra", start_val, n)?;

            let mut dist = vec![i64::MAX; n];
            dist[start] = 0;

            // Simple priority queue via sorted map: dist → list of nodes
            // Using a BTreeMap of (dist, node) for a clean O(n² log n) impl
            // without pulling in a binary heap dep beyond std.
            let mut pq: std::collections::BTreeMap<(i64, usize), ()> =
                std::collections::BTreeMap::new();
            pq.insert((0, start), ());

            while let Some((&(d, u), _)) = pq.iter().next() {
                pq.remove(&(d, u));
                if d > dist[u] {
                    continue;
                }
                for &(v, w) in &adj_w[u] {
                    let nd = dist[u].saturating_add(w);
                    if nd < dist[v] {
                        dist[v] = nd;
                        pq.insert((nd, v), ());
                    }
                }
            }

            let result: Vec<Value> = dist
                .iter()
                .map(|&d| {
                    if d == i64::MAX {
                        Value::Int(-1)
                    } else {
                        Value::Int(d)
                    }
                })
                .collect();
            Ok(Value::Array(result))
        }
        _ => Err(format!(
            "graph_dijkstra: expected 2 arguments (adj, start), got {}",
            args.len()
        )),
    }
}

/// Extract weighted adjacency list: `Array<Array<Array<int>>>` →
/// `Vec<Vec<(usize, i64)>>` (neighbour, weight).
fn extract_weighted_adj(name: &str, v: &Value) -> RResult<(WeightedAdj, usize)> {
    let outer = match v {
        Value::Array(a) => a,
        other => {
            return Err(format!(
                "{name}: adjacency list must be Array<Array<Array<int>>>, got {other}"
            ));
        }
    };
    let n = outer.len();
    let mut adj: Vec<Vec<(usize, i64)>> = Vec::with_capacity(n);

    for (i, row) in outer.iter().enumerate() {
        let edges = match row {
            Value::Array(a) => a,
            other => {
                return Err(format!(
                    "{name}: adj[{i}] must be Array<Array<int>>, got {other}"
                ));
            }
        };
        let mut node_edges = Vec::with_capacity(edges.len());
        for (j, edge) in edges.iter().enumerate() {
            match edge {
                Value::Array(pair) if pair.len() == 2 => {
                    let nb = match &pair[0] {
                        Value::Int(k) => *k,
                        other => {
                            return Err(format!(
                                "{name}: adj[{i}][{j}][0] (neighbour) must be int, got {other}"
                            ));
                        }
                    };
                    let w = match &pair[1] {
                        Value::Int(k) => *k,
                        other => {
                            return Err(format!(
                                "{name}: adj[{i}][{j}][1] (weight) must be int, got {other}"
                            ));
                        }
                    };
                    if nb < 0 || nb as usize >= n {
                        return Err(format!(
                            "{name}: neighbour {nb} at adj[{i}][{j}] out of range [0, {n})"
                        ));
                    }
                    if w < 0 {
                        return Err(format!(
                            "{name}: weight {w} at adj[{i}][{j}] must be non-negative"
                        ));
                    }
                    node_edges.push((nb as usize, w));
                }
                Value::Array(pair) => {
                    return Err(format!(
                        "{name}: adj[{i}][{j}] must be [neighbour, weight] pair (length 2), got length {}",
                        pair.len()
                    ));
                }
                other => {
                    return Err(format!(
                        "{name}: adj[{i}][{j}] must be Array [neighbour, weight], got {other}"
                    ));
                }
            }
        }
        adj.push(node_edges);
    }
    Ok((adj, n))
}

// ── graph_num_components ─────────────────────────────────────────────────────

/// `graph_num_components(adj) -> int`
///
/// Returns the number of connected components (treating edges as undirected).
///
/// ```text
/// let adj = [[1], [0], [3], [2], []];
/// graph_num_components(adj)  // == 3
/// ```
pub(crate) fn builtin_graph_num_components(args: &[Value]) -> RResult<Value> {
    match builtin_graph_connected_components(args)? {
        Value::Array(labels) => {
            let max = labels
                .iter()
                .filter_map(|v| {
                    if let Value::Int(i) = v {
                        Some(*i)
                    } else {
                        None
                    }
                })
                .max()
                .map(|m| m + 1)
                .unwrap_or(0);
            Ok(Value::Int(max))
        }
        other => Err(format!("graph_num_components: internal error, got {other}")),
    }
}

// ── graph_in_degrees / graph_out_degrees ─────────────────────────────────────

/// `graph_out_degrees(adj) -> Array<int>`
///
/// Returns the out-degree of each node.
pub(crate) fn builtin_graph_out_degrees(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val] => {
            let adj = extract_adj("graph_out_degrees", adj_val)?;
            let result: Vec<Value> = adj
                .iter()
                .map(|neighbours| Value::Int(neighbours.len() as i64))
                .collect();
            Ok(Value::Array(result))
        }
        _ => Err(format!(
            "graph_out_degrees: expected 1 argument (adj), got {}",
            args.len()
        )),
    }
}

/// `graph_in_degrees(adj) -> Array<int>`
///
/// Returns the in-degree of each node.
pub(crate) fn builtin_graph_in_degrees(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val] => {
            let adj = extract_adj("graph_in_degrees", adj_val)?;
            let n = adj.len();
            let mut in_deg = vec![0i64; n];
            for neighbours in &adj {
                for &nb in neighbours {
                    in_deg[nb] += 1;
                }
            }
            let result: Vec<Value> = in_deg.iter().map(|&d| Value::Int(d)).collect();
            Ok(Value::Array(result))
        }
        _ => Err(format!(
            "graph_in_degrees: expected 1 argument (adj), got {}",
            args.len()
        )),
    }
}

// ── graph_reverse ─────────────────────────────────────────────────────────────

/// `graph_reverse(adj) -> Array<Array<int>>`
///
/// Returns the transpose (reverse) of the directed graph.
pub(crate) fn builtin_graph_reverse(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val] => {
            let adj = extract_adj("graph_reverse", adj_val)?;
            let n = adj.len();
            // RES-1944: two-pass — first count in-degrees, then
            // allocate each inner Vec with its exact capacity. The
            // single-pass `vec![Vec::new(); n]` shape forced every
            // reverse-edge insert to grow through the default
            // 0→4→8→… doubling chain per inner Vec.
            let mut in_deg = vec![0usize; n];
            for neighbours in &adj {
                for &v in neighbours {
                    in_deg[v] += 1;
                }
            }
            let mut rev: Vec<Vec<i64>> = in_deg.iter().map(|&d| Vec::with_capacity(d)).collect();
            for (u, neighbours) in adj.iter().enumerate() {
                for &v in neighbours {
                    rev[v].push(u as i64);
                }
            }
            let result: Vec<Value> = rev
                .into_iter()
                .map(|nbrs| Value::Array(nbrs.into_iter().map(Value::Int).collect()))
                .collect();
            Ok(Value::Array(result))
        }
        _ => Err(format!(
            "graph_reverse: expected 1 argument (adj), got {}",
            args.len()
        )),
    }
}

// ── graph_is_dag ──────────────────────────────────────────────────────────────

/// `graph_is_dag(adj) -> bool`
///
/// Returns `true` if the directed graph is a DAG (no cycles).
pub(crate) fn builtin_graph_is_dag(args: &[Value]) -> RResult<Value> {
    match args {
        [adj_val] => {
            let adj = extract_adj("graph_is_dag", adj_val)?;
            let n = adj.len();
            let mut in_degree = vec![0usize; n];
            for neighbours in &adj {
                for &nb in neighbours {
                    in_degree[nb] += 1;
                }
            }
            let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
            let mut processed = 0;
            while let Some(node) = queue.pop_front() {
                processed += 1;
                for &nb in &adj[node] {
                    in_degree[nb] -= 1;
                    if in_degree[nb] == 0 {
                        queue.push_back(nb);
                    }
                }
            }
            Ok(Value::Bool(processed == n))
        }
        _ => Err(format!(
            "graph_is_dag: expected 1 argument (adj), got {}",
            args.len()
        )),
    }
}

// ── graph_reachable ───────────────────────────────────────────────────────────

/// `graph_reachable(adj, start) -> Array<int>`
///
/// Returns all nodes reachable from `start` (including `start` itself),
/// in BFS order.
pub(crate) fn builtin_graph_reachable(args: &[Value]) -> RResult<Value> {
    // Delegates to BFS — same result.
    builtin_graph_bfs(args)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    // ── BFS ──────────────────────────────────────────────────────────────────

    #[test]
    fn bfs_linear_chain() {
        // 0 → 1 → 2 → 3
        let r = run(r#"let adj = [[1], [2], [3], []];
println(graph_bfs(adj, 0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[0, 1, 2, 3]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn bfs_diamond() {
        // 0→1, 0→2, 1→3, 2→3
        let r = run(r#"let adj = [[1, 2], [3], [3], []];
println(graph_bfs(adj, 0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[0, 1, 2, 3]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn bfs_single_node() {
        let r = run(r#"let adj = [[]];
println(graph_bfs(adj, 0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[0]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn bfs_disconnected_subgraph() {
        // 0→1, 2→3 — BFS from 0 should NOT visit 2 or 3
        let r = run(r#"let adj = [[1], [], [3], []];
let visited = graph_bfs(adj, 0);
println(len(visited));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('2'), "stdout: {}", r.stdout);
    }

    // ── DFS ──────────────────────────────────────────────────────────────────

    #[test]
    fn dfs_linear_chain() {
        let r = run(r#"let adj = [[1], [2], [3], []];
println(graph_dfs(adj, 0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[0, 1, 2, 3]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn dfs_visits_all_reachable() {
        let r = run(r#"let adj = [[1, 2], [3], [3], []];
let order = graph_dfs(adj, 0);
println(len(order));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('4'), "stdout: {}", r.stdout);
    }

    // ── has_path ─────────────────────────────────────────────────────────────

    #[test]
    fn has_path_direct() {
        let r = run(r#"let adj = [[1, 2], [3], [3], []];
println(graph_has_path(adj, 0, 3));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn has_path_no_path() {
        let r = run(r#"let adj = [[1], [], [3], []];
println(graph_has_path(adj, 0, 2));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("false"), "stdout: {}", r.stdout);
    }

    #[test]
    fn has_path_self() {
        let r = run(r#"let adj = [[1], []];
println(graph_has_path(adj, 0, 0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    // ── topological_sort ─────────────────────────────────────────────────────

    #[test]
    fn topo_sort_simple_dag() {
        // 3→1, 3→2, 1→0, 2→0  => 0 has no outgoing, 3 has in-degree 0
        let r = run(r#"let adj = [[], [0], [0], [1, 2]];
let order = graph_topological_sort(adj);
println(len(order));
println(order[0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "4", "expected 4 nodes");
        assert_eq!(lines[1], "3", "node 3 must come first");
    }

    #[test]
    fn topo_sort_linear() {
        let r = run(r#"let adj = [[1], [2], [3], []];
println(graph_topological_sort(adj));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[0, 1, 2, 3]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn topo_sort_cycle_errors() {
        let r = run(r#"let adj = [[1], [2], [0]];
graph_topological_sort(adj);"#);
        assert!(!r.ok, "expected error for cycle");
    }

    // ── connected_components ─────────────────────────────────────────────────

    #[test]
    fn connected_components_two_components() {
        // 0-1 edge, 2-3 edge, 4 isolated
        let r = run(r#"let adj = [[1], [0], [3], [2], []];
let labels = graph_connected_components(adj);
println(len(labels));
println(labels[0] == labels[1]);
println(labels[2] == labels[3]);
println(labels[0] == labels[2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "5");
        assert_eq!(lines[1], "true", "0 and 1 same component");
        assert_eq!(lines[2], "true", "2 and 3 same component");
        assert_eq!(lines[3], "false", "0 and 2 different components");
    }

    #[test]
    fn connected_components_single_component() {
        let r = run(r#"let adj = [[1, 2], [0, 2], [0, 1]];
let labels = graph_connected_components(adj);
println(labels[0] == labels[1]);
println(labels[1] == labels[2]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true\ntrue"), "stdout: {}", r.stdout);
    }

    // ── num_components ───────────────────────────────────────────────────────

    #[test]
    fn num_components_three() {
        let r = run(r#"let adj = [[1], [0], [3], [2], []];
println(graph_num_components(adj));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('3'), "stdout: {}", r.stdout);
    }

    // ── dijkstra ─────────────────────────────────────────────────────────────

    #[test]
    fn dijkstra_simple() {
        // 0→1 (w=4), 0→2 (w=1), 2→1 (w=2), 1→3 (w=1)
        // dist: 0=0, 1=3(via 2), 2=1, 3=4
        let r = run(r#"let adj = [[[1, 4], [2, 1]], [[3, 1]], [[1, 2]], []];
println(graph_dijkstra(adj, 0));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[0, 3, 1, 4]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn dijkstra_unreachable() {
        let r = run(r#"let adj = [[[1, 1]], [], [[3, 2]], []];
let d = graph_dijkstra(adj, 0);
println(d[2]);
println(d[3]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "-1", "node 2 unreachable");
        assert_eq!(lines[1], "-1", "node 3 unreachable");
    }

    #[test]
    fn dijkstra_start_to_self() {
        let r = run(r#"let adj = [[[1, 5]], []];
let d = graph_dijkstra(adj, 0);
println(d[0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    // ── in/out degrees ───────────────────────────────────────────────────────

    #[test]
    fn out_degrees() {
        let r = run(r#"let adj = [[1, 2], [2], []];
println(graph_out_degrees(adj));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[2, 1, 0]"), "stdout: {}", r.stdout);
    }

    #[test]
    fn in_degrees() {
        let r = run(r#"let adj = [[1, 2], [2], []];
println(graph_in_degrees(adj));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("[0, 1, 2]"), "stdout: {}", r.stdout);
    }

    // ── reverse ──────────────────────────────────────────────────────────────

    #[test]
    fn graph_reverse_simple() {
        // 0→1, 1→2  reversed: 1→0, 2→1
        let r = run(r#"let adj = [[1], [2], []];
let rev = graph_reverse(adj);
println(len(rev[0]));
println(rev[1][0]);
println(rev[2][0]);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        assert_eq!(lines[0], "0", "node 0 has no incoming after reverse");
        assert_eq!(lines[1], "0", "node 1's incoming was 0");
        assert_eq!(lines[2], "1", "node 2's incoming was 1");
    }

    // ── is_dag ───────────────────────────────────────────────────────────────

    #[test]
    fn is_dag_true() {
        let r = run(r#"let adj = [[1], [2], []];
println(graph_is_dag(adj));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn is_dag_false_cycle() {
        let r = run(r#"let adj = [[1], [2], [0]];
println(graph_is_dag(adj));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("false"), "stdout: {}", r.stdout);
    }

    // ── reachable ─────────────────────────────────────────────────────────────

    #[test]
    fn reachable_set() {
        let r = run(r#"let adj = [[1, 2], [3], [3], []];
let r = graph_reachable(adj, 0);
println(len(r));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('4'), "stdout: {}", r.stdout);
    }

    // ── error cases ──────────────────────────────────────────────────────────

    #[test]
    fn bfs_out_of_range_start_errors() {
        let r = run(r#"let adj = [[1], []];
graph_bfs(adj, 5);"#);
        assert!(!r.ok, "expected error for out-of-range start");
    }

    #[test]
    fn dijkstra_negative_weight_errors() {
        let r = run(r#"let adj = [[[1, 0 - 1]], []];
graph_dijkstra(adj, 0);"#);
        assert!(!r.ok, "expected error for negative weight");
    }
}
