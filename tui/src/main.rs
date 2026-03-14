mod agent;
mod app;
mod claude;
mod gemini;
mod ui;

use anyhow::Result;
use app::App;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging (writes to file, not terminal — TUI owns stdout)
    tracing_subscriber::fmt()
        .with_writer(std::fs::File::create("/tmp/kcp-copilot.log")?)
        .init();

    let agent_addr = std::env::var("KCP_AGENT_ADDR")
        .unwrap_or_else(|_| "http://127.0.0.1:50051".to_string());

    let anthropic_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("ANTHROPIC_API_KEY must be set");

    // Gemini API key is optional — insights panel is disabled without it
    let gemini_key = std::env::var("GEMINI_API_KEY").ok();

    let mut app = App::new(&agent_addr, &anthropic_key, gemini_key.as_deref()).await?;
    app.run().await
}
