//! `:panelmap` — the introspector.
//!
//! It ships in the same commit as the dispatcher cutover, and that is not a
//! scheduling nicety. The cutover replaces a fixed keyset with a resolution
//! *model*: forest depth, probe order, seals, a capability gate, an
//! arbitration seam and two consumption policies. Every one of those can make
//! a key silently do nothing, and a config surface with no way to ask "why is
//! my key dead?" turns each of them into an unanswerable bug report.
//!
//! Two tiers live here, both pure string builders over the plane:
//!
//! - [`list_report`] — every live rule, grouped by surface in forest order,
//!   in the exact syntax a user would type back into a vimrc.
//! - [`explain_report`] — one key, resolved: the sampled chain, the surface
//!   stack, which probe matched, which rule won and on which surface, which
//!   gate fired if none did, and what the consumption policy would do with
//!   the outcome.
//!
//! Neither takes a `Gd<T>`, so both are golden-snapshot testable with no
//! editor running. The transport is `PendingUiAction::PanelCommand`, and the
//! report goes to the Output panel through `godot_print!` rather than to the
//! one-line status bar — a resolution trace is a dozen lines and the default
//! `log` level is Off.
//!
//! See `DESIGN-rebindable-nav.md` (design notes, kept outside this repo) §5.9 and the `P6` block in §10.

use std::fmt::Write as _;

use vim_core::keymap::{KeyEvent, MappingOwner};

use super::action::{ActionRegistry, RuleTarget};
use super::bind::{BindingIndex, Consumption, PanelDiagnostic, Repeat, Rule};
use super::keys::{parse_lhs, Probes};
use super::resolve::{resolve, CandidateTarget, Resolution, ResolveInput, Stop};
use super::surface::{Anchor, FocusChain, Seal, SurfacePath};

/// Render one rule the way a user would write it.
///
/// Round-trippable on purpose: the listing is not a pretty-printer, it is the
/// text you paste into a vimrc to reproduce what you are looking at. That is
/// the same anti-drift device the provider defaults use — one grammar, one
/// parser, one spelling.
pub(crate) fn render_rule(rule: &Rule, registry: &ActionRegistry) -> String {
    let mut out = String::from("panelmap");
    if rule.physical {
        out.push_str(" <physical>");
    }
    if rule.consume == Consumption::Void {
        out.push_str(" <void>");
    }
    if rule.repeat == Repeat::Suppress {
        out.push_str(" <norepeat>");
    }
    if rule.shift_tolerant {
        out.push_str(" <shift>");
    }
    if rule.nowait {
        out.push_str(" <nowait>");
    }
    let lhs: String = rule.lhs.iter().map(KeyEvent::to_vim_notation).collect();
    let target = match &rule.target {
        RuleTarget::Action(id) => registry
            .name_of(*id)
            .unwrap_or("<unregistered>")
            .to_string(),
        RuleTarget::Native => "native".to_string(),
        RuleTarget::Shortcut(path) => format!("<Shortcut>({path})"),
    };
    let _ = write!(out, " {} {lhs} {target}", rule.surface);
    for (key, value) in rule.params.iter() {
        let _ = write!(out, " {key}={value}");
    }
    out
}

/// Who installed a rule, for the "why did this one win" column.
fn owner_label(owner: &MappingOwner) -> String {
    match owner {
        MappingOwner::Host(tag) => tag.to_string(),
        MappingOwner::User => "user".to_string(),
        other => format!("{other:?}"),
    }
}

/// `:panelmap` with no arguments — every live binding, by surface, and every
/// line of the user's config that did not become one.
///
/// Surfaces are listed in forest (probe) order, and rules within a surface in
/// slot-allocation order, because both are deterministic and neither is a
/// hash iteration. The introspector's golden snapshots depend on that, and so
/// does a user diffing two runs.
///
/// `diagnostics` is the residue of `bind::apply_text` over the vimrc. It is a
/// parameter rather than something read off the index because a rejected line
/// installs nothing — there is no rule to hang it on, which is precisely why
/// it was invisible: the only other place it went was a `log::warn!`, and the
/// default Log Level is Off. A rejected binding the user cannot read about is
/// the same as a silent dead key.
pub(crate) fn list_report(
    index: &BindingIndex,
    registry: &ActionRegistry,
    diagnostics: &[PanelDiagnostic],
) -> String {
    let mut out = String::new();
    out.push_str("--- panel bindings ---\n");
    let mut total = 0_usize;
    for surface in index.forest().ids() {
        let rules: Vec<&Rule> = index.rules_on(surface).collect();
        let spec = index.forest().get(surface);
        let seal = spec.map_or(Seal::Open, |s| s.seal);
        if rules.is_empty() {
            // Printed anyway: "this surface exists and binds nothing" is the
            // answer to half the questions the listing is asked.
            let note = match seal {
                Seal::Barrier => "  (barrier — takes no bindings)",
                _ => "  (no bindings)",
            };
            let _ = writeln!(out, "{surface}{note}");
            continue;
        }
        let header = match spec.and_then(|s| s.overlay.as_ref()) {
            Some(o) => format!("(overlay: {}, seal: {seal:?})", o.when),
            None => format!(
                "(parent: {}, seal: {seal:?})",
                spec.and_then(|s| s.parent).unwrap_or("-")
            ),
        };
        let _ = writeln!(out, "{surface}  {header}");
        for rule in rules {
            total += 1;
            let _ = writeln!(
                out,
                "  {}    [{}]",
                render_rule(rule, registry),
                owner_label(&rule.owner)
            );
        }
        // Reservations, printed under the surface that owns them.
        //
        // Not decoration: binding a multi-key LHS implicitly takes the bare
        // first key on this surface, and a reservation nobody can see is
        // exactly the silent dead key the introspector exists to prevent.
        for key in index.reservations(surface) {
            let sequences: Vec<String> = index
                .sequences_from(surface, key)
                .map(|rule| rule.lhs.iter().map(KeyEvent::to_vim_notation).collect())
                .collect();
            let _ = writeln!(
                out,
                "  reserves {}    (consumed bare on {surface}, then waits timeoutlen for: {})",
                key.to_vim_notation(),
                sequences.join(", ")
            );
        }
    }
    let _ = writeln!(out, "--- {total} binding(s) ---");
    if !diagnostics.is_empty() {
        let _ = writeln!(
            out,
            "--- {} rejected line(s) — these bound NOTHING ---",
            diagnostics.len()
        );
        for diagnostic in diagnostics {
            let _ = writeln!(out, "  {diagnostic}");
        }
    }
    out
}

/// `:panelmap <lhs>` — resolve one key against the live chain and explain it.
///
/// The report answers the five questions the model can silently fail on, in
/// the order the model asks them: where am I, what is above me, which
/// interpretation of the key matched, which rule claimed it, and what happens
/// to the event afterwards.
pub(crate) fn explain_report(
    lhs: &str,
    chain: &FocusChain,
    path: &SurfacePath,
    overlay: Option<&SurfacePath>,
    index: &BindingIndex,
    registry: &ActionRegistry,
    vim_claims: &dyn Fn(KeyEvent) -> bool,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "--- panelmap {lhs} ---");

    let keys = match parse_lhs(lhs) {
        Ok(keys) => keys,
        Err(err) => {
            let _ = writeln!(out, "cannot parse '{lhs}': {err}");
            return out;
        }
    };
    let Some(&first) = keys.first() else {
        let _ = writeln!(out, "cannot parse '{lhs}': empty key sequence");
        return out;
    };
    if keys.len() > 1 {
        // A sequence is resolved one keystroke at a time against a pending
        // buffer (§5.10), and the *first* key is where the interesting
        // decision happens: reserved or not. Replaying the rest against a
        // buffer that does not exist would be inventing an answer, so the
        // report says which key it answered for and prints the reservation
        // below.
        let _ = writeln!(
            out,
            "a sequence resolves one keystroke at a time against the pending buffer; \
             explaining the first key only"
        );
    }
    let key = first;

    // A synthesized probe list: exactly the as-typed interpretation. A real
    // keystroke may carry two more (a Latin collapse and a US-QWERTY
    // position), which is stated rather than faked — the plane has no way to
    // know which layout the user is on from a written `<C-h>`.
    let probes = Probes::from_key(key);

    // ── Where am I ───────────────────────────────────────────────────
    let _ = writeln!(out, "focus chain (leaf first):");
    if chain.nodes.is_empty() {
        let _ = writeln!(out, "  <no focus owner>");
    } else {
        for (i, node) in chain.nodes.iter().enumerate() {
            let _ = writeln!(out, "  [{i}] {} ({})", node.class, node.name);
        }
    }
    let _ = writeln!(
        out,
        "facts: attached_editor={:?} mode={:?} in_filesystem_dock={} sibling_nav={} prompt={}",
        chain.attached_editor,
        chain.editor_mode,
        chain.in_filesystem_dock,
        chain.sibling_nav_control.is_some(),
        chain.is_plugin_prompt
    );

    // ── What this report structurally cannot know ────────────────────
    //
    // `Probes::from_key` builds ONE as-typed probe with the positional index
    // unset, so the walk below runs against a one-element list while a real
    // keystroke carries up to three. Until this line existed the caveat was a
    // source comment addressed to the maintainer and the user was told
    // nothing, which on a non-QWERTY layout means the report answers a
    // different question than the one the user asked. Printed once, above the
    // walk, because it hedges the probe list and not any one path.
    let probe_list: Vec<String> = probes
        .iter()
        .map(|probe| probe.to_vim_notation().into_owned())
        .collect();
    let _ = writeln!(
        out,
        "probes: {} (as typed only — a real keystroke on a non-QWERTY layout also carries a \
         Latin collapse and a US-QWERTY position, which a written LHS cannot derive)",
        probe_list.join(", ")
    );

    explain_on_path(&mut out, path, &probes, index, registry, vim_claims);

    // ── The counterfactual overlay ───────────────────────────────────
    //
    // The live path can never be a completion path: the command line owns
    // focus while `:panelmap` is typed, so the popup is already cancelled.
    // The report answers anyway, through the same `Forest::overlay` and the
    // same walk, with one input flipped, and says plainly that it is a
    // counterfactual.
    if let Some(overlay_path) = overlay {
        let id = overlay_path.ids.first().copied().unwrap_or("<overlay>");
        let when = index
            .forest()
            .get(id)
            .and_then(|s| s.overlay.as_ref())
            .map_or("", |o| o.when);
        let _ = writeln!(
            out,
            "\n--- if the script editor had focus in insert mode with the completion popup open ---"
        );
        let _ = writeln!(
            out,
            "overlay '{id}' ({when}) is not on the path this report sampled, and it cannot be \
             while the command line has focus. This is how the key would resolve if it were:"
        );
        explain_on_path(&mut out, overlay_path, &probes, index, registry, vim_claims);
    }
    out
}

/// The walk half of the explainer: surface stack, candidates and verdict for
/// ONE path. Extracted so the live path and the counterfactual overlay render
/// through the same code, which is the whole honesty argument.
fn explain_on_path(
    out: &mut String,
    path: &SurfacePath,
    probes: &Probes,
    index: &BindingIndex,
    registry: &ActionRegistry,
    vim_claims: &dyn Fn(KeyEvent) -> bool,
) {
    // ── What is above me ─────────────────────────────────────────────
    let _ = writeln!(
        out,
        "surface stack (deepest first): {}",
        path.ids.join(" -> ")
    );
    let anchor = match path.anchor {
        Anchor::Node(i) => format!("chain[{i}]"),
        Anchor::Rootless => "rootless (no focus owner)".to_string(),
    };
    let _ = writeln!(
        out,
        "anchor: {anchor}  caps: {:?}  seal: {:?}  yields_to_engine: {}  refuses_positional: {}",
        path.caps, path.seal, path.anchor_yields_to_engine, path.anchor_refuses_positional
    );

    // ── Candidates, surface by surface ───────────────────────────────
    let _ = writeln!(out, "candidates:");
    for &surface in &path.ids {
        let positional = !path.anchor_refuses_positional && index.has_physical_rule(surface);
        let mut any = false;
        for probe in probes.iter_scoped(positional) {
            // Reservation first, because it is what happens FIRST at dispatch:
            // a reserved key never reaches the single-key walk below, so
            // printing the rule without the reservation would explain the
            // wrong half of the behaviour.
            if index.is_reserved(surface, probe) {
                any = true;
                let sequences: Vec<String> = index
                    .sequences_from(surface, probe)
                    .map(|rule| rule.lhs.iter().map(KeyEvent::to_vim_notation).collect())
                    .collect();
                let _ = writeln!(
                    out,
                    "  {surface}: RESERVED — {} is consumed bare here and the plugin waits \
                     timeoutlen for: {}",
                    probe.to_vim_notation(),
                    sequences.join(", ")
                );
            }
            let Some(rule) = index.rule_for(surface, &[probe]) else {
                continue;
            };
            any = true;
            let verdict = match &rule.target {
                RuleTarget::Action(id) => registry.get(*id).map_or_else(
                    || "SKIPPED (action not registered)".to_string(),
                    |spec| {
                        if path.caps.satisfies(spec.requires) {
                            format!("eligible ({})", spec.id)
                        } else {
                            format!(
                                "SKIPPED — {} needs {:?}, path offers {:?}",
                                spec.id, spec.requires, path.caps
                            )
                        }
                    },
                ),
                RuleTarget::Native => "native — hands the key back and stops the walk".to_string(),
                // Not "eligible". `run_candidate` declines every `<Shortcut>`
                // unconditionally, so a report that called this eligible was
                // actively confirming a key that can never fire. Registration
                // refuses such a rule now, so this arm is only reachable for a
                // rule installed through `upsert` — but the introspector must
                // not be the one that lies about it.
                RuleTarget::Shortcut(p) => {
                    format!("NOT DISPATCHED — editor shortcut {p} always declines")
                }
            };
            let _ = writeln!(
                out,
                "  {surface}: {} via {} -> {verdict}",
                render_rule(rule, registry),
                probe.to_vim_notation()
            );
        }
        if !any {
            let _ = writeln!(out, "  {surface}: no rule for this key");
        }
        // The other half of the same honesty. A `<physical>` rule whose LHS is
        // a single character is reachable from whichever key sits at that
        // character's US-QWERTY position — on Dvorak the QWERTY-J position
        // emits `c`, so `c` in the FileSystem dock is `godotvim.fs.delete`'s
        // neighbour, and a user asking about `c` needs to see that. Named keys
        // are excluded because their position does not move between layouts.
        //
        // Gated on `positional`, which is already `!refuses_positional &&
        // has_physical_rule`: inside the attached editor probe 3 is withheld
        // from the whole walk, so claiming positional reachability there would
        // be a new lie replacing the old one.
        if positional {
            let shadows: Vec<String> = index
                .rules_on(surface)
                .filter(|rule| {
                    rule.physical
                        && rule.lhs.len() == 1
                        && matches!(rule.lhs[0].key(), vim_core::keymap::Key::Char(_))
                })
                .map(|rule| {
                    format!(
                        "{} -> {}",
                        rule.lhs[0].to_vim_notation(),
                        match &rule.target {
                            RuleTarget::Action(id) => registry
                                .name_of(*id)
                                .unwrap_or("<unregistered>")
                                .to_string(),
                            RuleTarget::Native => "native".to_string(),
                            RuleTarget::Shortcut(p) => format!("<Shortcut>({p})"),
                        }
                    )
                })
                .collect();
            if !shadows.is_empty() {
                let _ = writeln!(
                    out,
                    "  {surface}: reachable positionally from another physical key on \
                     non-QWERTY layouts: {}",
                    shadows.join(", ")
                );
            }
        }
    }

    // ── The verdict ──────────────────────────────────────────────────
    let resolution = resolve(&ResolveInput {
        probes,
        path,
        index,
        registry,
        vim_claims,
    });
    match resolution {
        Resolution::Run {
            matched,
            candidates,
        } => {
            let _ = writeln!(out, "matched: {}", matched.to_vim_notation());
            for candidate in &candidates {
                let name = match &candidate.target {
                    CandidateTarget::Action(_, spec) => spec.id.to_string(),
                    CandidateTarget::Shortcut(p) => {
                        format!("<Shortcut>({p}) — NOT DISPATCHED, always declines")
                    }
                };
                let consumption = match candidate.consume {
                    Consumption::Void => {
                        "consumed ALWAYS (void) — even if the action declines, and the walk stops"
                    }
                    Consumption::Elastic => {
                        "consumed only if the action accepts (elastic); otherwise Godot gets the key"
                    }
                    Consumption::Handoff => {
                        "not consumed, and the vim engine is skipped, so the editor's own \
                         handling gets this key (handoff)"
                    }
                };
                let _ = writeln!(out, "runs: {name} on '{}'", candidate.surface);
                let _ = writeln!(out, "consumption: {consumption}");
                if candidate.repeat == Repeat::Suppress {
                    let _ = writeln!(
                        out,
                        "repeat: key-repeat echoes are consumed WITHOUT running the action"
                    );
                }
            }
        }
        Resolution::None(stop) => {
            let _ = writeln!(out, "nothing runs, and the key is NOT consumed.");
            let _ = writeln!(out, "reason: {}", explain_stop(stop));
        }
    }
}

/// One sentence per stop reason, each naming the fix.
fn explain_stop(stop: Stop) -> String {
    match stop {
        Stop::Barrier => {
            "the anchor surface is a barrier (a foreign text input, or the editor in an \
             insert-like mode). Nothing is ever intercepted there."
                .to_string()
        }
        Stop::Sealed(surface) => format!(
            "'{surface}' is sealed and this key carries no Ctrl/Alt/Meta, so it stops at the \
             anchor and reaches the control's own input handling. A modifier-bearing key would \
             have continued to the forest root."
        ),
        Stop::Native(surface) => format!(
            "'{surface}' binds this key to `native`, which hands it back to Godot and terminates \
             the walk. `panelunmap` is the other verb: it removes the rule and lets the walk \
             continue to the parent surface."
        ),
        Stop::Yielded(key) => format!(
            "the script editor has focus and the Vim engine claims {} — your own `:map` wins. \
             Remove the mapping, or bind the panel action to a different chord.",
            key.to_vim_notation()
        ),
        Stop::Exhausted => "no surface on the stack binds this key, or every candidate was \
             gated out by a missing capability (see above)."
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::Params;
    use crate::actions::bind::builtin_index;
    use crate::actions::caps::Caps;
    use crate::actions::providers;
    use crate::actions::specs;
    use crate::actions::surface::fixtures::{code_edit, id, line_edit, plain, tree};
    use vim_core::keymap::{Key as VimKey, Modifiers};

    /// The whole shipped registry — `specs::SHIPPED` **plus** every
    /// `Provider::actions` table. Looping `SHIPPED` alone here would leave a
    /// provider's own verbs unregistered, and `builtin_index` would then
    /// reject that provider's defaults with `UnknownAction` — a
    /// `debug_assert!` under `Provenance::Builtin`, so the failure is loud
    /// but the cause reads as unrelated.
    fn registry() -> ActionRegistry {
        specs::registry()
    }

    const NEVER: &dyn Fn(KeyEvent) -> bool = &|_| false;
    const ALWAYS: &dyn Fn(KeyEvent) -> bool = &|_| true;

    fn explain(lhs: &str, chain: &FocusChain, claims: &dyn Fn(KeyEvent) -> bool) -> String {
        let reg = registry();
        let index = builtin_index(&reg);
        let path = providers::forest().classify(chain).expect("total probe");
        explain_report(lhs, chain, &path, None, &index, &reg, claims)
    }

    /// The report the plugin builds for `:panelmap {keys}`: the live path
    /// plus the counterfactual overlay, popup open.
    fn explain_counterfactual(lhs: &str, chain: &FocusChain) -> String {
        let reg = registry();
        let index = builtin_index(&reg);
        let path = providers::forest().classify(chain).expect("total probe");
        let overlay = index
            .forest()
            .overlay(&crate::actions::surface::OverlayFacts {
                at_attached_editor: true,
                mode: Some(vim_core::primitives::Mode::Insert),
                selected_index: 0,
                caret: (0, 0),
            })
            .expect("the hypothetical facts activate the overlay");
        explain_report(lhs, chain, &path, Some(&overlay), &index, &reg, NEVER)
    }

    fn fs_chain() -> FocusChain {
        FocusChain {
            nodes: vec![
                tree("FileSystemTree", 1),
                plain("VBoxContainer", 2),
                plain("FileSystemDock", 3),
            ],
            in_filesystem_dock: true,
            ..Default::default()
        }
    }

    /// What `plugin::input::run_candidate` can actually *do* with a target,
    /// modelled arm for arm.
    ///
    /// A stand-in rather than the real thing because `run_candidate` takes
    /// `Gd<Control>` and cannot be constructed under `cargo test` in a
    /// cdylib. Exhaustive on purpose: a fourth `RuleTarget` added without a
    /// decision here fails to compile, which is the only way this stays
    /// honest.
    const fn dispatcher_can_fire(target: &RuleTarget) -> bool {
        match target {
            // Built into an `ActionCtx` and run.
            RuleTarget::Action(_) => true,
            // Terminates the walk before `run_candidate` is reached at all —
            // deliberately, and the report says so in its own words.
            RuleTarget::Native => false,
            // `log::warn!` (Log Level defaults to Off) then
            // `Outcome::Declined`, unconditionally.
            RuleTarget::Shortcut(_) => false,
        }
    }

    fn explain_with(
        index: &BindingIndex,
        lhs: &str,
        chain: &FocusChain,
        claims: &dyn Fn(KeyEvent) -> bool,
    ) -> String {
        let reg = registry();
        let path = providers::forest().classify(chain).expect("total probe");
        explain_report(lhs, chain, &path, None, index, &reg, claims)
    }

    #[test]
    fn the_report_calls_a_key_eligible_exactly_when_the_dispatcher_can_fire_it() {
        // The anti-drift pair. `<Shortcut>` used to print as
        // "eligible (editor shortcut …)" while `run_candidate` declined it
        // unconditionally — the introspector actively confirming a dead key,
        // which is the exact failure the introspector exists to prevent.
        // Installed through `upsert` rather than `try_insert` because
        // registration now refuses the undispatched targets; the report must
        // still be honest about a rule that reached the index some other way.
        let reg = registry();
        let targets = [
            RuleTarget::Action(reg.id_of("godotvim.item.next").expect("shipped verb")),
            RuleTarget::Native,
            RuleTarget::Shortcut("filesystem_dock/rename".into()),
        ];
        for target in targets {
            let mut index = builtin_index(&reg);
            index.upsert(Rule {
                surface: "dock",
                lhs: vec![KeyEvent::new(VimKey::Char('q'), Modifiers::NONE)],
                target: target.clone(),
                params: Params::new(),
                consume: Consumption::Elastic,
                repeat: Repeat::Allow,
                physical: false,
                shift_tolerant: false,
                nowait: false,
                owner: MappingOwner::User,
                desc: "under test".into(),
            });
            let report = explain_with(&index, "q", &fs_chain(), NEVER);
            assert_eq!(
                report.contains("eligible"),
                dispatcher_can_fire(&target),
                "the report and the dispatcher disagree about {target:?}:\n{report}"
            );
        }
    }

    fn editor_chain(mode: Option<vim_core::primitives::Mode>) -> FocusChain {
        FocusChain {
            nodes: vec![code_edit(7), plain("CodeTextEditor", 8)],
            attached_editor: Some(id(7)),
            editor_mode: mode,
            ..Default::default()
        }
    }

    // ── The listing ──────────────────────────────────────────────────

    #[test]
    fn the_listing_is_a_golden_snapshot_of_the_shipped_keyset() {
        let reg = registry();
        let report = list_report(&builtin_index(&reg), &reg, &[]);
        insta_like(&report);
    }

    /// Asserted literally rather than through a snapshot crate: the shipped
    /// keyset is thirty-one rules and a hand-written expectation is reviewable
    /// in a diff, which is the same argument the provider array makes.
    ///
    /// `dock.debugger` appearing between `dock.filesystem` and `dock` is not
    /// cosmetic — the listing walks `PROVIDERS` order, which *is* the probe
    /// order, so a child printed after its parent here would be a child that
    /// never classifies.
    fn insta_like(report: &str) {
        let expected = "\
--- panel bindings ---
editor.nav  (no bindings)
editor.insert  (barrier — takes no bindings)
editor.completion  (overlay: while the script editor is in an insert-like mode, seal: Open)
  panelmap editor.completion <C-@> godotvim.completion.trigger    [godotvim.completion]
  panelmap editor.completion <C-n> godotvim.completion.next    [godotvim.completion]
  panelmap editor.completion <C-p> godotvim.completion.prev    [godotvim.completion]
  panelmap editor.completion <C-y> godotvim.completion.confirm require_selection=0    [godotvim.completion]
  panelmap editor.completion <C-e> godotvim.completion.dismiss    [godotvim.completion]
  panelmap editor.completion <Tab> godotvim.completion.confirm    [godotvim.completion]
  panelmap editor.completion <CR> godotvim.completion.confirm    [godotvim.completion]
  panelmap editor.completion <Up> godotvim.completion.navigate    [godotvim.completion]
  panelmap editor.completion <Down> godotvim.completion.navigate    [godotvim.completion]
prompt  (no bindings)
searchbox  (parent: panel, seal: Sealed)
  panelmap <shift> searchbox <CR> godotvim.search.accept    [godotvim.searchbox]
  panelmap <shift> searchbox <Esc> godotvim.search.accept    [godotvim.searchbox]
dock.filesystem  (parent: dock, seal: Open)
  panelmap <physical> dock.filesystem a godotvim.fs.create    [godotvim.filesystem]
  panelmap <physical> dock.filesystem d godotvim.fs.delete    [godotvim.filesystem]
  panelmap <physical> dock.filesystem r godotvim.fs.rename    [godotvim.filesystem]
  panelmap <physical> dock.filesystem y godotvim.fs.yank_path    [godotvim.filesystem]
  panelmap <physical> dock.filesystem R godotvim.fs.refresh    [godotvim.filesystem]
dock.debugger  (parent: dock, seal: Open)
  panelmap dock.debugger J godotvim.debugger.frame_next    [godotvim.debugger]
  panelmap dock.debugger K godotvim.debugger.frame_prev    [godotvim.debugger]
  panelmap dock.debugger G godotvim.debugger.frame_last    [godotvim.debugger]
  panelmap dock.debugger y godotvim.debugger.yank_frame    [godotvim.debugger]
dock  (parent: panel, seal: Open)
  panelmap <physical> dock h godotvim.item.collapse    [godotvim.dock]
  panelmap <physical> dock j godotvim.item.next    [godotvim.dock]
  panelmap <physical> dock k godotvim.item.prev    [godotvim.dock]
  panelmap <physical> dock l godotvim.item.expand    [godotvim.dock]
  panelmap <physical> dock / godotvim.dock.search    [godotvim.dock]
  panelmap dock <CR> godotvim.item.activate    [godotvim.dock]
  panelmap dock <Esc> godotvim.focus.editor    [godotvim.dock]
foreign  (barrier — takes no bindings)
unknown  (no bindings)
panel  (parent: -, seal: Open)
  panelmap <physical> <void> <norepeat> panel <C-h> godotvim.focus.left    [godotvim.panel]
  panelmap <physical> <void> <norepeat> panel <C-j> godotvim.focus.down    [godotvim.panel]
  panelmap <physical> <void> <norepeat> panel <C-k> godotvim.focus.up    [godotvim.panel]
  panelmap <physical> <void> <norepeat> panel <C-l> godotvim.focus.right    [godotvim.panel]
--- 31 binding(s) ---
";
        assert_eq!(report, expected);
    }

    #[test]
    fn every_listed_line_parses_back() {
        // The listing is the config syntax, not a pretty-printer. If this
        // ever fails, `:panelmap` is telling users to write something the
        // parser rejects.
        let reg = registry();
        let index = builtin_index(&reg);
        for rule in index.rules() {
            let line = render_rule(rule, &reg);
            let parsed = crate::config::panelmap::parse_panel_line(&line);
            assert!(parsed.is_ok(), "'{line}' does not parse back: {parsed:?}");
        }
    }

    // ── The explainer ────────────────────────────────────────────────

    #[test]
    fn it_explains_a_key_that_wins_on_the_deepest_surface() {
        let report = explain("d", &fs_chain(), NEVER);
        assert!(
            report.contains("dock.filesystem -> dock -> panel"),
            "{report}"
        );
        assert!(
            report.contains("runs: godotvim.fs.delete on 'dock.filesystem'"),
            "{report}"
        );
        assert!(report.contains("elastic"), "{report}");
    }

    #[test]
    fn it_explains_a_key_that_falls_through_to_a_parent() {
        let report = explain("j", &fs_chain(), NEVER);
        assert!(
            report.contains("dock.filesystem: no rule for this key"),
            "{report}"
        );
        assert!(
            report.contains("runs: godotvim.item.next on 'dock'"),
            "{report}"
        );
    }

    #[test]
    fn it_names_the_capability_that_gated_a_candidate_out() {
        // The single most useful line in the whole report: `l` on an ItemList
        // looks identical to `l` being unbound unless the gate is printed.
        let chain = FocusChain {
            nodes: vec![
                crate::actions::surface::fixtures::item_list("ItemList", 1),
                plain("VBoxContainer", 2),
            ],
            ..Default::default()
        };
        let report = explain("l", &chain, NEVER);
        assert!(report.contains("SKIPPED"), "{report}");
        assert!(report.contains("HIERARCHY"), "{report}");
        assert!(report.contains("NOT consumed"), "{report}");
    }

    #[test]
    fn it_names_the_arbitration_seam_when_the_engine_wins() {
        let report = explain(
            "<C-h>",
            &editor_chain(Some(vim_core::primitives::Mode::Normal)),
            ALWAYS,
        );
        assert!(report.contains("your own `:map` wins"), "{report}");
    }

    #[test]
    fn it_names_the_barrier_in_an_insert_like_mode() {
        for mode in [
            vim_core::primitives::Mode::Insert,
            vim_core::primitives::Mode::Replace,
            vim_core::primitives::Mode::CommandLine,
        ] {
            let report = explain("<C-h>", &editor_chain(Some(mode)), NEVER);
            assert!(report.contains("barrier"), "{mode:?}: {report}");
        }
    }

    #[test]
    fn it_names_the_seal_for_a_bare_key_in_a_filter_box() {
        let chain = FocusChain {
            nodes: vec![line_edit(1), plain("VBoxContainer", 2)],
            sibling_nav_control: Some(id(9)),
            ..Default::default()
        };
        let report = explain("x", &chain, NEVER);
        assert!(report.contains("sealed"), "{report}");
        // …and the same surface passes a chord straight through to `panel`.
        let chord = explain("<C-l>", &chain, NEVER);
        assert!(
            chord.contains("runs: godotvim.focus.right on 'panel'"),
            "{chord}"
        );
    }

    #[test]
    fn it_reports_the_void_policy_on_the_panel_chords() {
        let chain = FocusChain::default();
        let report = explain("<C-h>", &chain, NEVER);
        assert!(report.contains("<no focus owner>"), "{report}");
        assert!(report.contains("rootless"), "{report}");
        assert!(report.contains("consumed ALWAYS (void)"), "{report}");
        assert!(
            report.contains("echoes are consumed WITHOUT running"),
            "{report}"
        );
    }

    // ── Layout honesty (§5.3) ────────────────────────────────────────

    #[test]
    fn the_report_says_its_probe_list_is_as_typed_only() {
        // `Probes::from_key` builds ONE probe with the positional index unset,
        // so the walk runs against a one-element list while a real keystroke
        // carries up to three. That was acknowledged only in a source comment
        // addressed to the maintainer; on Colemak the user was simply given a
        // confident answer to a question they did not ask.
        let report = explain("j", &fs_chain(), NEVER);
        assert!(report.contains("probes: j (as typed only"), "{report}");
        assert!(report.contains("Latin collapse"), "{report}");
        assert!(report.contains("US-QWERTY position"), "{report}");
        assert!(
            report.contains("a written LHS cannot derive"),
            "the caveat must name WHY the list is short: {report}"
        );
    }

    #[test]
    fn the_caveat_renders_the_probe_in_the_notation_the_user_typed() {
        // Not a fixed string: the line has to name the key it is hedging
        // about, or a user reading two reports side by side cannot tell them
        // apart.
        for (lhs, rendered) in [("<C-h>", "probes: <C-h> "), ("<CR>", "probes: <CR> ")] {
            let report = explain(lhs, &fs_chain(), NEVER);
            assert!(report.contains(rendered), "{lhs}: {report}");
        }
    }

    #[test]
    fn the_report_names_the_physical_rules_that_could_shadow_a_key() {
        // The half a caveat alone cannot give: WHICH other key lands here. On
        // Dvorak the QWERTY-D position emits `e`, so `e` in the FileSystem
        // dock is `godotvim.fs.delete` — and `:panelmap e` would otherwise say
        // "no surface on the stack binds this key".
        let report = explain("j", &fs_chain(), NEVER);
        assert!(
            report.contains(
                "dock.filesystem: reachable positionally from another physical key on \
                 non-QWERTY layouts:"
            ),
            "{report}"
        );
        for rule in [
            "a -> godotvim.fs.create",
            "d -> godotvim.fs.delete",
            "r -> godotvim.fs.rename",
            "y -> godotvim.fs.yank_path",
            "R -> godotvim.fs.refresh",
        ] {
            assert!(report.contains(rule), "{rule} missing from: {report}");
        }
        // …and the parent's own physical rules, which are equally reachable.
        assert!(report.contains("j -> godotvim.item.next"), "{report}");
        assert!(report.contains("<C-h> -> godotvim.focus.left"), "{report}");
    }

    #[test]
    fn a_surface_that_refuses_positional_claims_no_positional_reachability() {
        // `editor.nav` withholds probe 3 from the WHOLE walk, `panel`
        // included, so `panel`'s `<physical>` chords are not positionally
        // reachable from inside the attached editor. Printing the note there
        // would replace the old lie with a new one — a Dvorak user would be
        // told `Ctrl+d` might be panel-left, which is exactly the outcome
        // `refuses_positional` exists to prevent.
        let report = explain(
            "<C-h>",
            &editor_chain(Some(vim_core::primitives::Mode::Normal)),
            NEVER,
        );
        assert!(
            report.contains("refuses_positional: true"),
            "fixture must be on the surface that refuses: {report}"
        );
        assert!(!report.contains("reachable positionally"), "{report}");
        // The caveat itself still prints: the probe list is short there too.
        assert!(report.contains("probes: <C-h> "), "{report}");
    }

    /// Every "reachable positionally" line in a report, joined.
    ///
    /// A helper rather than a `contains` over the whole report, because the
    /// negative assertions below have to be scoped to these lines: `<Tab>` and
    /// `q` both appear elsewhere in a report that binds them.
    fn shadow_lines(report: &str) -> String {
        report
            .lines()
            .filter(|l| l.contains("reachable positionally"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_named_key_is_never_listed_as_positionally_reachable() {
        // A named key sits at the same physical position on every layout, so
        // listing it would be noise that dilutes the real warning. Asserted
        // against a rule that carries `<physical>` AND a named LHS, because no
        // SHIPPED rule is both — which is exactly why dropping the `Char(_)`
        // arm survived until this test existed.
        let index = index_with("panelmap <physical> dock <Tab> godotvim.item.activate");
        let report = explain_with(&index, "j", &fs_chain(), NEVER);
        let shadows = shadow_lines(&report);
        assert!(!shadows.is_empty(), "{report}");
        assert!(
            !shadows.contains("<Tab>"),
            "a named key has no layout-dependent position:\n{shadows}"
        );
        // …while the physical single-Char rules on the same surface still are.
        assert!(shadows.contains("j -> godotvim.item.next"), "{shadows}");
    }

    #[test]
    fn a_rule_without_physical_is_never_listed_as_positionally_reachable() {
        // `<physical>` is what admits probe 3 for a rule at all. A bare
        // `panelmap dock q …` is reachable only by the key the user actually
        // typed, so claiming otherwise would send a Colemak user hunting for a
        // conflict that does not exist. No shipped rule on this path is a
        // single Char without `<physical>`, so the flag check was unpinned.
        let index = index_with("panelmap dock q godotvim.item.activate");
        let report = explain_with(&index, "j", &fs_chain(), NEVER);
        let shadows = shadow_lines(&report);
        assert!(!shadows.is_empty(), "{report}");
        assert!(
            !shadows.contains("q -> godotvim.item.activate"),
            "a rule without <physical> is not positionally reachable:\n{shadows}"
        );
    }

    #[test]
    fn the_hedging_belongs_to_the_explainer_and_not_the_listing() {
        // The golden snapshot is the shipped keyset's contract and a caveat in
        // it would be re-asserted thirty times for no gain.
        let reg = registry();
        let report = list_report(&builtin_index(&reg), &reg, &[]);
        assert!(!report.contains("as typed only"), "{report}");
        assert!(!report.contains("reachable positionally"), "{report}");
    }

    #[test]
    fn it_refuses_an_unparseable_key_instead_of_guessing() {
        // Nine keys: over `MAX_KEY_SEQUENCE_LEN`, so `parse_lhs` rejects it
        // rather than silently truncating into a binding nobody wrote.
        let report = explain("abcdefghi", &fs_chain(), NEVER);
        assert!(report.contains("cannot parse"), "{report}");
        assert!(
            !report.contains("surface stack"),
            "a report that could not parse its key must not pretend to resolve: {report}"
        );
    }

    #[test]
    fn it_says_so_rather_than_guessing_at_a_sequence() {
        // `<C-` is not an error to vim-core's parser — it is four literal
        // keys. Explaining a sequence needs the pending buffer, which the
        // shell plane does not have yet, so the report says which key it
        // answered for instead of inventing a walk.
        let report = explain("<C-", &fs_chain(), NEVER);
        assert!(report.contains("first key only"), "{report}");
    }

    #[test]
    fn it_reports_an_unbound_key_as_unbound() {
        let report = explain("q", &fs_chain(), NEVER);
        assert!(
            report.contains("no surface on the stack binds this key"),
            "{report}"
        );
    }

    #[test]
    fn caps_are_printed_so_a_gate_is_diagnosable() {
        let report = explain("d", &fs_chain(), NEVER);
        // The path's caps must be printed in full: a gate the user cannot see
        // is indistinguishable from an unbound key.
        for bit in ["VNAV", "HIERARCHY", "ACTIVATE", "FILEOPS"] {
            assert!(report.contains(bit), "{bit} missing from: {report}");
        }
        // …and the action's own requirement, so the two sets can be compared
        // side by side rather than inferred.
        let tree_in_fs_dock = Caps::VNAV | Caps::HIERARCHY | Caps::ACTIVATE | Caps::FILEOPS;
        assert!(
            report.contains(&format!("{tree_in_fs_dock:?}")),
            "the resolved path caps must appear verbatim: {report}"
        );
    }

    #[test]
    fn a_named_key_round_trips_through_the_explainer() {
        let chain = fs_chain();
        let report = explain("<CR>", &chain, NEVER);
        assert!(report.contains("runs: godotvim.item.activate"), "{report}");
    }

    // ── The counterfactual overlay ───────────────────────────────────

    #[test]
    fn the_report_explains_a_completion_key_through_the_counterfactual_overlay() {
        // The question this whole section exists to answer: "why is my <CR>
        // dead / what will it do over the popup?" could never be answered
        // live, because typing the command puts focus on the command line.
        let report = explain_counterfactual("<CR>", &fs_chain());
        assert!(report.contains("editor.completion"), "{report}");
        assert!(report.contains("caps: Caps(POPUP)"), "{report}");
        assert!(
            report.contains("eligible (godotvim.completion.confirm)"),
            "{report}"
        );
        assert!(
            report.contains("runs: godotvim.completion.confirm on 'editor.completion'"),
            "{report}"
        );
        assert!(report.contains("elastic"), "{report}");
    }

    #[test]
    fn the_counterfactual_section_says_it_is_not_live() {
        let report = explain_counterfactual("<CR>", &fs_chain());
        assert!(
            report.contains("while the script editor is in an insert-like mode"),
            "{report}"
        );
        assert!(
            report.contains("is not on the path this report sampled"),
            "{report}"
        );
    }

    #[test]
    fn the_counterfactual_reports_a_handoff_rule_honestly() {
        let report = explain_counterfactual("<Up>", &fs_chain());
        assert!(report.contains("the vim engine is skipped"), "{report}");
        assert!(report.contains("(handoff)"), "{report}");
    }

    #[test]
    fn the_counterfactual_prints_the_withheld_positional_probe() {
        // `<physical>` is inert on the overlay for a PRINTED reason now:
        // `refuses_positional: true` appears on the counterfactual's own
        // anchor line, where the flag used to be silently dead.
        let report = explain_counterfactual("<CR>", &fs_chain());
        assert!(report.contains("refuses_positional: true"), "{report}");
    }

    #[test]
    fn a_rejected_vimrc_line_is_printed_by_the_listing() {
        // `binding_diagnostics` was written by `rebuild_bindings` and read by
        // nothing — three references in the whole tree, all of them writes.
        // The only other channel was `log::warn!`, and the default Log Level
        // is Off, so every accepted residual was silent BY CONSTRUCTION
        // rather than merely by log level. `:checkhealth`, which the design
        // names 39 times as the mitigation for exactly this, does not exist.
        //
        // One good line and one malformed one, the way a real vimrc arrives:
        // the good one must still install (warn-and-skip is per line) and the
        // bad one must be visible to a user who types `:panelmap`.
        let reg = registry();
        let mut index = builtin_index(&reg);
        let mut diagnostics = Vec::new();
        crate::actions::bind::apply_text(
            &mut index,
            &reg,
            "panelmap dock q godotvim.item.activate\n\
             panelmap dock w godotvim.item.nextt",
            &MappingOwner::User,
            "user://.godot-vimrc",
            crate::actions::bind::Provenance::User,
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 1, "one bad line, one diagnostic");

        let report = list_report(&index, &reg, &diagnostics);
        assert!(
            report.contains("rejected line(s)"),
            "the listing must announce the rejects:\n{report}"
        );
        assert!(
            report.contains("user://.godot-vimrc:2: no action named 'godotvim.item.nextt'"),
            "the diagnostic must name the file, the line and the cause:\n{report}"
        );
        // …and the good line is still there, unaffected.
        assert!(report.contains("dock q godotvim.item.activate"), "{report}");
    }

    #[test]
    fn a_clean_config_prints_no_rejection_section_at_all() {
        let reg = registry();
        let report = list_report(&builtin_index(&reg), &reg, &[]);
        assert!(!report.contains("rejected"), "{report}");
    }

    #[test]
    fn shift_folding_is_visible_in_the_listing() {
        // `R` and `r` are two keys, not one key plus a modifier. If the
        // listing rendered `<S-r>` the user would copy back a binding that
        // can never fire.
        let reg = registry();
        let report = list_report(&builtin_index(&reg), &reg, &[]);
        assert!(report.contains("dock.filesystem R godotvim.fs.refresh"));
        assert!(!report.contains("<S-r>"));
        assert!(!report.contains("<S-R>"));
    }

    // ── Reservations (§5.10) ─────────────────────────────────────────

    /// The shipped defaults plus `lines`, in the syntax a user would type.
    fn index_with(lines: &str) -> BindingIndex {
        let reg = registry();
        let mut index = builtin_index(&reg);
        let mut diagnostics = Vec::new();
        crate::actions::bind::apply_text(
            &mut index,
            &reg,
            lines,
            &MappingOwner::User,
            "test",
            crate::actions::bind::Provenance::User,
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        index
    }

    #[test]
    fn the_listing_prints_every_reservation_and_who_owns_it() {
        // A reservation is invisible in the rule itself — `panelmap dock gg …`
        // says nothing about bare `g` — and an invisible reservation is a
        // silent dead key. The listing must name the key, the surface and the
        // sequences it is waiting for.
        let reg = registry();
        let index = index_with(
            "panelmap dock gg godotvim.item.prev\n\
             panelmap dock gj godotvim.item.next",
        );
        let report = list_report(&index, &reg, &[]);
        assert!(
            report.contains(
                "reserves g    (consumed bare on dock, then waits timeoutlen for: gg, gj)"
            ),
            "{report}"
        );
    }

    #[test]
    fn the_listing_prints_no_reservation_line_for_the_shipped_keyset() {
        // The zero-config guarantee, seen from the introspector: no
        // reservations means no `set_allow_search(false)` and no pending
        // buffer for a user who never bound a sequence.
        let reg = registry();
        let report = list_report(&builtin_index(&reg), &reg, &[]);
        assert!(!report.contains("reserves"), "{report}");
    }

    #[test]
    fn the_explainer_names_a_reservation_before_the_single_key_rule() {
        // `d` is bound on `dock.filesystem` AND reserved there by `dd`. The
        // report must say the reservation wins, or the user reads "runs
        // godotvim.fs.delete" and cannot explain why a single `d` waits.
        let reg = registry();
        let index = index_with("panelmap dock.filesystem dd godotvim.fs.delete");
        let chain = fs_chain();
        let path = providers::forest().classify(&chain).expect("total probe");
        let report = explain_report("d", &chain, &path, None, &index, &reg, NEVER);
        assert!(report.contains("dock.filesystem: RESERVED"), "{report}");
        assert!(report.contains("waits timeoutlen for: dd"), "{report}");
    }

    #[test]
    fn the_explainer_says_nothing_about_reservations_for_an_unreserved_key() {
        let report = explain("j", &fs_chain(), NEVER);
        assert!(!report.contains("RESERVED"), "{report}");
    }

    #[test]
    fn an_action_id_never_renders_as_an_action_pseudo_key() {
        // `Key::Action(7).to_vim_notation()` is literally `<Action>(7)`, so
        // the trie can never be the listing's source of truth. This is the
        // regression guard for reading the RHS instead of the arena.
        let reg = registry();
        let report = list_report(&builtin_index(&reg), &reg, &[]);
        assert!(!report.contains("<Action>("), "{report}");
    }

    #[test]
    fn the_key_notation_helper_agrees_with_the_parser() {
        for notation in ["<C-h>", "<CR>", "<Esc>", "a", "/", "R"] {
            let keys = parse_lhs(notation).expect("shipped notation parses");
            assert_eq!(keys.len(), 1);
            let round = parse_lhs(&keys[0].to_vim_notation()).expect("renders back");
            assert_eq!(round, keys, "{notation} does not round-trip");
        }
    }

    #[test]
    fn the_bare_char_key_events_are_what_the_runtime_produces() {
        assert_eq!(
            parse_lhs("R").expect("parses")[0],
            KeyEvent::new(VimKey::Char('R'), Modifiers::NONE)
        );
    }
}
