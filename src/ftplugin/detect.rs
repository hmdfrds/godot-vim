//! Filetype detection for a CodeEdit, from what Godot exposes about it.
//!
//! Godot has no filetype and gives an extension no public way to ask which
//! file a script editor tab shows. The signals below are the ones verified
//! in a headless Godot 4.7 editor, in the order they are trusted:
//!
//! 1. **The script of the tab that owns the CodeEdit.** Only when that tab is
//!    the script editor's current tab: `ScriptEditor.get_current_script()`
//!    answers for the current tab, and the shader editor's CodeEdit is not
//!    in a script tab at all, so asking without the check labels a shader
//!    with the last script's language. `get_class()` is `"GDScript"` for
//!    files and for built-in scripts (`res://x.tscn::GDScript_abc`).
//! 2. **The syntax highlighter's class.** Godot picks it by extension when a
//!    text tab opens (`EditorJSONSyntaxHighlighter` for `.json`, and so on),
//!    and it is the only signal that tells JSON, Markdown and plain text
//!    apart. If the user picks another highlighter from the menu, the class
//!    follows, which is what they asked Godot to treat the file as.
//! 3. **The comment delimiters.** Script tabs set them from the script's
//!    language and the shader editor sets `//` and `/* */`; text tabs set
//!    none. `#` is taken as GDScript. `//` is ambiguous (shader or C#) and
//!    an empty list only says "some text file", so both give no filetype:
//!    a plain-text guess would let JSON wrap.
//!
//! [`detect`] is a pure function over those values, so the order is tested
//! without Godot; [`read_signals`] collects them from the scene tree.

/// A filetype godot-vim can recognize. The names are Vim's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Filetype {
    GdScript,
    GdShader,
    Cs,
    Json,
    Markdown,
    Text,
    Cfg,
}

impl Filetype {
    /// Vim's name for the filetype, which filetype-specific mappings match.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::GdScript => "gdscript",
            Self::GdShader => "gdshader",
            Self::Cs => "cs",
            Self::Json => "json",
            Self::Markdown => "markdown",
            Self::Text => "text",
            Self::Cfg => "cfg",
        }
    }
}

/// What Godot says about one CodeEdit, read at attach.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Signals {
    /// `get_class()` of the current script, set only when the tab that owns
    /// the CodeEdit is the script editor's current tab.
    pub(crate) script_class: Option<String>,
    /// `get_class()` of the CodeEdit's syntax highlighter.
    pub(crate) highlighter_class: Option<String>,
    /// `CodeEdit.get_comment_delimiters()`: line delimiters are one token
    /// (`#`), block delimiters a space-separated pair (`/* */`).
    pub(crate) comment_delimiters: Vec<String>,
}

/// The filetype the signals point to, trying them in the order of the
/// module docs. `None` means "unknown": no filetype plugin runs.
pub(crate) fn detect(signals: &Signals) -> Option<Filetype> {
    if let Some(ft) = signals.script_class.as_deref().and_then(from_script_class) {
        return Some(ft);
    }
    if let Some(ft) = signals
        .highlighter_class
        .as_deref()
        .and_then(from_highlighter_class)
    {
        return Some(ft);
    }
    from_delimiters(&signals.comment_delimiters)
}

fn from_script_class(class: &str) -> Option<Filetype> {
    match class {
        "GDScript" => Some(Filetype::GdScript),
        "CSharpScript" => Some(Filetype::Cs),
        _ => None,
    }
}

/// Class names verified in Godot 4.7. They are the engine's internal class
/// names, not exposed in `extension_api.json`, so a rename in a later Godot
/// falls through to the delimiters instead of failing.
fn from_highlighter_class(class: &str) -> Option<Filetype> {
    match class {
        "GDScriptSyntaxHighlighter" => Some(Filetype::GdScript),
        "GDShaderSyntaxHighlighter" => Some(Filetype::GdShader),
        "EditorJSONSyntaxHighlighter" => Some(Filetype::Json),
        "EditorMarkdownSyntaxHighlighter" => Some(Filetype::Markdown),
        "EditorPlainTextSyntaxHighlighter" => Some(Filetype::Text),
        "EditorConfigFileSyntaxHighlighter" => Some(Filetype::Cfg),
        _ => None,
    }
}

fn from_delimiters(delimiters: &[String]) -> Option<Filetype> {
    delimiters
        .iter()
        .any(|d| d == "#")
        .then_some(Filetype::GdScript)
}

/// The line comment delimiter to build a `commentstring` from: the
/// shortest single-token delimiter. GDScript registers both `#` and `##`
/// (doc comments), and the regular prefix is the shorter one. Block pairs
/// (`/* */`) contain a space and are skipped.
pub(crate) fn line_comment_delimiter(delimiters: &[String]) -> Option<&str> {
    delimiters
        .iter()
        .map(String::as_str)
        .filter(|d| !d.is_empty() && !d.contains(' '))
        .min_by_key(|d| d.len())
}

/// Collect the [`Signals`] for `editor` from the scene tree.
pub(crate) fn read_signals(editor: &godot::obj::Gd<godot::classes::CodeEdit>) -> Signals {
    use godot::classes::{EditorInterface, ScriptEditorBase};
    use godot::prelude::*;

    /// The CodeEdit sits at most a few levels under its tab:
    /// `CodeEdit < CodeTextEditor < [VSplitContainer <] ScriptTextEditor`.
    const MAX_OWNER_DEPTH: usize = 6;

    let mut owner = None;
    let mut node = editor.get_parent();
    for _ in 0..MAX_OWNER_DEPTH {
        let Some(current) = node else { break };
        match current.try_cast::<ScriptEditorBase>() {
            Ok(tab) => {
                owner = Some(tab);
                break;
            }
            Err(current) => node = current.get_parent(),
        }
    }

    let script_class = owner.and_then(|tab| {
        let mut script_editor = EditorInterface::singleton().get_script_editor()?;
        let current = script_editor.get_current_editor()?;
        if current.instance_id() != tab.instance_id() {
            return None;
        }
        let script = script_editor.get_current_script()?;
        Some(script.get_class().to_string())
    });

    let highlighter_class = editor
        .get_syntax_highlighter()
        .map(|h| h.get_class().to_string());

    let delimiters = editor.get_comment_delimiters();
    let comment_delimiters = (0..delimiters.len())
        .filter_map(|i| delimiters.get(i))
        .map(|d| d.to_string())
        .collect();

    Signals {
        script_class,
        highlighter_class,
        comment_delimiters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(script: Option<&str>, highlighter: Option<&str>, delimiters: &[&str]) -> Signals {
        Signals {
            script_class: script.map(str::to_owned),
            highlighter_class: highlighter.map(str::to_owned),
            comment_delimiters: delimiters.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    /// One row per file type in the Godot 4.7 probe, with the values it
    /// recorded for each.
    #[test]
    fn verified_probe_values() {
        let rows: [(&str, Signals, Option<Filetype>); 7] = [
            (
                "a.gd",
                sig(
                    Some("GDScript"),
                    Some("GDScriptSyntaxHighlighter"),
                    &["##", "#"],
                ),
                Some(Filetype::GdScript),
            ),
            (
                "built-in script in scene.tscn",
                sig(
                    Some("GDScript"),
                    Some("GDScriptSyntaxHighlighter"),
                    &["##", "#"],
                ),
                Some(Filetype::GdScript),
            ),
            (
                "d.json",
                sig(None, Some("EditorJSONSyntaxHighlighter"), &[]),
                Some(Filetype::Json),
            ),
            (
                "r.md",
                sig(None, Some("EditorMarkdownSyntaxHighlighter"), &[]),
                Some(Filetype::Markdown),
            ),
            (
                "t.txt",
                sig(None, Some("EditorPlainTextSyntaxHighlighter"), &[]),
                Some(Filetype::Text),
            ),
            (
                "c.cfg",
                sig(None, Some("EditorConfigFileSyntaxHighlighter"), &[]),
                Some(Filetype::Cfg),
            ),
            (
                // The shader editor is not a script tab, so no script class
                // even though get_current_script() still names the last one.
                "s.gdshader",
                sig(None, Some("GDShaderSyntaxHighlighter"), &["//", "/* */"]),
                Some(Filetype::GdShader),
            ),
        ];
        for (file, signals, want) in rows {
            assert_eq!(detect(&signals), want, "{file}");
        }
    }

    #[test]
    fn the_script_class_wins_over_the_highlighter() {
        let s = sig(
            Some("GDScript"),
            Some("EditorPlainTextSyntaxHighlighter"),
            &[],
        );
        assert_eq!(detect(&s), Some(Filetype::GdScript));
    }

    #[test]
    fn an_unknown_script_class_falls_through() {
        let s = sig(
            Some("SomeAddonScript"),
            Some("GDShaderSyntaxHighlighter"),
            &[],
        );
        assert_eq!(detect(&s), Some(Filetype::GdShader));
    }

    /// The probe swapped a .txt tab's highlighter to GDScript: the class
    /// followed and the delimiters stayed empty.
    #[test]
    fn a_text_tab_switched_to_the_gdscript_highlighter_is_gdscript() {
        let s = sig(None, Some("GDScriptSyntaxHighlighter"), &[]);
        assert_eq!(detect(&s), Some(Filetype::GdScript));
    }

    #[test]
    fn an_unknown_highlighter_falls_back_to_delimiters() {
        let s = sig(None, Some("SomeAddonHighlighter"), &["##", "#"]);
        assert_eq!(detect(&s), Some(Filetype::GdScript));
    }

    #[test]
    fn slash_delimiters_alone_are_ambiguous() {
        assert_eq!(detect(&sig(None, None, &["//", "/* */"])), None);
    }

    /// JSON, Markdown and text all have no delimiters, so a guess here could
    /// give JSON the text plugin, which keeps `t` and wraps.
    #[test]
    fn no_signal_is_unknown_not_text() {
        assert_eq!(detect(&Signals::default()), None);
    }

    #[test]
    fn csharp_scripts_are_cs() {
        let s = sig(Some("CSharpScript"), None, &["//", "/* */"]);
        assert_eq!(detect(&s), Some(Filetype::Cs));
    }

    #[test]
    fn line_delimiter_is_the_shortest_single_token() {
        let d = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(line_comment_delimiter(&d(&["##", "#"])), Some("#"));
        assert_eq!(line_comment_delimiter(&d(&["/* */", "//"])), Some("//"));
        assert_eq!(line_comment_delimiter(&d(&["/* */"])), None);
        assert_eq!(line_comment_delimiter(&d(&[])), None);
    }
}
