//! The `panelmap` / `panelunmap` line grammar.
//!
//! ```text
//! panelmap   [<flag> ...] <surface> <lhs> <target> [key=value ...]
//! panelunmap <surface> <lhs>
//! ```
//!
//! Single-line, with no cross-line state: the load policy judges each line
//! on its own, so a directive whose meaning depended on a previous line could
//! not be judged.
//!
//! # Why the shipped defaults go through here too
//!
//! Every provider authors its default bindings as `panelmap` text and hands it
//! to this parser (`actions::bind::builtin_index`). That is the anti-drift
//! device of the whole design: if defaults were built by calling constructors
//! directly, they would be expressible in a dialect the documented grammar does
//! not describe, and a user could not reproduce the shipped semantics by
//! writing them out. Here, "what the plugin ships" and "what a user may type"
//! are the same sentences read by the same code.
//!
//! The parser is **pure**: it resolves nothing against the action registry or
//! the surface forest, and it therefore cannot reject an unknown action id or
//! an undeclared surface. Those are registration-time checks and live in
//! `actions::bind`, because only that layer knows what is registered.
//!
//! See `DESIGN-rebindable-nav.md` (design notes, kept outside this repo) §6.2 and §6.3.

// P5 builds and tests the parser; P6 is the phase that reads it on the
// dispatch path. Shipping it inert is what makes this commit revertable on
// its own.

use std::fmt::Write as _;

use vim_core::keymap::KeyEvent;
use vim_core::VimString;

use crate::actions::action::{is_valid_action_id, Params, MAX_ACTION_COUNT};
use crate::actions::keys::{parse_lhs, LhsError};

/// The verb that introduces a binding line.
pub(crate) const MAP: &str = "panelmap";
/// The verb that removes one.
pub(crate) const UNMAP: &str = "panelunmap";

/// Upper bound on `key=value` pairs, per §6.2.
const MAX_PARAMS: usize = 4;

/// The right-hand side, still unresolved.
///
/// Resolution against [`crate::actions::action::ActionRegistry`] happens at
/// registration; keeping it textual here is what lets the parser stay pure and
/// unit-testable with no registry in scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TargetSpec {
    /// A dotted, registered action id.
    Action(VimString),
    /// `native` — give the key back to Godot at this surface.
    Native,
    /// `<Shortcut>(section/path)` — delegate to one of Godot's own shortcuts.
    Shortcut(VimString),
}

/// The five documented flags, each legal at most once and in any order.
///
/// A struct rather than a bitflag set because each one means something
/// different to a *different* consumer, and naming them at the use site is
/// what keeps `<void>` from being confused with `<nowait>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Flags {
    /// `<nowait>` — build the trie entry so a shorter LHS fires immediately.
    pub(crate) nowait: bool,
    /// `<physical>` — opt this rule into the US-QWERTY position probe.
    pub(crate) physical: bool,
    /// `<void>` — consume regardless of the action's outcome.
    pub(crate) void: bool,
    /// `<norepeat>` — drop `InputEventKey::is_echo()` repeats.
    pub(crate) norepeat: bool,
    /// `<shift>` — also match this LHS with SHIFT set.
    pub(crate) shift: bool,
}

impl Flags {
    /// Set the flag `token` names, or report why it cannot be set.
    fn set(&mut self, token: &str) -> Result<(), PanelParseError> {
        let slot = match token {
            "<nowait>" => &mut self.nowait,
            "<physical>" => &mut self.physical,
            "<void>" => &mut self.void,
            "<norepeat>" => &mut self.norepeat,
            "<shift>" => &mut self.shift,
            _ => return Err(PanelParseError::UnknownFlag(token.into())),
        };
        if *slot {
            return Err(PanelParseError::DuplicateFlag(token.into()));
        }
        *slot = true;
        Ok(())
    }
}

/// A parsed `panelmap` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelMap {
    pub(crate) flags: Flags,
    pub(crate) surface: VimString,
    /// Canonicalized, non-empty, at most `MAX_KEY_SEQUENCE_LEN` long.
    pub(crate) lhs: Vec<KeyEvent>,
    pub(crate) target: TargetSpec,
    pub(crate) params: Params,
}

/// A parsed line of either verb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PanelLine {
    /// Boxed because `PanelMap` is several times the size of `Unmap`, and the
    /// enum is moved around per config line.
    Map(Box<PanelMap>),
    Unmap {
        surface: VimString,
        lhs: Vec<KeyEvent>,
    },
}

/// Why a line that *is* a panel directive could not be parsed.
///
/// Every variant names the offending token, because the diagnostic a user sees
/// is `line N: <this>` and "syntax error" would send them back to the docs to
/// guess which of six operands was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PanelParseError {
    /// A required operand was absent.
    MissingOperand(&'static str),
    /// `panelunmap` takes exactly two operands.
    TrailingOperand(VimString),
    UnknownFlag(VimString),
    DuplicateFlag(VimString),
    /// Not `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`.
    BadSurface(VimString),
    Lhs(LhsError),
    /// Not an action id, not `native`, not `<Shortcut>(path)`.
    BadTarget(VimString),
    /// Not `key=value`, or the key is not `^[A-Za-z0-9_.]+$`.
    BadParam(VimString),
    DuplicateParam(VimString),
    /// Values are decimal integers only, `^-?[0-9]{1,10}$`.
    ParamNotAnInteger(VimString),
    TooManyParams(usize),
    /// `count` is validated at load as well as clamped at runtime: an
    /// unbounded count is an editor freeze, not a slow keystroke.
    CountOutOfRange(i64),
}

impl std::fmt::Display for PanelParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingOperand(what) => write!(f, "missing {what}"),
            Self::TrailingOperand(t) => {
                write!(f, "unexpected extra operand '{t}' (panelunmap takes two)")
            }
            Self::UnknownFlag(t) => write!(
                f,
                "unknown flag '{t}'; expected one of \
                 <nowait> <physical> <void> <norepeat> <shift>"
            ),
            Self::DuplicateFlag(t) => write!(f, "flag '{t}' given twice"),
            Self::BadSurface(s) => write!(f, "'{s}' is not a well-formed surface id"),
            Self::Lhs(e) => write!(f, "{e}"),
            Self::BadTarget(t) => write!(
                f,
                "'{t}' is not an action id, 'native', or <Shortcut>(path)"
            ),
            Self::BadParam(t) => write!(f, "'{t}' is not a key=value pair"),
            Self::DuplicateParam(k) => write!(f, "parameter '{k}' given twice"),
            Self::ParamNotAnInteger(v) => {
                write!(f, "parameter value '{v}' must be a decimal integer")
            }
            Self::TooManyParams(n) => {
                write!(f, "{n} parameters given, at most {MAX_PARAMS} allowed")
            }
            Self::CountOutOfRange(n) => {
                write!(f, "count={n} is outside 1..={MAX_ACTION_COUNT}")
            }
        }
    }
}

/// Whether `line` opens with one of the two panel verbs.
#[cfg(test)]
pub(crate) fn is_panel_line(line: &str) -> bool {
    verb_of(line.trim()).is_some()
}

/// The verb and the remainder, if `trimmed` is a panel directive.
///
/// Matched on a whole token: `panelmapping` is not `panelmap`, and a bare
/// `panelmap` with no operands *is* a panel line — one that then fails with a
/// diagnostic rather than being silently ignored as unrelated text.
fn verb_of(trimmed: &str) -> Option<(&'static str, &str)> {
    for verb in [UNMAP, MAP] {
        if let Some(rest) = trimmed.strip_prefix(verb) {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return Some((verb, rest));
            }
        }
    }
    None
}

/// Whether `s` matches `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$`.
fn is_valid_surface_id(s: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|seg| {
            seg.starts_with(|c: char| c.is_ascii_lowercase())
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// Parse one already-trimmed config line.
///
/// Three answers, and the distinction between the first two is the whole
/// contract: `Ok(None)` means "not mine, leave it to the mapping parser",
/// while `Err` means "mine, and malformed". Collapsing them would let
/// `panelmp dock j …` fall through as an unrecognized line and vanish.
pub(crate) fn parse_panel_line(line: &str) -> Result<Option<PanelLine>, PanelParseError> {
    let trimmed = line.trim();
    let Some((verb, rest)) = verb_of(trimmed) else {
        return Ok(None);
    };
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let parsed = if verb == UNMAP {
        parse_unmap(&tokens)?
    } else {
        parse_map(&tokens)?
    };
    Ok(Some(parsed))
}

fn parse_unmap(tokens: &[&str]) -> Result<PanelLine, PanelParseError> {
    let surface = *tokens
        .first()
        .ok_or(PanelParseError::MissingOperand("surface"))?;
    if !is_valid_surface_id(surface) {
        return Err(PanelParseError::BadSurface(surface.into()));
    }
    let lhs = *tokens
        .get(1)
        .ok_or(PanelParseError::MissingOperand("key sequence"))?;
    let lhs = parse_lhs(lhs).map_err(PanelParseError::Lhs)?;
    if let Some(extra) = tokens.get(2) {
        return Err(PanelParseError::TrailingOperand((*extra).into()));
    }
    Ok(PanelLine::Unmap {
        surface: surface.into(),
        lhs,
    })
}

fn parse_map(tokens: &[&str]) -> Result<PanelLine, PanelParseError> {
    let mut flags = Flags::default();
    let mut rest = tokens;
    // Flags occupy the leading slots, and a surface id can never begin with
    // `<` — which is what makes this greedy scan unambiguous, and what makes a
    // typo'd `<physicl>` an error instead of a surface named `<physicl>`.
    while let Some(token) = rest.first() {
        if !token.starts_with('<') {
            break;
        }
        flags.set(token)?;
        rest = &rest[1..];
    }

    let surface = *rest
        .first()
        .ok_or(PanelParseError::MissingOperand("surface"))?;
    if !is_valid_surface_id(surface) {
        return Err(PanelParseError::BadSurface(surface.into()));
    }
    let lhs = *rest
        .get(1)
        .ok_or(PanelParseError::MissingOperand("key sequence"))?;
    let lhs = parse_lhs(lhs).map_err(PanelParseError::Lhs)?;
    let target = parse_target(
        rest.get(2)
            .copied()
            .ok_or(PanelParseError::MissingOperand("target"))?,
    )?;
    let params = parse_params(&rest[3..])?;

    Ok(PanelLine::Map(Box::new(PanelMap {
        flags,
        surface: surface.into(),
        lhs,
        target,
        params,
    })))
}

fn parse_target(token: &str) -> Result<TargetSpec, PanelParseError> {
    if token == "native" {
        return Ok(TargetSpec::Native);
    }
    if let Some(inner) = token
        .strip_prefix("<Shortcut>(")
        .and_then(|s| s.strip_suffix(')'))
    {
        if inner.is_empty() {
            return Err(PanelParseError::BadTarget(token.into()));
        }
        return Ok(TargetSpec::Shortcut(inner.into()));
    }
    // The dot is what separates this namespace from Godot's slash-separated
    // shortcut paths, so a bare word can never be mistaken for an action.
    if is_valid_action_id(token) {
        return Ok(TargetSpec::Action(token.into()));
    }
    Err(PanelParseError::BadTarget(token.into()))
}

/// Render a parsed line back into the grammar it came from.
///
/// The inverse of [`parse_panel_line`], and the whole implementation of the
/// writer's `PanelMap` arm. A stored panel line is re-emitted from its
/// **parse**, never from the raw text it arrived as — which is what makes the
/// document-level round-trip property able to see a line that silently lost
/// its identity. Re-emitting raw text would round-trip a `ConfigLine::Comment`
/// just as faithfully and prove nothing.
///
/// Flag order is canonical rather than as-typed. Parsing is order-independent
/// (`flags_are_order_independent`), so `parse(render(parse(x))) == parse(x)`
/// holds regardless; only the text of the first re-write can differ from what
/// the user typed, and only in flag order.
pub(crate) fn render(line: &PanelLine) -> String {
    match line {
        PanelLine::Unmap { surface, lhs } => format!("{UNMAP} {surface} {}", render_lhs(lhs)),
        PanelLine::Map(map) => {
            let mut out = String::from(MAP);
            for (present, token) in [
                (map.flags.physical, "<physical>"),
                (map.flags.void, "<void>"),
                (map.flags.norepeat, "<norepeat>"),
                (map.flags.shift, "<shift>"),
                (map.flags.nowait, "<nowait>"),
            ] {
                if present {
                    out.push(' ');
                    out.push_str(token);
                }
            }
            let target = match &map.target {
                TargetSpec::Action(id) => id.to_string(),
                TargetSpec::Native => String::from("native"),
                TargetSpec::Shortcut(path) => format!("<Shortcut>({path})"),
            };
            let _ = write!(out, " {} {} {target}", map.surface, render_lhs(&map.lhs));
            for (key, value) in map.params.iter() {
                let _ = write!(out, " {key}={value}");
            }
            out
        }
    }
}

fn render_lhs(lhs: &[KeyEvent]) -> String {
    lhs.iter().map(KeyEvent::to_vim_notation).collect()
}

fn parse_params(tokens: &[&str]) -> Result<Params, PanelParseError> {
    if tokens.len() > MAX_PARAMS {
        return Err(PanelParseError::TooManyParams(tokens.len()));
    }
    let mut params = Params::new();
    let mut seen: Vec<&str> = Vec::new();
    for token in tokens {
        let Some((key, value)) = token.split_once('=') else {
            return Err(PanelParseError::BadParam((*token).into()));
        };
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            return Err(PanelParseError::BadParam((*token).into()));
        }
        if seen.contains(&key) {
            return Err(PanelParseError::DuplicateParam(key.into()));
        }
        seen.push(key);
        // Decimal integers only, and deliberately no enum or string form: a
        // closed integer vocabulary is what makes the sandbox extension
        // provable — a parameter can never expand into `:!` or `:source`.
        let digits = value.strip_prefix('-').unwrap_or(value);
        if digits.is_empty() || digits.len() > 10 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(PanelParseError::ParamNotAnInteger((*value).into()));
        }
        let Ok(int) = value.parse::<i64>() else {
            return Err(PanelParseError::ParamNotAnInteger((*value).into()));
        };
        // `count` is bounded at LOAD as well as clamped at runtime, so a typo
        // is reported rather than silently rounded down to something sane.
        if key == "count" && !(1..=MAX_ACTION_COUNT).contains(&int) {
            return Err(PanelParseError::CountOutOfRange(int));
        }
        params.set_int(key, int);
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vim_core::keymap::{Key, Modifiers};

    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(Key::Char(c), Modifiers::NONE)
    }

    fn map(line: &str) -> PanelMap {
        match parse_panel_line(line) {
            Ok(Some(PanelLine::Map(m))) => *m,
            other => panic!("expected a panelmap from '{line}', got {other:?}"),
        }
    }

    fn err(line: &str) -> PanelParseError {
        parse_panel_line(line).expect_err(line)
    }

    // ── What is and is not a panel line ──────────────────────────────

    #[test]
    fn unrelated_lines_are_not_claimed() {
        // `Ok(None)` and not `Err`: these belong to the mapping parser, and
        // claiming them would turn every vimrc into a wall of diagnostics.
        for line in [
            "",
            "   ",
            "\" a comment",
            "set number",
            "nnoremap <leader>ff <Action>(godotvim.fs.create)",
            "let mapleader = \" \"",
            // A near-miss on the verb must NOT be claimed as a panel line...
            "panelmapping dock j godotvim.item.next",
        ] {
            assert_eq!(parse_panel_line(line), Ok(None), "'{line}'");
        }
    }

    #[test]
    fn a_typo_in_the_verb_falls_through_rather_than_half_matching() {
        // `panelmp` is not a panel verb. It is not this parser's job to guess
        // — but it must also not be silently accepted, which is why the
        // caller reports unclaimed non-mapping lines.
        assert_eq!(parse_panel_line("panelmp dock j x.y"), Ok(None));
        assert!(!is_panel_line("panelmp dock j x.y"));
        assert!(is_panel_line("panelmap dock j x.y"));
        assert!(is_panel_line("  panelunmap dock j"));
    }

    #[test]
    fn a_bare_verb_is_a_claimed_line_that_fails() {
        // The important half of the previous test: claimed, then diagnosed.
        // Falling through here would make `panelmap` typed alone vanish.
        assert_eq!(
            parse_panel_line("panelmap"),
            Err(PanelParseError::MissingOperand("surface"))
        );
        assert_eq!(
            parse_panel_line("panelunmap"),
            Err(PanelParseError::MissingOperand("surface"))
        );
    }

    // ── The happy path ───────────────────────────────────────────────

    #[test]
    fn the_shipped_panel_line_parses_exactly_as_documented() {
        // §6.4 example 1, byte for byte.
        let m = map("panelmap <physical> <void> <norepeat> panel <C-h> godotvim.focus.left");
        assert_eq!(
            m.flags,
            Flags {
                physical: true,
                void: true,
                norepeat: true,
                ..Flags::default()
            }
        );
        assert_eq!(m.surface, "panel");
        assert_eq!(m.lhs, vec![KeyEvent::new(Key::Char('h'), Modifiers::CTRL)]);
        assert_eq!(m.target, TargetSpec::Action("godotvim.focus.left".into()));
        assert!(m.params.is_empty());
    }

    #[test]
    fn flags_are_order_independent() {
        let a = map("panelmap <void> <physical> dock j godotvim.item.next");
        let b = map("panelmap <physical> <void> dock j godotvim.item.next");
        assert_eq!(a, b);
    }

    #[test]
    fn a_line_with_no_flags_at_all_parses() {
        let m = map("panelmap prompt <Esc> godotvim.focus.editor");
        assert_eq!(m.flags, Flags::default());
        assert_eq!(m.lhs, vec![KeyEvent::new(Key::Escape, Modifiers::NONE)]);
    }

    #[test]
    fn leading_and_inner_whitespace_is_tolerated() {
        let m = map("   panelmap   <shift>   searchbox   <CR>   godotvim.search.accept  ");
        assert!(m.flags.shift);
        assert_eq!(m.surface, "searchbox");
        assert_eq!(m.lhs, vec![KeyEvent::new(Key::Enter, Modifiers::NONE)]);
    }

    #[test]
    fn a_multi_key_sequence_parses_as_a_sequence() {
        let m = map("panelmap dock.filesystem dd godotvim.fs.delete");
        assert_eq!(m.lhs, vec![ch('d'), ch('d')]);
    }

    #[test]
    fn the_lhs_is_canonicalized_by_the_shared_vocabulary() {
        // `<S-r>`, `<S-R>` and `R` must intern identically or the binding is
        // dead on arrival — the runtime never delivers Char('r')+SHIFT.
        for spelling in ["<S-r>", "<S-R>", "R"] {
            let m = map(&format!(
                "panelmap dock.filesystem {spelling} godotvim.fs.refresh"
            ));
            assert_eq!(m.lhs, vec![ch('R')], "{spelling}");
        }
    }

    // ── Targets ──────────────────────────────────────────────────────

    #[test]
    fn the_three_target_forms_are_recognized() {
        assert_eq!(
            map("panelmap dock j godotvim.item.next").target,
            TargetSpec::Action("godotvim.item.next".into())
        );
        assert_eq!(
            map("panelmap dock.filesystem <C-h> native").target,
            TargetSpec::Native
        );
        assert_eq!(
            map("panelmap dock.filesystem <C-r> <Shortcut>(filesystem_dock/rename)").target,
            TargetSpec::Shortcut("filesystem_dock/rename".into())
        );
    }

    #[test]
    fn a_target_that_is_neither_an_action_nor_a_keyword_is_rejected() {
        // §6.4's rejection list. `:!rm -rf /` is the reason the target
        // vocabulary is closed rather than "anything the host understands".
        for target in [
            ":!rm",
            "rm",
            "focus",
            "filesystem_dock/rename",
            "<Shortcut>()",
        ] {
            let e = err(&format!("panelmap dock j {target}"));
            assert!(
                matches!(e, PanelParseError::BadTarget(_)),
                "{target} produced {e:?}"
            );
        }
    }

    #[test]
    fn a_godot_shortcut_path_must_be_wrapped_to_be_a_target() {
        // Bare `filesystem_dock/rename` is Godot's namespace, not ours, and
        // accepting it unwrapped would make the two indistinguishable.
        assert!(matches!(
            err("panelmap dock j filesystem_dock/rename"),
            PanelParseError::BadTarget(_)
        ));
    }

    // ── Flags ────────────────────────────────────────────────────────

    #[test]
    fn an_unknown_flag_is_rejected_rather_than_read_as_a_surface() {
        // A surface id can never start with `<`, so this is decidable. If it
        // were not, `<physicl>` would become a surface nobody declared and the
        // rule would be silently dead.
        let e = err("panelmap <physicl> dock j godotvim.item.next");
        assert_eq!(e, PanelParseError::UnknownFlag("<physicl>".into()));
        assert!(e.to_string().contains("<physical>"), "{e}");
    }

    #[test]
    fn a_repeated_flag_is_rejected() {
        assert_eq!(
            err("panelmap <void> <void> dock j godotvim.item.next"),
            PanelParseError::DuplicateFlag("<void>".into())
        );
    }

    #[test]
    fn there_is_no_yield_token() {
        // Arbitration is a property of the SurfaceSpec and unreachable from
        // config by construction. A `<yield>` flag would reintroduce exactly
        // the editor/panel duplication the surface plane removed.
        assert!(matches!(
            err("panelmap <yield> panel <C-h> godotvim.focus.left"),
            PanelParseError::UnknownFlag(_)
        ));
    }

    // ── Surfaces ─────────────────────────────────────────────────────

    #[test]
    fn well_formed_surface_ids_are_accepted() {
        for id in ["panel", "dock", "dock.filesystem", "editor.nav", "a1_b.c2"] {
            assert!(is_valid_surface_id(id), "{id}");
        }
    }

    #[test]
    fn malformed_surface_ids_are_rejected() {
        for id in ["", "Dock", "dock.", ".dock", "dock..fs", "dock-fs", "1dock"] {
            assert!(!is_valid_surface_id(id), "{id}");
        }
        assert_eq!(
            err("panelmap Dock j godotvim.item.next"),
            PanelParseError::BadSurface("Dock".into())
        );
    }

    // ── Left-hand sides ──────────────────────────────────────────────

    #[test]
    fn an_unspellable_shifted_lhs_is_rejected_with_advice() {
        // §6.8: `<S-1>` is `!` on US and `+` on DE, so the binding could never
        // fire. Failing at load with "write the literal character" beats
        // accepting a key that silently does nothing.
        let e = err("panelmap dock <S-1> godotvim.item.next");
        assert_eq!(e, PanelParseError::Lhs(LhsError::UnspellableShift('1')));
        assert!(e.to_string().contains("write the literal character"), "{e}");
    }

    #[test]
    fn a_nine_key_left_hand_side_is_rejected() {
        // MAX_KEY_SEQUENCE_LEN is 8 in vim-core; a longer LHS cannot round
        // trip through a KeySequence and would be silently truncated.
        let e = err("panelmap dock abcdefghi godotvim.item.next");
        assert_eq!(e, PanelParseError::Lhs(LhsError::TooLong(9)));
        assert!(map("panelmap dock abcdefgh godotvim.item.next").lhs.len() == 8);
    }

    // ── Parameters ───────────────────────────────────────────────────

    #[test]
    fn parameters_parse_as_decimal_integers() {
        let m = map("panelmap dock <C-d> godotvim.item.next count=10");
        assert_eq!(m.params.count(), 10);
        let m = map("panelmap dock x a.b flag=1 depth=-3");
        assert_eq!(m.params.int("flag", 0), 1);
        assert_eq!(m.params.int("depth", 0), -3);
    }

    #[test]
    fn a_non_integer_parameter_value_is_rejected() {
        // There is no enum-token form and the grammar must not promise one.
        for pair in [
            "count=ten",
            "count=",
            "count=1.5",
            "count=0x10",
            "count=1e3",
        ] {
            assert!(
                matches!(
                    err(&format!("panelmap dock j a.b {pair}")),
                    PanelParseError::ParamNotAnInteger(_)
                ),
                "{pair}"
            );
        }
    }

    #[test]
    fn a_malformed_parameter_token_is_rejected() {
        for token in ["count", "=5", "co unt=5"] {
            let line = format!("panelmap dock j a.b {token}");
            assert!(parse_panel_line(&line).is_err(), "{token}");
        }
    }

    #[test]
    fn a_repeated_parameter_key_is_rejected() {
        assert_eq!(
            err("panelmap dock j a.b count=1 count=2"),
            PanelParseError::DuplicateParam("count".into())
        );
    }

    #[test]
    fn more_than_four_parameters_are_rejected() {
        assert_eq!(
            err("panelmap dock j a.b a=1 b=2 c=3 d=4 e=5"),
            PanelParseError::TooManyParams(5)
        );
    }

    #[test]
    fn a_count_outside_the_survivable_range_is_rejected_at_load() {
        // `find_navigable_target` walks up to 1000 items per call, so an
        // unbounded count is a frozen editor rather than a slow keystroke.
        // Runtime clamping alone would hide the typo.
        for n in [0, -1, 101, 1_000_000] {
            assert_eq!(
                err(&format!("panelmap dock j godotvim.item.next count={n}")),
                PanelParseError::CountOutOfRange(n),
                "count={n}"
            );
        }
        assert!(parse_panel_line("panelmap dock j godotvim.item.next count=100").is_ok());
        assert!(parse_panel_line("panelmap dock j godotvim.item.next count=1").is_ok());
    }

    // ── panelunmap ───────────────────────────────────────────────────

    #[test]
    fn unmap_takes_a_surface_and_a_key_sequence() {
        assert_eq!(
            parse_panel_line("panelunmap dock.filesystem a"),
            Ok(Some(PanelLine::Unmap {
                surface: "dock.filesystem".into(),
                lhs: vec![ch('a')],
            }))
        );
    }

    #[test]
    fn unmap_rejects_a_third_operand() {
        // `panelunmap dock j godotvim.item.next` is a user writing `panelmap`
        // by muscle memory. Accepting and ignoring the tail would remove a
        // binding they meant to add.
        assert_eq!(
            err("panelunmap dock j godotvim.item.next"),
            PanelParseError::TrailingOperand("godotvim.item.next".into())
        );
    }

    #[test]
    fn unmap_validates_its_operands_the_same_way() {
        assert!(matches!(
            err("panelunmap Dock j"),
            PanelParseError::BadSurface(_)
        ));
        assert!(matches!(
            err("panelunmap dock <S-1>"),
            PanelParseError::Lhs(_)
        ));
        assert_eq!(
            err("panelunmap dock"),
            PanelParseError::MissingOperand("key sequence")
        );
    }

    // ── Rendering, and the fixpoint it exists to guarantee ───────────

    #[test]
    fn a_rendered_line_is_the_line_a_user_would_type() {
        assert_eq!(
            render(
                &parse_panel_line("panelmap dock j godotvim.item.next")
                    .unwrap()
                    .unwrap()
            ),
            "panelmap dock j godotvim.item.next"
        );
        assert_eq!(
            render(
                &parse_panel_line("panelunmap dock.filesystem a")
                    .unwrap()
                    .unwrap()
            ),
            "panelunmap dock.filesystem a"
        );
        assert_eq!(
            render(
                &parse_panel_line(
                    "panelmap <physical> <void> <norepeat> panel <C-h> godotvim.focus.left"
                )
                .unwrap()
                .unwrap()
            ),
            "panelmap <physical> <void> <norepeat> panel <C-h> godotvim.focus.left"
        );
        assert_eq!(
            render(
                &parse_panel_line("panelmap dock <C-d> godotvim.item.next count=10")
                    .unwrap()
                    .unwrap()
            ),
            "panelmap dock <C-d> godotvim.item.next count=10"
        );
        assert_eq!(
            render(
                &parse_panel_line("panelmap dock.filesystem <C-h> native")
                    .unwrap()
                    .unwrap()
            ),
            "panelmap dock.filesystem <C-h> native"
        );
        assert_eq!(
            render(
                &parse_panel_line(
                    "panelmap dock.filesystem <C-r> <Shortcut>(filesystem_dock/rename)"
                )
                .unwrap()
                .unwrap()
            ),
            "panelmap dock.filesystem <C-r> <Shortcut>(filesystem_dock/rename)"
        );
    }

    #[test]
    fn parse_render_parse_is_a_fixpoint_even_when_the_text_is_not() {
        // Flag order and whitespace are normalized on the first render, so
        // text equality is NOT the property. Parse equality is, and it is the
        // one the config round-trip depends on.
        for line in [
            "panelmap   <void>   <physical>  dock  j   godotvim.item.next",
            "panelmap <shift> searchbox <CR> godotvim.search.accept",
            "panelmap <nowait> dock.filesystem dd godotvim.fs.delete",
            "panelmap dock x a.b flag=1 depth=-3",
            "panelunmap    panel   <C-h>",
            "panelmap dock.filesystem R godotvim.fs.refresh",
        ] {
            let once = parse_panel_line(line).unwrap().unwrap();
            let twice = parse_panel_line(&render(&once)).unwrap().unwrap();
            assert_eq!(once, twice, "'{line}' is not a fixpoint");
            assert_eq!(render(&once), render(&twice), "'{line}' renders unstably");
        }
    }

    // ── Diagnostics ──────────────────────────────────────────────────

    #[test]
    fn every_error_renders_a_message_naming_the_offender() {
        // A diagnostic reads `line N: <this>`, so "syntax error" would send
        // the user back to the docs to guess which operand was wrong.
        let cases = [
            ("panelmap", "surface"),
            ("panelmap <nope> dock j a.b", "<nope>"),
            ("panelmap <void> <void> dock j a.b", "<void>"),
            ("panelmap Dock j a.b", "Dock"),
            ("panelmap dock j :!rm", ":!rm"),
            ("panelmap dock j a.b zzz", "zzz"),
            ("panelmap dock j a.b count=x", "x"),
            ("panelmap dock j a.b a=1 b=2 c=3 d=4 e=5", "5"),
            ("panelmap dock j a.b count=999", "999"),
            ("panelunmap dock j extra", "extra"),
        ];
        for (line, needle) in cases {
            let text = err(line).to_string();
            assert!(
                text.contains(needle),
                "'{line}' → '{text}' lacks '{needle}'"
            );
        }
    }
}
