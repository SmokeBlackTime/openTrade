//! Ollama HTTP client for local LLM inference.
//!
//! Communicates with Ollama's REST API at /api/chat and /api/generate.
//! Supports both streaming and non-streaming modes.
//! Handles the ChatML format used by Qwen/thinking models.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

/// A single message in the chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

/// Options for Ollama inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl Default for OllamaOptions {
    fn default() -> Self {
        Self {
            temperature: Some(0.3),
            top_p: Some(0.9),
            top_k: Some(40),
            num_ctx: Some(8192),
            num_predict: Some(4096),
            stop: None,
            seed: None,
        }
    }
}

/// Request body for /api/chat.
#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
}

/// Response from /api/chat (non-streaming).
#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub model: String,
    pub message: ChatMessage,
    #[serde(default)]
    pub done: bool,
    pub total_duration: Option<u64>,
    pub load_duration: Option<u64>,
    pub prompt_eval_count: Option<u32>,
    pub eval_count: Option<u32>,
    pub eval_duration: Option<u64>,
}

impl ChatResponse {
    /// Extract the thinking block content (between <think> tags).
    pub fn thinking(&self) -> Option<&str> {
        let content = &self.message.content;
        let start = content.find("<think>")?;
        let end = content.find("</think>")?;
        if end > start + 7 {
            Some(content[start + 7..end].trim())
        } else {
            None
        }
    }

    /// Extract the final answer (after </think> tag, or full content if no thinking).
    pub fn answer(&self) -> &str {
        if let Some(end) = self.message.content.find("</think>") {
            self.message.content[end + 8..].trim()
        } else {
            self.message.content.trim()
        }
    }

    /// Tokens per second throughput.
    pub fn tokens_per_second(&self) -> Option<f64> {
        let eval_count = self.eval_count? as f64;
        let eval_duration = self.eval_duration? as f64;
        if eval_duration > 0.0 {
            Some(eval_count / (eval_duration / 1_000_000_000.0))
        } else {
            None
        }
    }
}

/// Request body for /api/generate.
#[derive(Debug, Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
}

/// Response from /api/generate.
#[derive(Debug, Deserialize)]
pub struct GenerateResponse {
    pub model: String,
    pub response: String,
    #[serde(default)]
    pub done: bool,
    pub total_duration: Option<u64>,
    pub eval_count: Option<u32>,
    pub eval_duration: Option<u64>,
}

/// Response from /api/tags (list models).
#[derive(Debug, Deserialize)]
pub struct TagsResponse {
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub model: String,
    pub size: u64,
    pub digest: String,
    #[serde(default)]
    pub details: ModelDetails,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelDetails {
    pub format: Option<String>,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub quantization_level: Option<String>,
}

/// Ollama HTTP client.
pub struct OllamaClient {
    client: Client,
    base_url: String,
}

impl OllamaClient {
    pub fn new(base_url: &str, timeout_secs: u64) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .pool_max_idle_per_host(4)
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Send a chat completion request (non-streaming).
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        options: Option<OllamaOptions>,
        json_mode: bool,
    ) -> Result<ChatResponse, OllamaError> {
        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: model.to_string(),
            messages,
            stream: false,
            options,
            format: if json_mode {
                Some("json".into())
            } else {
                None
            },
        };

        debug!(model = model, url = %url, "Sending chat request to Ollama");

        let response = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| OllamaError::Connection(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        response
            .json::<ChatResponse>()
            .await
            .map_err(|e| OllamaError::Parse(e.to_string()))
    }

    /// Send a generate (completion) request (non-streaming).
    pub async fn generate(
        &self,
        model: &str,
        prompt: &str,
        system: Option<&str>,
        options: Option<OllamaOptions>,
        json_mode: bool,
    ) -> Result<GenerateResponse, OllamaError> {
        let url = format!("{}/api/generate", self.base_url);
        let req = GenerateRequest {
            model: model.to_string(),
            prompt: prompt.to_string(),
            stream: false,
            system: system.map(String::from),
            options,
            format: if json_mode {
                Some("json".into())
            } else {
                None
            },
        };

        let response = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| OllamaError::Connection(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api {
                status: status.as_u16(),
                message: body,
            });
        }

        response
            .json::<GenerateResponse>()
            .await
            .map_err(|e| OllamaError::Parse(e.to_string()))
    }

    /// List available models on this Ollama server.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, OllamaError> {
        let url = format!("{}/api/tags", self.base_url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| OllamaError::Connection(e.to_string()))?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(OllamaError::Api {
                status: 500,
                message: body,
            });
        }

        let tags: TagsResponse = response
            .json()
            .await
            .map_err(|e| OllamaError::Parse(e.to_string()))?;

        Ok(tags.models)
    }

    /// Health check - is the server responsive?
    pub async fn health_check(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        match self.client.get(&url).send().await {
            Ok(r) => r.status().is_success(),
            Err(e) => {
                warn!(url = %self.base_url, error = %e, "Ollama health check failed");
                false
            }
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

/// Errors from Ollama communication.
#[derive(Debug, thiserror::Error)]
pub enum OllamaError {
    #[error("Connection error: {0}")]
    Connection(String),

    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Timeout after {0}s")]
    Timeout(u64),

    #[error("All servers unavailable")]
    NoServersAvailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_message_constructors() {
        let sys = ChatMessage::system("You are a trader");
        assert_eq!(sys.role, "system");
        let user = ChatMessage::user("Analyze BTC");
        assert_eq!(user.role, "user");
    }

    #[test]
    fn parse_thinking_block() {
        let resp = ChatResponse {
            model: "test".into(),
            message: ChatMessage::assistant(
                "<think>\nBTC is showing strong momentum\n</think>\nBuy signal detected.",
            ),
            done: true,
            total_duration: None,
            load_duration: None,
            prompt_eval_count: None,
            eval_count: None,
            eval_duration: None,
        };

        assert_eq!(resp.thinking(), Some("BTC is showing strong momentum"));
        assert_eq!(resp.answer(), "Buy signal detected.");
    }

    #[test]
    fn parse_no_thinking_block() {
        let resp = ChatResponse {
            model: "test".into(),
            message: ChatMessage::assistant("Just a plain response"),
            done: true,
            total_duration: None,
            load_duration: None,
            prompt_eval_count: None,
            eval_count: None,
            eval_duration: None,
        };

        assert!(resp.thinking().is_none());
        assert_eq!(resp.answer(), "Just a plain response");
    }
}
