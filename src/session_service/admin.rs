use anyhow::Context;
use serde::de::DeserializeOwned;

use super::remote::{PairingOffer, PendingPairingInfo};

#[derive(Clone)]
pub struct LocalAdminClient {
    base_url: String,
    client: reqwest::Client,
}

impl LocalAdminClient {
    pub fn from_env() -> Self {
        let port = std::env::var("AKMUX_SESSION_PORT")
            .or_else(|_| std::env::var("CCSWITCH_SESSION_PORT"))
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(17321);
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_millis(750))
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .expect("Local admin HTTP client configuration is valid"),
        }
    }

    pub async fn create_pairing(&self) -> anyhow::Result<PairingOffer> {
        self.post_json("/api/pairing").await
    }

    pub async fn pending_pairings(&self) -> anyhow::Result<Vec<PendingPairingInfo>> {
        self.get_json("/api/pairing").await
    }

    pub async fn confirm_pairing(&self, id: &str) -> anyhow::Result<()> {
        self.post_empty(&format!("/api/pairing/{id}/confirm")).await
    }

    pub async fn cancel_pairing(&self, id: &str) -> anyhow::Result<()> {
        self.delete_empty(&format!("/api/pairing/{id}")).await
    }

    pub async fn revoke_device(&self, token_id: &str) -> anyhow::Result<()> {
        self.post_empty(&format!("/api/devices/{token_id}/revoke")).await
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let response = self.client.get(format!("{}{path}", self.base_url)).send().await?;
        parse_json_response(response).await
    }

    async fn post_json<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let response = self.client.post(format!("{}{path}", self.base_url)).json(&serde_json::json!({})).send().await?;
        parse_json_response(response).await
    }

    async fn post_empty(&self, path: &str) -> anyhow::Result<()> {
        let response = self.client.post(format!("{}{path}", self.base_url)).json(&serde_json::json!({})).send().await?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let body = response.json::<serde_json::Value>().await.unwrap_or_default();
        anyhow::bail!(
            "{}",
            body.get("error")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Local backend request failed with {status}"))
        )
    }

    async fn delete_empty(&self, path: &str) -> anyhow::Result<()> {
        let response = self.client.delete(format!("{}{path}", self.base_url)).send().await?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let body = response.json::<serde_json::Value>().await.unwrap_or_default();
        anyhow::bail!(
            "{}",
            body.get("error")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Local backend request failed with {status}"))
        )
    }
}

async fn parse_json_response<T: DeserializeOwned>(response: reqwest::Response) -> anyhow::Result<T> {
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        let body = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default();
        anyhow::bail!(
            "{}",
            body.get("error")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Local backend request failed with {status}"))
        );
    }
    serde_json::from_slice(&bytes).context("Local backend returned an invalid response")
}
