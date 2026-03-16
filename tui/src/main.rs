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

    // ─── Validate API keys before starting TUI ─────────────────
    let anthropic_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() && key != "your-actual-key-here" => key,
        _ => {
            eprintln!();
            eprintln!("  KCP Copilot — API Key Setup");
            eprintln!("  ───────────────────────────────────────────");
            eprintln!();
            eprintln!("  ANTHROPIC_API_KEY is required (powers the NLP chat).");
            eprintln!();
            eprintln!("  Get your key from:");
            eprintln!("    https://console.anthropic.com/settings/keys");
            eprintln!();
            eprintln!("  Then run:");
            eprintln!("    export ANTHROPIC_API_KEY=\"sk-ant-...\"");
            eprintln!("    cargo run");
            eprintln!();
            eprintln!("  Optional — for proactive Gemini background insights:");
            eprintln!("    export GEMINI_API_KEY=\"AIza...\"");
            eprintln!();
            std::process::exit(1);
        }
    };

    // Gemini API key is optional — insights panel is disabled without it
    let gemini_key = std::env::var("GEMINI_API_KEY").ok().filter(|k| !k.is_empty());

    if gemini_key.is_none() {
        eprintln!("  info: Gemini insights disabled (set GEMINI_API_KEY to enable)");
        eprintln!("  Connecting to cluster at {}...", agent_addr);
    } else {
        eprintln!("  Connecting to cluster at {} (Claude + Gemini active)...", agent_addr);
    }

    let mut app = App::new(&agent_addr, &anthropic_key, gemini_key.as_deref()).await?;
    app.run().await
}
