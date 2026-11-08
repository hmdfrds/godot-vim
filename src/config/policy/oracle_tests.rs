//! The text Sandbox of earlier versions as the policy's oracle: a line the
//! policy lets through in a project vimrc is one the text Sandbox kept, or
//! one listed as relaxed with the reason; a line the text Sandbox kept and
//! the policy refuses is a new refusal, listed too.

use vim_core::effects::Effect;
use vim_core::execution::{
    parse_keys_from_string, ConfigCommand, ConfigHost, ConfigText, HostRequest, HostSession,
    SourceReply, SourceRequest, Verdict,
};
use vim_core::keymap::KeyEvent;

use super::tests::{vectors, Expected};
use super::{LoadPolicy, Tier};
use crate::controller::VimController;
use crate::settings::ProjectVimrc;

const PROJECT: &str = "res://.godot-vimrc";

/// The policy, with what it said of each command of the top file.
struct Recorder<'a> {
    policy: LoadPolicy<'a>,
    /// Line and whether the command there runs or is claimed.
    verdicts: Vec<(u32, bool)>,
}

impl ConfigHost for Recorder<'_> {
    fn judge(&mut self, command: &ConfigCommand<'_>) -> Verdict {
        let verdict = self.policy.judge(command);
        if command.depth == 1 {
            self.verdicts
                .push((command.line, !matches!(verdict, Verdict::Refuse(_))));
        }
        verdict
    }

    fn has_feature(&self, feature: &str) -> bool {
        self.policy.has_feature(feature)
    }

    fn source(&mut self, request: &SourceRequest<'_>) -> SourceReply {
        self.policy.source(request)
    }
}

/// Per line of a Sandbox `text`: `Some(false)` where the policy refuses a
/// command, `Some(true)` where it lets the line through, and `None` for a
/// blank or comment line, a `\` line continuing another, and a line with an
/// error. The `:if` lines and the branch not taken are let through
/// unjudged: vim-core evaluates conditions itself, and they run nothing.
fn policy_lines(text: &str) -> Vec<Option<bool>> {
    let actions = crate::actions::specs::registry();
    let mut recorder = Recorder {
        policy: LoadPolicy::new(ProjectVimrc::Sandbox).with_actions(&actions),
        verdicts: Vec::new(),
    };
    let mut controller = VimController::new();
    let config = ConfigText::new(PROJECT, text).with_trust(Tier::Sandbox.tag());
    let mut response = controller
        .engine_mut()
        .source_script(config, &mut recorder)
        .response;
    let mut lines: Vec<Option<bool>> = text
        .lines()
        .map(|line| {
            let line = line.trim_start();
            (!line.is_empty() && !line.starts_with(['"', '\\'])).then_some(true)
        })
        .collect();
    for effect in response.take_effects() {
        if let Effect::ShowError {
            source: Some(place),
            ..
        } = effect
        {
            lines[place.line as usize - 1] = None;
        }
    }
    for (line, allowed) in recorder.verdicts {
        let at = &mut lines[line as usize - 1];
        *at = Some(at.unwrap_or(true) && allowed);
    }
    lines
}

/// Per line of `text`: whether the text Sandbox refuses a command that
/// starts there, because it strips the line or a `\` line continuing it.
fn oracle_lines(text: &str) -> Vec<bool> {
    let stripped = crate::config::sandbox::stripped_lines(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut refused = vec![false; lines.len()];
    let mut continued = false;
    for at in (0..lines.len()).rev() {
        let stripped = stripped.get(at).copied().unwrap_or(true);
        let line = lines[at].trim_start();
        if line.starts_with('\\') || line.starts_with("\"\\ ") {
            continued |= stripped;
            refused[at] = stripped;
        } else {
            refused[at] = stripped || continued;
            continued = false;
        }
    }
    refused
}

/// How the policy and the text Sandbox differ on a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Diff {
    /// The policy lets it through, the text Sandbox stripped it.
    Relaxed,
    /// The text Sandbox kept it, the policy refuses it.
    NewRefusal,
}

/// The lines of `text` where the two differ, numbered from 1.
fn diffs(text: &str) -> Vec<(u32, Diff)> {
    policy_lines(text)
        .into_iter()
        .zip(oracle_lines(text))
        .zip(1..)
        .filter_map(|((policy, refused), line)| match (policy, refused) {
            (Some(true), true) => Some((line, Diff::Relaxed)),
            (Some(false), false) => Some((line, Diff::NewRefusal)),
            _ => None,
        })
        .collect()
}

/// Each vector of the text Sandbox's tests is tagged with how the two
/// judge it, so the relaxed and the newly refused vectors are exactly those
/// tagged so.
#[test]
fn every_vector_is_judged_by_the_oracle_and_the_policy_as_tagged() {
    let mut counts = [0; 4];
    for (expected, line, reason) in vectors() {
        let diff = diffs(&line);
        let oracle_refused = oracle_lines(&line).first().copied().unwrap_or(false);
        let at = match expected {
            Expected::Kept => {
                assert!(diff.is_empty() && !oracle_refused, "{line}");
                0
            }
            Expected::Stripped => {
                assert!(diff.is_empty() && oracle_refused, "{line}");
                1
            }
            Expected::Refused => {
                assert_eq!(diff, [(1, Diff::NewRefusal)], "{line}");
                2
            }
            Expected::Relaxed => {
                assert_eq!(diff, [(1, Diff::Relaxed)], "{line}: {reason}");
                3
            }
        };
        counts[at] += 1;
    }
    assert_eq!(counts, [1643, 1061, 497, 99]);
}

/// A relaxed vector runs no shell, sources nothing, maps nothing and runs
/// none of godot-vim's commands when its keys are pressed: what the text
/// Sandbox stripped it for does not happen.
#[test]
fn a_relaxed_vector_reaches_nothing_the_sandbox_refuses() {
    for (expected, line, reason) in vectors() {
        if expected != Expected::Relaxed {
            continue;
        }
        let mut session = HostSession::new("a\nab|\nx\n");
        let _ = session.source_config(&line);
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or_default();
        let lhs = words.next().unwrap_or_default().replace("<Leader>", "\\");
        let mut keys = Vec::new();
        if command.starts_with('i') {
            keys.push(KeyEvent::char('i'));
        }
        keys.extend(parse_keys_from_string(&lhs));
        keys.push(KeyEvent::enter());
        for key in keys {
            for request in session.process_key_host(key).host_requests {
                let refused = match &request {
                    HostRequest::ExternalCommand { .. } | HostRequest::ReadConfigFile { .. } => {
                        true
                    }
                    HostRequest::CustomExCommand { command, .. } => {
                        crate::host::names_own_command(command)
                    }
                    _ => false,
                };
                assert!(!refused, "{line} ({reason}): {request:?}");
            }
        }
        assert!(
            !session.engine().could_start_mapping(KeyEvent::char('y')),
            "{line}"
        );
    }
}

/// A vimrc written for Vim and GodotVim both. The policy lets through what
/// the text Sandbox stripped on the lines in `RELAXED`, and refuses what it
/// kept on the lines in `NEW_REFUSALS`.
const SHARED_VIMRC: &str = r#"" Shared with Vim
set nocompatible
set ic scs so=5
let mapleader = ","
if has('godot')
  set tw=0
else
  syntax on
endif
nnoremap j gj
nnoremap <leader>n :noh<CR>
nnoremap <leader>h
      \ :noh<CR>
mapclear!
echo "loaded"
noh
set so=5 " the | is in the comment
panelmap preset
nnoremap <expr> <leader>E ':noh<CR>'
nnoremap <leader>q @q
nnoremap <leader>Q :@q<CR>
nnoremap <leader>f !ip
nnoremap <leader>d :normal dd<CR>
nnoremap <leader>g :g/x/normal dd<CR>
nnoremap <leader>c q:
nnoremap <leader>W :w other.gd<CR>
nnoremap <leader>v :mkvimrc<CR>
nnoremap <leader>s :action editor/close_scene<CR>
nnoremap <leader>a <Action>(editor/save_scene)
nnoremap <leader>r :run<CR>
nnoremap <leader>S :save<CR>
nnoremap <leader>p "+p
"#;

const RELAXED: &[(u32, &str)] = &[
    (5, "a condition vim-core evaluates itself, running nothing"),
    (7, "an :if line"),
    (8, "in the branch not taken"),
    (9, "an :if line"),
    (12, "a continued line, judged with the line it continues"),
    (14, ":mapclear only removes mappings"),
    (15, ":echo of a string"),
    (16, ":noh"),
    (17, "the | is in the comment of :set"),
    (18, "a later version's opt-in, claimed without a word"),
];

const NEW_REFUSALS: &[(u32, &str)] = &[
    (19, "<expr>"),
    (20, "runs a register"),
    (21, "runs a register"),
    (22, "runs the shell"),
    (23, "runs keys"),
    (24, "runs keys"),
    (25, "command-line window"),
    (26, "writes a named file"),
    (27, "writes a vimrc"),
    (28, "editor shortcut"),
    (29, "<Action>(editor/save_scene)"),
    (30, "godot-vim command"),
    (31, "godot-vim command"),
];

#[test]
fn a_shared_vimrc_differs_from_the_oracle_only_where_listed() {
    let mut expected: Vec<(u32, Diff)> = RELAXED
        .iter()
        .map(|(line, _)| (*line, Diff::Relaxed))
        .chain(
            NEW_REFUSALS
                .iter()
                .map(|(line, _)| (*line, Diff::NewRefusal)),
        )
        .collect();
    expected.sort_by_key(|(line, _)| *line);
    assert_eq!(diffs(SHARED_VIMRC), expected);

    let actions = crate::actions::specs::registry();
    let mut policy = LoadPolicy::new(ProjectVimrc::Sandbox).with_actions(&actions);
    let mut controller = VimController::new();
    let config = ConfigText::new(PROJECT, SHARED_VIMRC).with_trust(Tier::Sandbox.tag());
    let report = controller.load_config(config, &mut policy);
    assert_eq!(report.errors, 0, "{:?}", report.lines);
    assert_eq!(report.lines.len(), NEW_REFUSALS.len(), "{:?}", report.lines);
    for ((line, why), refusal) in NEW_REFUSALS.iter().zip(&report.lines) {
        assert!(
            refusal.starts_with(&format!("{PROJECT}:{line}: E48: ")) && refusal.contains(why),
            "{refusal}"
        );
    }
}
