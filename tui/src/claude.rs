//! Claude API client with tool-use loop.
//!
//! Implements the agentic pattern:
//! 1. Send user message + tool definitions to Claude
//! 2. Claude responds with tool_use blocks
//! 3. Execute tools via gRPC → Go agent → K8s API + GKE API
//! 4. Send tool results back to Claude
//! 5. Repeat until Claude responds with text (no more tool calls)

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::AgentClient;

const CLAUDE_API_URL: &str = "https://api.anthropic.com/v1/messages";
const MODEL: &str = "claude-sonnet-4-6";
const MAX_TOOL_ROUNDS: usize = 10;

pub struct ClaudeClient {
    http: reqwest::Client,
    api_key: String,
    conversation: Vec<Message>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Message {
    role: String,
    content: Value,
}

impl ClaudeClient {
    pub fn new(api_key: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.to_string(),
            conversation: Vec::new(),
        }
    }

    /// Run a full tool-use conversation loop and return the final text response.
    pub async fn chat(
        &mut self,
        user_input: &str,
        agent: &mut AgentClient,
    ) -> Result<String> {
        self.conversation.push(Message {
            role: "user".into(),
            content: Value::String(user_input.to_string()),
        });

        for _ in 0..MAX_TOOL_ROUNDS {
            let response = self.call_api().await?;

            let content = response["content"]
                .as_array()
                .ok_or_else(|| anyhow!("Missing content in response"))?;

            let stop_reason = response["stop_reason"]
                .as_str()
                .unwrap_or("end_turn");

            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();

            for block in content {
                match block["type"].as_str() {
                    Some("text") => {
                        if let Some(text) = block["text"].as_str() {
                            text_parts.push(text.to_string());
                        }
                    }
                    Some("tool_use") => {
                        tool_calls.push(block.clone());
                    }
                    _ => {}
                }
            }

            self.conversation.push(Message {
                role: "assistant".into(),
                content: Value::Array(content.clone()),
            });

            if stop_reason != "tool_use" || tool_calls.is_empty() {
                return Ok(text_parts.join("\n"));
            }

            let mut tool_results = Vec::new();
            for tool_call in &tool_calls {
                let tool_name = tool_call["name"].as_str().unwrap_or("");
                let tool_id = tool_call["id"].as_str().unwrap_or("");
                let input = &tool_call["input"];

                tracing::info!("Executing tool: {} with input: {}", tool_name, input);

                let result = execute_tool(tool_name, input, agent).await;

                tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tool_id,
                    "content": match &result {
                        Ok(output) => output.clone(),
                        Err(e) => format!("Error: {}", e),
                    },
                    "is_error": result.is_err(),
                }));
            }

            self.conversation.push(Message {
                role: "user".into(),
                content: Value::Array(tool_results),
            });
        }

        Err(anyhow!("Exceeded maximum tool-use rounds"))
    }

    async fn call_api(&self) -> Result<Value> {
        let body = json!({
            "model": MODEL,
            "max_tokens": 2048,
            "system": SYSTEM_PROMPT,
            "tools": tool_definitions(),
            "messages": self.conversation,
        });

        let resp = self
            .http
            .post(CLAUDE_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await?;
            return Err(anyhow!("Claude API error {}: {}", status, body));
        }

        Ok(resp.json().await?)
    }
}

/// Execute a tool call by dispatching to the appropriate AgentClient method.
async fn execute_tool(
    name: &str,
    input: &Value,
    agent: &mut AgentClient,
) -> Result<String> {
    match name {
        // ─── Kubernetes API tools ──────────────────────────
        "get_pods" => {
            agent.get_pods(
                input["namespace"].as_str().unwrap_or(""),
                input["label_selector"].as_str().unwrap_or(""),
            ).await
        }
        "get_events" => {
            agent.get_events(
                input["namespace"].as_str().unwrap_or(""),
                input["involved_object"].as_str().unwrap_or(""),
                input["limit"].as_i64().unwrap_or(20) as i32,
            ).await
        }
        "get_logs" => {
            agent.get_logs(
                input["namespace"].as_str().unwrap_or(""),
                input["pod"].as_str().unwrap_or(""),
                input["container"].as_str().unwrap_or(""),
                input["tail_lines"].as_i64().unwrap_or(50) as i32,
                input["previous"].as_bool().unwrap_or(false),
            ).await
        }
        "describe_resource" => {
            agent.describe_resource(
                input["kind"].as_str().unwrap_or(""),
                input["namespace"].as_str().unwrap_or(""),
                input["name"].as_str().unwrap_or(""),
            ).await
        }
        "scale_deployment" => {
            agent.scale_deployment(
                input["namespace"].as_str().unwrap_or("default"),
                input["name"].as_str().unwrap_or(""),
                input["replicas"].as_i64().unwrap_or(1) as i32,
            ).await
        }
        "rollback_deployment" => {
            agent.rollback_deployment(
                input["namespace"].as_str().unwrap_or("default"),
                input["name"].as_str().unwrap_or(""),
                input["revision"].as_i64().unwrap_or(0),
            ).await
        }

        // ─── GKE Control Plane API tools ───────────────────
        "get_gke_cluster_info" => {
            agent.get_gke_cluster_info().await
        }
        "get_node_pools" => {
            agent.get_node_pools().await
        }
        "get_upgrade_info" => {
            agent.get_upgrade_info().await
        }
        "get_maintenance_windows" => {
            agent.get_maintenance_windows().await
        }
        "get_security_posture" => {
            agent.get_security_posture().await
        }
        "get_cluster_operations" => {
            agent.get_cluster_operations(
                input["active_only"].as_bool().unwrap_or(false),
            ).await
        }

        _ => Err(anyhow!("Unknown tool: {}", name)),
    }
}

/// Tool definitions sent to Claude — K8s API + GKE API tools.
fn tool_definitions() -> Value {
    json!([
        // ═══ Kubernetes API Tools ══════════════════════════
        {
            "name": "get_pods",
            "description": "List pods in the Kubernetes cluster via the K8s API. Use to check pod status, find failing pods, or overview workloads.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "namespace": { "type": "string", "description": "K8s namespace. Empty for all namespaces." },
                    "label_selector": { "type": "string", "description": "Label selector, e.g. 'app=redis'." }
                },
                "required": []
            }
        },
        {
            "name": "get_events",
            "description": "Get K8s events. Events explain why pods fail, nodes go unhealthy, or scaling triggers.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "namespace": { "type": "string", "description": "Namespace filter. Empty for all." },
                    "involved_object": { "type": "string", "description": "'Kind/Name' format, e.g. 'Pod/redis-0'." },
                    "limit": { "type": "integer", "description": "Max events. Default 20." }
                },
                "required": []
            }
        },
        {
            "name": "get_logs",
            "description": "Get container logs from a pod. Essential for debugging crash loops and app errors.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "namespace": { "type": "string" },
                    "pod": { "type": "string" },
                    "container": { "type": "string", "description": "Empty for first container." },
                    "tail_lines": { "type": "integer", "description": "Recent lines. Default 50." },
                    "previous": { "type": "boolean", "description": "Logs from previous instance (crash loops)." }
                },
                "required": ["namespace", "pod"]
            }
        },
        {
            "name": "describe_resource",
            "description": "Detailed info about any K8s resource (like kubectl describe).",
            "input_schema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "Pod, Deployment, Service, Node, PVC, etc." },
                    "namespace": { "type": "string", "description": "Empty for cluster-scoped." },
                    "name": { "type": "string" }
                },
                "required": ["kind", "name"]
            }
        },
        {
            "name": "scale_deployment",
            "description": "Scale a Deployment's replica count.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "namespace": { "type": "string", "description": "Default 'default'." },
                    "name": { "type": "string" },
                    "replicas": { "type": "integer" }
                },
                "required": ["name", "replicas"]
            }
        },
        {
            "name": "rollback_deployment",
            "description": "Roll back a Deployment to a previous revision.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "namespace": { "type": "string", "description": "Default 'default'." },
                    "name": { "type": "string" },
                    "revision": { "type": "integer", "description": "0 = previous revision." }
                },
                "required": ["name"]
            }
        },

        // ═══ GKE Control Plane API Tools ═══════════════════
        {
            "name": "get_gke_cluster_info",
            "description": "Get GKE cluster metadata from the GKE API (container.googleapis.com). Returns master/node versions, release channel, Autopilot status, networking config, and more. This data is NOT available via kubectl.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        },
        {
            "name": "get_node_pools",
            "description": "Get GKE node pool details from the GKE API. Returns machine types, autoscaling config, Spot VM status, disk types, upgrade strategies, and zone placement. Use when users ask about capacity, machine types, or why nodes aren't scaling.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        },
        {
            "name": "get_upgrade_info",
            "description": "Check GKE upgrade availability and version status via the GKE API. Returns current version, available upgrades, release channel, auto-upgrade status, and master/node version skew warnings.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        },
        {
            "name": "get_maintenance_windows",
            "description": "Get GKE maintenance window and exclusion configuration from the GKE API. Answers 'when is the next maintenance window?' and 'are there any maintenance exclusions?'",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        },
        {
            "name": "get_security_posture",
            "description": "Assess the GKE cluster's security configuration via the GKE API. Returns Workload Identity, Binary Authorization, Shielded Nodes, Network Policy, Dataplane V2, secret encryption status, and flags security concerns. Use for 'is my cluster secure?' questions.",
            "input_schema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        },
        {
            "name": "get_cluster_operations",
            "description": "List GKE control plane operations (upgrades, repairs, scaling) via the GKE API. Shows in-progress and recent operations with status and progress percentage.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "active_only": { "type": "boolean", "description": "Only show in-progress operations. Default false." }
                },
                "required": []
            }
        }
    ])
}

const SYSTEM_PROMPT: &str = r#"You are KCP Copilot, an AI assistant for GKE (Google Kubernetes Engine) cluster operations. You are unique because you reason across BOTH the Kubernetes API and the GKE Control Plane API simultaneously, giving developers a unified view they can't get from kubectl or the Cloud Console alone.

Your capabilities span two APIs:

KUBERNETES API (via client-go):
- Query pod status, events, and logs to diagnose workload issues
- Describe any K8s resource in detail
- Scale deployments and roll back failed releases

GKE CONTROL PLANE API (via container.googleapis.com):
- Cluster metadata: version, release channel, Autopilot, networking
- Node pool details: machine types, autoscaling, Spot VMs, upgrade strategy
- Upgrade info: available versions, version skew, auto-upgrade status
- Maintenance windows and exclusions
- Security posture: Workload Identity, BinAuth, Shielded Nodes, Dataplane V2
- Active operations: upgrades, repairs, scaling in progress

Behavior guidelines:
- For "is my cluster healthy?" — use BOTH APIs: K8s for pod/node health, GKE for version status, security, and operations
- For workload issues — start with K8s API tools (pods → events → logs)
- For platform questions — use GKE API tools (versions, node pools, security)
- Chain multiple tool calls when needed across both APIs
- For mutating operations (scale, rollback), state what you'll do before executing
- Keep responses concise — this is a terminal UI
- Always suggest concrete next steps

Example: "is my cluster healthy?"
1. get_gke_cluster_info() → version, release channel, status
2. get_pods(namespace="") → workload health across all namespaces
3. get_upgrade_info() → any pending upgrades or version skew
4. get_security_posture() → any security concerns
5. Synthesize into a unified health report

Example: "why won't my node pool scale up?"
1. get_node_pools() → check autoscaling config, current vs max nodes
2. get_pods(field_selector="status.phase=Pending") → find unschedulable pods
3. get_events(involved_object="Pod/...") → check scheduling events
4. get_cluster_operations(active_only=true) → any in-progress operations blocking scaling
5. Explain the root cause and suggest fix"#;
