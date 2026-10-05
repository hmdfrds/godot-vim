//! The filetype plugins: a static table of `:setlocal` lines per filetype,
//! and the code that applies them to a buffer once and undoes them when the
//! buffer's filetype changes.
//!
//! Each line is transcribed from the Vim 9.1 runtime ftplugin it cites, with
//! two rules on top:
//!
//! - **No `r` or `o` in `formatoptions`.** They continue a comment on Enter
//!   and `o`, which the engine does not do yet; a table test enforces it.
//! - **Only the options the engine formats with:** `formatoptions`,
//!   `comments` and `commentstring`. Indent stays synced from the CodeEdit,
//!   which is Godot's source of truth, and folding, `suffixesadd`,
//!   `formatlistpat` and the like have no engine counterpart.
//!
//! **Deliberate deviation for GDScript.** Upstream `gdscript.vim` keeps the
//! global `formatoptions`, so `t` wraps code as you type, and `python.vim`,
//! its closest relative, does the same. In GDScript a newline ends the
//! statement, so a break outside brackets is a parse error or silently
//! changes the meaning. godot-vim therefore drops `t` and adds `c`, `q` and
//! `l`, as `c.vim` does for C (minus `r` and `o`): code never wraps, and
//! `#` and `##` comments wrap with their leader once `textwidth` is set.
//! `comments` is `b:##,b:#` without Python's `fb:-`, which would make a code
//! line starting with "- " (a continued expression) a comment and wrap it.
//! The shader entry does the same with `formatoptions` for the same reason;
//! upstream `gdshader.vim` keeps the global value too. The C# and cfg
//! entries follow their upstream plugins, which already drop `t`.
//!
//! Applying a line follows `:setlocal`: it writes the buffer's own value and
//! leaves the global one alone. The engine's `:setlocal` executor is not
//! reachable here without running a whole Ex command, which would also put
//! the line in the `:` register, so the few operators the table uses are
//! applied directly to the buffer's overrides. Undoing writes each touched
//! option's global value into the buffer, as Vim's `b:undo_ftplugin` does
//! with `setlocal fo<`.

use vim_core::primitives::{FormatFlags, OptionId, OptionOverrides, OptionValue};
use vim_core::VimOptions;

use super::detect::Filetype;

/// One `:setlocal` line and the upstream file it comes from.
#[derive(Debug)]
pub(crate) struct Line {
    pub(crate) ex: &'static str,
    pub(crate) source: &'static str,
}

/// The lines for one filetype.
#[derive(Debug)]
pub(crate) struct Ftplugin {
    pub(crate) filetype: Filetype,
    pub(crate) lines: &'static [Line],
}

/// The table. A filetype missing here gets no plugin.
pub(crate) const FTPLUGINS: &[Ftplugin] = &[
    Ftplugin {
        filetype: Filetype::GdScript,
        lines: &[
            Line {
                ex: r"setlocal commentstring=#\ %s",
                source: "ftplugin/gdscript.vim:22",
            },
            Line {
                ex: "setlocal comments=b:##,b:#",
                source: "deviation: gdscript.vim sets no comments; python.vim:39 \
                         is b:#,fb:- (b:## added for doc comments, fb:- dropped)",
            },
            Line {
                ex: "setlocal formatoptions-=t formatoptions+=cql",
                source: "deviation: gdscript.vim and python.vim keep t; \
                         c.vim:23 is fo-=t fo+=croql (r and o left out)",
            },
        ],
    },
    Ftplugin {
        filetype: Filetype::GdShader,
        lines: &[
            Line {
                ex: r"setlocal comments=sO:*\ -,mO:*\ \ ,exO:*/,s1:/*,mb:*,ex:*/,://",
                source: "ftplugin/gdshader.vim:15",
            },
            Line {
                ex: r"setlocal commentstring=//\ %s",
                source: "ftplugin/gdshader.vim:16",
            },
            Line {
                ex: "setlocal formatoptions-=t formatoptions+=cql",
                source: "deviation: gdshader.vim keeps fo; \
                         c.vim:23 is fo-=t fo+=croql (r and o left out)",
            },
        ],
    },
    Ftplugin {
        filetype: Filetype::Cs,
        lines: &[
            Line {
                ex: "setlocal formatoptions-=t formatoptions+=cql",
                source: "deviation: cs.vim:19 is fo-=t fo+=croql (r and o left out)",
            },
            Line {
                ex: r"setlocal comments=sO:*\ -,mO:*\ \ ,exO:*/,s1:/*,mb:*,ex:*/,:///,://",
                source: "ftplugin/cs.vim:22",
            },
            Line {
                ex: r"setlocal commentstring=//\ %s",
                source: "ftplugin/cs.vim:23",
            },
        ],
    },
    Ftplugin {
        filetype: Filetype::Json,
        lines: &[
            Line {
                ex: "setlocal formatoptions-=t",
                source: "ftplugin/json.vim:13",
            },
            Line {
                ex: "setlocal comments=",
                source: "ftplugin/json.vim:16",
            },
            Line {
                ex: "setlocal commentstring=",
                source: "ftplugin/json.vim:17",
            },
        ],
    },
    Ftplugin {
        filetype: Filetype::Markdown,
        lines: &[
            Line {
                ex: r"setlocal comments=fb:*,fb:-,fb:+,n:> commentstring=<!--\ %s\ -->",
                source: "ftplugin/markdown.vim:16",
            },
            Line {
                // `n` needs formatlistpat (markdown.vim:18), which the engine
                // does not have, so it has no effect yet.
                ex: "setlocal formatoptions+=tcqln formatoptions-=r formatoptions-=o",
                source: "ftplugin/markdown.vim:17",
            },
        ],
    },
    Ftplugin {
        filetype: Filetype::Text,
        lines: &[
            Line {
                ex: "setlocal comments=fb:-,fb:*,n:>",
                source: "ftplugin/text.vim:17",
            },
            Line {
                ex: "setlocal commentstring=",
                source: "ftplugin/text.vim:18",
            },
        ],
    },
    Ftplugin {
        filetype: Filetype::Cfg,
        lines: &[Line {
            ex: r"setlocal commentstring=#\ %s formatoptions-=t formatoptions+=cql",
            source: "deviation: cfg.vim:16 is cms=#\\ %s fo-=t fo+=croql (r and o left out)",
        }],
    },
];

/// The plugin for `filetype`, if the table has one.
pub(crate) fn ftplugin_for(filetype: Filetype) -> Option<&'static Ftplugin> {
    FTPLUGINS.iter().find(|p| p.filetype == filetype)
}

// ── Parsing ──────────────────────────────────────────────────────────────

/// A `:set` operator the table may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    /// `=`
    Assign,
    /// `+=`, on a flag list: append the flags, keeping the last of a repeat.
    AddFlags,
    /// `-=`, on a flag list: remove the flags as written.
    RemoveFlags,
}

/// One `name{op}value` item of a `:setlocal` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Assignment {
    pub(crate) id: OptionId,
    pub(crate) op: Op,
    pub(crate) value: String,
}

/// Parse a table line. Accepts `setlocal` or `setl`, then items separated
/// by unescaped blanks. A backslash keeps the next blank or backslash, as
/// in Vim (`commentstring=#\ %s`). Only the options in the module docs are
/// accepted, and `+=`/`-=` only on `formatoptions`.
pub(crate) fn parse_line(line: &str) -> Result<Vec<Assignment>, String> {
    let rest = line
        .strip_prefix("setlocal ")
        .or_else(|| line.strip_prefix("setl "))
        .ok_or_else(|| format!("not a :setlocal line: {line}"))?;
    split_items(rest)
        .into_iter()
        .map(|item| parse_item(&item))
        .collect()
}

fn split_items(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next @ (' ' | '\t' | '\\')) => current.push(next),
                Some(next) => {
                    current.push('\\');
                    current.push(next);
                }
                None => current.push('\\'),
            },
            ' ' | '\t' => {
                if !current.is_empty() {
                    items.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        items.push(current);
    }
    items
}

fn parse_item(item: &str) -> Result<Assignment, String> {
    let name_end = item
        .find(|c: char| !c.is_ascii_lowercase())
        .ok_or_else(|| format!("no value in '{item}'"))?;
    let (name, rest) = item.split_at(name_end);
    let (op, value) = if let Some(v) = rest.strip_prefix("+=") {
        (Op::AddFlags, v)
    } else if let Some(v) = rest.strip_prefix("-=") {
        (Op::RemoveFlags, v)
    } else if let Some(v) = rest.strip_prefix('=') {
        (Op::Assign, v)
    } else {
        return Err(format!("unsupported operator in '{item}'"));
    };
    let id = match name {
        "formatoptions" | "fo" => OptionId::FormatOptions,
        "comments" | "com" => OptionId::Comments,
        "commentstring" | "cms" => OptionId::CommentString,
        _ => return Err(format!("option not allowed in a filetype plugin: '{name}'")),
    };
    if op != Op::Assign && id != OptionId::FormatOptions {
        return Err(format!(
            "'{item}': += and -= are only used on formatoptions"
        ));
    }
    Ok(Assignment {
        id,
        op,
        value: value.to_owned(),
    })
}

// ── Applying and undoing ─────────────────────────────────────────────────

fn current_value(id: OptionId, global: &VimOptions, overrides: &OptionOverrides) -> String {
    let value = overrides
        .get(id)
        .cloned()
        .unwrap_or_else(|| global.get_option(id));
    match value {
        OptionValue::Str(s) => s.to_string(),
        other => format!("{other:?}"),
    }
}

/// Apply one assignment to the buffer's overrides, the way `:setlocal`
/// would: the starting value is the buffer's own, or the global one when
/// the buffer has none.
fn apply_assignment(a: &Assignment, global: &VimOptions, overrides: &mut OptionOverrides) {
    let new = match a.op {
        Op::Assign => a.value.clone(),
        // As vim-core's `:set` does it, after Vim: `+=` appends and then
        // keeps the last of any repeated flag, `-=` removes the operand
        // where it appears exactly as written.
        Op::AddFlags => {
            let current = current_value(a.id, global, overrides);
            FormatFlags::normalize(&format!("{current}{}", a.value))
        }
        Op::RemoveFlags => current_value(a.id, global, overrides).replacen(&a.value, "", 1),
    };
    overrides.set(a.id, OptionValue::Str(new.into()));
}

/// What to set up in one buffer. A change in any field makes the buffer's
/// setup run again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Setup {
    /// The detected filetype, `None` when unknown or detection is off.
    pub(crate) filetype: Option<Filetype>,
    /// Whether filetype plugins run (`filetype plugin on` and detection on).
    pub(crate) plugin: bool,
    /// `commentstring` built from Godot's line comment delimiter, applied
    /// before the plugin lines so a plugin's own value wins. Godot is the
    /// source of truth for the language's comment syntax, so this runs even
    /// with plugins off and keeps `gc` working in every script.
    pub(crate) commentstring: Option<String>,
}

/// What was set up in a buffer, kept with the buffer's state so the setup
/// runs once and can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Applied {
    pub(crate) setup: Setup,
    /// The options the setup wrote, in order, without repeats: the
    /// `b:undo_ftplugin` of this buffer.
    pub(crate) touched: Vec<OptionId>,
}

/// Bring a buffer's overrides to `setup`.
///
/// Returns `None` when `prev` already applied this setup: the setup runs
/// once per buffer, so a `:setlocal` the user made since is kept across
/// buffer switches. Otherwise the previous setup is undone first and the new
/// record is returned. A plugin line that does not parse is skipped and
/// logged; the table tests keep that from happening.
pub(crate) fn update(
    prev: Option<&Applied>,
    setup: Setup,
    global: &VimOptions,
    overrides: &mut OptionOverrides,
) -> Option<Applied> {
    if prev.is_some_and(|p| p.setup == setup) {
        return None;
    }
    if let Some(prev) = prev {
        undo(&prev.touched, global, overrides);
    }

    let mut touched = Vec::new();
    let mut touch = |id: OptionId| {
        if !touched.contains(&id) {
            touched.push(id);
        }
    };
    if let Some(cs) = &setup.commentstring {
        overrides.set(
            OptionId::CommentString,
            OptionValue::Str(cs.as_str().into()),
        );
        touch(OptionId::CommentString);
    }
    if setup.plugin {
        if let Some(plugin) = setup.filetype.and_then(ftplugin_for) {
            for line in plugin.lines {
                match parse_line(line.ex) {
                    Ok(assignments) => {
                        for a in &assignments {
                            apply_assignment(a, global, overrides);
                            touch(a.id);
                        }
                    }
                    Err(e) => log::warn!("ftplugin: skipped '{}' ({}): {e}", line.ex, line.source),
                }
            }
        }
    }
    Some(Applied { setup, touched })
}

/// Vim's `setlocal {option}<` for each touched option: the buffer gets a
/// copy of the global value.
pub(crate) fn undo(touched: &[OptionId], global: &VimOptions, overrides: &mut OptionOverrides) {
    for &id in touched {
        overrides.set(id, global.get_option(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vim_core::primitives::CommentSpec;

    fn local(overrides: &OptionOverrides, id: OptionId) -> Option<String> {
        match overrides.get(id) {
            Some(OptionValue::Str(s)) => Some(s.to_string()),
            Some(other) => panic!("{id:?} is not a string: {other:?}"),
            None => None,
        }
    }

    fn setup(ft: Option<Filetype>, plugin: bool, cs: Option<&str>) -> Setup {
        Setup {
            filetype: ft,
            plugin,
            commentstring: cs.map(str::to_owned),
        }
    }

    /// Run a filetype's plugin on fresh overrides over Vim-default globals.
    fn run(ft: Filetype) -> OptionOverrides {
        let mut ov = OptionOverrides::new();
        let applied = update(
            None,
            setup(Some(ft), true, None),
            &VimOptions::default(),
            &mut ov,
        );
        assert!(applied.is_some());
        ov
    }

    // ── Table validity ───────────────────────────────────────────────────

    #[test]
    fn every_line_parses_and_cites_a_source() {
        for plugin in FTPLUGINS {
            for line in plugin.lines {
                let parsed = parse_line(line.ex);
                assert!(parsed.is_ok(), "{:?}: {:?}", plugin.filetype, parsed);
                assert!(
                    line.source.starts_with("ftplugin/") || line.source.starts_with("deviation: "),
                    "{}: no source",
                    line.ex
                );
            }
        }
    }

    #[test]
    fn the_table_uses_only_the_allowed_options() {
        let allowed = [
            OptionId::FormatOptions,
            OptionId::Comments,
            OptionId::CommentString,
        ];
        for plugin in FTPLUGINS {
            for line in plugin.lines {
                for a in parse_line(line.ex).unwrap_or_default() {
                    assert!(allowed.contains(&a.id), "{}: {:?}", line.ex, a.id);
                }
            }
        }
    }

    /// `r` and `o` continue comments on Enter and `o`, which no plugin may
    /// turn on.
    #[test]
    fn no_plugin_adds_r_or_o() {
        for plugin in FTPLUGINS {
            for line in plugin.lines {
                for a in parse_line(line.ex).unwrap_or_default() {
                    if a.id == OptionId::FormatOptions && a.op != Op::RemoveFlags {
                        assert!(
                            !a.value.contains(['r', 'o']),
                            "{:?}: {}",
                            plugin.filetype,
                            line.ex
                        );
                    }
                }
            }
        }
        for plugin in FTPLUGINS {
            let fo = local(&run(plugin.filetype), OptionId::FormatOptions);
            if let Some(fo) = fo {
                assert!(!fo.contains(['r', 'o']), "{:?}: fo={fo}", plugin.filetype);
            }
        }
    }

    /// The engine accepts every value a plugin produces.
    #[test]
    fn every_result_is_a_valid_engine_value() {
        for plugin in FTPLUGINS {
            let ov = run(plugin.filetype);
            if let Some(fo) = local(&ov, OptionId::FormatOptions) {
                assert!(
                    FormatFlags::parse(&fo).is_ok(),
                    "{:?}: fo={fo}",
                    plugin.filetype
                );
            }
            if let Some(com) = local(&ov, OptionId::Comments) {
                assert!(
                    CommentSpec::parse(&com).is_ok(),
                    "{:?}: comments={com}",
                    plugin.filetype
                );
            }
            if let Some(cms) = local(&ov, OptionId::CommentString) {
                assert!(
                    cms.is_empty() || cms.contains("%s"),
                    "{:?}: commentstring={cms}",
                    plugin.filetype
                );
            }
        }
    }

    /// The table is applied without the engine's executor; this checks the
    /// result is what the engine's own `:setlocal` makes of the same lines.
    #[test]
    fn the_result_matches_the_engines_setlocal() {
        use vim_core::execution::{InputContext, VimEngine};
        for plugin in FTPLUGINS {
            let mut engine = VimEngine::new();
            for line in plugin.lines {
                let doc = crate::bridge::document::GodotDocument::new("");
                let ctx = InputContext::new(&doc, 0).validate_clamped();
                let _ = engine.execute_ex(line.ex, ctx);
            }
            let ov = run(plugin.filetype);
            for id in [
                OptionId::FormatOptions,
                OptionId::Comments,
                OptionId::CommentString,
            ] {
                let ours = ov
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| VimOptions::default().get_option(id));
                assert_eq!(
                    ours,
                    engine.effective_option(id),
                    "{:?} {id:?}",
                    plugin.filetype
                );
            }
            assert_eq!(
                engine.options().formatoptions(),
                VimOptions::default().formatoptions(),
                "{:?}: :setlocal left the global value",
                plugin.filetype
            );
        }
    }

    // ── Per-filetype results ─────────────────────────────────────────────

    #[test]
    fn gdscript_wraps_comments_and_never_code() {
        let ov = run(Filetype::GdScript);
        // Engine default fo is tcqj: -=t leaves cqj, +=cql gives jcql.
        assert_eq!(local(&ov, OptionId::FormatOptions).as_deref(), Some("jcql"));
        assert_eq!(local(&ov, OptionId::Comments).as_deref(), Some("b:##,b:#"));
        assert_eq!(local(&ov, OptionId::CommentString).as_deref(), Some("# %s"));
    }

    /// fb:- would make a code line starting with "- " a comment and wrap it.
    #[test]
    fn gdscript_comments_have_no_dash_leader() {
        let ov = run(Filetype::GdScript);
        let com = local(&ov, OptionId::Comments).unwrap_or_default();
        assert!(!com.split(',').any(|part| part.ends_with(":-")), "{com}");
    }

    #[test]
    fn gdshader_matches_upstream_comments() {
        let ov = run(Filetype::GdShader);
        assert_eq!(local(&ov, OptionId::FormatOptions).as_deref(), Some("jcql"));
        assert_eq!(
            local(&ov, OptionId::Comments).as_deref(),
            Some("sO:* -,mO:*  ,exO:*/,s1:/*,mb:*,ex:*/,://")
        );
        assert_eq!(
            local(&ov, OptionId::CommentString).as_deref(),
            Some("// %s")
        );
    }

    #[test]
    fn json_never_wraps() {
        let ov = run(Filetype::Json);
        let fo = local(&ov, OptionId::FormatOptions).unwrap_or_default();
        assert!(!fo.contains('t'), "fo={fo}");
        assert_eq!(local(&ov, OptionId::Comments).as_deref(), Some(""));
        assert_eq!(local(&ov, OptionId::CommentString).as_deref(), Some(""));
    }

    #[test]
    fn markdown_and_text_keep_t() {
        let md = run(Filetype::Markdown);
        assert_eq!(
            local(&md, OptionId::FormatOptions).as_deref(),
            Some("jtcqln")
        );
        assert_eq!(
            local(&md, OptionId::CommentString).as_deref(),
            Some("<!-- %s -->")
        );
        let txt = run(Filetype::Text);
        assert_eq!(
            local(&txt, OptionId::FormatOptions),
            None,
            "text.vim keeps fo"
        );
        assert_eq!(
            local(&txt, OptionId::Comments).as_deref(),
            Some("fb:-,fb:*,n:>")
        );
    }

    #[test]
    fn cs_wraps_comments_and_never_code() {
        let ov = run(Filetype::Cs);
        assert_eq!(local(&ov, OptionId::FormatOptions).as_deref(), Some("jcql"));
        assert_eq!(
            local(&ov, OptionId::Comments).as_deref(),
            Some("sO:* -,mO:*  ,exO:*/,s1:/*,mb:*,ex:*/,:///,://")
        );
        assert_eq!(
            local(&ov, OptionId::CommentString).as_deref(),
            Some("// %s")
        );
    }

    #[test]
    fn cfg_never_wraps_values() {
        let ov = run(Filetype::Cfg);
        assert_eq!(local(&ov, OptionId::FormatOptions).as_deref(), Some("jcql"));
        assert_eq!(local(&ov, OptionId::CommentString).as_deref(), Some("# %s"));
        assert_eq!(
            local(&ov, OptionId::Comments),
            None,
            "cfg.vim keeps comments"
        );
    }

    #[test]
    fn every_filetype_has_a_plugin() {
        for ft in [
            Filetype::GdScript,
            Filetype::GdShader,
            Filetype::Cs,
            Filetype::Json,
            Filetype::Markdown,
            Filetype::Text,
            Filetype::Cfg,
        ] {
            assert!(ftplugin_for(ft).is_some(), "{ft:?}");
        }
    }

    #[test]
    fn plus_equals_starts_from_the_global_value() {
        let mut global = VimOptions::default();
        global.set_formatoptions("qt");
        let mut ov = OptionOverrides::new();
        update(
            None,
            setup(Some(Filetype::GdScript), true, None),
            &global,
            &mut ov,
        );
        assert_eq!(local(&ov, OptionId::FormatOptions).as_deref(), Some("cql"));
        assert_eq!(
            global.formatoptions(),
            "qt",
            ":setlocal leaves the global value"
        );
    }

    // ── Once per buffer, undo, plugin switch ─────────────────────────────

    #[test]
    fn the_same_setup_runs_once() {
        let global = VimOptions::default();
        let mut ov = OptionOverrides::new();
        let s = setup(Some(Filetype::GdScript), true, Some("# %s"));
        let first = update(None, s.clone(), &global, &mut ov);
        // The user runs :setlocal fo+=t afterwards.
        ov.set(OptionId::FormatOptions, OptionValue::Str("cqjlt".into()));
        assert_eq!(update(first.as_ref(), s, &global, &mut ov), None);
        assert_eq!(
            local(&ov, OptionId::FormatOptions).as_deref(),
            Some("cqjlt")
        );
    }

    #[test]
    fn the_undo_set_is_every_option_written_once() {
        let mut ov = OptionOverrides::new();
        let applied = update(
            None,
            setup(Some(Filetype::GdScript), true, Some("# %s")),
            &VimOptions::default(),
            &mut ov,
        );
        assert_eq!(
            applied.map(|a| a.touched),
            Some(vec![
                OptionId::CommentString,
                OptionId::Comments,
                OptionId::FormatOptions
            ])
        );
    }

    /// A filetype change undoes the old plugin with `setlocal opt<`, which
    /// copies the global value into the buffer, then runs the new one.
    #[test]
    fn a_filetype_change_restores_the_global_value_first() {
        let mut global = VimOptions::default();
        global.set_comments("://");
        let mut ov = OptionOverrides::new();
        let gd = update(
            None,
            setup(Some(Filetype::GdScript), true, None),
            &global,
            &mut ov,
        );
        let txt = update(
            gd.as_ref(),
            setup(Some(Filetype::Text), true, None),
            &global,
            &mut ov,
        );
        assert!(txt.is_some());
        // text.vim keeps fo, so fo is back to the global value.
        assert_eq!(local(&ov, OptionId::FormatOptions).as_deref(), Some("tcqj"));
        assert_eq!(local(&ov, OptionId::CommentString).as_deref(), Some(""));
        assert_eq!(
            local(&ov, OptionId::Comments).as_deref(),
            Some("fb:-,fb:*,n:>")
        );

        let unknown = update(txt.as_ref(), setup(None, true, None), &global, &mut ov);
        assert!(unknown.is_some_and(|a| a.touched.is_empty()));
        assert_eq!(local(&ov, OptionId::Comments).as_deref(), Some("://"));
        assert_eq!(
            local(&ov, OptionId::CommentString).as_deref(),
            Some("// %s")
        );
    }

    #[test]
    fn plugins_off_keeps_only_the_commentstring_from_godot() {
        let mut ov = OptionOverrides::new();
        let applied = update(
            None,
            setup(Some(Filetype::GdScript), false, Some("# %s")),
            &VimOptions::default(),
            &mut ov,
        );
        assert_eq!(
            applied.map(|a| a.touched),
            Some(vec![OptionId::CommentString])
        );
        assert_eq!(local(&ov, OptionId::CommentString).as_deref(), Some("# %s"));
        assert_eq!(local(&ov, OptionId::FormatOptions), None);
    }

    #[test]
    fn turning_plugins_off_undoes_them_at_the_next_setup() {
        let global = VimOptions::default();
        let mut ov = OptionOverrides::new();
        let on = update(
            None,
            setup(Some(Filetype::GdScript), true, Some("# %s")),
            &global,
            &mut ov,
        );
        let off = update(
            on.as_ref(),
            setup(Some(Filetype::GdScript), false, Some("# %s")),
            &global,
            &mut ov,
        );
        assert!(off.is_some());
        assert_eq!(local(&ov, OptionId::FormatOptions).as_deref(), Some("tcqj"));
        assert_eq!(local(&ov, OptionId::CommentString).as_deref(), Some("# %s"));
    }

    #[test]
    fn a_plugin_commentstring_wins_over_godots() {
        let mut ov = OptionOverrides::new();
        update(
            None,
            setup(Some(Filetype::Json), true, Some("# %s")),
            &VimOptions::default(),
            &mut ov,
        );
        assert_eq!(local(&ov, OptionId::CommentString).as_deref(), Some(""));
    }

    // ── Parser ───────────────────────────────────────────────────────────

    #[test]
    fn parser_unescapes_blanks_and_backslashes() {
        let a = parse_line(r"setl cms=#\ %s com=a\\b").unwrap_or_default();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].value, "# %s");
        assert_eq!(a[1].value, r"a\b");
    }

    #[test]
    fn parser_rejects_other_options_and_operators() {
        assert!(parse_line("setlocal shell=sh").is_err());
        assert!(parse_line("setlocal tw=80").is_err());
        assert!(parse_line("setlocal comments+=b:#").is_err());
        assert!(parse_line("setlocal fo^=t").is_err());
        assert!(parse_line("set fo-=t").is_err());
    }
}
