use std::sync::Arc;

use crate::errors::StoreError;
use alloy_primitives::B256;
use ress_network::p2p::P2pHandler;
use reth::revm::primitives::Bytecode;

// handle complex logic regarding storage
pub struct StoreEngine {
    pub p2p_handler: Arc<P2pHandler>,
}

impl StoreEngine {
    pub fn new(p2p_handler: &Arc<P2pHandler>) -> Self {
        Self {
            p2p_handler: p2p_handler.clone(),
        }
    }

    pub fn get_account_code(&self, _code_hash: B256) -> Result<Option<Bytecode>, StoreError> {
        // todo get bytecode from libmbdx -> fall back network
        todo!()
    }
}
