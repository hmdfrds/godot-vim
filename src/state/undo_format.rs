//! Formatting helpers for vim-core `UndoTreeSnapshot` → Neovim-style `:undotree` text.

use std::collections::HashMap;
use std::fmt::Write;

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
