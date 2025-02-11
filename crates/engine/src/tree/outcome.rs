use alloy_primitives::B256;

/// The outcome of a tree operation.
#[derive(Debug)]
pub struct TreeOutcome<T> {
    /// The outcome of the operation.
    pub outcome: T,
    /// An optional event to tell the caller to do something.
    pub event: Option<TreeEvent>,
}

impl<T> TreeOutcome<T> {
    /// Create new tree outcome.
    pub const fn new(outcome: T) -> Self {
        Self { outcome, event: None }
    }

    /// Set event on the outcome.
    pub fn with_event(mut self, event: TreeEvent) -> Self {
        self.event = Some(event);
        self
    }
}

/// Events that are triggered by Tree Chain
#[derive(Clone, Debug)]
pub enum TreeEvent {
    /// Tree action is needed.
    TreeAction(TreeAction),
    /// Data needs to be downloaded.
    Download(DownloadRequest),
}

impl TreeEvent {
    /// Return witness download target hash if event is [`DownloadRequest::Witness`] of
    /// [`TreeEvent::Download`] variant.
    pub fn as_witness_download(&self) -> Option<B256> {
        if let Self::Download(DownloadRequest::Witness { block_hash }) = self {
            Some(*block_hash)
        } else {
            None
        }
    }
}

/// The actions that can be performed on the tree.
#[derive(Clone, Debug)]
pub enum TreeAction {
    /// Make target canonical.
    MakeCanonical {
        /// The sync target head hash
        sync_target_head: B256,
    },
}

/// The download request.
#[derive(Clone, Debug)]
pub enum DownloadRequest {
    /// Download block.
    Block {
        /// Target block hash.
        block_hash: B256,
    },
    /// Download witness.
    Witness {
        /// Target block hash.
        block_hash: B256,
    },
}
