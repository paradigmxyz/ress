use std::{fs::File, io::Read};

use ress_primitives::witness_rpc::ExecutionWitnessFromRpc;

pub fn read_example_witness(file_path: &str) -> eyre::Result<ExecutionWitnessFromRpc> {
    let mut file = File::open(file_path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let example_header: ExecutionWitnessFromRpc = serde_json::from_str(&contents)?;
    Ok(example_header)
}
