use std::collections::HashMap;

use alloy_primitives::{Address, BlockNumber, B256, U256};
use ress_subprotocol::protocol::proto::StateWitness;
use reth::revm::{
    primitives::{AccountInfo, Bytecode},
    Database,
};

use crate::bytecode_provider::{BytecodeProviderError, BytecodeProviderTrait};

/// ress will construct this `WitnessStateProvider` after getting `StateWitness` thru subprotocol communication.
pub struct WitnessStateProvider<B>
where
    B: BytecodeProviderTrait,
    B::Error: Into<WitnessStateProviderError>,
{
    state_witness: StateWitness,
    block_hashes: HashMap<BlockNumber, B256>,
    bytecode_provider: B,
}

impl<B> WitnessStateProvider<B>
where
    B: BytecodeProviderTrait,
    B::Error: Into<WitnessStateProviderError>,
{
    /// Create a new witness state provider.
    pub fn new(
        state_witness: StateWitness,
        block_hashes: HashMap<BlockNumber, B256>,
        bytecode_provider: B,
    ) -> Self {
        Self {
            state_witness,
            block_hashes,
            bytecode_provider,
        }
    }
}

/// Database error type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WitnessStateProviderError {
    /// Block hash not found.
    #[error("block hash not found")]
    BlockHashNotFound,

    /// Error from bytecode provider.
    #[error(transparent)]
    BytecodeProviderError(#[from] BytecodeProviderError),
}

impl<B> Database for WitnessStateProvider<B>
where
    B: BytecodeProviderTrait,
    B::Error: Into<WitnessStateProviderError>,
{
    #[doc = " The witness state provider error type."]
    type Error = WitnessStateProviderError;

    #[doc = " Get basic account information."]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        // TODO: do some magic with `state_witness`
        todo!()
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(self
            .bytecode_provider
            .code_by_hash(code_hash)
            .await
            .map_err(Into::into)?)
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        // TODO/QUESTION: how i get this
        todo!()
    }

    #[doc = " Get block hash by block number."]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Ok(*self
            .block_hashes
            .get(&number)
            .ok_or(WitnessStateProviderError::BlockHashNotFound)?)
    }
}
