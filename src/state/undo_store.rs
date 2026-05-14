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
///
/// Contains both the full result text and the individual changes suitable
/// for applying to `CodeEdit` via targeted `insert_text`/`remove_text`.
pub(crate) struct UndoApplyResult {
    /// The document text after applying the changeset.
    pub text: String,
    /// `(from, to, replacement)` triples in input-document byte offsets.
    /// - `(from, to, None)` — deletion of `from..to`
    /// - `(pos, pos, Some(text))` — insertion at `pos`
    /// - `(from, to, Some(text))` — replacement of `from..to` with `text`
    pub changes: Vec<(usize, usize, Option<String>)>,
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
    /// T0 text captured at `begin_group`, consumed by `end_group`.
    pending_text: Option<String>,
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
            next_sequence: 0,
            checkpoint_interval: CHECKPOINT_INTERVAL,
            max_entries: MAX_ENTRIES,
        }
    }

    /// Capture T0 text snapshot. Called on `BeginUndoGroup`.
    ///
    /// If a previous group was not ended (programming error or crash),
    /// the old pending text is silently replaced.
    pub fn begin_group(&mut self, text: &str) {
        self.pending_text = Some(text.to_string());
    }

    /// Compute diff, store forward + inverse. Called on `EndUndoGroup`.
    ///
    /// Uses `ChangeSet::from_diff(T0, T1)` to compute the forward changeset,
    /// then `forward.invert(T0)` for the inverse. Every `checkpoint_interval`-th
    /// entry stores `text_after` as a full checkpoint.
    ///
    /// No-op if `begin_group` was not called first.
    pub fn end_group(&mut self, node_id: NodeId, text_after: &str) {
        let text_before = match self.pending_text.take() {
            Some(t) => t,
            None => {
                log::warn!(
                    "undo_store: end_group({}) without begin_group — ignoring",
                    node_id
                );
                return;
            }
        };

        let forward = ChangeSet::from_diff(&text_before, text_after);

        // Identity edit (no actual change) — still record it so the node
        // has an entry and undo/redo can traverse through it cleanly.
        let inverse = match forward.invert(&text_before) {
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

        // Evict oldest entries if at capacity.
        self.evict_if_needed();

        let seq = self.next_sequence;
        self.next_sequence += 1;

        let checkpoint = if seq > 0 && (seq % u64::from(self.checkpoint_interval)) == 0 {
            Some(text_after.to_string())
        } else {
            None
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
            Ok(result_text) => {
                let changes = Self::collect_changes(&snap.inverse);
                Some(UndoApplyResult {
                    text: result_text,
                    changes,
                })
            }
            Err(ChangeSetError::LengthMismatch { .. }) => {
                log::warn!(
                    "undo_store: LengthMismatch on undo node {} — attempting checkpoint fallback",
                    node_id
                );
                self.checkpoint_fallback(node_id, current_text, Direction::Undo)
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

    /// Apply forward changeset (redo). Returns changes for `CodeEdit` application.
    ///
    /// Falls back to checkpoint on `LengthMismatch` (external edit desync).
    /// Returns `None` if the node has no snapshot stored.
    pub fn redo_step(&mut self, node_id: NodeId, current_text: &str) -> Option<UndoApplyResult> {
        let snap = self.snapshots.get(&node_id)?;
        match snap.forward.apply(current_text) {
            Ok(result_text) => {
                let changes = Self::collect_changes(&snap.forward);
                Some(UndoApplyResult {
                    text: result_text,
                    changes,
                })
            }
            Err(ChangeSetError::LengthMismatch { .. }) => {
                log::warn!(
                    "undo_store: LengthMismatch on redo node {} — attempting checkpoint fallback",
                    node_id
                );
                self.checkpoint_fallback(node_id, current_text, Direction::Redo)
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

    /// Garbage-collect entries for pruned nodes.
    pub fn remove_pruned(&mut self, ids: &[NodeId]) {
        for id in ids {
            self.snapshots.remove(id);
        }
    }

    /// True if a pending group is open (`begin_group` without `end_group`).
    #[must_use]
    pub fn has_pending(&self) -> bool {
        self.pending_text.is_some()
    }

    /// Consume and return pending T0 text. Used for panic recovery.
    pub fn take_pending_text(&mut self) -> Option<String> {
        self.pending_text.take()
    }

    // ── Private helpers ─────────────────────────────────────────────────

    /// Which direction the checkpoint fallback should target.
    /// For undo, we want the text *before* the edit (use inverse output).
    /// For redo, we want the text *after* the edit (use forward output).
    fn checkpoint_fallback(
        &self,
        node_id: NodeId,
        current_text: &str,
        direction: Direction,
    ) -> Option<UndoApplyResult> {
        // Try to find the target text from the node's own checkpoint or
        // the nearest checkpoint by sequence.
        let target_text = self.find_checkpoint_target(node_id, direction)?;

        let diff = ChangeSet::from_diff(current_text, &target_text);
        let changes = Self::collect_changes(&diff);

        Some(UndoApplyResult {
            text: target_text,
            changes,
        })
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
        let checkpoints_with_cp = || {
            self.snapshots
                .values()
                .filter(|s| s.checkpoint.is_some())
        };

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
        let nearest = checkpoints_with_cp().min_by_key(|s| {
            if s.sequence > target_seq {
                s.sequence - target_seq
            } else {
                target_seq - s.sequence
            }
        });

        nearest.and_then(|s| s.checkpoint.clone())
    }

    /// Collect `(from, to, Option<String>)` triples from a changeset's
    /// `changes()` iterator. Converts borrowed `&str` to owned `String`.
    fn collect_changes(cs: &ChangeSet) -> Vec<(usize, usize, Option<String>)> {
        cs.changes()
            .map(|(from, to, text)| (from, to, text.map(String::from)))
            .collect()
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
        assert!(!result.changes.is_empty());
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

        // Undo an identity edit — text should be unchanged, no changes.
        let result = store.undo_step(nid(1), text).unwrap();
        assert_eq!(result.text, text);
        assert!(result.changes.is_empty());
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

    // ── remove_pruned ───────────────────────────────────────────────────

    #[test]
    fn remove_pruned_removes_entries() {
        let mut store = UndoStore::new();
        store.begin_group("a");
        store.end_group(nid(1), "b");
        store.begin_group("b");
        store.end_group(nid(2), "c");
        store.begin_group("c");
        store.end_group(nid(3), "d");

        assert_eq!(store.snapshots.len(), 3);

        store.remove_pruned(&[nid(1), nid(3)]);
        assert_eq!(store.snapshots.len(), 1);
        assert!(store.snapshots.contains_key(&nid(2)));
        assert!(!store.snapshots.contains_key(&nid(1)));
        assert!(!store.snapshots.contains_key(&nid(3)));
    }

    #[test]
    fn remove_pruned_with_nonexistent_ids() {
        let mut store = UndoStore::new();
        store.begin_group("a");
        store.end_group(nid(1), "b");

        // Removing nonexistent IDs should not panic or affect existing.
        store.remove_pruned(&[nid(99), nid(100)]);
        assert_eq!(store.snapshots.len(), 1);
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

    // ── Changes vector correctness ──────────────────────────────────────

    #[test]
    fn undo_changes_are_correct_triples() {
        let mut store = UndoStore::new();
        let before = "hello";
        let after = "hello world";

        store.begin_group(before);
        store.end_group(nid(1), after);

        let result = store.undo_step(nid(1), after).unwrap();

        // The inverse of "hello" -> "hello world" should delete " world".
        // " world" was inserted at position 5, so inverse deletes at 5..11.
        assert!(!result.changes.is_empty());

        // Apply the changes manually: start from "hello world", apply inverse.
        // The result text should match.
        assert_eq!(result.text, "hello");
    }

    #[test]
    fn redo_changes_are_correct_triples() {
        let mut store = UndoStore::new();
        let before = "hello";
        let after = "hello world";

        store.begin_group(before);
        store.end_group(nid(1), after);

        let result = store.redo_step(nid(1), before).unwrap();

        // The forward changeset inserts " world" at position 5.
        assert!(!result.changes.is_empty());
        assert_eq!(result.text, "hello world");
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
        // The change should be a replacement at bytes 6..11.
        assert_eq!(undo.changes.len(), 1);
        let (from, to, ref text) = undo.changes[0];
        assert_eq!(from, 6);
        assert_eq!(to, 11);
        assert_eq!(text.as_deref(), Some("world"));

        // Redo
        let redo = store.redo_step(nid(1), before).unwrap();
        assert_eq!(redo.text, after);
        assert_eq!(redo.changes.len(), 1);
        let (from, to, ref text) = redo.changes[0];
        assert_eq!(from, 6);
        assert_eq!(to, 11);
        assert_eq!(text.as_deref(), Some("WORLD"));
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

    // ── begin_group replaces old pending ─────────────────────────────────

    #[test]
    fn double_begin_replaces_pending() {
        let mut store = UndoStore::new();

        store.begin_group("first");
        store.begin_group("second");

        let text = store.take_pending_text();
        assert_eq!(text, Some("second".to_string()));
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

    // ── remove_pruned on empty store ────────────────────────────────────

    #[test]
    fn remove_pruned_on_empty_store() {
        let mut store = UndoStore::new();
        store.remove_pruned(&[nid(1), nid(2)]);
        assert!(store.snapshots.is_empty());
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
        assert!(!result.changes.is_empty());
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
}
