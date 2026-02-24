//! Visual state subsystem — search highlights, substitute preview, messages.

use crate::bridge::vim_adapter::managers::preview::SubstitutePreview;
use crate::bridge::vim_adapter::managers::search::SearchManager;
use crate::bridge::vim_adapter::managers::visual_tracker::VisualTracker;

/// Visual state tracking: search highlights, substitute preview, messages.
pub struct VisualSubsystem {
    /// Visual change tracker for conditional updates
    pub visual_tracker: VisualTracker,
    /// Search highlight sync manager
    pub search_manager: SearchManager,
    /// Live substitute preview manager
    pub substitute_preview: SubstitutePreview,
    /// Message to display after mode change (for :w, :q, etc.)
    pub pending_message: Option<String>,
}

impl VisualSubsystem {
    /// Creates a new VisualSubsystem with default state.
    pub fn new() -> Self {
        Self {
            visual_tracker: VisualTracker::new(),
            search_manager: SearchManager::new(),
            substitute_preview: SubstitutePreview::new(),
            pending_message: None,
        }
    }
}
