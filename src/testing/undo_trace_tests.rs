//! `MockTextEdit`'s undo history against Godot's: `steps.txt` run on the
//! mock must read as `scripts/godot-undo-trace.sh` recorded it from a real
//! CodeEdit on each suite Godot, step by step.

use std::path::{Path, PathBuf};

use super::mock_text_edit::EditAction;
use crate::bridge::port::{FoldCapable, TextEditorPort};
use crate::testing::MockTextEdit;
use crate::types::CharLineCol;

fn trace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/godot-undo-trace")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (c, c == '\\') {
            (_, true) => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            (c, false) => out.push(c),
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// trace.gd's `reading()`.
fn reading(mock: &MockTextEdit, op: &str) -> String {
    let carets: Vec<String> = if matches!(op, "insert" | "remove") {
        vec!["*".to_owned()]
    } else {
        mock.carets_with_selection()
            .into_iter()
            .map(|(line, col, origin)| match origin {
                Some((ol, oc)) => format!("{line}:{col}/{ol}:{oc}"),
                None => format!("{line}:{col}"),
            })
            .collect()
    };
    let folded: Vec<String> = mock.folded_lines().iter().map(i32::to_string).collect();
    format!(
        "v={} undo={} redo={} carets={} folded={} text={}",
        mock.get_version(),
        u8::from(mock.has_undo()),
        u8::from(mock.has_redo()),
        carets.join(","),
        if folded.is_empty() {
            "-".to_owned()
        } else {
            folded.join(",")
        },
        escape(&mock.get_text())
    )
}

/// trace.gd's `run_steps()` on the mock.
fn mock_trace(steps: &str) -> Vec<String> {
    let mut mock = MockTextEdit::new("");
    let mut out = Vec::new();
    for line in steps.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (op, rest) = line.split_once(' ').unwrap_or((line, ""));
        let words: Vec<&str> = rest.split(' ').collect();
        let n = |i: usize| -> i32 { words[i].parse().expect("a number") };
        match op {
            "text" => mock = MockTextEdit::new(&unescape(rest)),
            "fold_region" => {
                mock.add_fold_region(n(0), n(1));
                continue;
            }
            "insert" => {
                let text = rest.splitn(3, ' ').nth(2).unwrap_or("");
                mock.insert_text(&unescape(text), n(0), n(1));
            }
            "remove" => mock.remove_text(n(0), n(1), n(2), n(3)),
            "begin" => mock.begin_complex_operation(),
            "end" => mock.end_complex_operation(),
            "typing" => mock.start_action(EditAction::Typing),
            "close" => mock.start_action(EditAction::None),
            "undo" => mock.undo(),
            "redo" => mock.redo(),
            "caret" => {
                mock.deselect();
                mock.remove_secondary_carets();
                mock.set_caret_line(n(0));
                mock.set_caret_column(n(1));
            }
            "select" => mock.select_for_caret(
                CharLineCol::new(n(0), n(1)),
                CharLineCol::new(n(2), n(3)),
                n(4),
            ),
            "add_caret" => {
                mock.add_caret(n(0), n(1));
            }
            "fold" => mock.fold_line(n(0)),
            _ => panic!("unknown step: {line}"),
        }
        out.push(format!("{line} | {}", reading(&mock, op)));
    }
    out
}

#[test]
fn mock_undo_reads_as_every_recorded_godot() {
    let dir = trace_dir();
    let steps = std::fs::read_to_string(dir.join("steps.txt")).expect("read steps.txt");
    let mock = mock_trace(&steps);
    let mut recordings = 0;
    for entry in std::fs::read_dir(dir.join("recorded")).expect("read recorded/") {
        let path = entry.expect("dir entry").path();
        let recorded = std::fs::read_to_string(&path).expect("read recording");
        let godot: Vec<&str> = recorded.lines().filter(|l| !l.starts_with('#')).collect();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        assert_eq!(godot.len(), mock.len(), "{name}: steps recorded");
        let differ: Vec<String> = godot
            .iter()
            .zip(&mock)
            .filter(|(g, m)| *g != *m)
            .map(|(g, m)| format!("\n  godot {g}\n  mock  {m}"))
            .collect();
        assert!(differ.is_empty(), "{name}:{}", differ.concat());
        recordings += 1;
    }
    assert!(recordings > 0, "no recordings in {}", dir.display());
}
