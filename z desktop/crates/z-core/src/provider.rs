//! Provider layer — model APIs behind one trait.
//!
//! v0.1 speaks the two wire formats that cover most of the ecosystem:
//! OpenAI-compatible `/chat/completions` and Anthropic-compatible
//! `/v1/messages`, both with SSE streaming. HTTP is blocking (`ureq`): the
//! agent loop already runs on a worker thread, so an async runtime would buy
//! nothing but compile time and complexity.

use serde_json::{json, Value};
use std::io::BufRead;
use z_protocol::ProviderConfig;

/// One message in the provider-neutral conversation format.
#[derive(Debug, Clone)]
pub enum ChatMessage {
    System(String),
    User(String),
    /// Assistant text plus any tool calls it made in this turn.
    Assistant { text: String, tool_calls: Vec<ToolCallSpec> },
    /// Result of one tool call (role "tool" on the wire).
    ToolResult { call_id: String, output: String },
}

#[derive(Debug, Clone)]
pub struct ToolCallSpec {
    pub id: String,
    pub name: String,
    pub arguments_json: String,
}

/// A tool definition advertised to the model (JSON-schema parameters).
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: u32,
}

/// Items streamed back by a provider.
#[derive(Debug)]
pub enum StreamItem {
    TextDelta(String),
    ToolCall(ToolCallSpec),
}

/// What one streaming call produced, after the stream ends.
#[derive(Debug, Default)]
pub struct StreamOutcome {
    pub text: String,
    pub tool_calls: Vec<ToolCallSpec>,
}

impl StreamOutcome {
    fn push(&mut self, item: StreamItem) {
        match item {
            StreamItem::TextDelta(d) => self.text.push_str(&d),
            StreamItem::ToolCall(c) => self.tool_calls.push(c),
        }
    }
}

pub trait Provider: Send + Sync {
    fn describe(&self) -> String;
    /// Configured model id — the capability-registry lookup key (prov-004).
    /// Default keeps test mocks compiling as "unknown model" (registry
    /// fallback: no tools); both real adapters override this.
    fn model(&self) -> &str {
        ""
    }
    /// Run one streaming chat request. `on_item` receives deltas as they
    /// arrive; the full outcome is also returned for the conversation history.
    fn stream(
        &self,
        req: &ChatRequest,
        on_item: &mut dyn FnMut(StreamItem),
    ) -> Result<StreamOutcome, String>;
}

// ---------------------------------------------------------------------------
// SSE line reader shared by both adapters
// ---------------------------------------------------------------------------

/// Call `body_line` for every `data:` payload of an SSE stream.
fn read_sse(reader: impl BufRead, mut body_line: impl FnMut(&str)) -> Result<(), String> {
    for line in reader.lines() {
        let line = line.map_err(|e| format!("stream read failed: {e}"))?;
        if let Some(data) = line.strip_prefix("data:") {
            body_line(data.trim());
        }
        // Non-data lines (event:, comments) carry nothing we need in v0.1.
    }
    Ok(())
}

fn http_error(status: u16, body: &str) -> String {
    // Keep the body short: error bodies can be huge HTML pages.
    let snippet: String = body.chars().take(400).collect();
    format!("provider returned HTTP {status}: {snippet}")
}

// ---------------------------------------------------------------------------
// OpenAI-compatible
// ---------------------------------------------------------------------------

pub struct OpenAiProvider {
    pub config: ProviderConfig,
}

impl Provider for OpenAiProvider {
    fn describe(&self) -> String {
        format!("openai-compatible · {} · {}", self.config.base_url, self.config.model)
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn stream(
        &self,
        req: &ChatRequest,
        on_item: &mut dyn FnMut(StreamItem),
    ) -> Result<StreamOutcome, String> {
        let url = format!("{}/chat/completions", self.config.base_url.trim_end_matches('/'));
        let mut messages = Vec::with_capacity(req.messages.len());
        for m in &req.messages {
            match m {
                ChatMessage::System(t) => messages.push(json!({"role":"system","content":t})),
                ChatMessage::User(t) => messages.push(json!({"role":"user","content":t})),
                ChatMessage::Assistant { text, tool_calls } => {
                    let calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id, "type": "function",
                                "function": {"name": c.name, "arguments": c.arguments_json}
                            })
                        })
                        .collect();
                    let mut msg = json!({"role":"assistant","content":text});
                    if !calls.is_empty() {
                        msg["tool_calls"] = Value::Array(calls);
                    }
                    messages.push(msg);
                }
                ChatMessage::ToolResult { call_id, output } => {
                    messages.push(json!({"role":"tool","tool_call_id":call_id,"content":output}));
                }
            }
        }

        let mut body = json!({
            "model": self.config.model,
            "messages": messages,
            "stream": true,
            "max_tokens": req.max_tokens,
        });
        if !req.tools.is_empty() {
            body["tools"] = json!(
                req.tools.iter().map(|t| json!({
                    "type":"function",
                    "function":{"name":t.name,"description":t.description,"parameters":t.parameters}
                })).collect::<Vec<_>>()
            );
        }

        let response = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.config.api_key))
            .send_json(body)
            .map_err(|e| match e {
                ureq::Error::Status(code, resp) => {
                    let text = resp.into_string().unwrap_or_default();
                    http_error(code, &text)
                }
                other => format!("request failed: {other}"),
            })?;

        let reader = std::io::BufReader::new(response.into_reader());
        let mut outcome = StreamOutcome::default();
        // Partial tool-call arguments arrive across several deltas; assemble them.
        let mut partial: Vec<(Value, String, String)> = Vec::new(); // (id?, name, args)

        read_sse(reader, |data| {
            if data == "[DONE]" {
                return;
            }
            let Ok(v) = serde_json::from_str::<Value>(data) else { return };
            let Some(delta) = v["choices"][0]["delta"].as_object() else { return };
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    on_item(StreamItem::TextDelta(text.to_string()));
                }
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let index = call["index"].as_u64().unwrap_or(0) as usize;
                    while partial.len() <= index {
                        partial.push((Value::Null, String::new(), String::new()));
                    }
                    if let Some(id) = call["id"].as_str() {
                        partial[index].0 = json!(id);
                    }
                    if let Some(name) = call["function"]["name"].as_str() {
                        partial[index].1.push_str(name);
                    }
                    if let Some(args) = call["function"]["arguments"].as_str() {
                        partial[index].2.push_str(args);
                    }
                }
            }
        })?;

        for (id, name, args) in partial {
            if name.is_empty() {
                continue;
            }
            let spec = ToolCallSpec {
                id: id.as_str().unwrap_or("call_0").to_string(),
                name,
                arguments_json: if args.is_empty() { "{}".into() } else { args },
            };
            on_item(StreamItem::ToolCall(spec.clone()));
            outcome.tool_calls.push(spec);
        }
        Ok(outcome)
    }
}

// ---------------------------------------------------------------------------
// Anthropic-compatible
// ---------------------------------------------------------------------------

pub struct AnthropicProvider {
    pub config: ProviderConfig,
}

impl Provider for AnthropicProvider {
    fn describe(&self) -> String {
        format!("anthropic-compatible · {} · {}", self.config.base_url, self.config.model)
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn stream(
        &self,
        req: &ChatRequest,
        on_item: &mut dyn FnMut(StreamItem),
    ) -> Result<StreamOutcome, String> {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let mut system = String::new();
        let mut messages = Vec::with_capacity(req.messages.len());
        for m in &req.messages {
            match m {
                ChatMessage::System(t) => {
                    if !system.is_empty() {
                        system.push_str("

");
                    }
                    system.push_str(t);
                }
                ChatMessage::User(t) => messages.push(json!({"role":"user","content":t})),
                ChatMessage::Assistant { text, tool_calls } => {
                    let mut blocks = Vec::new();
                    if !text.is_empty() {
                        blocks.push(json!({"type":"text","text":text}));
                    }
                    for c in tool_calls {
                        let input: Value =
                            serde_json::from_str(&c.arguments_json).unwrap_or(json!({}));
                        blocks.push(json!({"type":"tool_use","id":c.id,"name":c.name,"input":input}));
                    }
                    messages.push(json!({"role":"assistant","content":blocks}));
                }
                ChatMessage::ToolResult { call_id, output } => {
                    // Anthropic returns tool results inside the next user turn.
                    messages.push(json!({
                        "role":"user",
                        "content":[{"type":"tool_result","tool_use_id":call_id,"content":output}]
                    }));
                }
            }
        }

        let mut body = json!({
            "model": self.config.model,
            "max_tokens": req.max_tokens,
            "stream": true,
            "messages": messages,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if !req.tools.is_empty() {
            body["tools"] = json!(
                req.tools.iter().map(|t| json!({
                    "name":t.name,"description":t.description,"input_schema":t.parameters
                })).collect::<Vec<_>>()
            );
        }

        let response = ureq::post(&url)
            .set("x-api-key", &self.config.api_key)
            .set("anthropic-version", "2023-06-01")
            .send_json(body)
            .map_err(|e| match e {
                ureq::Error::Status(code, resp) => {
                    let text = resp.into_string().unwrap_or_default();
                    http_error(code, &text)
                }
                other => format!("request failed: {other}"),
            })?;

        let reader = std::io::BufReader::new(response.into_reader());
        let mut outcome = StreamOutcome::default();
        // Current tool_use block being assembled from input_json_delta chunks.
        let mut current_tool: Option<(String, String, String)> = None; // (id, name, args)

        read_sse(reader, |data| {
            let Ok(v) = serde_json::from_str::<Value>(data) else { return };
            match v["type"].as_str() {
                Some("content_block_start") => {
                    let block = &v["content_block"];
                    if block["type"] == "tool_use" {
                        current_tool = Some((
                            block["id"].as_str().unwrap_or("toolu_0").to_string(),
                            block["name"].as_str().unwrap_or("").to_string(),
                            String::new(),
                        ));
                    }
                }
                Some("content_block_delta") => {
                    let delta = &v["delta"];
                    match delta["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(t) = delta["text"].as_str() {
                                if !t.is_empty() {
                                    on_item(StreamItem::TextDelta(t.to_string()));
                                }
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some((_, _, args)) = current_tool.as_mut() {
                                if let Some(p) = delta["partial_json"].as_str() {
                                    args.push_str(p);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Some("content_block_stop") => {
                    if let Some((id, name, args)) = current_tool.take() {
                        if !name.is_empty() {
                            let spec = ToolCallSpec {
                                id,
                                name,
                                arguments_json: if args.is_empty() { "{}".into() } else { args },
                            };
                            on_item(StreamItem::ToolCall(spec.clone()));
                            outcome.tool_calls.push(spec);
                        }
                    }
                }
                _ => {}
            }
        })?;

        Ok(outcome)
    }
}

/// Build the provider described by a config. Unknown kinds are rejected here,
/// at the boundary, rather than failing mid-turn.
pub fn from_config(config: ProviderConfig) -> Result<Box<dyn Provider>, String> {
    use z_protocol::ProviderKind::*;
    match config.kind {
        OpenAi => Ok(Box::new(OpenAiProvider { config })),
        Anthropic => Ok(Box::new(AnthropicProvider { config })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_reader_extracts_data_lines_only() {
        let raw = r#"event: x
data: {"a":1}

data: [DONE]
note: ignored
"#;;
        let mut seen = Vec::new();
        read_sse(raw.as_bytes(), |d| seen.push(d.to_string())).unwrap();
        assert_eq!(seen, vec![r#"{"a":1}"#, "[DONE]"]);
    }

    #[test]
    fn unknown_provider_kind_is_rejected_at_the_boundary() {
        let cfg = ProviderConfig {
            name: "x".into(),
            kind: z_protocol::ProviderKind::OpenAi,
            base_url: "http://localhost".into(),
            model: "m".into(),
            api_key: String::new(),
        };
        assert!(from_config(cfg).is_ok());
    }
}