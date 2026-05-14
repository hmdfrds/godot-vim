//! Integration tests for the bridge effect dispatch layer.
//!
//! Each module tests one category of effects against `MockTextEdit`, verifying
//! that the byte-offset-to-line/col translation and Godot API calls produce
//! correct results. The `dispatch` module exercises the full pipeline
//! (effect list -> dispatch -> mock) which is the closest we can get to
//! end-to-end testing without the Godot runtime.
//!
//! ```text
//! bridge_tests/
//! ├── macros.rs         — assert_editor!, effects!, DispatchCtx, apply_* helpers
//! ├── text_mutations.rs — insert, delete, replace effect handlers
//! ├── cursor.rs         — set_cursor, set_selection (char/line/block)
//! ├── undo.rs           — undo/redo groups, caret restoration
//! ├── scroll.rs         — center/top/bottom, horizontal scroll
//! └── dispatch.rs       — full dispatch round-trip tests
//! ```

#[macro_use]
mod macros;

mod cursor;
mod dispatch;
mod multi_cursor_sync;
mod scroll;
mod text_mutations;
mod undo;
