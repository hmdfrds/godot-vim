//! Signal connection helpers that encapsulate the `is_connected` guard +
//! connect/disconnect pattern used across `attach.rs`, `lifecycle.rs`,
//! and `floating.rs`.

use godot::classes::object::ConnectFlags;
use godot::classes::Object;
use godot::prelude::*;

// ── Signal name constants ────────────────────────────────────────────
//
// Canonical registry of Godot signal names used by the plugin subsystem.
// Per-editor signals (gui_input, caret_changed, etc.) live in attach.rs
// because they are only used locally.

pub(super) const SIG_SETTINGS_CHANGED: &str = "settings_changed";
pub(super) const SIG_EDITOR_SCRIPT_CHANGED: &str = "editor_script_changed";
pub(super) const SIG_GUI_FOCUS_CHANGED: &str = "gui_focus_changed";
pub(super) const SIG_TIMEOUT: &str = "timeout";
pub(super) const SIG_CONFIG_SAVED: &str = "config_saved";
pub(super) const SIG_WINDOW_VISIBILITY_CHANGED: &str = "window_visibility_changed";
pub(super) const SIG_CHILD_ENTERED_TREE: &str = "child_entered_tree";
pub(super) const SIG_TREE_EXITED: &str = "tree_exited";
pub(super) const SIG_FOCUS_ENTERED: &str = "focus_entered";

/// Connect with DEFERRED delivery (idempotent). Required for signals that
/// fire during re-entrant contexts (e.g. `caret_changed` during text edits).
///
/// A handler connected this way must not take a manually managed Object
/// (`Node`, `Control`) as a typed `Gd<T>` parameter. The call waits in Godot's
/// message queue until end of frame, which grants a manually managed argument
/// no refcount, so it can be freed before the queue drains. gdext rejects a
/// freed object during parameter conversion, *before* the handler body runs,
/// so the handler cannot guard itself and the failure surfaces as an engine
/// error nothing in Rust can intercept. Take `Variant` and resolve it with
/// `try_to::<Gd<T>>()` inside the body, as `on_focus_changed`,
/// `on_script_changed` and `perform_attach` do. `RefCounted` arguments such as
/// `InputEvent` are exempt: Godot copies deferred arguments into the queue as
/// `Variant`s, and a `Variant` holding a `RefCounted` payload takes a strong
/// reference to it.
///
/// `deferred_delivery_contract` below is a ratchet on this rule, not a proof
/// of it.
pub(super) fn connect_deferred(
    target: &mut Gd<impl Inherits<Object>>,
    signal: &str,
    callable: &Callable,
) {
    let mut obj = target.clone().upcast::<Object>();
    if !obj.is_connected(signal, callable) {
        let err = obj.connect_flags(signal, callable, ConnectFlags::DEFERRED);
        if err != godot::global::Error::OK {
            log::warn!(
                "Failed to connect signal '{}' (deferred): {:?}",
                signal,
                err
            );
        }
    }
}

/// Connect with immediate delivery (idempotent). Used for signals that must
/// be handled synchronously (e.g. `gui_input` -- deferred delivery would miss
/// the `set_input_as_handled` window).
pub(super) fn connect_immediate(
    target: &mut Gd<impl Inherits<Object>>,
    signal: &str,
    callable: &Callable,
) {
    let mut obj = target.clone().upcast::<Object>();
    if !obj.is_connected(signal, callable) {
        let err = obj.connect(signal, callable);
        if err != godot::global::Error::OK {
            log::warn!(
                "Failed to connect signal '{}' (immediate): {:?}",
                signal,
                err
            );
        }
    }
}

/// Idempotent disconnect. Prevents Godot's "signal not connected" error
/// when detaching from a partially-attached or already-cleaned-up editor.
pub(super) fn safe_disconnect(
    target: &mut Gd<impl Inherits<Object>>,
    signal: &str,
    callable: &Callable,
) {
    if !target.is_instance_valid() {
        return;
    }
    let mut obj = target.clone().upcast::<Object>();
    if obj.is_connected(signal, callable) {
        obj.disconnect(signal, callable);
    }
}

#[cfg(test)]
mod deferred_delivery_contract {
    //! Source-level ratchets on the rule documented on `connect_deferred`.
    //!
    //! Nothing here executes the failure it guards. `Gd<T>` cannot be
    //! constructed under `cargo test` in a cdylib, and the defect lives in
    //! gdext's generated varcall glue, which only a running Godot invokes.
    //! Scanning source instead follows the precedent in `config::sandbox`.
    //!
    //! Known limit: both scans stop at `src/plugin/`, which is sound only
    //! while `connect_deferred` (`pub(super)`) is the single route to a
    //! deferred connection. A raw `connect_flags(.., ConnectFlags::DEFERRED)`
    //! written elsewhere is invisible here. Today no `#[func]` outside
    //! `plugin/mod.rs` takes a `Gd<T>` at all, so that gap cannot currently
    //! reproduce the bug.

    /// May take `Gd<T>` because the argument is `RefCounted`. Godot copies
    /// every deferred argument into the queue as a `Variant`, and a `Variant`
    /// holding a `RefCounted` payload takes a strong reference, so the object
    /// cannot be freed before the queue drains whatever the connect flag is.
    /// These need no connect-site check, which is why the one connect site
    /// outside `src/plugin/` (`navigation/filesystem_explorer.rs`, which
    /// cannot see the `pub(super)` helper above) does not have to be scanned.
    const SAFE_BY_REFCOUNT: &[(&str, &str)] = &[
        ("handle_gui_input", "Gd<InputEvent>"),
        ("on_fs_prompt_gui_input", "Gd<InputEvent>"),
    ];

    /// May take a manually managed `Gd<T>` *only* because every connect site
    /// is immediate. `Node` is not `RefCounted`, so a queued `Variant` grants
    /// it nothing and the connect flag is the entire safety argument.
    /// `deferred_connection_sites_are_pinned` is what keeps that true.
    const SAFE_BY_IMMEDIATE_CONNECTION: &[(&str, &str)] = &[("on_child_entered_tree", "Gd<Node>")];

    /// Every file in `src/plugin/`, with its count of `connect_deferred(`
    /// occurrences. Complete because `connect_deferred` is `pub(super)`, so
    /// no file outside this directory can call it, and the module-list
    /// assertion below fails the day a tenth file appears.
    const PLUGIN_FILES: &[(&str, &str, usize)] = &[
        ("attach.rs", include_str!("attach.rs"), 6),
        ("caret_reconcile.rs", include_str!("caret_reconcile.rs"), 0),
        (
            "completion_delay.rs",
            include_str!("completion_delay.rs"),
            0,
        ),
        ("discovery.rs", include_str!("discovery.rs"), 0),
        ("floating.rs", include_str!("floating.rs"), 1),
        ("input.rs", include_str!("input.rs"), 0),
        ("lifecycle.rs", include_str!("lifecycle.rs"), 2),
        ("mod.rs", include_str!("mod.rs"), 0),
        ("outcome.rs", include_str!("outcome.rs"), 0),
        (
            "processing_guard.rs",
            include_str!("processing_guard.rs"),
            0,
        ),
        // The definition of the helper itself.
        ("signals.rs", include_str!("signals.rs"), 1),
        ("source_cmd.rs", include_str!("source_cmd.rs"), 0),
    ];

    /// Production source only, cut at the first column-0 `#[cfg(test)]`.
    ///
    /// Required, not cosmetic: this module lives in `signals.rs` and scans
    /// `signals.rs`, so without the cut its own prose about deferred
    /// connections would be counted as deferred connections. Column-0
    /// specifically, because `processing_guard.rs` has an indented
    /// `#[cfg(test)]` on a test-only method partway through its production
    /// code, and cutting there would silently drop the rest of the file.
    fn production(src: &str) -> &str {
        src.split_once("\n#[cfg(test)]")
            .map_or(src, |(head, _)| head)
    }

    /// `(name, first `Gd<..>` in the signature)` for every `#[func]` that
    /// takes one, plus the number of `#[func]` blocks parsed.
    ///
    /// Two parsing decisions are load bearing, in opposite directions.
    /// Attribute and comment lines between `#[func]` and the signature are
    /// *skipped*, never read: `mod.rs` carries `// gdext requires Gd<T> by
    /// value` in exactly that gap, and a scanner accumulating the whole block
    /// would read `Gd<` out of a comment and fail a correct patch. The
    /// signature itself is matched on `fn ` anywhere in the line and
    /// accumulated to the closing paren, never on `starts_with("fn ")` and
    /// never one line only: `#[func] pub fn ..` is idiomatic in this crate
    /// (seven uses in `ui/line_numbers.rs`) and rustfmt wraps long
    /// signatures, and either would let a `Gd<T>` parameter through silently.
    /// Anything unrecognised panics rather than being guessed at.
    fn funcs_taking_gd(src: &str) -> (Vec<(String, String)>, usize) {
        let lines: Vec<&str> = src.lines().collect();
        let (mut found, mut parsed) = (Vec::new(), 0usize);
        for (i, line) in lines.iter().enumerate() {
            if !line.trim_start().starts_with("#[func") {
                continue;
            }
            let mut j = i + 1;
            while lines.get(j).is_some_and(|l| {
                let t = l.trim_start();
                t.is_empty() || t.starts_with("#[") || t.starts_with("//")
            }) {
                j += 1;
            }
            assert!(
                lines.get(j).is_some_and(|l| l.contains("fn ")),
                "scanner lost the #[func] block at line {}: expected a signature, found {:?}. \
                 Fix the scanner rather than the source.",
                i + 1,
                lines.get(j)
            );
            parsed += 1;
            let mut signature = String::new();
            for line in &lines[j..] {
                signature.push_str(line.trim());
                if line.contains(')') {
                    break;
                }
                signature.push(' ');
            }
            let head = &signature[signature.find("fn ").unwrap() + 3..];
            let name = head.split(['(', '<']).next().unwrap_or_default().trim();
            if let Some(at) = signature.find("Gd<") {
                let ty = &signature[at..];
                let end = ty.find('>').map_or(ty.len(), |e| e + 1);
                found.push((name.to_string(), ty[..end].to_string()));
            }
        }
        (found, parsed)
    }

    /// The parser above, against the three shapes that have to work. Each was
    /// a live false reading in review: `pub fn` (seven uses in
    /// `ui/line_numbers.rs`) silently misattributes a later signature and
    /// would pass a `Gd<T>` handler; a wrapped signature hides its parameters
    /// from a one-line match; and the `Gd<T>` inside the `#[allow]` comment in
    /// `mod.rs` is read as a parameter by a block-accumulating scan.
    #[test]
    fn the_parser_reads_the_shapes_this_crate_actually_writes() {
        const SAMPLE: &str = r"
    #[func]
    pub fn takes_gd_behind_pub(&mut self, node: Gd<Node>) {}

    #[func]
    fn wrapped(
        &mut self,
        event: Gd<InputEvent>,
    ) {}

    #[func]
    #[allow(clippy::needless_pass_by_value)] // gdext requires Gd<T> by value
    fn commented(&mut self, value: i64) {}

    #[func]
    fn plain(&mut self) {}
";
        let (found, parsed) = funcs_taking_gd(SAMPLE);
        assert_eq!(parsed, 4);
        assert_eq!(
            found,
            [
                ("takes_gd_behind_pub".to_string(), "Gd<Node>".to_string()),
                ("wrapped".to_string(), "Gd<InputEvent>".to_string()),
            ]
        );
    }

    /// A `#[func]` may take a manually managed `Gd<T>` only with a recorded
    /// reason. Catches a new handler taking one, and an existing `Variant`
    /// handler being tightened back to a typed parameter.
    #[test]
    fn every_func_taking_a_godot_object_has_a_recorded_reason() {
        let (mut found, parsed) = funcs_taking_gd(production(include_str!("mod.rs")));
        assert!(
            parsed >= 20,
            "scanner parsed only {parsed} #[func] blocks in plugin/mod.rs, so it has stopped \
             tracking the file it is supposed to guard"
        );
        found.sort();
        let mut expected: Vec<(String, String)> = SAFE_BY_REFCOUNT
            .iter()
            .chain(SAFE_BY_IMMEDIATE_CONNECTION)
            .map(|(n, t)| ((*n).to_string(), (*t).to_string()))
            .collect();
        expected.sort();
        assert_eq!(
            found, expected,
            "a #[func] takes a Gd<T> that is not accounted for above. If Godot can reach it \
             through the message queue, gdext rejects a freed argument during parameter \
             conversion and the body never runs: take `Variant` and resolve with `try_to` \
             in the body, the way `on_focus_changed` does. Otherwise add it to \
             SAFE_BY_REFCOUNT or SAFE_BY_IMMEDIATE_CONNECTION with the reason."
        );
    }

    /// The set of deferred connections, pinned per file.
    ///
    /// This is the only assertion that catches the other half of the rule:
    /// flipping an existing `Gd<T>` handler from `connect_immediate` to
    /// `connect_deferred` changes no signature at all, so the scan above
    /// stays green while `SAFE_BY_IMMEDIATE_CONNECTION` quietly becomes a
    /// false claim. It is a count, so a same-file compensating edit still
    /// passes; per-file narrows that to one file.
    #[test]
    fn deferred_connection_sites_are_pinned() {
        let mut modules: Vec<&str> = production(include_str!("mod.rs"))
            .lines()
            .filter_map(|l| l.strip_prefix("mod ")?.strip_suffix(';'))
            .collect();
        modules.sort_unstable();
        let mut scanned: Vec<&str> = PLUGIN_FILES
            .iter()
            .map(|(name, _, _)| name.trim_end_matches(".rs"))
            .filter(|name| *name != "mod")
            .collect();
        scanned.sort_unstable();
        assert_eq!(
            modules, scanned,
            "src/plugin/ gained or lost a module, so the scan below no longer covers every \
             file that can call the pub(super) connect_deferred helper. Add it to PLUGIN_FILES."
        );

        for (name, src, expected) in PLUGIN_FILES {
            let actual = production(src).matches("connect_deferred(").count();
            assert_eq!(
                actual, *expected,
                "{name} now has {actual} `connect_deferred(` occurrences, not {expected}. A \
                 handler reached this way runs a frame after the signal fired, so a manually \
                 managed argument may already be freed. Confirm the handler takes `Variant`, \
                 a value type, or a RefCounted object, then update this count."
            );
        }
    }
}
