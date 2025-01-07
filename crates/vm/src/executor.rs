use std::sync::Arc;

use alloy_eips::eip1559::INITIAL_BASE_FEE;
use alloy_primitives::{B256, U256};
use eyre::OptionExt;
use ress_storage::Storage;
use reth_chainspec::ChainSpec;
use reth_evm::{execute::BlockExecutionStrategy, ConfigureEvm, ConfigureEvmEnv};
use reth_evm_ethereum::{execute::EthExecutionStrategy, EthEvmConfig};
use reth_primitives::{BlockWithSenders, Receipt, SealedBlock};
use reth_primitives_traits::transaction::signed::SignedTransaction;
use reth_provider::BlockExecutionOutput;
use reth_revm::{
    db::State,
    primitives::{BlobExcessGasAndPrice, BlockEnv, CfgEnvWithHandlerCfg, EnvWithHandlerCfg, TxEnv},
    StateBuilder,
};

use crate::{db::WitnessState, errors::EvmError};

pub struct BlockExecutor {
    strategy: EthExecutionStrategy<WitnessState, EthEvmConfig>,
}

impl BlockExecutor {
    /// specific block's executor by initiate with parent block post execution state and hash
    pub fn new(storage: Arc<Storage>, block_hash: B256, chain_spec: Arc<ChainSpec>) -> Self {
        let eth_evm_config = EthEvmConfig::new(storage.chain_spec.clone());
        let state = StateBuilder::new_with_database(WitnessState {
            storage,
            block_hash,
        })
        .with_bundle_update()
        .without_state_clear()
        .build();
        let strategy = EthExecutionStrategy::new(state, chain_spec, eth_evm_config);
        Self { strategy }
    }

    pub fn execute(
        &mut self,
        block: &SealedBlock,
    ) -> Result<BlockExecutionOutput<Receipt>, EvmError> {
        // todo: is this correct
        // let total_difficulty = self
        //     .chain_config()
        //     .final_paris_total_difficulty(block.number)
        //     .unwrap();
        //TODO: smth like this
        self.strategy.apply_pre_execution_changes(
            &BlockWithSenders::new(block.unseal(), block.senders().unwrap()).unwrap(),
            U256::ZERO,
        );
        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;

        for transaction in block.body.transactions.iter() {
            let header = &block.header;
            let block_env = BlockEnv {
                number: U256::from(header.number),
                coinbase: header.header().beneficiary,
                timestamp: U256::from(header.timestamp),
                gas_limit: U256::from(header.gas_limit),
                basefee: U256::from(header.base_fee_per_gas.unwrap_or(INITIAL_BASE_FEE)),
                difficulty: header.difficulty,
                prevrandao: Some(header.header().mix_hash),
                blob_excess_gas_and_price: Some(BlobExcessGasAndPrice::new(
                    header.excess_blob_gas.unwrap_or_default(),
                )),
            };
            let mut tx_env = TxEnv::default();
            self.eth_evm_config.fill_tx_env(
                &mut tx_env,
                transaction,
                transaction
                    .recover_signer_unchecked()
                    .ok_or_eyre("failed to recover sender")
                    .unwrap(),
            );
            let db = &mut self.state.database;
            // todo: not sure how i can construct relevant env
            let mut cfg = CfgEnvWithHandlerCfg::new(Default::default(), Default::default());
            self.eth_evm_config
                .fill_cfg_env(&mut cfg, header, total_difficulty);
            let env_hander_cfg = EnvWithHandlerCfg::new_with_cfg_env(cfg, block_env, tx_env);
            let mut evm = self.eth_evm_config.evm_with_env(db, env_hander_cfg);
            // todo: rn error with `RejectCallerWithCode`
            let result = evm.transact_commit().unwrap();

            cumulative_gas_used += result.gas_used();
            let receipt = Receipt {
                tx_type: transaction.tx_type(),
                success: result.is_success(),
                cumulative_gas_used,
                logs: result.logs().to_vec(),
            };
            receipts.push(receipt);
        }

        if let Some(_withdrawals) = &block.body.withdrawals {
            //todo: process withdrawl
        }

        todo!()
    }
}
