use alloy_primitives::B256;
use itertools::Itertools;
use rayon::prelude::*;
use reth_trie::{HashedPostState, Nibbles};
use reth_trie_sparse::{
    errors::{SparseStateTrieResult, SparseTrieErrorKind},
    SparseStateTrie, SparseTrie,
};
use std::sync::mpsc;

/// Compute the state root given a revealed sparse trie and hashed state update.
pub fn calculate_state_root(
    trie: &mut SparseStateTrie,
    state: HashedPostState,
) -> SparseStateTrieResult<B256> {
    // Update storage slots with new values and calculate storage roots.
    let (storage_tx, storage_rx) = mpsc::channel();

    state
        .storages
        .into_iter()
        .map(|(address, storage)| (address, storage, trie.take_storage_trie(&address)))
        .par_bridge()
        .map(|(address, storage, storage_trie)| {
            let mut storage_trie = storage_trie.unwrap_or_else(SparseTrie::revealed_empty);

            if storage.wiped {
                storage_trie.wipe()?;
            }
            for (hashed_slot, value) in storage
                .storage
                .into_iter()
                .sorted_unstable_by_key(|(hashed_slot, _)| *hashed_slot)
            {
                let nibbles = Nibbles::unpack(hashed_slot);
                if value.is_zero() {
                    storage_trie.remove_leaf(&nibbles)?;
                } else {
                    storage_trie
                        .update_leaf(nibbles, alloy_rlp::encode_fixed_size(&value).to_vec())?;
                }
            }

            storage_trie.root();

            SparseStateTrieResult::Ok((address, storage_trie))
        })
        .for_each_init(
            || storage_tx.clone(),
            |storage_tx, result| storage_tx.send(result).unwrap(),
        );
    drop(storage_tx);
    for result in storage_rx {
        let (address, storage_trie) = result?;
        trie.insert_storage_trie(address, storage_trie);
    }

    // Update accounts with new values
    for (hashed_address, account) in state
        .accounts
        .into_iter()
        .sorted_unstable_by_key(|(hashed_address, _)| *hashed_address)
    {
        trie.update_account(hashed_address, account.unwrap_or_default())?;
    }

    trie.root().ok_or_else(|| SparseTrieErrorKind::Blind.into())
}
