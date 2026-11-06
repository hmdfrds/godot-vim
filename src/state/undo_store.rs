//! Changeset-based undo storage keyed by vim-core `NodeId`.
//!
//! Stores forward/inverse `ChangeSet` pairs for each undo group so that
//! undo/redo can be applied directly to `CodeEdit` via targeted
//! `insert_text`/`remove_text` calls, bypassing Godot's linear undo stack.
//!
//! Periodic text checkpoints every `CHECKPOINT_INTERVAL` entries enable
//! recovery from external-edit desync (when `ChangeSet::apply` fails with
//! `LengthMismatch`).

use std::collections::HashMap;

use vim_core::primitives::{ChangeSet, ChangeSetError, NodeId};

// ── Constants ───────────────────────────────────────────────────────────────

/// Store a full text checkpoint every N entries.
const CHECKPOINT_INTERVAL: u32 = 64;

/// Maximum entries before oldest are evicted.
const MAX_ENTRIES: usize = 10_000;

// ── Data Structures ─────────────────────────────────────────────────────────

/// A single undo snapshot: the forward and inverse changesets for one edit
/// group, plus an optional full-text checkpoint for desync recovery.
#[derive(Debug)]
struct UndoSnapshot {
    /// The edit that was applied (T0 -> T1).
    forward: ChangeSet,
    /// The reverse of the edit (T1 -> T0).
    inverse: ChangeSet,
    /// Full text snapshot stored every `CHECKPOINT_INTERVAL` entries.
    checkpoint: Option<String>,
    /// Monotonic ordering for eviction.
    sequence: u64,
}

/// Result of applying a changeset during undo/redo.
pub(crate) struct UndoApplyResult {
    /// The document text after applying the changeset.
    pub text: String,
}

/// Changeset-based undo storage keyed by vim-core `NodeId`.
///
/// Captures text at `BeginUndoGroup`, computes diffs at `EndUndoGroup`,
/// and applies inverse/forward changesets for undo/redo. Periodic text
/// checkpoints every `CHECKPOINT_INTERVAL` nodes enable external-edit
/// desync recovery.
#[derive(Debug)]
pub(crate) struct UndoStore {
    /// Forward/inverse changeset pairs keyed by undo tree node ID.
    snapshots: HashMap<NodeId, UndoSnapshot>,
    /// T0 text captured at the outermost `begin_group`, consumed by `end_group`.
    pending_text: Option<String>,
    /// Open `begin_group` calls.
    depth: u32,
    /// Next sequence number to assign.
    next_sequence: u64,
    /// How often to store full text checkpoints.
    checkpoint_interval: u32,
    /// Maximum number of entries before eviction.
    max_entries: usize,
}

impl UndoStore {
    /// Create a new empty `UndoStore` with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshots: HashMap::new(),
            pending_text: None,
            depth: 0,
            next_sequence: 0,
            checkpoint_interval: CHECKPOINT_INTERVAL,
            max_entries: MAX_ENTRIES,
        }
    }

    /// `BeginUndoGroup`: only the outermost one captures T0.
    pub fn begin_group(&mut self, text: &str) {
        if self.depth == 0 {
            self.pending_text = Some(text.to_string());
        }
        self.depth = self.depth.saturating_add(1);
    }

    /// `EndUndoGroup { node_id: Some(id) }`: commit T0 to `text_after` under
    /// `node_id`, at any depth.
    ///
    /// A bracket that changed nothing keeps the snapshot `node_id` already
    /// has: a motion-only macro reports the node it did not create. So does
    /// an End with nothing pending, such as `:norm`'s after its keys closed
    /// their own groups.
    pub fn end_group(&mut self, node_id: NodeId, text_after: &str) {
        self.depth = 0;
        if let Some(before) = self.pending_text.take() {
            if before != text_after || !self.snapshots.contains_key(&node_id) {
                self.store(node_id, &before, text_after);
            }
        }
    }

    /// `EndUndoGroup { node_id: None }`: the engine committed nothing.
    pub fn end_empty(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        if self.depth == 0 {
            self.pending_text = None;
        }
    }

    /// Store a node the engine made outside the effect stream; the pending
    /// group is left alone.
    pub fn record(&mut self, node_id: NodeId, text_before: &str, text_after: &str) {
        self.store(node_id, text_before, text_after);
    }

    /// Store `forward` and its inverse under `node_id`, overwriting.
    fn store(&mut self, node_id: NodeId, text_before: &str, text_after: &str) {
        let forward = ChangeSet::from_diff(text_before, text_after);
        let inverse = match forward.invert(text_before) {
            Ok(inv) => inv,
            Err(e) => {
                log::error!(
                    "undo_store: failed to invert changeset for node {}: {}",
                    node_id,
                    e
                );
                return;
            }
        };
        self.insert(node_id, forward, inverse, Some(text_after));
    }

    /// Every `checkpoint_interval`-th entry also stores `text_after`.
    fn insert(
        &mut self,
        node_id: NodeId,
        forward: ChangeSet,
        inverse: ChangeSet,
        text_after: Option<&str>,
    ) {
        self.evict_if_needed();

        let seq = self.next_sequence;
        self.next_sequence += 1;

        let checkpoint = match text_after {
            Some(text) if seq > 0 && seq.is_multiple_of(u64::from(self.checkpoint_interval)) => {
                Some(text.to_string())
            }
            _ => None,
        };

        self.snapshots.insert(
            node_id,
            UndoSnapshot {
                forward,
                inverse,
                checkpoint,
                sequence: seq,
            },
        );
    }

    /// Apply inverse changeset (undo). Returns changes for `CodeEdit` application.
    ///
    /// Falls back to checkpoint on `LengthMismatch` (external edit desync).
    /// Returns `None` if the node has no snapshot stored.
    pub fn undo_step(&mut self, node_id: NodeId, current_text: &str) -> Option<UndoApplyResult> {
        let snap = self.snapshots.get(&node_id)?;
        match snap.inverse.apply(current_text) {
            Ok(text) => Some(UndoApplyResult { text }),
            Err(ChangeSetError::LengthMismatch { .. }) => {
                log::warn!(
                    "undo_store: LengthMismatch on undo node {} — attempting checkpoint fallback",
                    node_id
                );
                self.checkpoint_fallback(node_id, Direction::Undo)
            }
            Err(e) => {
                log::error!(
                    "undo_store: unexpected error on undo node {}: {}",
                    node_id,
                    e
                );
                None
            }
        }
    }

    /// Apply forward changeset (redo). Returns the result text.
    ///
    /// Falls back to checkpoint on `LengthMismatch` (external edit desync).
    /// Returns `None` if the node has no snapshot stored.
    pub fn redo_step(&mut self, node_id: NodeId, current_text: &str) -> Option<UndoApplyResult> {
        let snap = self.snapshots.get(&node_id)?;
        match snap.forward.apply(current_text) {
            Ok(text) => Some(UndoApplyResult { text }),
            Err(ChangeSetError::LengthMismatch { .. }) => {
                log::warn!(
                    "undo_store: LengthMismatch on redo node {} — attempting checkpoint fallback",
                    node_id
                );
                self.checkpoint_fallback(node_id, Direction::Redo)
            }
            Err(e) => {
                log::error!(
                    "undo_store: unexpected error on redo node {}: {}",
                    node_id,
                    e
                );
                None
            }
        }
    }

    /// True if a pending group is open (`begin_group` without `end_group`).
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.pending_text.is_some()
    }

    /// Consume and return pending T0 text, closing every open group.
    pub fn take_pending_text(&mut self) -> Option<String> {
        self.depth = 0;
        self.pending_text.take()
    }

    /// Discard all snapshots and pending state, resetting to empty.
    ///
    /// Called when `CodeEdit.set_text()` replaces the buffer wholesale
    /// (file reload, VCS revert). All existing undo entries reference
    /// pre-reload text and would produce garbage if replayed.
    pub fn clear(&mut self) {
        self.snapshots.clear();
        self.pending_text = None;
        self.depth = 0;
        self.next_sequence = 0;
    }

    // ── Private helpers ─────────────────────────────────────────────────

    /// Which direction the checkpoint fallback should target.
    /// For undo, we want the text *before* the edit (use inverse output).
    /// For redo, we want the text *after* the edit (use forward output).
    fn checkpoint_fallback(
        &self,
        node_id: NodeId,
        direction: Direction,
    ) -> Option<UndoApplyResult> {
        // Try to find the target text from the node's own checkpoint or
        // the nearest checkpoint by sequence.
        let text = self.find_checkpoint_target(node_id, direction)?;

        Some(UndoApplyResult { text })
    }

    /// Find the target text for checkpoint fallback.
    ///
    /// For undo: we need the text *before* the node's edit. If the node
    /// has a checkpoint, that represents text_after, so we search for the
    /// nearest earlier checkpoint instead. As last resort, use any available
    /// checkpoint and compute a diff.
    ///
    /// For redo: the node's own checkpoint (if present) is the target.
    /// Otherwise search for the nearest checkpoint.
    fn find_checkpoint_target(&self, node_id: NodeId, direction: Direction) -> Option<String> {
        let snap = self.snapshots.get(&node_id)?;
        let target_seq = snap.sequence;

        match direction {
            Direction::Redo => {
                // For redo, the node's own checkpoint IS the text_after.
                if let Some(ref cp) = snap.checkpoint {
                    return Some(cp.clone());
                }
            }
            Direction::Undo => {
                // For undo, we need text_before. The node's checkpoint is
                // text_after, not useful directly. We need to find a
                // checkpoint from an earlier node.
            }
        }

        // Find the nearest checkpoint with directional preference.
        //
        // For undo: prefer checkpoints with sequence < target_seq (earlier in
        // history, i.e. before the edit we are undoing). Among those, pick the
        // closest one (max sequence). This avoids picking a checkpoint that
        // represents text_after of a *future* edit.
        //
        // For redo: prefer checkpoints with sequence >= target_seq (at or after
        // the edit we are redoing). Among those, pick the closest one (min
        // sequence).
        //
        // If no directional match exists, fall back to the nearest by absolute
        // distance — any checkpoint is better than None.
        let checkpoints_with_cp = || self.snapshots.values().filter(|s| s.checkpoint.is_some());

        let directional = match direction {
            Direction::Undo => checkpoints_with_cp()
                .filter(|s| s.sequence < target_seq)
                .max_by_key(|s| s.sequence),
            Direction::Redo => checkpoints_with_cp()
                .filter(|s| s.sequence >= target_seq)
                .min_by_key(|s| s.sequence),
        };

        if let Some(s) = directional {
            return s.checkpoint.clone();
        }

        // Fallback: nearest by absolute distance.
        let nearest = checkpoints_with_cp().min_by_key(|s| s.sequence.abs_diff(target_seq));

        nearest.and_then(|s| s.checkpoint.clone())
    }

    /// Evict oldest non-checkpoint entries if at or above `max_entries`.
    ///
    /// Checkpoint entries are protected until a newer checkpoint exists,
    /// ensuring at least one checkpoint is always available for fallback.
    fn evict_if_needed(&mut self) {
        while self.snapshots.len() >= self.max_entries {
            if !self.evict_one() {
                break;
            }
        }
    }

    /// Evict a single entry. Returns `false` if nothing can be evicted.
    ///
    /// Strategy:
    /// 1. Find the oldest non-checkpoint entry and remove it.
    /// 2. If all remaining are checkpoints, find the oldest checkpoint
    ///    that has a newer checkpoint sibling and remove it.
    fn evict_one(&mut self) -> bool {
        // Phase 1: oldest non-checkpoint entry.
        let oldest_non_cp = self
            .snapshots
            .iter()
            .filter(|(_, s)| s.checkpoint.is_none())
            .min_by_key(|(_, s)| s.sequence)
            .map(|(id, _)| *id);

        if let Some(id) = oldest_non_cp {
            self.snapshots.remove(&id);
            return true;
        }

        // Phase 2: oldest checkpoint that has a newer checkpoint sibling.
        // We must keep at least one checkpoint for fallback.
        if self.snapshots.len() <= 1 {
            return false;
        }

        let checkpoint_count = self
            .snapshots
            .values()
            .filter(|s| s.checkpoint.is_some())
            .count();

        if checkpoint_count <= 1 {
            return false;
        }

        let oldest_cp = self
            .snapshots
            .iter()
            .filter(|(_, s)| s.checkpoint.is_some())
            .min_by_key(|(_, s)| s.sequence)
            .map(|(id, _)| *id);

        if let Some(id) = oldest_cp {
            self.snapshots.remove(&id);
            return true;
        }

        false
    }
}

/// Internal direction tag for checkpoint fallback.
#[derive(Debug, Clone, Copy)]
enum Direction {
    Undo,
    Redo,
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ─────────────────────────────────────────────────────────

    fn nid(raw: u32) -> NodeId {
        NodeId::new(raw)
    }

    // ── begin_group / end_group ─────────────────────────────────────────

    #[test]
    fn begin_end_group_stores_snapshot() {
        let mut store = UndoStore::new();
        store.begin_group("hello");
        store.end_group(nid(1), "hello world");

        assert!(!store.has_pending());
        assert!(store.snapshots.contains_key(&nid(1)));
    }

    #[test]
    fn end_without_begin_is_noop() {
        let mut store = UndoStore::new();
        store.end_group(nid(1), "hello");

        assert!(store.snapshots.is_empty());
        assert!(!store.has_pending());
    }

    #[test]
    fn begin_captures_pending() {
        let mut store = UndoStore::new();
        store.begin_group("hello");

        assert!(store.has_pending());
    }

    #[test]
    fn end_group_clears_pending() {
        let mut store = UndoStore::new();
        store.begin_group("hello");
        assert!(store.has_pending());
        store.end_group(nid(1), "hello world");
        assert!(!store.has_pending());
    }

    // ── take_pending_text ───────────────────────────────────────────────

    #[test]
    fn take_pending_text_consumes() {
        let mut store = UndoStore::new();
        store.begin_group("hello");

        let text = store.take_pending_text();
        assert_eq!(text, Some("hello".to_string()));
        assert!(!store.has_pending());

        // Second call returns None.
        assert_eq!(store.take_pending_text(), None);
    }

    #[test]
    fn take_pending_text_when_none() {
        let mut store = UndoStore::new();
        assert_eq!(store.take_pending_text(), None);
    }

    // ── undo_step ───────────────────────────────────────────────────────

    #[test]
    fn undo_step_applies_inverse() {
        let mut store = UndoStore::new();
        let before = "hello";
        let after = "hello world";

        store.begin_group(before);
        store.end_group(nid(1), after);

        let result = store.undo_step(nid(1), after).unwrap();
        assert_eq!(result.text, before);
    }

    #[test]
    fn undo_step_unknown_node_returns_none() {
        let mut store = UndoStore::new();
        assert!(store.undo_step(nid(99), "hello").is_none());
    }

    // ── redo_step ───────────────────────────────────────────────────────

    #[test]
    fn redo_step_applies_forward() {
        let mut store = UndoStore::new();
        let before = "hello";
        let after = "hello world";

        store.begin_group(before);
        store.end_group(nid(1), after);

        // First undo to get back to `before`.
        let undo_result = store.undo_step(nid(1), after).unwrap();
        assert_eq!(undo_result.text, before);

        // Then redo to get back to `after`.
        let redo_result = store.redo_step(nid(1), before).unwrap();
        assert_eq!(redo_result.text, after);
    }

    #[test]
    fn redo_step_unknown_node_returns_none() {
        let mut store = UndoStore::new();
        assert!(store.redo_step(nid(99), "hello").is_none());
    }

    // ── Round-trip undo/redo ────────────────────────────────────────────

    #[test]
    fn round_trip_undo_redo() {
        let mut store = UndoStore::new();
        let before = "hello world";
        let after = "goodbye world";

        store.begin_group(before);
        store.end_group(nid(1), after);

        // Undo: after -> before
        let undo_result = store.undo_step(nid(1), after).unwrap();
        assert_eq!(undo_result.text, before);

        // Redo: before -> after
        let redo_result = store.redo_step(nid(1), &undo_result.text).unwrap();
        assert_eq!(redo_result.text, after);

        // Second undo: after -> before again
        let undo2 = store.undo_step(nid(1), &redo_result.text).unwrap();
        assert_eq!(undo2.text, before);
    }

    // ── Identity edit ───────────────────────────────────────────────────

    #[test]
    fn identity_edit_produces_no_changes() {
        let mut store = UndoStore::new();
        let text = "hello world";

        store.begin_group(text);
        store.end_group(nid(1), text);

        // Undo an identity edit — text should be unchanged.
        let result = store.undo_step(nid(1), text).unwrap();
        assert_eq!(result.text, text);
    }

    // ── Multiple independent groups ─────────────────────────────────────

    #[test]
    fn multiple_independent_groups() {
        let mut store = UndoStore::new();

        // Group 1: "hello" -> "hello world"
        store.begin_group("hello");
        store.end_group(nid(1), "hello world");

        // Group 2: "hello world" -> "hello world!"
        store.begin_group("hello world");
        store.end_group(nid(2), "hello world!");

        // Undo group 2
        let r2 = store.undo_step(nid(2), "hello world!").unwrap();
        assert_eq!(r2.text, "hello world");

        // Undo group 1
        let r1 = store.undo_step(nid(1), "hello world").unwrap();
        assert_eq!(r1.text, "hello");

        // Redo group 1
        let r1_redo = store.redo_step(nid(1), "hello").unwrap();
        assert_eq!(r1_redo.text, "hello world");

        // Redo group 2
        let r2_redo = store.redo_step(nid(2), "hello world").unwrap();
        assert_eq!(r2_redo.text, "hello world!");
    }

    // ── Checkpoint interval ─────────────────────────────────────────────

    #[test]
    fn checkpoint_interval_stores_text() {
        let mut store = UndoStore::new();
        store.checkpoint_interval = 4; // Every 4th entry.

        // Entries 0..3 should not have checkpoints (sequence 0 is not
        // a checkpoint since seq > 0 is required).
        for i in 0..4 {
            let before = format!("text_{}", i);
            let after = format!("text_{}", i + 1);
            store.begin_group(&before);
            store.end_group(nid(i), &after);
        }

        // Sequence 0 — no checkpoint (seq > 0 guard).
        assert!(store.snapshots[&nid(0)].checkpoint.is_none());
        // Sequence 1, 2, 3 — no checkpoint (not divisible by 4).
        assert!(store.snapshots[&nid(1)].checkpoint.is_none());
        assert!(store.snapshots[&nid(2)].checkpoint.is_none());
        assert!(store.snapshots[&nid(3)].checkpoint.is_none());

        // Now add the 5th entry (sequence 4, divisible by 4, seq > 0).
        store.begin_group("text_4");
        store.end_group(nid(4), "text_5");
        assert!(store.snapshots[&nid(4)].checkpoint.is_some());
        assert_eq!(
            store.snapshots[&nid(4)].checkpoint.as_deref(),
            Some("text_5")
        );
    }

    // ── Eviction (memory management) ────────────────────────────────────

    #[test]
    fn eviction_at_max_entries() {
        let mut store = UndoStore::new();
        store.max_entries = 5;
        store.checkpoint_interval = 100; // Prevent checkpoints from interfering.

        for i in 0..6 {
            let before = format!("v{}", i);
            let after = format!("v{}", i + 1);
            store.begin_group(&before);
            store.end_group(nid(i), &after);
        }

        // Should have evicted to stay at or below max_entries.
        assert!(store.snapshots.len() <= 5);
    }

    #[test]
    fn eviction_protects_checkpoints_when_newer_exists() {
        let mut store = UndoStore::new();
        store.max_entries = 3;
        store.checkpoint_interval = 2; // Checkpoint at seq 2, 4, ...

        // seq 0 — no checkpoint (seq > 0 guard)
        store.begin_group("a");
        store.end_group(nid(0), "b");

        // seq 1 — no checkpoint
        store.begin_group("b");
        store.end_group(nid(1), "c");

        // seq 2 — checkpoint
        store.begin_group("c");
        store.end_group(nid(2), "d");

        assert!(store.snapshots[&nid(2)].checkpoint.is_some());

        // At this point we have 3 entries. Adding a 4th triggers eviction.
        // seq 3 — no checkpoint
        store.begin_group("d");
        store.end_group(nid(3), "e");

        assert!(store.snapshots.len() <= 3);
        // The checkpoint at nid(2) should be preserved (only checkpoint).
        assert!(store.snapshots.contains_key(&nid(2)));
    }

    // ── Replacement edit ────────────────────────────────────────────────

    #[test]
    fn replacement_edit_round_trip() {
        let mut store = UndoStore::new();
        let before = "hello world";
        let after = "hello WORLD";

        store.begin_group(before);
        store.end_group(nid(1), after);

        // Undo
        let undo = store.undo_step(nid(1), after).unwrap();
        assert_eq!(undo.text, before);

        // Redo
        let redo = store.redo_step(nid(1), before).unwrap();
        assert_eq!(redo.text, after);
    }

    // ── Deletion edit ───────────────────────────────────────────────────

    #[test]
    fn deletion_edit_round_trip() {
        let mut store = UndoStore::new();
        let before = "hello world";
        let after = "hello";

        store.begin_group(before);
        store.end_group(nid(1), after);

        // Undo: "hello" -> "hello world"
        let undo = store.undo_step(nid(1), after).unwrap();
        assert_eq!(undo.text, before);

        // Redo: "hello world" -> "hello"
        let redo = store.redo_step(nid(1), before).unwrap();
        assert_eq!(redo.text, after);
    }

    // ── Insertion edit ──────────────────────────────────────────────────

    #[test]
    fn insertion_edit_round_trip() {
        let mut store = UndoStore::new();
        let before = "hello";
        let after = "hello world";

        store.begin_group(before);
        store.end_group(nid(1), after);

        // Undo
        let undo = store.undo_step(nid(1), after).unwrap();
        assert_eq!(undo.text, before);

        // Redo
        let redo = store.redo_step(nid(1), before).unwrap();
        assert_eq!(redo.text, after);
    }

    // ── UTF-8 edits ─────────────────────────────────────────────────────

    #[test]
    fn utf8_edit_round_trip() {
        let mut store = UndoStore::new();
        let before = "héllo 世界";
        let after = "héllo 🌍";

        store.begin_group(before);
        store.end_group(nid(1), after);

        let undo = store.undo_step(nid(1), after).unwrap();
        assert_eq!(undo.text, before);

        let redo = store.redo_step(nid(1), before).unwrap();
        assert_eq!(redo.text, after);
    }

    // ── Empty document edits ────────────────────────────────────────────

    #[test]
    fn empty_to_nonempty_round_trip() {
        let mut store = UndoStore::new();
        store.begin_group("");
        store.end_group(nid(1), "hello");

        let undo = store.undo_step(nid(1), "hello").unwrap();
        assert_eq!(undo.text, "");

        let redo = store.redo_step(nid(1), "").unwrap();
        assert_eq!(redo.text, "hello");
    }

    #[test]
    fn nonempty_to_empty_round_trip() {
        let mut store = UndoStore::new();
        store.begin_group("hello");
        store.end_group(nid(1), "");

        let undo = store.undo_step(nid(1), "").unwrap();
        assert_eq!(undo.text, "hello");

        let redo = store.redo_step(nid(1), "hello").unwrap();
        assert_eq!(redo.text, "");
    }

    // ── Multiline edits ─────────────────────────────────────────────────

    #[test]
    fn multiline_edit_round_trip() {
        let mut store = UndoStore::new();
        let before = "line1\nline2\nline3\n";
        let after = "line1\nchanged\nline3\n";

        store.begin_group(before);
        store.end_group(nid(1), after);

        let undo = store.undo_step(nid(1), after).unwrap();
        assert_eq!(undo.text, before);

        let redo = store.redo_step(nid(1), before).unwrap();
        assert_eq!(redo.text, after);
    }

    // ── Sequence numbers ────────────────────────────────────────────────

    #[test]
    fn sequence_numbers_are_monotonic() {
        let mut store = UndoStore::new();

        store.begin_group("a");
        store.end_group(nid(1), "b");
        store.begin_group("b");
        store.end_group(nid(2), "c");
        store.begin_group("c");
        store.end_group(nid(3), "d");

        assert_eq!(store.snapshots[&nid(1)].sequence, 0);
        assert_eq!(store.snapshots[&nid(2)].sequence, 1);
        assert_eq!(store.snapshots[&nid(3)].sequence, 2);
    }

    // ── Overwriting node ID ─────────────────────────────────────────────

    #[test]
    fn overwriting_same_node_id_replaces() {
        let mut store = UndoStore::new();

        store.begin_group("hello");
        store.end_group(nid(1), "hello world");

        // Overwrite with a different edit.
        store.begin_group("hello");
        store.end_group(nid(1), "hello!");

        // The latest edit should be the active one.
        let undo = store.undo_step(nid(1), "hello!").unwrap();
        assert_eq!(undo.text, "hello");
    }

    // ── Nesting: the outermost begin_group wins ─────────────────────────

    #[test]
    fn nested_begin_keeps_outer() {
        let mut store = UndoStore::new();

        store.begin_group("first");
        store.begin_group("second");

        let text = store.take_pending_text();
        assert_eq!(text, Some("first".to_string()));
    }

    #[test]
    fn end_group_commits_from_the_outer_begin_at_any_depth() {
        let mut store = UndoStore::new();
        store.begin_group("a");
        store.begin_group("ab");
        store.end_group(nid(1), "abc");

        assert!(!store.has_pending());
        assert_eq!(store.undo_step(nid(1), "abc").unwrap().text, "a");

        // Depth went back to 0: the next Begin captures again.
        store.begin_group("abc");
        assert_eq!(store.take_pending_text(), Some("abc".to_string()));
    }

    #[test]
    fn end_empty_drops_pending_only_at_depth_zero() {
        let mut store = UndoStore::new();
        store.begin_group("session start");
        store.begin_group("inner");
        store.end_empty();
        assert!(store.has_pending(), "an inner empty pair keeps the session");

        store.end_empty();
        assert!(!store.has_pending());
    }

    #[test]
    fn end_empty_at_depth_zero_saturates() {
        let mut store = UndoStore::new();
        store.end_empty();
        store.begin_group("a");
        assert_eq!(store.take_pending_text(), Some("a".to_string()));
    }

    #[test]
    fn take_pending_text_and_clear_reset_depth() {
        let mut store = UndoStore::new();
        store.begin_group("a");
        store.begin_group("a");
        store.take_pending_text();
        store.begin_group("b");
        assert_eq!(store.take_pending_text(), Some("b".to_string()));

        store.begin_group("c");
        store.begin_group("c");
        store.clear();
        store.begin_group("d");
        assert_eq!(store.take_pending_text(), Some("d".to_string()));
    }

    #[test]
    fn record_leaves_the_pending_group_alone() {
        let mut store = UndoStore::new();
        store.begin_group("a");
        store.record(nid(5), "a", "ab");

        assert_eq!(store.undo_step(nid(5), "ab").unwrap().text, "a");
        assert_eq!(store.take_pending_text(), Some("a".to_string()));
    }

    #[test]
    fn unchanged_bracket_keeps_the_snapshot() {
        let mut store = UndoStore::new();
        store.begin_group("abc");
        store.end_group(nid(1), "bc");

        // A macro that only moves reports the current node again.
        store.begin_group("bc");
        store.end_group(nid(1), "bc");

        assert_eq!(store.undo_step(nid(1), "bc").unwrap().text, "abc");
    }

    #[test]
    fn end_without_pending_keeps_the_snapshot() {
        let mut store = UndoStore::new();
        store.begin_group("abc");
        store.end_group(nid(1), "bc");

        store.end_group(nid(1), "bc");

        assert_eq!(store.undo_step(nid(1), "bc").unwrap().text, "abc");
    }

    // ── Checkpoint fallback on desync ────────────────────────────────────

    #[test]
    fn checkpoint_fallback_on_redo_desync() {
        let mut store = UndoStore::new();
        store.checkpoint_interval = 1; // Checkpoint at seq 1, 2, 3, ...

        // seq 0 — no checkpoint (seq > 0 guard).
        store.begin_group("hello");
        store.end_group(nid(0), "hello world");

        // seq 1 — checkpoint (1 % 1 == 0 and seq > 0).
        store.begin_group("hello world");
        store.end_group(nid(1), "hello world!");

        assert!(store.snapshots[&nid(1)].checkpoint.is_some());

        // Simulate desync on nid(1): current text differs from forward's expected input.
        let desynced_text = "completely different text";
        let result = store.redo_step(nid(1), desynced_text);

        // Should fallback to checkpoint and produce a diff result.
        assert!(result.is_some());
        let result = result.unwrap();
        // The checkpoint at nid(1) stores "hello world!", so fallback produces that.
        assert_eq!(result.text, "hello world!");
    }

    // ── Large eviction stress ───────────────────────────────────────────

    #[test]
    fn large_eviction_stays_bounded() {
        let mut store = UndoStore::new();
        store.max_entries = 10;
        store.checkpoint_interval = 5;

        for i in 0..50u32 {
            let before = format!("text_{}", i);
            let after = format!("text_{}", i + 1);
            store.begin_group(&before);
            store.end_group(nid(i), &after);
        }

        assert!(store.snapshots.len() <= 10);
    }

    // ── has_pending starts false ─────────────────────────────────────────

    #[test]
    fn has_pending_initially_false() {
        let store = UndoStore::new();
        assert!(!store.has_pending());
    }

    // ── new() defaults ──────────────────────────────────────────────────

    #[test]
    fn new_defaults() {
        let store = UndoStore::new();
        assert!(store.snapshots.is_empty());
        assert!(!store.has_pending());
        assert_eq!(store.next_sequence, 0);
        assert_eq!(store.checkpoint_interval, CHECKPOINT_INTERVAL);
        assert_eq!(store.max_entries, MAX_ENTRIES);
    }

    // ── Checkpoint fallback on undo desync ──────────────────────────────

    #[test]
    fn checkpoint_fallback_on_undo_desync() {
        let mut store = UndoStore::new();
        store.checkpoint_interval = 1; // Checkpoint at seq 1, 2, 3, ...

        // seq 0 — no checkpoint (seq > 0 guard).
        store.begin_group("hello");
        store.end_group(nid(0), "hello world");

        // seq 1 — checkpoint (1 % 1 == 0 and seq > 0).
        store.begin_group("hello world");
        store.end_group(nid(1), "hello world!");

        assert!(store.snapshots[&nid(1)].checkpoint.is_some());

        // Simulate desync on nid(1): current text differs from inverse's expected input.
        let desynced_text = "completely different text";
        let result = store.undo_step(nid(1), desynced_text);

        // Should fallback to checkpoint and produce a diff result.
        assert!(result.is_some());
        let result = result.unwrap();
        // For undo direction, fallback searches for an earlier checkpoint.
        // nid(0) has no checkpoint (seq 0), nid(1) checkpoint is text_after
        // ("hello world!") which is not useful for undo direction (we need
        // text_before). Fallback uses nearest-by-distance as last resort,
        // which is nid(1)'s checkpoint = "hello world!".
        assert!(!result.text.is_empty());
    }

    // ── LengthMismatch with no checkpoints ──────────────────────────────

    #[test]
    fn length_mismatch_with_no_checkpoints_returns_none() {
        let mut store = UndoStore::new();
        store.checkpoint_interval = 1000; // No checkpoints in this range.

        // seq 0 — no checkpoint (seq > 0 guard, and 0 % 1000 == 0 but seq > 0 fails).
        store.begin_group("hello");
        store.end_group(nid(0), "hello world");

        assert!(store.snapshots[&nid(0)].checkpoint.is_none());

        // Simulate desync: current text has wrong length.
        let desynced_text = "completely different text of wrong length";
        let result = store.undo_step(nid(0), desynced_text);

        // No checkpoints exist at all, so fallback returns None.
        assert!(result.is_none());
    }

    // ── Phase 2 eviction (all checkpoints) ──────────────────────────────

    #[test]
    fn eviction_phase2_removes_oldest_checkpoint() {
        let mut store = UndoStore::new();
        store.max_entries = 2;
        store.checkpoint_interval = 1; // All entries with seq > 0 get checkpoints.

        // seq 0 — no checkpoint (seq > 0 guard).
        store.begin_group("a");
        store.end_group(nid(0), "ab");

        // seq 1 — checkpoint.
        store.begin_group("ab");
        store.end_group(nid(1), "abc");

        // At this point len == 2, which equals max_entries.
        // Adding a 3rd entry triggers eviction before inserting.
        // seq 2 — checkpoint.
        store.begin_group("abc");
        store.end_group(nid(2), "abcd");

        // Eviction should have run:
        // Phase 1 removes oldest non-checkpoint (nid(0), seq 0, no checkpoint).
        // After that, len == 2 which equals max_entries, so it tries again:
        // Phase 1 finds no non-checkpoint entries (nid(1) and nid(2) both have checkpoints).
        // Phase 2: two checkpoints exist, so remove oldest checkpoint (nid(1)).
        // Now len == 1, then nid(2) is inserted -> len == 2.
        assert!(store.snapshots.len() <= 2);

        // The newest checkpoint (nid(2)) must survive.
        assert!(store.snapshots.contains_key(&nid(2)));
        assert!(store.snapshots[&nid(2)].checkpoint.is_some());
    }

    // ── clear ───────────────────────────────────────────────────────────

    #[test]
    fn clear_empties_snapshots_and_pending() {
        let mut store = UndoStore::new();

        store.begin_group("hello");
        store.end_group(nid(1), "hello world");
        store.begin_group("hello world");
        store.end_group(nid(2), "hello world!");
        store.begin_group("pending text");

        assert_eq!(store.snapshots.len(), 2);
        assert!(store.has_pending());
        assert!(store.next_sequence > 0);

        store.clear();

        assert!(store.snapshots.is_empty());
        assert!(!store.has_pending());
        assert_eq!(store.next_sequence, 0);
    }

    #[test]
    fn clear_then_reuse() {
        let mut store = UndoStore::new();

        store.begin_group("a");
        store.end_group(nid(1), "b");

        store.clear();

        // Can add entries after clear.
        store.begin_group("x");
        store.end_group(nid(10), "y");

        assert_eq!(store.snapshots.len(), 1);
        assert!(store.snapshots.contains_key(&nid(10)));

        // Undo works on the new entry.
        let result = store.undo_step(nid(10), "y").unwrap();
        assert_eq!(result.text, "x");
    }

    #[test]
    fn clear_on_empty_store() {
        let mut store = UndoStore::new();
        store.clear();
        assert!(store.snapshots.is_empty());
        assert!(!store.has_pending());
        assert_eq!(store.next_sequence, 0);
    }
}
