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
#[derive(Debug)]
pub enum TreeEvent {
    /// Tree action is needed.
    TreeAction(TreeAction),
    /// Data needs to be downloaded.
    Download(DownloadRequest),
    // TODO:
    //   /// Backfill action is needed.
    //    BackfillAction(BackfillAction),
    //    /// Block download is needed.
    //   Download(DownloadRequest),
}

impl TreeEvent {
    /// Returns true if event is witness download request.
    pub fn is_witness_download_request(&self) -> bool {
        matches!(self, Self::Download(DownloadRequest::Witness { .. }))
    }
}

/// The actions that can be performed on the tree.
#[derive(Debug)]
pub enum TreeAction {
    /// Make target canonical.
    MakeCanonical {
        /// The sync target head hash
        sync_target_head: B256,
    },
}

/// The download request.
#[derive(Debug)]
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
