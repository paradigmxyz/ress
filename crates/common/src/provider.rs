use std::str::FromStr;

use alloy_primitives::{Address, BlockNumber, Bytes};
use alloy_provider::{Provider, ProviderBuilder, RootProvider};
use alloy_transport_http::Http;
use reqwest::{Client, Url};

pub struct ProviderWrapper {
    provider: RootProvider<Http<Client>>,
}

impl ProviderWrapper {
    pub fn new(rpc_url: String) -> Self {
        let provider =
            ProviderBuilder::new().on_http(Url::from_str(rpc_url.as_str()).expect("invalid rpc"));
        Self { provider }
    }

    /// eth_getCode from address
    pub async fn get_code(
        &self,
        address: Address,
        block_number: Option<BlockNumber>,
    ) -> Result<Bytes, Box<dyn std::error::Error>> {
        let bytes = match block_number {
            Some(number) => self.provider.get_code_at(address).number(number).await?,
            None => self.provider.get_code_at(address).latest().await?,
        };
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_code() {
        let provider = ProviderWrapper::new("https://ethereum-rpc.publicnode.com".to_string());
        let _ = provider
            .get_code(
                Address::from_str("0x5B56438000bAc5ed2c6E0c1EcFF4354aBfFaf889").unwrap(),
                None,
            )
            .await;
    }
}
