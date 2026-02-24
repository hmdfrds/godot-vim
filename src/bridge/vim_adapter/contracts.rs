//! Canonical adapter contracts for controller <-> engine communication.

use crate::bridge::types::cursor::CursorPos;
use vim_core::domain::capabilities::viewport::ViewportCapability;
use vim_core::domain::fold::FoldProvider;
use vim_core::domain::search_provider::SearchProvider;
use vim_core::runtime::EngineCapabilities;

/// Input execution policy selected by the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPolicy {
    Exclusive,
    Passive,
}

/// Per-execution context passed from adapter controller to vim-core engine.
#[derive(Clone, Copy)]
pub struct ExecutionContext<'a> {
    pub cursor: CursorPos,
    pub capabilities: EngineCapabilities<'a>,
}

impl<'a> ExecutionContext<'a> {
    #[must_use]
    pub fn with_ports(
        cursor: CursorPos,
        viewport: Option<&'a dyn ViewportCapability>,
        fold_provider: Option<&'a dyn FoldProvider>,
        search_provider: Option<&'a dyn SearchProvider>,
    ) -> Self {
        Self {
            cursor,
            capabilities: EngineCapabilities::with_ports(viewport, fold_provider, search_provider),
        }
    }

    #[must_use]
    pub fn from_snapshot<D>(cursor: CursorPos, snapshot: &'a D) -> Self
    where
        D: ViewportCapability + FoldProvider + SearchProvider,
    {
        Self::with_ports(cursor, Some(snapshot), Some(snapshot), Some(snapshot))
    }
}

/// Canonical engine output batch consumed by controller dispatch.
pub type DispatchBatch = super::output::VimOutput;
