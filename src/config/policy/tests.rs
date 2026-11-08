use std::collections::HashMap;

use vim_core::execution::{ConfigText, TrustAll, VimEngine};
use vim_core::keymap::{KeyEvent, MappingMode};

use super::*;
use crate::config::path::ResolvedConfig;
use crate::controller::{LoadReport, VimController};

const MODES: [MappingMode; 7] = [
    MappingMode::Normal,
    MappingMode::Visual,
    MappingMode::Operator,
    MappingMode::Insert,
    MappingMode::Command,
    MappingMode::VisualOnly,
    MappingMode::SelectOnly,
];

const PROJECT: &str = "res://.godot-vimrc";
const USER: &str = "user://.godot-vimrc";

/// What a config can leave in the engine, in a form two engines compare by.
fn state(engine: &VimEngine) -> (String, vim_core::primitives::VimOptions) {
    let mut mappings: Vec<String> = MODES
        .iter()
        .flat_map(|mode| {
            engine
                .keymap()
                .list_mappings(*mode)
                .into_iter()
                .map(move |(lhs, entry, local)| {
                    format!(
                        "{mode:?} {lhs:?} {:?} {:?} {:?} {:?} {local}",
                        entry.sequence(),
                        entry.kind(),
                        entry.flags(),
                        entry.expression()
                    )
                })
        })
        .collect();
    mappings.sort();
    let mut abbreviations: Vec<String> = engine
        .abbreviations()
        .list(vim_core::primitives::AbbrevMode::Both)
        .iter()
        .map(|entry| {
            format!(
                "{} {} {:?} {}",
                entry.trigger, entry.replacement, entry.mode, entry.noremap
            )
        })
        .collect();
    abbreviations.sort();
    let text = format!(
        "{mappings:?}\n{abbreviations:?}\nhandlers {}\nleader {:?}",
        engine.handler_map().len(),
        engine.leader(),
    );
    (text, engine.options().clone())
}

/// `text` loaded at `tier` as a project file.
fn load(tier: Tier, text: &str) -> (VimController, LoadReport) {
    let actions = crate::actions::specs::registry();
    let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox).with_actions(&actions);
    let mut controller = VimController::new();
    let config = ConfigText::new(PROJECT, text).with_trust(tier.tag());
    let report = controller.load_config(config, &mut policy);
    assert_eq!(controller.engine().pending_request_count(), 0, "{text}");
    (controller, report)
}

fn sandboxed(text: &str) -> (VimController, LoadReport) {
    load(Tier::Sandbox, text)
}

/// `text` run with every command allowed.
fn run_in_full(text: &str) -> VimController {
    let mut controller = VimController::new();
    let _ = controller
        .engine_mut()
        .source_script(ConfigText::new(PROJECT, text), &mut TrustAll::new());
    controller
}

fn maps(controller: &VimController, key: char) -> bool {
    controller.could_start_mapping(KeyEvent::char(key))
}

fn only_line(report: &LoadReport) -> &str {
    assert_eq!(report.lines.len(), 1, "{:?}", report.lines);
    &report.lines[0]
}

// ── The text Sandbox's vectors ────────────────────────────────────────

/// What the policy does with a vector of the text Sandbox's tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Expected {
    /// Kept by both: it runs in full.
    Kept,
    /// Stripped by both: it does not run in full.
    Stripped,
    /// Kept by the text Sandbox, refused now: a `!` its keys may type as
    /// the filter operator, `:normal`, or a write to a named file.
    Refused,
    /// Stripped by the text Sandbox, run in full now, for the reason given
    /// with the vector.
    Relaxed,
}

/// Every keep and strip vector the text Sandbox's tests judged, one line
/// each with `\` and tab escaped, tagged with what the policy does, and a
/// relaxed one with the reason.
pub(super) fn vectors() -> Vec<(Expected, String, &'static str)> {
    include_str!("sandbox_vectors.txt")
        .lines()
        .map(|entry| {
            let mut fields = entry.split('\t');
            let (Some(tag), Some(line)) = (fields.next(), fields.next()) else {
                panic!("a tag and a line: {entry}");
            };
            let expected = match tag {
                "K" => Expected::Kept,
                "S" => Expected::Stripped,
                "N" => Expected::Refused,
                "R" => Expected::Relaxed,
                _ => panic!("unknown tag {tag}"),
            };
            let reason = fields.next().unwrap_or_default();
            assert_eq!(expected == Expected::Relaxed, !reason.is_empty(), "{entry}");
            (expected, unescape(line), reason)
        })
        .collect()
}

fn unescape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[test]
fn every_text_sandbox_vector_leaves_the_engine_as_expected() {
    let fresh = state(VimController::new().engine());
    let vectors = vectors();
    assert!(vectors.len() > 3000);
    for (expected, line, _) in vectors {
        let (controller, report) = sandboxed(&line);
        let sandboxed = state(controller.engine());
        let full = state(run_in_full(&line).engine());
        match expected {
            Expected::Kept | Expected::Relaxed => {
                assert_eq!(sandboxed, full, "{expected:?}: {line} {:?}", report.lines);
                assert_eq!(report.refused, 0, "{expected:?}: {line} {:?}", report.lines);
            }
            Expected::Stripped => assert!(
                sandboxed == fresh || sandboxed != full || report.refused + report.errors > 0,
                "{line}"
            ),
            Expected::Refused => {
                assert_ne!(sandboxed, full, "{line}");
                let reason = only_line(&report);
                assert!(
                    ["(runs the shell)", "(runs keys)", "(writes a named file: "]
                        .iter()
                        .any(|why| reason.contains(why)),
                    "{line}: {reason}"
                );
            }
        }
    }
}

// ── The Sandbox allowlist ─────────────────────────────────────────────

#[test]
fn the_sandbox_names_why_it_refuses() {
    for (line, reason) in [
        ("set langmap=ab", "(option 'langmap')"),
        ("set shell=sh", "(option 'shell')"),
        ("set invlangremap", "(option 'langremap')"),
        ("iabbrev teh the", "(abbreviation)"),
        ("abclear", "(abbreviation)"),
        ("sethandler <C-a> n:vim", "(handler)"),
        ("source f.vim", "(sources a file)"),
        ("w", "(editor command)"),
        ("Scene", "(godot-vim command)"),
        ("Foo", "(not a vim-core command)"),
        ("let g:x = 1", "(not on the Sandbox allowlist)"),
        ("nnoremap <expr> x 'y'", "(<expr>)"),
        ("nmap x y", "(recursive map)"),
        ("nnoremap x <Plug>(foo)", "(<Plug>(foo))"),
        (
            "nnoremap x <Action>(godotvim.nope)",
            "(<Action>(godotvim.nope))",
        ),
        ("nnoremap x :!id<CR>", "(runs the shell)"),
        ("nnoremap x @a", "(runs a register)"),
        ("nnoremap x <Cmd>so f<CR>", "(sources a file: so f)"),
        (
            "nnoremap x :nnoremap q y<CR>",
            "(changes mappings: nnoremap q y)",
        ),
        (
            "nnoremap x :iabbrev a b<CR>",
            "(changes abbreviations: iabbrev a b)",
        ),
        ("nnoremap x :let mapleader=','<CR>", "(sets the leader: "),
        (
            "nnoremap x :sethandler <C-a> n:vim<CR>",
            "(changes handlers: ",
        ),
        (
            "nnoremap x :set shell=sh<CR>",
            "(sets a secure option: set shell=sh)",
        ),
        ("nnoremap x :set nolrm<CR>", "(blocked option: set nolrm)"),
        (
            "nnoremap x :windo set lmap=ab<CR>",
            "(sets a secure option: windo set lmap=ab)",
        ),
        (
            "nnoremap x :windo set lrm<CR>",
            "(blocked option: windo set lrm)",
        ),
        ("nnoremap x :w foo<CR>", "(writes a named file: w foo)"),
        ("nnoremap x :mkvimrc<CR>", "(writes a vimrc: mkvimrc)"),
        ("nnoremap x :norm dd<CR>", "(runs keys)"),
        (
            "nnoremap x :action editor/close<CR>",
            "(editor shortcut: action editor/close)",
        ),
        ("nnoremap x :run<CR>", "(godot-vim command: run)"),
        ("nnoremap x :save<CR>", "(godot-vim command: save)"),
        ("nnoremap x <Cmd>Scene<CR>", "(godot-vim command: Scene)"),
        ("nnoremap x :vimdebug<CR>", "(godot-vim command: vimdebug)"),
        ("nnoremap x :save foo<CR>", "(godot-vim command: save foo)"),
        ("nnoremap x :stop now<CR>", "(godot-vim command: stop now)"),
        ("nnoremap x :Scene x<CR>", "(godot-vim command: Scene x)"),
        ("cnoremap x :!id<CR>", "(runs the shell"),
    ] {
        let (controller, report) = sandboxed(line);
        let message = only_line(&report);
        assert_eq!(report.refused, 1, "{line}");
        assert!(
            message.starts_with(&format!(
                "{PROJECT}:1: E48: Not allowed in sandbox: {line} ("
            )),
            "{message}"
        );
        assert!(message.contains(reason), "{line}: {message}");
        assert!(!maps(&controller, 'x'), "{line}");
        assert!(controller
            .engine()
            .keymap()
            .list_mappings(MappingMode::Command)
            .is_empty());
    }
}

/// A longer word than a godot-vim command is not one: it gives E492 when
/// the mapping runs.
#[test]
fn a_mapping_to_a_longer_word_than_a_godot_vim_command_is_kept() {
    let (controller, report) = sandboxed("nnoremap x :runx<CR>");
    assert!(report.lines.is_empty(), "{:?}", report.lines);
    assert!(maps(&controller, 'x'));
}

/// Vim's sandbox refuses every command without `EX_SBOXOK` (ex_docmd.c
/// `do_one_cmd()`, ex_cmds.h), the map family among them, so a mapping
/// that would define one when it runs is refused.
#[test]
fn a_mapping_that_would_define_a_mapping_defines_nothing() {
    let (controller, _) = sandboxed("nnoremap x :nnoremap y :!id<lt>CR><CR>");
    assert!(!maps(&controller, 'x'));
    assert!(!maps(&controller, 'y'));
}

#[test]
fn the_sandbox_keeps_what_a_shared_vimrc_needs() {
    let text = "set ic scs so=5\n\
                let mapleader = \",\"\n\
                nnoremap j gj\n\
                nnoremap <leader>w :w<CR>\n\
                nnoremap <leader>n :noh<CR>\n\
                vnoremap <leader>s :s/a/b/g<CR>\n\
                nnoremap <leader>a <Action>(godotvim.item.next)\n\
                nnoremap <leader>c :action godotvim.item.next<CR>\n\
                cnoremap <C-a> <Home>\n\
                nunmap j\n\
                nnoremap j gj\n\
                nnoremap k gk\n\
                echo \"hi\"";
    let (controller, report) = sandboxed(text);
    assert!(report.lines.is_empty(), "{:?}", report.lines);
    assert_eq!(
        state(controller.engine()),
        state(run_in_full(text).engine())
    );
    assert!(maps(&controller, 'k'));
}

#[test]
fn an_unknown_tag_is_judged_as_the_sandbox() {
    assert_eq!(Tier::of(TrustTag::new(7)), Tier::Sandbox);
    assert_eq!(Tier::of(Tier::Trusted.tag()), Tier::Trusted);
    assert_eq!(Tier::of(Tier::Builtin.tag()), Tier::Builtin);
}

#[test]
fn the_tier_of_a_file_follows_where_it_is() {
    assert_eq!(
        Tier::of_file(false, ProjectVimrc::Disabled),
        Some(Tier::Trusted)
    );
    assert_eq!(Tier::of_file(true, ProjectVimrc::Disabled), None);
    assert_eq!(
        Tier::of_file(true, ProjectVimrc::Sandbox),
        Some(Tier::Sandbox)
    );
    assert_eq!(
        Tier::of_file(true, ProjectVimrc::Trusted),
        Some(Tier::Trusted)
    );
}

// ── Host lines ────────────────────────────────────────────────────────

/// A later version's opt-in for panel keys is claimed without a word, so
/// a vimrc shared with it loads cleanly here.
#[test]
fn a_panelmap_preset_line_is_ignored_without_a_message() {
    for tier in [Tier::Sandbox, Tier::Trusted] {
        for line in [
            "panelmap preset",
            "panelmap preset vim",
            "  :panelmap   preset\t",
        ] {
            let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox).collecting_panels();
            let mut controller = VimController::new();
            let config = ConfigText::new(PROJECT, line).with_trust(tier.tag());
            let report = controller.load_config(config, &mut policy);
            assert!(
                report.lines.is_empty(),
                "{tier:?} {line}: {:?}",
                report.lines
            );
            assert!(policy.panel_lines().is_empty(), "{line}");
            assert_eq!(policy.ignored_panel_lines(), 0);
        }
        // After a bar too.
        let (_, report) = load(tier, "set so=5 | panelmap preset");
        assert!(report.lines.is_empty(), "{tier:?}: {:?}", report.lines);
    }
}

#[test]
fn panel_lines_are_claimed_from_the_top_file_only() {
    let text = "panelmap dock q godotvim.item.activate\n\
                set so=5 | panelunmap dock q\n\
                :panelmap dock w godotvim.item.next\n\
                panelmap dock e <Shortcut>(editor/close)";
    let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox).collecting_panels();
    let mut controller = VimController::new();
    let report = controller.load_config(
        ConfigText::new(PROJECT, text).with_trust(Tier::Sandbox.tag()),
        &mut policy,
    );
    assert_eq!(
        report.lines,
        [format!(
            "{PROJECT}:4: E48: Not allowed in sandbox: \
             panelmap dock e <Shortcut>(editor/close) (panel target)"
        )]
    );
    assert_eq!(
        policy.panel_lines(),
        [
            (1, "panelmap dock q godotvim.item.activate".to_owned()),
            (2, "panelunmap dock q".to_owned()),
            (3, "panelmap dock w godotvim.item.next".to_owned()),
        ]
    );

    let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox);
    let report = controller.load_config(
        ConfigText::new("user://extra.vim", text).with_trust(Tier::Trusted.tag()),
        &mut policy,
    );
    assert!(report.lines.is_empty(), "{:?}", report.lines);
    assert!(policy.panel_lines().is_empty());
    assert_eq!(policy.ignored_panel_lines(), 4);
}

#[test]
fn filetype_lines_are_claimed() {
    for tier in [Tier::Sandbox, Tier::Trusted] {
        let (_, report) = load(tier, "filetype plugin off\nfiletype\nfiletype sideways");
        assert_eq!(
            report.lines,
            [format!("{PROJECT}:3: E474: Invalid argument: sideways")]
        );
    }
}

// ── The Trusted tier ──────────────────────────────────────────────────

#[test]
fn a_trusted_file_runs_no_editor_command() {
    for (line, error) in [
        ("w", "E523: Not allowed here: w"),
        ("q!", "E523: Not allowed here: q!"),
        (
            "action editor/close",
            "E523: Not allowed here: action editor/close",
        ),
        ("Scene", "E523: Not allowed here: Scene"),
        ("saveall", "E523: Not allowed here: saveall"),
        // vim-core's `:saveas`, which it does not run.
        (
            "save",
            "E319: Sorry, the command is not available in this version: save",
        ),
        ("vimdebug on", "E523: Not allowed here: vimdebug on"),
        ("Foo bar", "E492: Not an editor command: Foo bar"),
    ] {
        let (_, report) = load(Tier::Trusted, line);
        assert_eq!(report.lines, [format!("{PROJECT}:1: {error}")], "{line}");
        assert_eq!((report.errors, report.refused), (1, 0), "{line}");
    }
}

#[test]
fn a_trusted_file_keeps_what_the_sandbox_refuses() {
    let text =
        "nnoremap x :!id<CR>\nnmap y x\niabbrev teh the\nset langmap=ab\nlet mapleader = ','";
    let (controller, report) = load(Tier::Trusted, text);
    assert!(report.lines.is_empty(), "{:?}", report.lines);
    assert_eq!(
        state(controller.engine()),
        state(run_in_full(text).engine())
    );
    assert!(maps(&controller, 'x'));
}

#[test]
fn has_godot_is_true() {
    let (controller, _) = sandboxed("if has('godot')\nnnoremap x y\nendif");
    assert!(maps(&controller, 'x'));
}

// ── Nested :source ────────────────────────────────────────────────────

fn globalize(path: &str) -> String {
    path.replace("res://", "/proj/")
        .replace("user://", "/data/")
}

/// Load `top` as the user's vimrc, with `files` to source from.
fn load_with_files(
    project_vimrc: ProjectVimrc,
    top: &str,
    files: &[(&str, &str)],
) -> (VimController, LoadReport) {
    let active = ResolvedConfig {
        path: USER.into(),
        is_project_level: false,
    };
    let files: HashMap<String, String> = files
        .iter()
        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
        .collect();
    let read = |path: &str| files.get(path).cloned();
    let access = SourceAccess {
        env: SourceEnv {
            active: &active,
            current_script: None,
            home: None,
            cwd: None,
            globalize: &globalize,
        },
        scope: FileAccessScope::ProjectOnly,
        read: &read,
    };
    let actions = crate::actions::specs::registry();
    let mut policy = LoadPolicy::new(project_vimrc)
        .with_actions(&actions)
        .with_files(&access);
    let mut controller = VimController::new();
    let config = ConfigText::new(USER, top).with_trust(Tier::Trusted.tag());
    let report = controller.load_config(config, &mut policy);
    assert_eq!(controller.engine().pending_request_count(), 0, "{top}");
    (controller, report)
}

#[test]
fn a_nested_source_takes_the_tier_of_where_the_file_is() {
    let files = [
        ("user://a.vim", "nnoremap a :!id<CR>"),
        ("res://b.vim", "nnoremap b :!id<CR>\nnnoremap c gk"),
    ];
    let top = "source user://a.vim\nsource res://b.vim";
    let (controller, report) = load_with_files(ProjectVimrc::Sandbox, top, &files);
    assert!(maps(&controller, 'a'), "a user:// file is trusted");
    assert!(!maps(&controller, 'b'), "a project file is sandboxed");
    assert!(maps(&controller, 'c'));
    assert_eq!(report.refused, 1, "{:?}", report.lines);
    assert!(
        only_line(&report).contains("res://b.vim:1: E48: Not allowed in sandbox: "),
        "{:?}",
        report.lines
    );

    let (controller, report) = load_with_files(ProjectVimrc::Trusted, top, &files);
    assert!(maps(&controller, 'b'));
    assert!(report.lines.is_empty(), "{:?}", report.lines);
}

#[test]
fn a_nested_source_is_refused_where_the_file_may_not_be_read() {
    let files = [
        ("res://b.vim", "nnoremap b gk"),
        ("/etc/b.vim", "nnoremap b gk"),
    ];
    for (project_vimrc, top, error) in [
        (
            ProjectVimrc::Disabled,
            "source res://b.vim",
            "res://b.vim not sourced: Security > Project Vimrc is Disabled",
        ),
        (ProjectVimrc::Sandbox, "source /etc/b.vim", "/etc/b.vim"),
        (
            ProjectVimrc::Sandbox,
            "source user://none.vim",
            "E484: Can't open file user://none.vim",
        ),
    ] {
        let (controller, report) = load_with_files(project_vimrc, top, &files);
        assert!(!maps(&controller, 'b'), "{top}");
        let line = only_line(&report);
        assert!(line.starts_with(&format!("{USER}:1: ")), "{line}");
        assert!(line.contains(error), "{line}");
    }
}

// ── Reload ────────────────────────────────────────────────────────────

fn reload(controller: &mut VimController, text: Option<&str>) -> LoadReport {
    let actions = crate::actions::specs::registry();
    let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox)
        .with_actions(&actions)
        .collecting_panels();
    let config = text.map(|text| ConfigText::new(USER, text).with_trust(Tier::Trusted.tag()));
    let report = controller.reload_config(config, &mut policy);
    assert_eq!(controller.engine().pending_request_count(), 0);
    report
}

#[test]
fn reloading_twice_gives_the_same_keymap() {
    let text = "let mapleader = ','\n\
                nnoremap x gj\n\
                nmap y x\n\
                iabbrev teh the\n\
                sethandler <C-a> n:vim\n\
                w";
    let mut controller = VimController::new();
    let first = reload(&mut controller, Some(text));
    let once = state(controller.engine());
    let second = reload(&mut controller, Some(text));
    assert_eq!(first, second);
    assert_eq!(state(controller.engine()), once);
    assert_eq!(
        first.lines,
        [format!("{USER}:6: E523: Not allowed here: w")]
    );
}

#[test]
fn a_reload_without_a_file_still_resets_and_seeds() {
    let mut fresh = VimController::new();
    let report = reload(&mut fresh, None);
    assert!(report.lines.is_empty(), "{:?}", report.lines);
    let seeded = state(fresh.engine());

    let mut controller = VimController::new();
    let _ = reload(
        &mut controller,
        Some("let mapleader = ','\nnnoremap x gj\niabbrev teh the\nsethandler <C-a> n:vim"),
    );
    assert_ne!(state(controller.engine()), seeded);
    let _ = reload(&mut controller, None);
    assert_eq!(state(controller.engine()), seeded);
}

// ── Reporting ─────────────────────────────────────────────────────────

/// Vim heads errors from a sourced file with the file and line, then the
/// message (message.c `msg_source()`).
#[test]
fn errors_and_refusals_go_to_messages_too() {
    let (controller, report) = sandboxed("nnoremap x :!id<CR>\nset nosuch");
    assert_eq!(
        (report.errors, report.refused),
        (1, 1),
        "{:?}",
        report.lines
    );
    let history: Vec<String> = controller
        .engine()
        .state()
        .message_history()
        .entries()
        .iter()
        .map(|entry| entry.text.to_string())
        .collect();
    for line in &report.lines {
        let message = line
            .strip_prefix(&format!("{PROJECT}:"))
            .and_then(|rest| rest.split_once(": "))
            .map(|(_, message)| message)
            .expect("a placed line");
        assert!(
            history.iter().any(|entry| entry == message),
            "{message}: {history:?}"
        );
    }
    assert!(history
        .iter()
        .any(|entry| entry == &format!("Error detected while processing {PROJECT}:")));
}
