//! Auto-proxy middleware: OpenAI-compatible API that routes to local Ollama.
//!
//! Acts as a middleman between any OpenAI-compatible client and local Ollama servers.
//! Translates OpenAI chat/completions format to Ollama's native API.
//! 100% local — no external API calls.

use crate::ollama::{ChatMessage, OllamaOptions};
use crate::pool::OllamaPool;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::debug;

/// OpenAI-compatible chat completion request.
#[derive(Debug, Deserialize)]
pub struct OpenAiChatRequest {
    pub model: String,
    pub messages: Vec<OpenAiMessage>,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub top_p: Option<f64>,
    /// If "json_object", enable JSON mode.
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

fn default_temperature() -> f64 {
    0.3
}

#[derive(Debug, Deserialize)]
pub struct OpenAiMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ResponseFormat {
    #[serde(rename = "type")]
    pub format_type: String,
}

/// OpenAI-compatible chat completion response.
#[derive(Debug, Serialize)]
pub struct OpenAiChatResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<OpenAiChoice>,
    pub usage: OpenAiUsage,
}

#[derive(Debug, Serialize)]
pub struct OpenAiChoice {
    pub index: u32,
    pub message: OpenAiResponseMessage,
    pub finish_reason: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAiResponseMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAiUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// The neural proxy that translates OpenAI API → local Ollama.
///
/// This is the "middleman" that expands neural thinking by routing
/// requests through the local Ollama pool. It supports:
/// - Model aliasing (map OpenAI model names to local Ollama models)
/// - Automatic JSON mode detection
/// - Token usage tracking
/// - Request/response logging
pub struct NeuralProxy {
    pool: Arc<OllamaPool>,
    /// Map from requested model name to actual Ollama model name.
    model_aliases: std::collections::HashMap<String, String>,
    /// Default model to use if none specified.
    default_model: String,
}

impl NeuralProxy {
    pub fn new(pool: Arc<OllamaPool>, default_model: String) -> Self {
        let mut model_aliases = std::collections::HashMap::new();
        // Standard aliases for common model names
        model_aliases.insert("gpt-4".into(), default_model.clone());
        model_aliases.insert("gpt-4-turbo".into(), default_model.clone());
        model_aliases.insert("gpt-3.5-turbo".into(), default_model.clone());
        model_aliases.insert("claude-3-opus".into(), default_model.clone());
        model_aliases.insert("claude-3-sonnet".into(), default_model.clone());

        Self {
            pool,
            model_aliases,
            default_model,
        }
    }

    /// Add a model alias mapping.
    pub fn add_alias(&mut self, from: &str, to: &str) {
        self.model_aliases.insert(from.to_string(), to.to_string());
    }

    /// Resolve a model name through aliases.
    fn resolve_model(&self, requested: &str) -> String {
        self.model_aliases
            .get(requested)
            .cloned()
            .unwrap_or_else(|| requested.to_string())
    }

    /// Process an OpenAI-compatible chat request, routing to local Ollama.
    pub async fn chat_completion(
        &self,
        request: OpenAiChatRequest,
    ) -> Result<OpenAiChatResponse, String> {
        let model = self.resolve_model(&request.model);
        debug!(
            requested_model = %request.model,
            resolved_model = %model,
            messages = request.messages.len(),
            "Proxying chat completion to local Ollama"
        );

        // Convert OpenAI messages → Ollama messages
        let messages: Vec<ChatMessage> = request
            .messages
            .iter()
            .map(|m| ChatMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let json_mode = request
            .response_format
            .as_ref()
            .map(|f| f.format_type == "json_object")
            .unwrap_or(false);

        let options = OllamaOptions {
            temperature: Some(request.temperature),
            top_p: request.top_p,
            num_predict: request.max_tokens,
            ..Default::default()
        };

        let (response, server_name) = self
            .pool
            .chat(&model, messages, Some(options), json_mode)
            .await
            .map_err(|e| format!("Ollama error: {}", e))?;

        debug!(
            server = %server_name,
            eval_count = ?response.eval_count,
            "Response received from local Ollama"
        );

        let prompt_tokens = response.prompt_eval_count.unwrap_or(0);
        let completion_tokens = response.eval_count.unwrap_or(0);

        Ok(OpenAiChatResponse {
            id: format!("ot-{}", uuid::Uuid::new_v4().simple()),
            object: "chat.completion".into(),
            created: chrono::Utc::now().timestamp(),
            model: response.model,
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiResponseMessage {
                    role: "assistant".into(),
                    content: response.message.content,
                },
                finish_reason: "stop".into(),
            }],
            usage: OpenAiUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            },
        })
    }

    /// Quick helper: send a simple prompt and get back the text response.
    pub async fn ask(
        &self,
        system: &str,
        prompt: &str,
        json_mode: bool,
    ) -> Result<String, String> {
        let request = OpenAiChatRequest {
            model: self.default_model.clone(),
            messages: vec![
                OpenAiMessage {
                    role: "system".into(),
                    content: system.into(),
                },
                OpenAiMessage {
                    role: "user".into(),
                    content: prompt.into(),
                },
            ],
            temperature: 0.3,
            max_tokens: Some(4096),
            stream: false,
            top_p: None,
            response_format: if json_mode {
                Some(ResponseFormat {
                    format_type: "json_object".into(),
                })
            } else {
                None
            },
        };

        let response = self.chat_completion(request).await?;
        Ok(response
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OllamaServerConfig;

    #[test]
    fn model_alias_resolution() {
        let pool = Arc::new(OllamaPool::new(&[], 5));
        let proxy = NeuralProxy::new(pool, "qwen-trading:latest".into());

        assert_eq!(proxy.resolve_model("gpt-4"), "qwen-trading:latest");
        assert_eq!(proxy.resolve_model("unknown-model"), "unknown-model");
    }
}
