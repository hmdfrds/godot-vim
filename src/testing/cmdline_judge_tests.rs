//! A Sandbox mapping's command line judged when it runs: configs loaded
//! through the controller, keys pressed on the mock editor.

use vim_core::execution::{ConfigText, HostRequest, TrustAll, TrustTag};
use vim_core::keymap::KeyEvent;

use super::mock_host::Harness;
use crate::config::policy::{LoadPolicy, Tier};
use crate::controller::{LoadReport, VimController};
use crate::settings::ProjectVimrc;
use crate::types::StatusMessage;

const TEXT: &str = "a: b/c?d\nfoo:bar\n\nx\n";
const PROJECT: &str = "res://.godot-vimrc";
const USER: &str = "user://.godot-vimrc";

/// What a vector does when it runs.
#[derive(Debug, Clone, Copy)]
enum Effect {
    Langremap,
    CommaLeader,
    /// Asks the host to run the shell or source a file.
    Asks,
}

impl Effect {
    fn seen(self, harness: &Harness) -> bool {
        match self {
            Self::Langremap => harness.engine().options().langremap(),
            Self::CommaLeader => harness.engine().leader_keys() == [KeyEvent::char(',')],
            Self::Asks => asks(harness),
        }
    }

    /// Press `keys`: whether they had the effect.
    fn press(self, harness: Harness, keys: &str) -> (Harness, bool) {
        let before = self.seen(&harness);
        let harness = press(harness, keys);
        let after = self.seen(&harness);
        (harness, after && !before)
    }
}

fn asks(harness: &Harness) -> bool {
    harness.requests().iter().any(|request| {
        matches!(
            request,
            HostRequest::ExternalCommand { .. }
                | HostRequest::ReadFile { .. }
                | HostRequest::ReadConfigFile { .. }
                | HostRequest::FilterDocumentRange { .. }
                | HostRequest::ForEachWindow { .. }
        )
    })
}

/// A vector: its config, the keys the user presses, the command the
/// Sandbox refuses and what running it would do.
struct Vector {
    config: &'static str,
    keys: &'static str,
    refused: &'static str,
    effect: Effect,
}

/// `config` loaded as the active config `name` at `tier`, the way every
/// load goes, on a buffer of [`TEXT`].
fn loaded(name: &str, tier: Tier, config: &str) -> (Harness, LoadReport) {
    let actions = crate::actions::specs::registry();
    let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox).with_actions(&actions);
    let mut controller = VimController::new();
    let report = controller.reload_config(
        Some(ConfigText::new(name, config).with_trust(tier.tag())),
        &mut policy,
    );
    (harness(controller), report)
}

/// `config` run in full with the tag `tag` after a load installed the
/// judge, so a mapping the load policy refuses is there and only the
/// run-time judge stands in its way.
fn past_load(tag: TrustTag, config: &str) -> Harness {
    let actions = crate::actions::specs::registry();
    let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox).with_actions(&actions);
    let mut controller = VimController::new();
    let _ = controller.reload_config(None, &mut policy);
    let _ = controller.engine_mut().source_script(
        ConfigText::new(PROJECT, config).with_trust(tag),
        &mut TrustAll::new(),
    );
    harness(controller)
}

fn harness(mut controller: VimController) -> Harness {
    let engine = std::mem::replace(
        controller.engine_mut(),
        vim_core::execution::VimEngine::new(),
    );
    Harness::with_engine(engine, TEXT)
}

fn press(mut harness: Harness, keys: &str) -> Harness {
    harness.feed(keys);
    harness
}

fn sandbox_errors(harness: &Harness) -> Vec<String> {
    harness
        .engine()
        .state()
        .message_history()
        .to_vec()
        .into_iter()
        .map(|entry| entry.text.to_string())
        .filter(|text| text.contains("(Sandbox mapping: "))
        .collect()
}

/// Refused at run time when the load policy is out of the way, with the
/// error in the status bar and `:messages`; never takes effect from a
/// project vimrc; runs from `user://`.
fn check(vector: &Vector) {
    let keys = vector.keys;
    let refusal = format!(
        "E48: Not allowed in sandbox: {} (Sandbox mapping: ",
        vector.refused
    );

    let (judged, ran) = vector
        .effect
        .press(past_load(Tier::Sandbox.tag(), vector.config), keys);
    assert!(!ran, "{keys}: took effect");
    assert!(!asks(&judged), "{keys}: {:?}", judged.requests());
    let errors = sandbox_errors(&judged);
    assert!(
        errors.iter().any(|error| error.starts_with(&refusal)),
        "{keys}: {errors:?}"
    );
    match judged.status() {
        StatusMessage::Error(error) => assert!(error.starts_with(&refusal), "{keys}: {error}"),
        other => panic!("{keys}: {other:?}"),
    }

    let (project, _) = loaded(PROJECT, Tier::Sandbox, vector.config);
    let (project, ran) = vector.effect.press(project, keys);
    assert!(!ran, "{keys}: took effect");
    assert!(!asks(&project), "{keys}: {:?}", project.requests());

    let (user, report) = loaded(USER, Tier::Trusted, vector.config);
    assert!(report.lines.is_empty(), "{:?}", report.lines);
    let (user, ran) = vector.effect.press(user, keys);
    assert!(ran, "{keys}: did not run from {USER}");
    assert!(sandbox_errors(&user).is_empty());
}

/// The four ways past the load policy the run-time judge closes.
const OPEN_AFTER_LOAD: [Vector; 4] = [
    // A letter typed into a line left open.
    Vector {
        config: "set nolangremap\nnnoremap x :set langremap<Left><C-h>",
        keys: "xa<CR>",
        refused: "set langremap",
        effect: Effect::Langremap,
    },
    // A `|` typed before a command-line mapping.
    Vector {
        config: "cnoremap <CR> let mapleader=','<Bar><CR>",
        keys: ":noh |<CR>",
        refused: "let mapleader=','",
        effect: Effect::CommaLeader,
    },
    // A command the user starts that a command-line mapping completes.
    Vector {
        config: "cnoremap x <Space>f",
        keys: ":sox<CR>",
        refused: "so f",
        effect: Effect::Asks,
    },
    // A `!` typed into a line left open.
    Vector {
        config: "nnoremap x :r",
        keys: "x !echo hi<CR>",
        refused: "r !echo hi",
        effect: Effect::Asks,
    },
];

#[test]
fn what_the_user_types_into_a_sandbox_line_is_judged() {
    for vector in &OPEN_AFTER_LOAD {
        check(vector);
    }
}

/// Three of the mappings pass the load policy, so only the run-time judge
/// stops them; the fourth sets the leader on its own.
#[test]
fn the_load_policy_keeps_three_of_the_four() {
    let kept: Vec<bool> = OPEN_AFTER_LOAD
        .iter()
        .map(|vector| {
            let (_, report) = loaded(PROJECT, Tier::Sandbox, vector.config);
            report.lines.iter().all(|line| !line.contains("noremap"))
        })
        .collect();
    assert_eq!(kept, [true, false, true, true]);
}

/// A `:source` from a Sandbox line never reaches the host, which would
/// trust the file by where it is.
#[test]
fn a_source_from_a_sandbox_line_is_refused() {
    for (config, keys) in [
        ("cnoremap x <Space>user://.godot-vimrc", ":sox<CR>"),
        ("nnoremap x :so<Cmd>nosuchcommand<CR><Space>f", "x<CR>"),
    ] {
        let judged = press(past_load(Tier::Sandbox.tag(), config), keys);
        assert!(
            !judged
                .requests()
                .iter()
                .any(|request| matches!(request, HostRequest::ReadConfigFile { .. })),
            "{config}: {:?}",
            judged.requests()
        );
        assert!(!sandbox_errors(&judged).is_empty(), "{config}");
    }
}

#[test]
fn a_line_left_open_is_judged_with_what_the_user_types() {
    for (config, keys, refused, effect) in [
        (
            "nnoremap x :let mapleader=','<Bar>",
            "x <CR>",
            "let mapleader=','",
            Effect::CommaLeader,
        ),
        (
            "set nolrm\nnnoremap x :set langremap<Bar>",
            "x1<CR>",
            "set langremap",
            Effect::Langremap,
        ),
        (
            "set nolrm\nnnoremap x :set lrm<Bar>",
            "x1<CR>",
            "set lrm",
            Effect::Langremap,
        ),
        (
            "set nolrm\nnnoremap x :setlocal langremap<Bar>",
            "x1<CR>",
            "setlocal langremap",
            Effect::Langremap,
        ),
        (
            "set nolrm\nnnoremap x :setglobal langremap<Bar>",
            "x1<CR>",
            "setglobal langremap",
            Effect::Langremap,
        ),
        ("nnoremap x :sof<Left>", "x <CR>", "so f", Effect::Asks),
        // The host is asked for the command vim-core lacks, so no error
        // cuts the mapping short here.
        (
            "nnoremap x :so<Cmd>nosuchcommand<CR><Space>f",
            "x<CR>",
            "so f",
            Effect::Asks,
        ),
        (
            "nnoremap x <Cmd>!echo hi",
            "x<CR>",
            "!echo hi",
            Effect::Asks,
        ),
    ] {
        check(&Vector {
            config,
            keys,
            refused,
            effect,
        });
    }
}

#[test]
fn a_cmap_is_judged_with_the_users_text() {
    for (config, keys, refused, effect) in [
        (
            "set nolrm\ncnoremap x langremap",
            ":set x<CR>",
            "set langremap",
            Effect::Langremap,
        ),
        (
            "cnoremap x mapleader=','",
            ":let x<CR>",
            "let mapleader=','",
            Effect::CommaLeader,
        ),
        (
            "cnoremap x let mapleader=','<Bar>",
            ":xx<CR>",
            "let mapleader=','",
            Effect::CommaLeader,
        ),
        (
            "cnoremap x <,'>!echo hi",
            "Vj<Esc>:'x<CR>",
            "'<,'>!echo hi",
            Effect::Asks,
        ),
    ] {
        check(&Vector {
            config,
            keys,
            refused,
            effect,
        });
    }
}

#[test]
fn a_visual_range_and_a_windo_are_judged() {
    for (config, keys, refused) in [
        ("vnoremap x :!echo hi<CR>", "Vx", "1,1!echo hi"),
        ("nnoremap x :windo r !echo hi<CR>", "x", "windo r !echo hi"),
    ] {
        check(&Vector {
            config,
            keys,
            refused,
            effect: Effect::Asks,
        });
    }
}

/// A `:` that may be the argument of the key before it.
#[test]
fn a_colon_after_a_key_that_takes_one_never_runs_the_shell() {
    for rhs in [
        "f:w:!echo hi<CR>",
        "t:j:!echo hi<CR>",
        "wya:x:!echo hi<CR>",
        "d:T<Del>*w:<Left>!echo hi<CR>",
    ] {
        let config = format!("nnoremap x {rhs}");
        let judged = press(past_load(Tier::Sandbox.tag(), &config), "x");
        assert!(!asks(&judged), "{rhs}: {:?}", judged.requests());
    }
}

/// A search line runs no command, and a line of ranges runs nothing the
/// Sandbox refuses.
#[test]
fn what_runs_no_refused_command_still_runs() {
    let deep = format!("nnoremap x /{}a{}<CR>", "\\%(".repeat(12), "\\)".repeat(12));
    let cmd_ends = format!(
        "nnoremap x {}{}{}",
        ":".repeat(16),
        "1,".repeat(500),
        "<Cmd><CR>".repeat(500)
    );
    for (config, keys) in [
        (deep.as_str(), "x"),
        ("vnoremap // y/<C-r>\"<CR>", "vl//"),
        ("nnoremap x /<C-r><C-w><CR>", "x"),
        (cmd_ends.as_str(), "x"),
        ("nnoremap x :set tw=40<CR>", "x"),
    ] {
        let judged = press(past_load(Tier::Sandbox.tag(), config), keys);
        assert!(sandbox_errors(&judged).is_empty(), "{config}");
    }
    let judged = press(
        past_load(Tier::Sandbox.tag(), "nnoremap x :set tw=40<CR>"),
        "x",
    );
    assert_eq!(judged.engine().options().textwidth(), 40);
}

/// A line holding a Sandbox key is judged whatever other keys it holds; an
/// unknown tag is judged as the Sandbox.
#[test]
fn the_strictest_tag_of_a_line_decides() {
    let actions = crate::actions::specs::registry();
    let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox).with_actions(&actions);
    let mut controller = VimController::new();
    let _ = controller.reload_config(None, &mut policy);
    for (config, tag) in [
        ("nnoremap y :r", Tier::Trusted),
        ("cnoremap z <Space>!echo hi", Tier::Sandbox),
    ] {
        let _ = controller.engine_mut().source_script(
            ConfigText::new(PROJECT, config).with_trust(tag.tag()),
            &mut TrustAll::new(),
        );
    }
    let mixed = press(harness(controller), "yz<CR>");
    assert!(!asks(&mixed), "{:?}", mixed.requests());
    assert_eq!(sandbox_errors(&mixed).len(), 1);

    let unknown = press(
        past_load(TrustTag::new(7), "nnoremap x :r !echo hi<CR>"),
        "x",
    );
    assert!(!asks(&unknown));
    assert_eq!(sandbox_errors(&unknown).len(), 1);

    let trusted = press(
        past_load(Tier::Trusted.tag(), "nnoremap x :r !echo hi<CR>"),
        "x",
    );
    assert!(asks(&trusted));
}

/// Each load installs the judge with its policy's action names.
#[test]
fn a_reload_replaces_the_judge() {
    let actions = crate::actions::specs::registry();
    let (_, spec) = actions.iter().next().expect("an action");
    let config = format!("nnoremap x :action {}<CR>", spec.id);

    let mut controller = VimController::new();
    let _ = controller.reload_config(None, &mut LoadPolicy::new(ProjectVimrc::Sandbox));
    let _ = controller.engine_mut().source_script(
        ConfigText::new(PROJECT, &config).with_trust(Tier::Sandbox.tag()),
        &mut TrustAll::new(),
    );
    let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox).with_actions(&actions);
    let _ = controller.load_config(ConfigText::new(USER, ""), &mut policy);
    let allowed = press(harness(controller), "x");
    assert!(sandbox_errors(&allowed).is_empty());

    let mut controller = VimController::new();
    let _ = controller.reload_config(None, &mut policy);
    let _ = controller.engine_mut().source_script(
        ConfigText::new(PROJECT, &config).with_trust(Tier::Sandbox.tag()),
        &mut TrustAll::new(),
    );
    let _ = controller.load_config(
        ConfigText::new(USER, ""),
        &mut LoadPolicy::new(ProjectVimrc::Sandbox),
    );
    let refused = press(harness(controller), "x");
    assert_eq!(sandbox_errors(&refused).len(), 1);
}
