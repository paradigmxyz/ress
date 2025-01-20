use alloy_primitives::BlockHash;
use ress_primitives::witness_rpc::ExecutionWitnessFromRpc;
use std::{fs::File, io::Read};

/// read witness data from the file via given block hash
pub fn read_example_witness(block_hash: BlockHash) -> eyre::Result<ExecutionWitnessFromRpc> {
    let file_path = get_witness_path(block_hash);
    let mut file = File::open(file_path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let example_header: ExecutionWitnessFromRpc = serde_json::from_str(&contents)?;
    Ok(example_header)
}

/// Get file path to save witness data
pub fn get_witness_path(block_hash: BlockHash) -> String {
    format!("./fixtures/witness-{}.json", block_hash)
}
