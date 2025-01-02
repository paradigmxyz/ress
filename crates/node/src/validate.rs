use reth::{
    consensus::ConsensusError,
    consensus_common::validation::{
        validate_against_parent_4844, validate_against_parent_eip1559_base_fee,
        validate_against_parent_hash_number, validate_against_parent_timestamp,
    },
    primitives::SealedHeader,
};

use crate::engine::ConsensusEngine;

pub fn validate_pre_execution_block() {}

impl ConsensusEngine {}
