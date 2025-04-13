use crate::tree::root::calculate_state_root;
use alloy_consensus::BlockHeader;
use alloy_primitives::{keccak256, map::B256Map};
use alloy_rlp::Decodable;
use rayon::iter::IntoParallelRefIterator;
use ress_evm::BlockExecutor;
use reth_chainspec::ChainSpec;
use reth_consensus::ConsensusError;
use reth_engine_tree::tree::error::InsertBlockErrorKind;
use reth_errors::ProviderError;
use reth_node_ethereum::consensus::validate_block_post_execution;
use reth_primitives::{Block, Bytecode, GotExpected, Header, Receipt, RecoveredBlock};
use reth_primitives_traits::SealedHeader;
use reth_provider::BlockExecutionOutput;
use reth_ress_protocol::RLPExecutionWitness;
use reth_trie::{HashedPostState, KeccakKeyHasher};
use reth_trie_sparse::{blinded::DefaultBlindedProviderFactory, SparseStateTrie};
use std::{collections::HashMap, sync::Arc};
use tracing::*;

pub(crate) fn stateless_block_validation(
    chain_spec: Arc<ChainSpec>,
    parent: SealedHeader,
    block: RecoveredBlock<Block>,
    execution_witness: RLPExecutionWitness,
) -> Result<BlockExecutionOutput<Receipt>, InsertBlockErrorKind> {
    let block_num_hash = block.num_hash();

    // TODO: Most of the code below can be replaced with a stateless_verify_method
    // ===================== Witness =====================
    let mut trie = SparseStateTrie::new(DefaultBlindedProviderFactory);
    let mut state_witness = B256Map::default();
    for encoded in &execution_witness.state {
        state_witness.insert(keccak256(encoded), encoded.clone());
    }
    trie.reveal_witness(parent.state_root, &state_witness).map_err(|error| {
        InsertBlockErrorKind::Provider(ProviderError::TrieWitnessError(error.to_string()))
    })?;

    let mut codes = B256Map::default();
    for code in execution_witness.codes {
        let bytecode = Bytecode::new_raw_checked(code)
            .map_err(|err| InsertBlockErrorKind::Other(Box::new(err)))?;
        let code_hash = keccak256(bytecode.0.bytes());
        codes.insert(code_hash, bytecode.0);
    }

    // TODO: Code for checking the headers has been copied from reth PR 15591
    let mut ancestor_hashes = HashMap::new();

    let mut child_header = block.clone_header();

    // Next verify that headers supplied are contiguous
    // TODO: similar to the reth PR, this currently assumes the headers are in increasing order
    // TODO: We can use a HashMap to allow any ordering for the header.
    for parent_header in execution_witness.headers.iter().rev() {
        let bytes = parent_header.as_ref();
        let parent_header = Header::decode(&mut &bytes[..]).unwrap();

        let parent_hash = child_header.parent_hash();
        ancestor_hashes.insert(parent_header.number, parent_hash);

        if (parent_hash != parent_header.hash_slow()) ||
            (parent_header.number + 1 != child_header.number)
        {
            // TODO: use this custom error until we add a variant kind
            let err = InsertBlockErrorKind::Other(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "headers are not contiguous",
            )));
            return Err(err)
        }

        child_header = parent_header
    }

    // ===================== Execution =====================
    let start_time = std::time::Instant::now();
    let block_executor = BlockExecutor::new(chain_spec.clone(), &trie, codes, ancestor_hashes);
    let output = block_executor.execute(&block).map_err(InsertBlockErrorKind::Execution)?;
    debug!(target: "ress::engine", block = ?block_num_hash, elapsed = ?start_time.elapsed(), "Executed new payload");

    // ===================== Post Execution Validation =====================
    validate_block_post_execution(&block, &chain_spec, &output.receipts, &output.requests)?;

    // ===================== State Root =====================
    let hashed_state =
        HashedPostState::from_bundle_state::<KeccakKeyHasher>(output.state.state.par_iter());
    let state_root = calculate_state_root(&mut trie, hashed_state).map_err(|error| {
        InsertBlockErrorKind::Provider(ProviderError::TrieWitnessError(error.to_string()))
    })?;
    if state_root != block.state_root {
        return Err(ConsensusError::BodyStateRootDiff(
            GotExpected { got: state_root, expected: block.state_root }.into(),
        )
        .into());
    }

    return Ok(output)
}
