//! Fold operations: FoldOpen, FoldClose, FoldToggle, FoldAll.

use godot::classes::CodeEdit;
use godot::prelude::*;

pub fn handle_fold_open(editor: &mut Gd<CodeEdit>) {
    let line = editor.get_caret_line();
    editor.unfold_line(line);
}

pub fn handle_fold_close(editor: &mut Gd<CodeEdit>) {
    let line = editor.get_caret_line();
    editor.fold_line(line);
}

pub fn handle_fold_toggle(editor: &mut Gd<CodeEdit>) {
    let line = editor.get_caret_line();
    if editor.is_line_folded(line) {
        editor.unfold_line(line);
    } else {
        editor.fold_line(line);
    }
}
