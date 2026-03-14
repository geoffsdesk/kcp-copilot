use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::DefaultTerminal;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::agent::AgentClient;
use crate::claude::ClaudeClient;
use crate::gemini::{self, GeminiInsight, InsightSeverity};
use crate::ui;

/// A single message in the chat history.
#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Role {
    User,
    Assistant,
    System,
}

/// Cluster state for the right panel.
#[derive(Clone, Debug, Default)]
pub struct ClusterState {
    pub namespaces: Vec<NamespaceDisplay>,
    pub nodes: Vec<NodeDisplay>,
    pub warnings: Vec<String>,
    pub last_updated: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NamespaceDisplay {
    pub name: String,
    pub pods_summary: String,
    pub issues: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct NodeDisplay {
    pub name: String,
    pub ready: bool,
    pub version: String,
}

/// A displayable Gemini insight for the bottom panel.
#[derive(Clone, Debug)]
pub struct InsightDisplay {
    pub icon: &'static str,
    pub message: String,
    pub timestamp: String,
    pub severity: InsightSeverity,
}

pub struct App {
    pub input: String,
    pub chat_history: Vec<ChatMessage>,
    pub cluster_state: ClusterState,
    pub gemini_insights: Vec<InsightDisplay>,
    pub gemini_enabled: bool,
    pub is_loading: bool,
    pub scroll_offset: u16,
    pub should_quit: bool,

    agent: AgentClient,
    claude: ClaudeClient,
    gemini_rx: Option<mpsc::Receiver<GeminiInsight>>,
}

impl App {
    pub async fn new(
        agent_addr: &str,
        anthropic_key: &str,
        gemini_key: Option<&str>,
    ) -> Result<Self> {
        let agent = AgentClient::connect(agent_addr).await?;
        let claude = ClaudeClient::new(anthropic_key);

        // Spawn Gemini background analyst if API key is provided
        let (gemini_enabled, gemini_rx) = if let Some(key) = gemini_key {
            let rx = gemini::spawn_gemini_analyst(agent.clone(), key.to_string());
            (true, Some(rx))
        } else {
            (false, None)
        };

        Ok(Self {
            input: String::new(),
            chat_history: vec![ChatMessage {
                role: Role::System,
                content: if gemini_enabled {
                    "Connected to GKE cluster. Claude + Gemini active. Ask me anything!"
                } else {
                    "Connected to GKE cluster. Ask me anything! (Set GEMINI_API_KEY for proactive insights)"
                }.into(),
            }],
            cluster_state: ClusterState::default(),
            gemini_insights: Vec::new(),
            gemini_enabled,
            is_loading: false,
            scroll_offset: 0,
            should_quit: false,
            agent,
            claude,
            gemini_rx,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut terminal = ratatui::init();

        // Initial cluster overview fetch
        self.refresh_cluster_overview().await;

        let result = self.event_loop(&mut terminal).await;

        ratatui::restore();
        result
    }

    async fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            // Draw UI
            terminal.draw(|frame| ui::render(frame, self))?;

            // Check for Gemini insights (non-blocking)
            if let Some(rx) = &mut self.gemini_rx {
                while let Ok(insight) = rx.try_recv() {
                    let display = InsightDisplay {
                        icon: match &insight.severity {
                            InsightSeverity::Critical => "🔴",
                            InsightSeverity::Warning => "🟡",
                            InsightSeverity::Info => "🔵",
                        },
                        message: insight.message,
                        timestamp: insight.timestamp,
                        severity: insight.severity,
                    };
                    self.gemini_insights.push(display);
                    // Keep last 20 insights
                    if self.gemini_insights.len() > 20 {
                        self.gemini_insights.remove(0);
                    }
                }
            }

            // Poll for keyboard events with timeout (allows async work)
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    match (key.code, key.modifiers) {
                        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                            self.should_quit = true;
                        }
                        (KeyCode::Enter, _) if !self.input.is_empty() && !self.is_loading => {
                            self.handle_submit().await?;
                        }
                        (KeyCode::Char(c), _) if !self.is_loading => {
                            self.input.push(c);
                        }
                        (KeyCode::Backspace, _) if !self.is_loading => {
                            self.input.pop();
                        }
                        (KeyCode::Up, _) => {
                            self.scroll_offset = self.scroll_offset.saturating_add(1);
                        }
                        (KeyCode::Down, _) => {
                            self.scroll_offset = self.scroll_offset.saturating_sub(1);
                        }
                        _ => {}
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }

    async fn handle_submit(&mut self) -> Result<()> {
        let user_input = std::mem::take(&mut self.input);

        self.chat_history.push(ChatMessage {
            role: Role::User,
            content: user_input.clone(),
        });

        self.is_loading = true;

        match self.claude.chat(&user_input, &mut self.agent).await {
            Ok(response) => {
                self.chat_history.push(ChatMessage {
                    role: Role::Assistant,
                    content: response,
                });
                self.refresh_cluster_overview().await;
            }
            Err(e) => {
                self.chat_history.push(ChatMessage {
                    role: Role::System,
                    content: format!("Error: {}", e),
                });
            }
        }

        self.is_loading = false;
        self.scroll_offset = 0;
        Ok(())
    }

    async fn refresh_cluster_overview(&mut self) {
        match self.agent.get_cluster_overview().await {
            Ok(overview) => {
                self.cluster_state = ClusterState {
                    namespaces: overview
                        .namespaces
                        .into_iter()
                        .map(|ns| NamespaceDisplay {
                            name: ns.namespace,
                            pods_summary: format!("{}/{} running", ns.running_pods, ns.total_pods),
                            issues: ns.issues,
                        })
                        .collect(),
                    nodes: overview
                        .nodes
                        .into_iter()
                        .map(|n| NodeDisplay {
                            name: n.name,
                            ready: n.ready,
                            version: n.version,
                        })
                        .collect(),
                    warnings: overview
                        .recent_warnings
                        .into_iter()
                        .map(|w| format!("{}: {}", w.involved_object, w.message))
                        .collect(),
                    last_updated: Some(chrono::Local::now().format("%H:%M:%S").to_string()),
                };
            }
            Err(e) => {
                tracing::warn!("Failed to refresh cluster overview: {}", e);
            }
        }
    }
}
