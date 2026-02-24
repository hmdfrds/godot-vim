//! Motion handler functions extracted from `VimController`.
//!
//! These free functions handle motion dispatch, pure motion application,
//! and scroll/viewport motions. They operate on `VimController` via &mut self
//! or on editor directly.

mod fold_vertical;
mod pure_apply;
mod screen_line;
mod scroll;
mod search;

pub use fold_vertical::execute_vertical_motion_fold_aware_public;
pub use pure_apply::apply_pure_motion;
pub use screen_line::{execute_screen_line_motion, execute_window_motion};
pub use scroll::execute_scroll_motion;
pub use search::execute_search_motion;
