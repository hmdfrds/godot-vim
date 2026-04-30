//! Branching undo tree with periodic text snapshots and LRU pruning.
//!
//! This is the shell-side undo tree that shadows Godot's built-in undo system.
//! Godot's `CodeEdit` has a linear undo stack, but Vim's undo model is a tree:
//! undoing to an older state and editing creates a sibling branch rather than
//! discarding the future. This module maintains that tree structure so
//! `:undotree` can display it, and periodic full-text snapshots enable fast
//! random access without replaying the entire edit history.

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;
use std::time::Instant;

use compact_str::CompactString;

/// Newtype over `u64` to prevent accidental confusion with cursor offsets,
/// snapshot intervals, or other numeric values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct UndoNodeId(u64);

impl UndoNodeId {
    #[cfg(test)]
    #[must_use]
    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for UndoNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A single node in the undo tree. Children represent branches created by
/// editing after undo -- unlike a linear stack, no timeline is ever discarded.
#[derive(Debug, Clone)]
pub(crate) struct UndoNode {
    #[allow(dead_code)] // only inspected in tests via HashMap lookup
    pub(crate) id: UndoNodeId,
    pub(crate) parent: Option<UndoNodeId>,
    /// Multiple children = branch point (undo + new edit created a sibling).
    pub(crate) children: Vec<UndoNodeId>,
    pub(crate) timestamp: Instant,
    pub(crate) description: CompactString,
    /// Full text snapshot for O(1) random access. Stored every
    /// `SNAPSHOT_INTERVAL` nodes; intermediate states are reconstructed
    /// by replaying edits from the nearest snapshot.
    pub(crate) text_snapshot: Option<Arc<str>>,
}

#[derive(Debug)]
pub(crate) struct UndoTree {
    nodes: HashMap<UndoNodeId, UndoNode>,
    current: UndoNodeId,
    next_id: u64,
    snapshot_interval: u64,
    max_nodes: usize,
}

const SNAPSHOT_INTERVAL: u64 = 50;
/// ~200 bytes/node (sans snapshots), so 10k nodes ~ 2 MB.
const MAX_NODES: usize = 10_000;

impl UndoTree {
    #[must_use]
    pub(crate) fn new(initial_text: &str) -> Self {
        let root_id = UndoNodeId(0);
        let root = UndoNode {
            id: root_id,
            parent: None,
            children: Vec::new(),
            timestamp: Instant::now(),
            description: CompactString::from("(initial)"),
            text_snapshot: Some(Arc::from(initial_text)),
        };
        let mut nodes = HashMap::new();
        nodes.insert(root_id, root);
        Self {
            nodes,
            current: root_id,
            next_id: 1,
            snapshot_interval: SNAPSHOT_INTERVAL,
            max_nodes: MAX_NODES,
        }
    }

    pub(crate) fn record_edit(&mut self, description: &str, text: &str) -> UndoNodeId {
        while self.nodes.len() >= self.max_nodes {
            if !self.prune_oldest_leaf() {
                break;
            }
        }

        let raw_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("undo node ID space exhausted (u64 overflow)");
        let id = UndoNodeId(raw_id);

        let text_snapshot = if raw_id.is_multiple_of(self.snapshot_interval) {
            Some(Arc::from(text))
        } else {
            None
        };

        let node = UndoNode {
            id,
            parent: Some(self.current),
            children: Vec::new(),
            timestamp: Instant::now(),
            description: CompactString::from(description),
            text_snapshot,
        };

        if let Some(parent) = self.nodes.get_mut(&self.current) {
            parent.children.push(id);
        }

        self.nodes.insert(id, node);
        self.current = id;
        id
    }

    #[cfg(test)]
    pub(crate) fn navigate_to(&mut self, target_id: UndoNodeId) -> bool {
        if self.nodes.contains_key(&target_id) {
            self.current = target_id;
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn current_id(&self) -> UndoNodeId {
        self.current
    }
    #[cfg(test)]
    #[must_use]
    pub(crate) fn node(&self, id: UndoNodeId) -> Option<&UndoNode> {
        self.nodes.get(&id)
    }
    #[cfg(test)]
    #[must_use]
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(crate) fn format_tree(&self) -> String {
        let mut out = String::from("Undo Tree:\n");
        self.format_node(UndoNodeId(0), &mut out);
        out
    }

    /// Root-to-current path; these nodes are protected from pruning.
    fn current_path_ids(&self) -> Vec<UndoNodeId> {
        let mut path = Vec::new();
        let mut id = self.current;
        loop {
            path.push(id);
            match self.nodes.get(&id).and_then(|n| n.parent) {
                Some(parent) => id = parent,
                None => break,
            }
        }
        path
    }

    /// Two-phase pruning: (1) remove oldest unprotected non-snapshot leaf,
    /// or (2) compress oldest single-child interior node by re-linking its
    /// child to its grandparent. Returns `false` when nothing can be pruned
    /// (all remaining nodes are protected or are snapshot anchors).
    fn prune_oldest_leaf(&mut self) -> bool {
        let protected: std::collections::HashSet<UndoNodeId> =
            self.current_path_ids().into_iter().collect();
        let root_id = UndoNodeId(0);

        // Phase 1: oldest unprotected non-snapshot leaf.
        let oldest_id = self
            .nodes
            .iter()
            .filter(|(id, node)| {
                node.children.is_empty() && !protected.contains(id) && node.text_snapshot.is_none()
            })
            .min_by_key(|(_, node)| node.timestamp)
            .map(|(id, _)| *id);

        if let Some(id) = oldest_id {
            if let Some(parent_id) = self.nodes.get(&id).and_then(|n| n.parent) {
                if let Some(parent) = self.nodes.get_mut(&parent_id) {
                    parent.children.retain(|&c| c != id);
                }
            }
            self.nodes.remove(&id);
            return true;
        }

        // Phase 2: compress oldest single-child interior node (skip root,
        // current, and snapshot nodes).
        let compress_id = self
            .nodes
            .iter()
            .filter(|(&id, node)| {
                id != root_id
                    && node.children.len() == 1
                    && node.text_snapshot.is_none()
                    && id != self.current
            })
            .min_by_key(|(_, node)| node.timestamp)
            .map(|(id, _)| *id);

        if let Some(id) = compress_id {
            let (parent_id, child_id) = {
                let node = &self.nodes[&id];
                let Some(parent_id) = node.parent else {
                    log::error!("undo_tree: compress target {} has no parent", id);
                    return false;
                };
                (parent_id, node.children[0])
            };
            if let Some(parent) = self.nodes.get_mut(&parent_id) {
                let Some(pos) = parent.children.iter().position(|&c| c == id) else {
                    log::error!("undo_tree: parent {} does not list child {}", parent_id, id);
                    return false;
                };
                parent.children[pos] = child_id;
            }
            if let Some(child) = self.nodes.get_mut(&child_id) {
                child.parent = Some(parent_id);
            }
            self.nodes.remove(&id);
            true
        } else {
            false
        }
    }

    fn format_node(&self, root_id: UndoNodeId, out: &mut String) {
        // Iterative DFS: trees can be up to MAX_NODES deep, so recursion would overflow.
        let mut stack: Vec<(UndoNodeId, usize)> = vec![(root_id, 0)];
        while let Some((id, depth)) = stack.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            let marker = if id == self.current { ">" } else { " " };
            let has_snapshot = if node.text_snapshot.is_some() {
                " [S]"
            } else {
                ""
            };
            for _ in 0..depth {
                out.push_str("  ");
            }
            let _ = writeln!(
                out,
                "{} #{} {}{}",
                marker, id, node.description, has_snapshot
            );
            // Reverse so leftmost child pops first (pre-order traversal).
            for &child in node.children.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
}

// ── vim-core UndoTreeSnapshot formatting ────────────────────────────────────

use vim_core::primitives::{NodeId, UndoTreeNodeView, UndoTreeSnapshot};

/// Render a vim-core `UndoTreeSnapshot` into Neovim-style `:undotree` text.
pub(crate) fn format_undo_tree_snapshot(snapshot: &UndoTreeSnapshot) -> String {
    if snapshot.nodes.is_empty() {
        return String::from("Undo tree: (empty)");
    }

    let by_id: HashMap<NodeId, &UndoTreeNodeView> =
        snapshot.nodes.iter().map(|n| (n.id, n)).collect();

    const MAX_UNDO_DISPLAY_DEPTH: usize = 256;
    let mut out = format!("Undo tree: {} changes\n", snapshot.change_count);
    format_undo_node(&by_id, NodeId::ROOT, 0, MAX_UNDO_DISPLAY_DEPTH, &mut out);
    out
}

fn format_undo_node(
    by_id: &HashMap<NodeId, &UndoTreeNodeView>,
    id: NodeId,
    depth: usize,
    max_depth: usize,
    out: &mut String,
) {
    if depth >= max_depth {
        let indent = "  ".repeat(depth);
        out.push_str(&format!("{}  (truncated)\n", indent));
        return;
    }
    let Some(node) = by_id.get(&id) else { return };
    let indent = "  ".repeat(depth);
    let marker = if node.is_current { ">" } else { " " };
    let _ = writeln!(out, "{}{} #{} seq={}", indent, marker, id, node.sequence);
    for &child in &node.children {
        format_undo_node(by_id, child, depth + 1, max_depth, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nid(raw: u64) -> UndoNodeId {
        UndoNodeId(raw)
    }

    #[test]
    fn new_tree_has_root() {
        let tree = UndoTree::new("hello");
        assert_eq!(tree.node_count(), 1);
        assert_eq!(tree.current_id(), nid(0));
        assert!(tree.node(nid(0)).unwrap().text_snapshot.is_some());
    }

    #[test]
    fn record_creates_child() {
        let mut tree = UndoTree::new("hello");
        let id = tree.record_edit("insert 'x'", "hellox");
        assert_eq!(id, nid(1));
        assert_eq!(tree.current_id(), nid(1));
        assert_eq!(tree.node(nid(0)).unwrap().children, vec![nid(1)]);
        assert_eq!(tree.node(nid(1)).unwrap().parent, Some(nid(0)));
    }

    #[test]
    fn branching_creates_multiple_children() {
        let mut tree = UndoTree::new("hello");
        tree.record_edit("edit A", "text");
        tree.navigate_to(nid(0));
        tree.record_edit("edit B", "text");
        assert_eq!(tree.node(nid(0)).unwrap().children, vec![nid(1), nid(2)]);
    }

    #[test]
    fn navigate_to_invalid_returns_false() {
        let mut tree = UndoTree::new("hello");
        assert!(!tree.navigate_to(nid(999)));
        assert_eq!(tree.current_id(), nid(0));
    }

    #[test]
    fn snapshot_interval() {
        let mut tree = UndoTree::new("text");
        // Nodes 1-49 should NOT have snapshots (unless text is provided)
        for i in 1..50 {
            tree.record_edit(&format!("edit {}", i), "text");
        }
        // Node 50 should have a snapshot (50 % 50 == 0)
        let id = tree.record_edit("edit 50", "text at 50");
        assert!(tree.node(id).unwrap().text_snapshot.is_some());
        // Node 51 should not
        let id2 = tree.record_edit("edit 51", "text at 51");
        assert!(tree.node(id2).unwrap().text_snapshot.is_none());
    }

    #[test]
    fn format_tree_shows_structure() {
        let mut tree = UndoTree::new("hello");
        tree.record_edit("insert 'x'", "text");
        tree.record_edit("delete 'o'", "text");
        let output = tree.format_tree();
        assert!(output.contains("#0"));
        assert!(output.contains("#1"));
        assert!(output.contains("#2"));
        assert!(output.contains(">")); // current marker
    }

    fn tree_with_max(initial_text: &str, max_nodes: usize) -> UndoTree {
        let mut tree = UndoTree::new(initial_text);
        tree.max_nodes = max_nodes;
        tree
    }

    #[test]
    fn prune_at_capacity() {
        let mut tree = tree_with_max("text", 5);
        for i in 1..=4 {
            tree.record_edit(&format!("edit {}", i), "text");
        }
        assert_eq!(tree.node_count(), 5);
        tree.record_edit("edit 5", "text");
        assert!(tree.node_count() <= 5);
    }

    #[test]
    fn prune_preserves_current_path() {
        let mut tree = tree_with_max("text", 5);
        for i in 1..=4 {
            tree.record_edit(&format!("edit {}", i), "text");
        }
        tree.navigate_to(nid(0));
        tree.record_edit("branch", "text");
        assert!(tree.node_count() <= 5);
        assert!(tree.node(tree.current_id()).is_some());
        assert!(tree.node(nid(0)).is_some());
    }

    #[test]
    fn prune_preserves_snapshot_nodes() {
        let mut tree = tree_with_max("text", 55);
        for i in 1..=54 {
            tree.record_edit(&format!("edit {}", i), "text");
        }
        assert_eq!(tree.node_count(), 55);
        tree.navigate_to(nid(0));
        tree.record_edit("branch", "text");
        assert!(tree.node(nid(50)).is_some());
    }

    #[test]
    fn prune_removes_oldest_leaf() {
        let mut tree = tree_with_max("text", 4);
        tree.record_edit("edit 1", "text");
        tree.record_edit("edit 2", "text");
        tree.navigate_to(nid(0));
        tree.record_edit("edit 3", "text");
        assert_eq!(tree.node_count(), 4);
        tree.record_edit("edit 4", "text");
        assert!(tree.node(nid(2)).is_none() || tree.node(nid(1)).is_none());
        assert!(tree.node_count() <= 4);
    }

    #[test]
    fn prune_all_protected_graceful() {
        let mut tree = tree_with_max("text", 3);
        tree.record_edit("edit 1", "text");
        tree.record_edit("edit 2", "text");
        assert_eq!(tree.node_count(), 3);
        tree.record_edit("edit 3", "text");
        assert!(tree.node_count() >= 3);
        assert!(tree.node(tree.current_id()).is_some());
    }
}
