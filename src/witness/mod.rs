use alloy_primitives::{
    map::{B256HashMap, HashMap},
    Address, BlockNumber, Bytes, StorageKey, StorageValue, B256,
};
use bytecode::BytecodeProvider;
use rayon::iter::IntoParallelRefIterator;
use reth::{
    primitives::{Account, Bytecode},
    providers::{
        AccountReader, BlockHashReader, HashedPostStateProvider, ProviderError, ProviderResult,
        StateProofProvider, StateProvider, StateRootProvider, StorageRootProvider,
    },
    revm::db::BundleState,
};
use reth_trie::{
    updates::TrieUpdates, witness::TrieWitness, AccountProof, HashedPostState, HashedStorage,
    KeccakKeyHasher, MultiProof, MultiProofTargets, StorageMultiProof, StorageProof, TrieInput,
};

pub mod bytecode;

/// hashmap representation of multiple mpt state proof
/// `keccak(rlp(node)) -> rlp(nod)`
type StateWitness = HashMap<B256, Bytes>;

pub struct WitnessStateProvider<B: BytecodeProvider + Send + Sync> {
    pub(crate) state_witness: StateWitness,
    pub(crate) block_hashes: HashMap<BlockNumber, B256>,
    pub(crate) bytecode_provider: B,
}

impl<B> WitnessStateProvider<B>
where
    B: BytecodeProvider + Send + Sync,
{
    pub fn new(bytecode_provider: B) -> Self {
        Self {
            state_witness: HashMap::default(),
            block_hashes: HashMap::default(),
            bytecode_provider,
        }
    }

    // TODO: we need to update block_hashes, state witness from given block number no?
    // TODO: so somehow when calling this function, rlpx network send req(give me witness of blockhash) -> res
    pub fn update(mut self, block_number: BlockNumber) {
        // 1. get state witness of target block number
        self.state_witness = HashMap::default();

        // get hashed post state from
        let hps = todo!();

        // TrieWitness::from_tx(provider.tx_ref())
        //     .compute(HashedPostState {
        //         accounts: HashMap::from_iter([(hashed_address, Some(Account::default()))]),
        //         storages: HashMap::default(),
        //     })
        //     .unwrap();

        // 2. get block hashes from target block ~ until historical 256
        // Calculate the start block for historical block hashes
        // We need 256 previous block hashes, but don't go below block 0
        let start_block = block_number.saturating_sub(256);
    }
}

impl<B> StateProvider for WitnessStateProvider<B>
where
    B: BytecodeProvider + Send + Sync,
{
    /// Get storage of given account.
    fn storage(
        &self,
        account: Address,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        todo!()
    }

    /// Get account code by its hash
    fn bytecode_by_hash(&self, code_hash: B256) -> ProviderResult<Option<Bytecode>> {
        self.bytecode_provider.bytecode_by_hash(code_hash)
    }
}

impl<B> AccountReader for WitnessStateProvider<B>
where
    B: BytecodeProvider + Send + Sync,
{
    fn basic_account(&self, address: Address) -> ProviderResult<Option<Account>> {
        // TODO - key is keccak(address)? seems it retrieve rlp(Node) and decode to turun in Account, but where to get the key
        Ok(None)
    }
}

impl<B> BlockHashReader for WitnessStateProvider<B>
where
    B: BytecodeProvider + Send + Sync,
{
    /// Get the hash of the block with the given number. Returns `None` if no block with this number
    /// exists.
    fn block_hash(&self, number: BlockNumber) -> ProviderResult<Option<B256>> {
        Ok(self.block_hashes.get(&number).copied())
    }

    /// Get headers in range of block hashes or numbers
    ///
    /// Returns the available hashes of that range.
    ///
    /// Note: The range is `start..end`, so the expected result is `[start..end)`
    fn canonical_hashes_range(
        &self,
        start: BlockNumber,
        end: BlockNumber,
    ) -> ProviderResult<Vec<B256>> {
        // Check if all blocks in range are available
        if (start..end).all(|num| self.block_hashes.contains_key(&num)) {
            Ok((start..end).map(|num| self.block_hashes[&num]).collect())
        } else {
            // TODO: incorrect error type - but do we need to support more than 256 historical block hash?
            Err(ProviderError::UnsupportedProvider)
        }
    }
}

impl<B> HashedPostStateProvider for WitnessStateProvider<B>
where
    B: BytecodeProvider + Send + Sync,
{
    /// Returns the `HashedPostState` of the provided [`BundleState`].
    fn hashed_post_state(&self, bundle_state: &BundleState) -> HashedPostState {
        // TODO: can't we have `state_par_iter()` mothod on `BundleState`?
        HashedPostState::from_bundle_state::<KeccakKeyHasher>(bundle_state.state.par_iter())
    }
}

impl<B> StateProofProvider for WitnessStateProvider<B>
where
    B: BytecodeProvider + Send + Sync,
{
    /// Get account and storage proofs of target keys in the `HashedPostState`
    /// on top of the current state.
    fn proof(
        &self,
        input: TrieInput,
        address: Address,
        slots: &[B256],
    ) -> ProviderResult<AccountProof> {
        todo!()
    }

    /// Generate [`MultiProof`] for target hashed account and corresponding
    /// hashed storage slot keys.
    fn multiproof(
        &self,
        input: TrieInput,
        targets: MultiProofTargets,
    ) -> ProviderResult<MultiProof> {
        todo!()
    }

    /// Get trie witness for provided state.
    fn witness(
        &self,
        input: TrieInput,
        target: HashedPostState,
    ) -> ProviderResult<B256HashMap<Bytes>> {
        todo!()
    }
}

impl<B> StateRootProvider for WitnessStateProvider<B>
where
    B: BytecodeProvider + Send + Sync,
{
    /// Returns the state root of the `BundleState` on top of the current state.
    ///
    /// # Note
    ///
    /// It is recommended to provide a different implementation from
    /// `state_root_with_updates` since it affects the memory usage during state root
    /// computation.
    fn state_root(&self, hashed_state: HashedPostState) -> ProviderResult<B256> {
        todo!()
    }

    /// Returns the state root of the `HashedPostState` on top of the current state but re-uses the
    /// intermediate nodes to speed up the computation. It's up to the caller to construct the
    /// prefix sets and inform the provider of the trie paths that have changes.
    fn state_root_from_nodes(&self, input: TrieInput) -> ProviderResult<B256> {
        todo!()
    }

    /// Returns the state root of the `HashedPostState` on top of the current state with trie
    /// updates to be committed to the database.
    fn state_root_with_updates(
        &self,
        hashed_state: HashedPostState,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        todo!()
    }

    /// Returns state root and trie updates.
    /// See [`StateRootProvider::state_root_from_nodes`] for more info.
    fn state_root_from_nodes_with_updates(
        &self,
        input: TrieInput,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        todo!()
    }
}

impl<B> StorageRootProvider for WitnessStateProvider<B>
where
    B: BytecodeProvider + Send + Sync,
{
    /// Returns the storage root of the `HashedStorage` for target address on top of the current
    /// state.
    fn storage_root(
        &self,
        address: Address,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<B256> {
        todo!()
    }

    /// Returns the storage proof of the `HashedStorage` for target slot on top of the current
    /// state.
    fn storage_proof(
        &self,
        address: Address,
        slot: B256,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageProof> {
        todo!()
    }

    /// Returns the storage multiproof for target slots.
    fn storage_multiproof(
        &self,
        address: Address,
        slots: &[B256],
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageMultiProof> {
        todo!()
    }
}
