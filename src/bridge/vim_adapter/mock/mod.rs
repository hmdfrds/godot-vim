//! Mock module for testing shell handlers without Godot runtime.
//!
//! # Architecture
//!
//! This module provides test doubles for Godot types, enabling unit tests
//! for shell handlers that would otherwise require the full Godot runtime.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     MOCK LAYER                                   │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  MockEditor        - In-memory CodeEdit simulation              │
//! │  MockClipboard     - DisplayServer clipboard simulation         │
//! │  TestHarness       - Complete test environment builder          │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::bridge::mock::{MockEditor, TestHarness};
//!
//! #[test]
//! fn test_motion_j() {
//!     let mut harness = TestHarness::new()
//!         .with_content("line 1\nline 2\nline 3")
//!         .with_cursor(0, 0);
//!     
//!     harness.execute_motion(Motion::Down);
//!     
//!     assert_eq!(harness.cursor(), (1, 0));
//! }
//! ```

mod clipboard;
mod editor;
mod harness;

pub use clipboard::MockClipboard;
pub use editor::MockEditor;
pub use harness::TestHarness;
