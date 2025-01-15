use alloy_primitives::BlockHash;

pub fn get_witness_path(block_hash: BlockHash) -> String {
    format!("./fixtures/witness-{}.json", block_hash)
}
