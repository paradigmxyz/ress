use std::{fs::File, io::Read};

use reth::rpc::types::engine::ExecutionPayloadV3;

pub fn read_example_payload(file_path: &str) -> eyre::Result<ExecutionPayloadV3> {
    let mut file = File::open(file_path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let example_payload: ExecutionPayloadV3 = serde_json::from_str(&contents)?;
    Ok(example_payload)
}
