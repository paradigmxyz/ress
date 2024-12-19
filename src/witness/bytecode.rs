//! BytecodeProvider will return contract's bytecode from request of block number and contract address

use alloy_primitives::B256;
use reth::{primitives::Bytecode, providers::ProviderResult};

pub trait BytecodeProvider {
    /// Get account code by its hash
    fn bytecode_by_hash(&self, code_hash: B256) -> ProviderResult<Option<Bytecode>>;
}

pub struct TestBytecodeProvider {}

impl TestBytecodeProvider {
    pub fn new() -> Self {
        Self {}
    }
}

impl BytecodeProvider for TestBytecodeProvider {
    /// Get account code by its hash
    // TODO: getting code bytecode via rlpx network somehow
    fn bytecode_by_hash(&self, code_hash: B256) -> ProviderResult<Option<Bytecode>> {
        todo!()
    }
}
