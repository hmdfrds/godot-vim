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
//!    and it tells JSON, Markdown and config files apart. If the user picks
//!    another highlighter from the menu, the class follows, which is what
//!    they asked Godot to treat the file as. The plain-text highlighter is
//!    the exception: Godot gives it to every text file it has no highlighter
//!    for (`.txt`, `.log`, `.yml`, `.yaml`, `.toml`, `.xml` by default), so
//!    for it **the file's extension** decides, read from the script list
//!    (see [`script_list_path`]). Only `.txt` is `text`; a path that cannot
//!    be read or an extension Vim has no filetype for, like `.log`, gives
//!    none.
//! 3. **The comment delimiters.** Script tabs set them from the script's
//!    language and the shader editor sets `//` and `/* */`; text tabs set
//!    none. `#` is taken as GDScript. `//` is ambiguous (shader or C#) and
//!    an empty list only says "some text file", so both give no filetype:
//!    a plain-text guess would let JSON or YAML wrap.
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
    Yaml,
    Xml,
    Toml,
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
            Self::Yaml => "yaml",
            Self::Xml => "xml",
            Self::Toml => "toml",
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
    /// The `res://` path of the file the tab shows, when Godot's script
    /// list names it. Read only to tell plain-text files apart.
    pub(crate) file_path: Option<String>,
}

/// The filetype the signals point to, trying them in the order of the
/// module docs. `None` means "unknown": no filetype plugin runs.
pub(crate) fn detect(signals: &Signals) -> Option<Filetype> {
    if let Some(ft) = signals.script_class.as_deref().and_then(from_script_class) {
        return Some(ft);
    }
    // Godot's plain-text highlighter is its catch-all for text files, so it
    // says nothing about the file: the extension decides, and without one
    // the delimiters do (text tabs have none, a script tab keeps its own).
    let from_highlighter = match signals.highlighter_class.as_deref() {
        Some(PLAIN_TEXT_HIGHLIGHTER) => signals.file_path.as_deref().and_then(from_extension),
        Some(class) => from_highlighter_class(class),
        None => None,
    };
    from_highlighter.or_else(|| from_delimiters(&signals.comment_delimiters))
}

/// The highlighter Godot gives every text file whose extension it has no
/// highlighter for: with the default `docks/filesystem/textfile_extensions`
/// that is `.txt`, `.log`, `.yml`, `.yaml`, `.toml` and `.xml`.
const PLAIN_TEXT_HIGHLIGHTER: &str = "EditorPlainTextSyntaxHighlighter";

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
        "EditorConfigFileSyntaxHighlighter" => Some(Filetype::Cfg),
        _ => None,
    }
}

/// The filetype of a plain-text tab, from its file's extension, as Vim's
/// `filetype.vim` names it. `.log` has no filetype in Vim, and any other
/// extension a user adds to `textfile_extensions` stays unknown too. The
/// extensions Godot has its own highlighter for are listed as well, for a
/// tab the user switched to plain text.
fn from_extension(path: &str) -> Option<Filetype> {
    // A built-in resource is `res://scene.tscn::Resource_id`; only files.
    if path.contains("::") || !path.contains("://") {
        return None;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    match ext.to_ascii_lowercase().as_str() {
        "txt" => Some(Filetype::Text),
        "yml" | "yaml" => Some(Filetype::Yaml),
        "xml" => Some(Filetype::Xml),
        "toml" => Some(Filetype::Toml),
        "json" => Some(Filetype::Json),
        "md" | "markdown" => Some(Filetype::Markdown),
        "cfg" => Some(Filetype::Cfg),
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

    let file_path = owner.as_ref().and_then(script_list_path);

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
        file_path,
    }
}

/// The path of the file `tab` shows, from the script editor's script list.
///
/// Godot keeps no public path for a text tab, but the script list shows it:
/// each item's metadata is its tab's index in the script editor's
/// TabContainer, and its tooltip is the edited resource's path (Godot 4.7
/// `ScriptEditor::_update_script_names`; `DocumentList::update_list` after
/// it). The member and help outlines are ItemLists too, with line numbers
/// for metadata but no tooltips, so only a tooltip that is a path counts.
///
/// `None` when the item is not listed: the script list filter hides it, or
/// the file is unsaved (its tooltip is "Unsaved file."). Attach runs two
/// deferred calls after the focus change, by which time a new tab is in
/// the list.
fn script_list_path(tab: &godot::obj::Gd<godot::classes::ScriptEditorBase>) -> Option<String> {
    use godot::classes::{Control, EditorInterface, ItemList, TabContainer};
    use godot::prelude::*;

    let tabs = tab.get_parent()?.try_cast::<TabContainer>().ok()?;
    let index = i64::from(tabs.get_tab_idx_from_control(&tab.clone().upcast::<Control>()));
    if index < 0 {
        return None;
    }
    let script_editor = EditorInterface::singleton().get_script_editor()?;
    let lists = script_editor
        .find_children_ex("*")
        .type_("ItemList")
        .recursive(true)
        .owned(false)
        .done();
    lists
        .iter_shared()
        .filter_map(|node| node.try_cast::<ItemList>().ok())
        .find_map(|list| {
            (0..list.get_item_count()).find_map(|i| {
                let listed = list.get_item_metadata(i).try_to::<i64>().ok()?;
                let tooltip = list.get_item_tooltip(i).to_string();
                (listed == index && tooltip.contains("://")).then_some(tooltip)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(script: Option<&str>, highlighter: Option<&str>, delimiters: &[&str]) -> Signals {
        Signals {
            script_class: script.map(str::to_owned),
            highlighter_class: highlighter.map(str::to_owned),
            comment_delimiters: delimiters.iter().map(|d| (*d).to_owned()).collect(),
            file_path: None,
        }
    }

    fn plain_text(path: Option<&str>) -> Signals {
        Signals {
            file_path: path.map(str::to_owned),
            ..sig(None, Some("EditorPlainTextSyntaxHighlighter"), &[])
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
                plain_text(Some("res://t.txt")),
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

    /// Godot 4.7 opens every default text extension it has no highlighter
    /// for with the plain-text one, so the extension decides.
    #[test]
    fn plain_text_tabs_go_by_extension() {
        let rows = [
            ("res://t.txt", Some(Filetype::Text)),
            ("res://a/b.yml", Some(Filetype::Yaml)),
            ("res://b.yaml", Some(Filetype::Yaml)),
            ("res://B.YML", Some(Filetype::Yaml)),
            ("res://c.xml", Some(Filetype::Xml)),
            ("res://d.toml", Some(Filetype::Toml)),
            ("res://e.log", None),
            ("res://f.csv", None),
            ("res://Makefile", None),
            ("res://dir.v2/README", None),
            // A tab the user switched to plain text.
            ("res://g.json", Some(Filetype::Json)),
            ("res://h.md", Some(Filetype::Markdown)),
            ("Unsaved file.", None),
            ("res://scene.tscn::Resource_x.txt", None),
        ];
        for (path, want) in rows {
            assert_eq!(detect(&plain_text(Some(path))), want, "{path}");
        }
    }

    /// The highlighter alone no longer proves `text`: a `.yml` tab whose
    /// path could not be read must not get the text plugin, which wraps.
    #[test]
    fn plain_text_without_a_path_is_unknown() {
        assert_eq!(detect(&plain_text(None)), None);
    }

    /// A script tab switched to the plain-text highlighter keeps the
    /// language its delimiters give.
    #[test]
    fn a_script_tab_switched_to_plain_text_keeps_its_delimiters() {
        let mut s = plain_text(Some("res://a.gd"));
        s.comment_delimiters = vec!["##".into(), "#".into()];
        assert_eq!(detect(&s), Some(Filetype::GdScript));
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
