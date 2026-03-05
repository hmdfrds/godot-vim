//! Attach/detach state machine for editor session lifecycle.

use godot::obj::InstanceId;

/// Editor attachment lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachState {
    Detached,
    Attaching(InstanceId),
    Attached(InstanceId),
    Detaching(InstanceId),
}

/// Invalid lifecycle transition error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachTransitionError {
    InvalidTransition {
        from: AttachState,
        attempted: AttachState,
    },
}

impl std::fmt::Display for AttachTransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTransition { from, attempted } => {
                write!(
                    f,
                    "invalid attach transition: from {:?} to {:?}",
                    from, attempted
                )
            }
        }
    }
}

impl std::error::Error for AttachTransitionError {}

/// Mutable lifecycle state holder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachSession {
    state: AttachState,
}

impl Default for AttachSession {
    fn default() -> Self {
        Self::new()
    }
}

impl AttachSession {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: AttachState::Detached,
        }
    }

    #[must_use]
    pub const fn state(self) -> AttachState {
        self.state
    }

    #[must_use]
    pub fn attached_editor_id(self) -> Option<InstanceId> {
        match self.state {
            AttachState::Attached(id) | AttachState::Attaching(id) | AttachState::Detaching(id) => {
                Some(id)
            }
            AttachState::Detached => None,
        }
    }

    #[must_use]
    pub fn is_attached_to(self, editor_id: InstanceId) -> bool {
        matches!(self.state, AttachState::Attached(id) if id == editor_id)
    }

    pub fn begin_attach(&mut self, editor_id: InstanceId) -> Result<(), AttachTransitionError> {
        let next = AttachState::Attaching(editor_id);
        match self.state {
            AttachState::Detached | AttachState::Detaching(_) => {
                self.state = next;
                Ok(())
            }
            AttachState::Attaching(id) | AttachState::Attached(id) if id == editor_id => {
                self.state = next;
                Ok(())
            }
            _ => Err(AttachTransitionError::InvalidTransition {
                from: self.state,
                attempted: next,
            }),
        }
    }

    pub fn mark_attached(&mut self, editor_id: InstanceId) -> Result<(), AttachTransitionError> {
        let next = AttachState::Attached(editor_id);
        match self.state {
            AttachState::Attaching(id) | AttachState::Attached(id) if id == editor_id => {
                self.state = next;
                Ok(())
            }
            _ => Err(AttachTransitionError::InvalidTransition {
                from: self.state,
                attempted: next,
            }),
        }
    }

    pub fn begin_detach(&mut self, editor_id: InstanceId) -> Result<(), AttachTransitionError> {
        let next = AttachState::Detaching(editor_id);
        match self.state {
            AttachState::Detached => {
                self.state = AttachState::Detached;
                Ok(())
            }
            AttachState::Attaching(id) | AttachState::Attached(id) | AttachState::Detaching(id)
                if id == editor_id =>
            {
                self.state = next;
                Ok(())
            }
            _ => Err(AttachTransitionError::InvalidTransition {
                from: self.state,
                attempted: next,
            }),
        }
    }

    pub fn mark_detached(&mut self) {
        self.state = AttachState::Detached;
    }
}
