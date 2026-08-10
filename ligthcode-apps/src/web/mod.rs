pub mod fetch;
pub mod search;

use std::time::Duration;

pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Shared HTTP client for the web layer: bounded timeout, no static state.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("lightcode")
        .build()
        .expect("reqwest client with static builder config cannot fail")
}
