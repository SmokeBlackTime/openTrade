//! Multi-server Ollama pool with health checks and load balancing.
//!
//! Manages multiple Ollama instances across the network.
//! Provides automatic failover, health monitoring, and weighted routing.
//! Supports discovering models across all servers.

use crate::ollama::{ChatMessage, ChatResponse, OllamaClient, OllamaError, OllamaOptions};
use crate::OllamaServerConfig;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Health status of a single server.
#[derive(Debug, Clone)]
pub struct ServerHealth {
    pub name: String,
    pub base_url: String,
    pub is_healthy: bool,
    pub last_check_ms: i64,
    pub latency_ms: Option<u64>,
    pub available_models: Vec<String>,
    pub consecutive_failures: u32,
}

/// A server entry in the pool.
struct PoolEntry {
    config: OllamaServerConfig,
    client: OllamaClient,
    health: ServerHealth,
}

/// Multi-server Ollama pool.
///
/// Distributes inference requests across multiple Ollama instances.
/// Supports weighted load balancing, automatic health checks,
/// and model-aware routing (route to a server that has the requested model).
pub struct OllamaPool {
    entries: Arc<RwLock<Vec<PoolEntry>>>,
    /// Model → server indices that have this model.
    model_index: Arc<RwLock<HashMap<String, Vec<usize>>>>,
}

impl OllamaPool {
    /// Create a new pool from server configurations.
    pub fn new(configs: &[OllamaServerConfig], timeout_secs: u64) -> Self {
        let entries: Vec<PoolEntry> = configs
            .iter()
            .filter(|c| c.enabled)
            .map(|config| {
                info!(
                    server = %config.name,
                    url = %config.base_url,
                    models = ?config.models,
                    weight = config.weight,
                    "Registering Ollama server in pool"
                );
                let client = OllamaClient::new(&config.base_url, timeout_secs);
                let health = ServerHealth {
                    name: config.name.clone(),
                    base_url: config.base_url.clone(),
                    is_healthy: true, // assume healthy until proven otherwise
                    last_check_ms: 0,
                    latency_ms: None,
                    available_models: config.models.clone(),
                    consecutive_failures: 0,
                };
                PoolEntry {
                    config: config.clone(),
                    client,
                    health,
                }
            })
            .collect();

        Self {
            entries: Arc::new(RwLock::new(entries)),
            model_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Run health checks on all servers and discover available models.
    pub async fn health_check_all(&self) {
        let mut entries = self.entries.write().await;
        let mut model_idx: HashMap<String, Vec<usize>> = HashMap::new();

        for (i, entry) in entries.iter_mut().enumerate() {
            let start = std::time::Instant::now();
            info!(
                server = %entry.config.name,
                url = %entry.config.base_url,
                "Running health check"
            );
            let healthy = entry.client.health_check().await;
            let latency = start.elapsed().as_millis() as u64;

            if healthy {
                entry.health.is_healthy = true;
                entry.health.latency_ms = Some(latency);
                entry.health.consecutive_failures = 0;

                // Discover models
                if let Ok(models) = entry.client.list_models().await {
                    let model_names: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
                    entry.health.available_models = model_names.clone();
                    for model_name in &model_names {
                        model_idx.entry(model_name.clone()).or_default().push(i);
                    }
                    info!(
                        server = %entry.config.name,
                        models = model_names.len(),
                        latency_ms = latency,
                        "Server healthy"
                    );
                }
            } else {
                entry.health.is_healthy = false;
                entry.health.consecutive_failures += 1;
                warn!(
                    server = %entry.config.name,
                    failures = entry.health.consecutive_failures,
                    "Server unhealthy"
                );
            }
            entry.health.last_check_ms = Utc::now().timestamp_millis();
        }

        // Also index pre-configured models
        for (i, entry) in entries.iter().enumerate() {
            for model in &entry.config.models {
                model_idx.entry(model.clone()).or_default().push(i);
            }
        }

        *self.model_index.write().await = model_idx;
    }

    /// Send a chat request, automatically routing to a healthy server with the model.
    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        options: Option<OllamaOptions>,
        json_mode: bool,
    ) -> Result<(ChatResponse, String), OllamaError> {
        let entries = self.entries.read().await;

        // Find servers that have this model, preferring healthy ones with lowest latency
        let model_idx = self.model_index.read().await;
        let candidates: Vec<usize> = if let Some(indices) = model_idx.get(model) {
            indices
                .iter()
                .filter(|&&i| entries.get(i).map(|e| e.health.is_healthy).unwrap_or(false))
                .copied()
                .collect()
        } else {
            // Fall back to any healthy server
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.health.is_healthy)
                .map(|(i, _)| i)
                .collect()
        };

        if candidates.is_empty() {
            // Last resort: try all servers regardless of health
            for entry in entries.iter() {
                match entry
                    .client
                    .chat(model, messages.clone(), options.clone(), json_mode)
                    .await
                {
                    Ok(resp) => return Ok((resp, entry.config.name.clone())),
                    Err(_) => continue,
                }
            }
            return Err(OllamaError::NoServersAvailable);
        }

        // Pick the server with lowest latency among candidates, weighted by config weight
        let best_idx = candidates
            .iter()
            .min_by_key(|&&i| {
                let entry = &entries[i];
                let latency = entry.health.latency_ms.unwrap_or(1000);
                let weight_factor = if entry.config.weight > 0 {
                    100 / entry.config.weight as u64
                } else {
                    100
                };
                latency * weight_factor
            })
            .copied()
            .unwrap_or(candidates[0]);

        let entry = &entries[best_idx];
        info!(
            server = %entry.config.name,
            url = %entry.config.base_url,
            model = model,
            latency_ms = entry.health.latency_ms,
            candidates = candidates.len(),
            "Routing chat request"
        );

        let resp = entry
            .client
            .chat(model, messages, options, json_mode)
            .await?;
        Ok((resp, entry.config.name.clone()))
    }

    /// Send a chat request to a specific server by name.
    pub async fn chat_on_server(
        &self,
        server_name: &str,
        model: &str,
        messages: Vec<ChatMessage>,
        options: Option<OllamaOptions>,
        json_mode: bool,
    ) -> Result<(ChatResponse, String), OllamaError> {
        let entries = self.entries.read().await;
        // Try healthy first, fall back to any matching server
        let entry = entries
            .iter()
            .find(|e| e.config.name == server_name && e.health.is_healthy)
            .or_else(|| {
                warn!(
                    server = server_name,
                    "Server not healthy, attempting anyway"
                );
                entries.iter().find(|e| e.config.name == server_name)
            })
            .ok_or_else(|| {
                warn!(
                    server = server_name,
                    registered = entries.iter().map(|e| e.config.name.as_str()).collect::<Vec<_>>().join(", "),
                    "Server not found in pool"
                );
                OllamaError::NoServersAvailable
            })?;

        info!(
            server = %entry.config.name,
            url = %entry.config.base_url,
            model = model,
            "Routing chat request to specific server"
        );

        let resp = entry
            .client
            .chat(model, messages, options, json_mode)
            .await?;
        Ok((resp, entry.config.name.clone()))
    }

    /// Get health status of all servers.
    pub async fn health_status(&self) -> Vec<ServerHealth> {
        self.entries
            .read()
            .await
            .iter()
            .map(|e| e.health.clone())
            .collect()
    }

    /// Get all available models across all servers.
    pub async fn available_models(&self) -> Vec<String> {
        let idx = self.model_index.read().await;
        idx.keys().cloned().collect()
    }

    /// Number of healthy servers.
    pub async fn healthy_count(&self) -> usize {
        self.entries
            .read()
            .await
            .iter()
            .filter(|e| e.health.is_healthy)
            .count()
    }

    /// Total number of servers in the pool.
    pub async fn total_count(&self) -> usize {
        self.entries.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_pool_returns_no_servers() {
        let pool = OllamaPool::new(&[], 5);
        let result = pool
            .chat(
                "test",
                vec![ChatMessage::user("hello")],
                None,
                false,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn pool_tracks_server_count() {
        let configs = vec![
            OllamaServerConfig {
                name: "server1".into(),
                base_url: "http://localhost:11434".into(),
                weight: 1,
                models: vec!["qwen-trading:latest".into()],
                enabled: true,
            },
            OllamaServerConfig {
                name: "server2".into(),
                base_url: "http://192.168.1.100:11434".into(),
                weight: 2,
                models: vec!["qwen-trading:latest".into()],
                enabled: true,
            },
        ];
        let pool = OllamaPool::new(&configs, 5);
        assert_eq!(pool.total_count().await, 2);
    }
}
