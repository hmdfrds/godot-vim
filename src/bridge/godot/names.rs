// API Catalogue: intentionally defines all known Godot API names for type-safe access.
// Not all constants are used yet — this is the canonical reference for Godot interactions.
#![allow(dead_code)]
//! Godot API names organized by component.
//!
//! This module centralizes all string literals used for Godot API interactions
//! (signals, method calls, property names), organized by the Godot class they belong to.
//!
//! # Organization
//! - Each Godot class has its own submodule
//! - Each submodule contains `signals`, `methods`, `properties` as needed
//! - Custom callbacks (Rust methods exposed to Godot) are in `callbacks`
//!
//! # Rules
//! 1. No magic strings outside this module — all Godot API strings are defined here.
//! 2. Verify names against Godot source/documentation.
//! 3. Group by component for discoverability.

// ═══════════════════════════════════════════════════════════════════════════════
// TEXT EDIT (base class for CodeEdit)
// ═══════════════════════════════════════════════════════════════════════════════

/// TextEdit - Base text editing component
pub mod text_edit {
    pub mod signals {
        pub const TEXT_SET: &str = "text_set";
        pub const TEXT_CHANGED: &str = "text_changed";
        pub const LINES_EDITED_FROM: &str = "lines_edited_from";
        pub const CARET_CHANGED: &str = "caret_changed";
        pub const GUTTER_CLICKED: &str = "gutter_clicked";
        pub const GUTTER_ADDED: &str = "gutter_added";
        pub const GUTTER_REMOVED: &str = "gutter_removed";
    }

    pub mod methods {
        // Caret
        pub const SET_CARET_LINE: &str = "set_caret_line";
        pub const SET_CARET_COLUMN: &str = "set_caret_column";
        pub const GET_CARET_LINE: &str = "get_caret_line";
        pub const GET_CARET_COLUMN: &str = "get_caret_column";

        // Text
        pub const GET_TEXT: &str = "get_text";
        pub const SET_TEXT: &str = "set_text";
        pub const GET_LINE: &str = "get_line";
        pub const SET_LINE: &str = "set_line";
        pub const GET_LINE_COUNT: &str = "get_line_count";

        // Selection
        pub const SELECT: &str = "select";
        pub const SELECT_ALL: &str = "select_all";
        pub const DESELECT: &str = "deselect";
        pub const HAS_SELECTION: &str = "has_selection";
        pub const GET_SELECTED_TEXT: &str = "get_selected_text";
        pub const GET_SELECTION_FROM_LINE: &str = "get_selection_from_line";
        pub const GET_SELECTION_TO_LINE: &str = "get_selection_to_line";
        pub const GET_SELECTION_FROM_COLUMN: &str = "get_selection_from_column";
        pub const GET_SELECTION_TO_COLUMN: &str = "get_selection_to_column";

        // Editing
        pub const INSERT_TEXT_AT_CARET: &str = "insert_text_at_caret";
        pub const DELETE_SELECTION: &str = "delete_selection";
        pub const CUT: &str = "cut";
        pub const COPY: &str = "copy";
        pub const PASTE: &str = "paste";
        pub const UNDO: &str = "undo";
        pub const REDO: &str = "redo";
        pub const BEGIN_COMPLEX_OPERATION: &str = "begin_complex_operation";
        pub const END_COMPLEX_OPERATION: &str = "end_complex_operation";

        // Scroll
        pub const SET_V_SCROLL: &str = "set_v_scroll";
        pub const GET_V_SCROLL: &str = "get_v_scroll";
        pub const SET_H_SCROLL: &str = "set_h_scroll";
        pub const GET_H_SCROLL: &str = "get_h_scroll";
        pub const CENTER_VIEWPORT_TO_CARET: &str = "center_viewport_to_caret";

        // Gutter
        pub const ADD_GUTTER: &str = "add_gutter";
        pub const REMOVE_GUTTER: &str = "remove_gutter";
        pub const GET_GUTTER_COUNT: &str = "get_gutter_count";
        pub const SET_GUTTER_NAME: &str = "set_gutter_name";
        pub const GET_GUTTER_NAME: &str = "get_gutter_name";
        pub const SET_GUTTER_WIDTH: &str = "set_gutter_width";
        pub const SET_GUTTER_DRAW: &str = "set_gutter_draw";
        pub const SET_GUTTER_CLICKABLE: &str = "set_gutter_clickable";
        pub const SET_GUTTER_CUSTOM_DRAW: &str = "set_gutter_custom_draw";
        pub const SET_LINE_GUTTER_TEXT: &str = "set_line_gutter_text";
        pub const SET_LINE_GUTTER_ICON: &str = "set_line_gutter_icon";
        pub const SET_LINE_GUTTER_CLICKABLE: &str = "set_line_gutter_clickable";

        // Search
        pub const SEARCH: &str = "search";
        pub const SET_SEARCH_TEXT: &str = "set_search_text";
        pub const SET_SEARCH_FLAGS: &str = "set_search_flags";
    }

    pub mod properties {
        pub const CARET_BLINK: &str = "caret_blink";
        pub const CARET_TYPE: &str = "caret_type";
        pub const EDITABLE: &str = "editable";
        pub const WRAP_MODE: &str = "wrap_mode";
        pub const SCROLL_SMOOTH: &str = "scroll_smooth";
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CODE EDIT (extends TextEdit)
// ═══════════════════════════════════════════════════════════════════════════════

/// CodeEdit - Code editing with syntax highlighting, folding, etc.
pub mod code_edit {
    pub mod signals {
        pub const BREAKPOINT_TOGGLED: &str = "breakpoint_toggled";
        pub const CODE_COMPLETION_REQUESTED: &str = "code_completion_requested";
        pub const SYMBOL_LOOKUP: &str = "symbol_lookup";
        pub const SYMBOL_VALIDATE: &str = "symbol_validate";
        pub const SYMBOL_HOVERED: &str = "symbol_hovered";
    }

    pub mod methods {
        // Indentation
        pub const SET_INDENT_SIZE: &str = "set_indent_size";
        pub const GET_INDENT_SIZE: &str = "get_indent_size";
        pub const SET_INDENT_USING_SPACES: &str = "set_indent_using_spaces";
        pub const DO_INDENT: &str = "do_indent";
        pub const INDENT_LINES: &str = "indent_lines";
        pub const UNINDENT_LINES: &str = "unindent_lines";
        pub const CONVERT_INDENT: &str = "convert_indent";

        // Folding
        pub const SET_LINE_FOLDING_ENABLED: &str = "set_line_folding_enabled";
        pub const IS_LINE_FOLDING_ENABLED: &str = "is_line_folding_enabled";
        pub const CAN_FOLD_LINE: &str = "can_fold_line";
        pub const FOLD_LINE: &str = "fold_line";
        pub const UNFOLD_LINE: &str = "unfold_line";
        pub const FOLD_ALL_LINES: &str = "fold_all_lines";
        pub const UNFOLD_ALL_LINES: &str = "unfold_all_lines";
        pub const TOGGLE_FOLDABLE_LINE: &str = "toggle_foldable_line";
        pub const TOGGLE_FOLDABLE_LINES_AT_CARETS: &str = "toggle_foldable_lines_at_carets";
        pub const IS_LINE_FOLDED: &str = "is_line_folded";
        pub const GET_FOLDED_LINES: &str = "get_folded_lines";

        // Gutters
        pub const SET_DRAW_BREAKPOINTS_GUTTER: &str = "set_draw_breakpoints_gutter";
        pub const SET_DRAW_BOOKMARKS_GUTTER: &str = "set_draw_bookmarks_gutter";
        pub const SET_DRAW_EXECUTING_LINES_GUTTER: &str = "set_draw_executing_lines_gutter";
        pub const SET_DRAW_LINE_NUMBERS: &str = "set_draw_line_numbers";
        pub const SET_DRAW_FOLD_GUTTER: &str = "set_draw_fold_gutter";

        // Breakpoints
        pub const SET_LINE_AS_BREAKPOINT: &str = "set_line_as_breakpoint";
        pub const IS_LINE_BREAKPOINTED: &str = "is_line_breakpointed";
        pub const CLEAR_BREAKPOINTED_LINES: &str = "clear_breakpointed_lines";
        pub const GET_BREAKPOINTED_LINES: &str = "get_breakpointed_lines";

        // Bookmarks
        pub const SET_LINE_AS_BOOKMARKED: &str = "set_line_as_bookmarked";
        pub const IS_LINE_BOOKMARKED: &str = "is_line_bookmarked";
        pub const CLEAR_BOOKMARKED_LINES: &str = "clear_bookmarked_lines";
        pub const GET_BOOKMARKED_LINES: &str = "get_bookmarked_lines";

        // Executing lines
        pub const SET_LINE_AS_EXECUTING: &str = "set_line_as_executing";
        pub const IS_LINE_EXECUTING: &str = "is_line_executing";
        pub const CLEAR_EXECUTING_LINES: &str = "clear_executing_lines";
        pub const GET_EXECUTING_LINES: &str = "get_executing_lines";

        // Code completion
        pub const REQUEST_CODE_COMPLETION: &str = "request_code_completion";
        pub const ADD_CODE_COMPLETION_OPTION: &str = "add_code_completion_option";
        pub const UPDATE_CODE_COMPLETION_OPTIONS: &str = "update_code_completion_options";
        pub const GET_CODE_COMPLETION_OPTIONS: &str = "get_code_completion_options";
        pub const GET_CODE_COMPLETION_OPTION: &str = "get_code_completion_option";
        pub const GET_CODE_COMPLETION_SELECTED_INDEX: &str = "get_code_completion_selected_index";
        pub const SET_CODE_COMPLETION_SELECTED_INDEX: &str = "set_code_completion_selected_index";
        pub const CONFIRM_CODE_COMPLETION: &str = "confirm_code_completion";
        pub const CANCEL_CODE_COMPLETION: &str = "cancel_code_completion";

        // Code regions
        pub const CREATE_CODE_REGION: &str = "create_code_region";
        pub const GET_CODE_REGION_START_TAG: &str = "get_code_region_start_tag";
        pub const GET_CODE_REGION_END_TAG: &str = "get_code_region_end_tag";
        pub const SET_CODE_REGION_TAGS: &str = "set_code_region_tags";
        pub const IS_LINE_CODE_REGION_START: &str = "is_line_code_region_start";
        pub const IS_LINE_CODE_REGION_END: &str = "is_line_code_region_end";

        // String/comment detection
        pub const IS_IN_STRING: &str = "is_in_string";
        pub const IS_IN_COMMENT: &str = "is_in_comment";
        pub const ADD_STRING_DELIMITER: &str = "add_string_delimiter";
        pub const ADD_COMMENT_DELIMITER: &str = "add_comment_delimiter";

        // Auto brace
        pub const SET_AUTO_BRACE_COMPLETION_ENABLED: &str = "set_auto_brace_completion_enabled";
        pub const ADD_AUTO_BRACE_COMPLETION_PAIR: &str = "add_auto_brace_completion_pair";
    }

    pub mod properties {
        pub const INDENT_SIZE: &str = "indent_size";
        pub const INDENT_AUTOMATIC: &str = "indent_automatic";
        pub const INDENT_USE_SPACES: &str = "indent_use_spaces";
        pub const GUTTERS_DRAW_BREAKPOINTS_GUTTER: &str = "gutters_draw_breakpoints_gutter";
        pub const GUTTERS_DRAW_BOOKMARKS: &str = "gutters_draw_bookmarks";
        pub const GUTTERS_DRAW_EXECUTING_LINES: &str = "gutters_draw_executing_lines";
        pub const GUTTERS_DRAW_LINE_NUMBERS: &str = "gutters_draw_line_numbers";
        pub const GUTTERS_DRAW_FOLD_GUTTER: &str = "gutters_draw_fold_gutter";
    }

    /// Custom gutter names created by this plugin
    pub mod gutters {
        pub const CONNECTION_GUTTER: &str = "connection_gutter";
        pub const RELATIVE_NUMBERS: &str = "vim_relative_numbers";
        pub const CUSTOM_FOLD: &str = "vim_custom_fold";
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// SCRIPT EDITOR
// ═══════════════════════════════════════════════════════════════════════════════

/// ScriptEditor - The script editing panel
pub mod script_editor {
    pub mod signals {
        pub const EDITOR_SCRIPT_CHANGED: &str = "editor_script_changed";
        pub const SCRIPT_CLOSE: &str = "script_close";
    }

    pub mod methods {
        pub const GET_OPEN_SCRIPT_EDITORS: &str = "get_open_script_editors";
        pub const GET_BREAKPOINTS: &str = "get_breakpoints";
        pub const GOTO_LINE: &str = "goto_line";
        pub const GET_CURRENT_SCRIPT: &str = "get_current_script";
        pub const GET_OPEN_SCRIPTS: &str = "get_open_scripts";
        pub const OPEN_SCRIPT_CREATE_DIALOG: &str = "open_script_create_dialog";
        pub const GOTO_HELP: &str = "goto_help";
        pub const UPDATE_DOCS_FROM_SCRIPT: &str = "update_docs_from_script";
        pub const CLEAR_DOCS_FROM_SCRIPT: &str = "clear_docs_from_script";
        pub const REGISTER_SYNTAX_HIGHLIGHTER: &str = "register_syntax_highlighter";
        pub const UNREGISTER_SYNTAX_HIGHLIGHTER: &str = "unregister_syntax_highlighter";
    }
}

/// ScriptEditorBase - Base class for script editor tabs
pub mod script {
    pub mod methods {
        pub const RELOAD: &str = "reload";
    }
}

pub mod script_editor_base {
    pub mod signals {
        pub const NAME_CHANGED: &str = "name_changed";
        pub const EDITED_SCRIPT_CHANGED: &str = "edited_script_changed";
        pub const REQUEST_HELP: &str = "request_help";
        pub const REQUEST_OPEN_SCRIPT_AT_LINE: &str = "request_open_script_at_line";
        pub const REQUEST_SAVE_HISTORY: &str = "request_save_history";
        pub const GO_TO_HELP: &str = "go_to_help";
        pub const SEARCH_IN_FILES_REQUESTED: &str = "search_in_files_requested";
        pub const REPLACE_IN_FILES_REQUESTED: &str = "replace_in_files_requested";
        pub const GO_TO_METHOD: &str = "go_to_method";
        pub const GOTO_LINE: &str = "goto_line";
    }

    pub mod methods {
        pub const GET_BASE_EDITOR: &str = "get_base_editor";
        pub const ADD_SYNTAX_HIGHLIGHTER: &str = "add_syntax_highlighter";
        pub const IS_UNSAVED: &str = "is_unsaved";
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// EDITOR INTERFACE
// ═══════════════════════════════════════════════════════════════════════════════

/// EditorInterface - Main editor access point
pub mod editor_interface {
    pub mod methods {
        // Core access
        pub const GET_SCRIPT_EDITOR: &str = "get_script_editor";
        pub const GET_BASE_CONTROL: &str = "get_base_control";
        pub const GET_EDITOR_MAIN_SCREEN: &str = "get_editor_main_screen";
        pub const GET_EDITOR_THEME: &str = "get_editor_theme";
        pub const GET_EDITOR_SETTINGS: &str = "get_editor_settings";
        pub const GET_EDITOR_UNDO_REDO: &str = "get_editor_undo_redo";
        pub const GET_COMMAND_PALETTE: &str = "get_command_palette";
        pub const GET_FILE_SYSTEM_DOCK: &str = "get_file_system_dock";
        pub const GET_INSPECTOR: &str = "get_inspector";
        pub const GET_RESOURCE_FILESYSTEM: &str = "get_resource_filesystem";
        pub const GET_RESOURCE_PREVIEWER: &str = "get_resource_previewer";
        pub const GET_SELECTION: &str = "get_selection";

        // Scene management
        pub const GET_OPEN_SCENES: &str = "get_open_scenes";
        pub const GET_OPEN_SCENE_ROOTS: &str = "get_open_scene_roots";
        pub const GET_EDITED_SCENE_ROOT: &str = "get_edited_scene_root";
        pub const OPEN_SCENE_FROM_PATH: &str = "open_scene_from_path";
        pub const RELOAD_SCENE_FROM_PATH: &str = "reload_scene_from_path";
        pub const SAVE_SCENE: &str = "save_scene";
        pub const SAVE_SCENE_AS: &str = "save_scene_as";
        pub const SAVE_ALL_SCENES: &str = "save_all_scenes";
        pub const ADD_ROOT_NODE: &str = "add_root_node";

        // Editing
        pub const EDIT_RESOURCE: &str = "edit_resource";
        pub const EDIT_NODE: &str = "edit_node";
        pub const EDIT_SCRIPT: &str = "edit_script";
        pub const INSPECT_OBJECT: &str = "inspect_object";
        pub const SET_OBJECT_EDITED: &str = "set_object_edited";
        pub const IS_OBJECT_EDITED: &str = "is_object_edited";

        // UI
        pub const SET_MAIN_SCREEN_EDITOR: &str = "set_main_screen_editor";
        pub const SET_DISTRACTION_FREE_MODE: &str = "set_distraction_free_mode";
        pub const IS_DISTRACTION_FREE_MODE_ENABLED: &str = "is_distraction_free_mode_enabled";
        pub const GET_EDITOR_SCALE: &str = "get_editor_scale";
        pub const POPUP_DIALOG: &str = "popup_dialog";
        pub const POPUP_DIALOG_CENTERED: &str = "popup_dialog_centered";
        pub const POPUP_QUICK_OPEN: &str = "popup_quick_open";

        // Files
        pub const SELECT_FILE: &str = "select_file";
        pub const GET_SELECTED_PATHS: &str = "get_selected_paths";
        pub const GET_CURRENT_PATH: &str = "get_current_path";
        pub const GET_CURRENT_DIRECTORY: &str = "get_current_directory";

        // Plugins
        pub const SET_PLUGIN_ENABLED: &str = "set_plugin_enabled";
        pub const IS_PLUGIN_ENABLED: &str = "is_plugin_enabled";

        // Misc
        pub const RESTART_EDITOR: &str = "restart_editor";
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// DEBUGGER
// ═══════════════════════════════════════════════════════════════════════════════

/// EditorDebugger - Debugging interface
pub mod debugger {
    pub const CLASS_NAME: &str = "EditorDebuggerNode";

    pub mod methods {
        pub const GET_DEFAULT_DEBUGGER: &str = "get_default_debugger";
        pub const DEBUG_NEXT: &str = "debug_next";
        pub const DEBUG_STEP: &str = "debug_step";
        pub const DEBUG_CONTINUE: &str = "debug_continue";
        pub const DEBUG_BREAK: &str = "debug_break";
        pub const IS_SESSION_ACTIVE: &str = "is_session_active";
        pub const IS_BREAKED: &str = "is_breaked";
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CONTROL (base UI class)
// ═══════════════════════════════════════════════════════════════════════════════

/// Control - Base class for all UI nodes
pub mod control {
    pub mod signals {
        pub const GUI_INPUT: &str = "gui_input";
        pub const FOCUS_ENTERED: &str = "focus_entered";
        pub const FOCUS_EXITED: &str = "focus_exited";
        pub const THEME_CHANGED: &str = "theme_changed";
        pub const VISIBILITY_CHANGED: &str = "visibility_changed";
        pub const MINIMUM_SIZE_CHANGED: &str = "minimum_size_changed";
    }

    pub mod methods {
        pub const GRAB_FOCUS: &str = "grab_focus";
        pub const HAS_FOCUS: &str = "has_focus";
        pub const RELEASE_FOCUS: &str = "release_focus";
        pub const SET_FOCUS_MODE: &str = "set_focus_mode";
        pub const GET_RECT: &str = "get_rect";
        pub const GET_GLOBAL_RECT: &str = "get_global_rect";
        pub const GET_MINIMUM_SIZE: &str = "get_minimum_size";
        pub const SET_CUSTOM_MINIMUM_SIZE: &str = "set_custom_minimum_size";
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// CANVAS ITEM (base rendering class)
// ═══════════════════════════════════════════════════════════════════════════════

/// CanvasItem - Base class for all 2D nodes (parent of Control)
pub mod canvas_item {
    pub mod signals {
        pub const DRAW: &str = "draw";
        pub const VISIBILITY_CHANGED: &str = "visibility_changed";
        pub const HIDDEN: &str = "hidden";
        pub const ITEM_RECT_CHANGED: &str = "item_rect_changed";
    }

    pub mod methods {
        pub const QUEUE_REDRAW: &str = "queue_redraw";
        pub const SHOW: &str = "show";
        pub const HIDE: &str = "hide";
        pub const IS_VISIBLE: &str = "is_visible";
        pub const IS_VISIBLE_IN_TREE: &str = "is_visible_in_tree";
        pub const SET_MATERIAL: &str = "set_material";
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// VIEWPORT
// ═══════════════════════════════════════════════════════════════════════════════

/// Viewport - Base class for rendering targets
pub mod viewport {
    pub mod signals {
        pub const GUI_FOCUS_CHANGED: &str = "gui_focus_changed";
        pub const SIZE_CHANGED: &str = "size_changed";
    }

    pub mod methods {
        pub const GUI_GET_FOCUS_OWNER: &str = "gui_get_focus_owner";
        pub const SET_INPUT_AS_HANDLED: &str = "set_input_as_handled";
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// COMMON UI COMPONENTS
// ═══════════════════════════════════════════════════════════════════════════════

/// LineEdit - Single line text input
pub mod line_edit {
    pub mod signals {
        pub const TEXT_SUBMITTED: &str = "text_submitted";
        pub const TEXT_CHANGED: &str = "text_changed";
        pub const TEXT_CHANGE_REJECTED: &str = "text_change_rejected";
    }

    pub mod methods {
        pub const GET_TEXT: &str = "get_text";
        pub const SET_TEXT: &str = "set_text";
        pub const CLEAR: &str = "clear";
        pub const SELECT_ALL: &str = "select_all";
    }
}

/// ItemList - List of selectable items
pub mod item_list {
    pub mod signals {
        pub const ITEM_SELECTED: &str = "item_selected";
        pub const ITEM_ACTIVATED: &str = "item_activated";
        pub const MULTI_SELECTED: &str = "multi_selected";
    }

    pub mod methods {
        pub const ADD_ITEM: &str = "add_item";
        pub const REMOVE_ITEM: &str = "remove_item";
        pub const CLEAR: &str = "clear";
        pub const GET_ITEM_COUNT: &str = "get_item_count";
        pub const SELECT: &str = "select";
        pub const DESELECT_ALL: &str = "deselect_all";
        pub const GET_SELECTED_ITEMS: &str = "get_selected_items";
        pub const SCROLL_TO_ITEM: &str = "scroll_to_item";
    }
}

/// Tree - Hierarchical tree widget
pub mod tree {
    pub mod signals {
        pub const ITEM_SELECTED: &str = "item_selected";
        pub const ITEM_ACTIVATED: &str = "item_activated";
        pub const ITEM_EDITED: &str = "item_edited";
        pub const CELL_SELECTED: &str = "cell_selected";
    }

    pub mod methods {
        pub const CREATE_ITEM: &str = "create_item";
        pub const GET_ROOT: &str = "get_root";
        pub const GET_SELECTED: &str = "get_selected";
        pub const CLEAR: &str = "clear";
        pub const SCROLL_TO_ITEM: &str = "scroll_to_item";
    }
}

/// Timer
pub mod timer {
    pub mod signals {
        pub const TIMEOUT: &str = "timeout";
    }

    pub mod methods {
        pub const START: &str = "start";
        pub const STOP: &str = "stop";
        pub const SET_WAIT_TIME: &str = "set_wait_time";
        pub const SET_ONE_SHOT: &str = "set_one_shot";
    }
}

/// Button
pub mod button {
    pub mod signals {
        pub const PRESSED: &str = "pressed";
        pub const TOGGLED: &str = "toggled";
    }
}

/// Range (base for ScrollBar, Slider, etc.)
pub mod range {
    pub mod signals {
        pub const VALUE_CHANGED: &str = "value_changed";
        pub const CHANGED: &str = "changed";
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// THEME
// ═══════════════════════════════════════════════════════════════════════════════

/// Theme resource names
pub mod theme {
    pub const FONT: &str = "font";
    pub const FONT_SIZE: &str = "font_size";
    pub const LINE_NUMBER_COLOR: &str = "line_number_color";
    pub const CAN_FOLD_ICON: &str = "can_fold";
    pub const FOLDED_ICON: &str = "folded";
    pub const CARET_COLOR: &str = "caret_color";
    pub const BACKGROUND_COLOR: &str = "background_color";
    pub const CURRENT_LINE_COLOR: &str = "current_line_color";
    pub const SELECTION_COLOR: &str = "selection_color";
}

// ═══════════════════════════════════════════════════════════════════════════════
// CUSTOM CALLBACKS (Rust methods exposed to Godot)
// ═══════════════════════════════════════════════════════════════════════════════

/// Custom method names exposed to Godot via #[func]
pub mod callbacks {
    // Command bar
    pub const ON_CMD_SUBMITTED: &str = "on_cmd_submitted";
    pub const ON_CMD_TEXT_CHANGED: &str = "on_cmd_text_changed";
    pub const ON_CMD_INPUT_GUI_INPUT: &str = "on_cmd_input_gui_input";

    // Editor input
    pub const HANDLE_GUI_INPUT: &str = "handle_gui_input";
    pub const ON_CARET_MOVED: &str = "on_caret_moved";
    pub const ON_CURSOR_VISUAL_UPDATE: &str = "on_cursor_visual_update";
    pub const ON_SCROLLBAR_CHANGED: &str = "on_scrollbar_changed";
    pub const ON_MAPPING_TIMEOUT: &str = "on_mapping_timeout";
    pub const ON_GLOBAL_MAPPING_TIMEOUT: &str = "on_global_mapping_timeout";
    pub const ON_DOCK_GUI_INPUT: &str = "on_dock_gui_input";
    pub const ON_SCRIPT_CHANGED: &str = "on_script_changed";
    pub const ON_FOCUS_CHANGED: &str = "on_focus_changed";
    pub const PERFORM_ATTACH: &str = "perform_attach";
    pub const RELOAD_SETTINGS: &str = "reload_settings";
    pub const OBSERVE_DOCK_CONTROL: &str = "observe_dock_control";
    pub const EXECUTE_COMMAND_DEFERRED: &str = "execute_command_deferred";

    // Gutter drawing
    pub const DRAW_RELATIVE_NUMBERS: &str = "draw_relative_numbers";
    pub const DRAW_CUSTOM_FOLD: &str = "draw_custom_fold";

    // Line numbers component
    pub const UPDATE_GUTTERS: &str = "update_gutters";
    pub const UPDATE_GUTTERS_DEFERRED: &str = "update_gutters_deferred";
    pub const ON_SCROLL_CHANGED: &str = "on_scroll_changed";
    pub const ON_GUTTER_CLICKED: &str = "on_gutter_clicked";
    pub const ON_THEME_CHANGED: &str = "on_theme_changed";

    // Cmdline component
    pub const ON_PANEL_GUI_INPUT: &str = "on_panel_gui_input";

    // Mapping panel
    pub const ON_ADD_PRESSED: &str = "on_add_pressed";
    pub const ON_TREE_ITEM_EDITED: &str = "on_tree_item_edited";
    pub const ON_DELETE_PRESSED: &str = "on_delete_pressed";
    pub const ON_RELOAD_SETTINGS_PRESSED: &str = "on_reload_settings_pressed";
    pub const ON_TIMEOUT_CHANGED: &str = "on_timeout_changed";

    // Dock search
    pub const ON_DOCK_SEARCH_INPUT: &str = "on_dock_search_input";
}

// ═══════════════════════════════════════════════════════════════════════════════
// MISC
// ═══════════════════════════════════════════════════════════════════════════════

/// Object base class methods
pub mod object {
    pub mod methods {
        pub const CONNECT: &str = "connect";
        pub const DISCONNECT: &str = "disconnect";
        pub const EMIT_SIGNAL: &str = "emit_signal";
        pub const CALL: &str = "call";
        pub const CALL_DEFERRED: &str = "call_deferred";
        pub const SET: &str = "set";
        pub const GET: &str = "get";
        pub const SET_META: &str = "set_meta";
        pub const GET_META: &str = "get_meta";
    }
}

/// Node class
pub mod node {
    pub mod signals {
        pub const READY: &str = "ready";
        pub const TREE_ENTERED: &str = "tree_entered";
        pub const TREE_EXITING: &str = "tree_exiting";
    }

    pub mod methods {
        pub const ADD_CHILD: &str = "add_child";
        pub const REMOVE_CHILD: &str = "remove_child";
        pub const GET_PARENT: &str = "get_parent";
        pub const GET_CHILDREN: &str = "get_children";
        pub const GET_NODE: &str = "get_node";
        pub const QUEUE_FREE: &str = "queue_free";
        pub const FIND_CHILD: &str = "find_child";
    }
}

/// Regex
pub mod regex {
    pub mod methods {
        pub const SUB: &str = "sub";
        pub const SEARCH: &str = "search";
        pub const SEARCH_ALL: &str = "search_all";
        pub const COMPILE: &str = "compile";
    }
}

/// EditorSettings
pub mod editor_settings {
    pub mod signals {
        pub const SETTINGS_CHANGED: &str = "settings_changed";
    }

    pub mod methods {
        pub const GET_SETTING: &str = "get_setting";
        pub const SET_SETTING: &str = "set_setting";
        pub const HAS_SETTING: &str = "has_setting";
    }
}
