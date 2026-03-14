# KCP Copilot — AI-Powered GKE Control Plane TUI

> Talk to your GKE cluster in natural language. The first tool that reasons across both the **Kubernetes API** and the **GKE API** simultaneously. Built with **Rust** (TUI) + **Go** (K8s/GKE agent) + **Claude** (interactive reasoning) + **Gemini** (proactive monitoring).

## What It Does

KCP Copilot is a terminal UI that combines two AI models with two Google Cloud APIs to give GKE developers a unified operational view they can't get from kubectl or the Cloud Console alone.

```
┌────────────────────────────────────────────────────────────────────┐
│  KCP Copilot │ Claude ● Connected │ Gemini ●  │ Ctrl+C ↑↓ scroll  │
├──────────────────────────────┬─────────────────────────────────────┤
│  Chat (Claude)               │  Cluster Overview (14:02:31)       │
│                              │                                     │
│  You: is my cluster healthy  │  Nodes                              │
│       and secure?            │  ● gke-node-a  v1.29.4  Ready      │
│                              │  ● gke-node-b  v1.29.4  Ready      │
│  🧠 Analyzing cluster state… │  ● gke-node-c  v1.29.4  Ready      │
│                              │                                     │
│  GKE Cluster: prod-cluster   │  Namespaces                        │
│  Version: 1.29.4-gke.1043002│  ● default    (12/14 running)      │
│  Release Channel: REGULAR    │    └ redis-0: CrashLoopBackOff     │
│  Autopilot: Disabled         │    └ mem-hog: OOMKilled            │
│                              │  ● kube-system (8/8 running)       │
│  Workloads: 20/22 healthy    │                                     │
│  ⚠ redis-0: CrashLoopBackOff│  Recent Warnings                   │
│  ⚠ mem-hog: OOMKilled        │  ⚡ redis-0 ImagePullBackOff       │
│                              │  ⚡ mem-hog OOMKilled               │
│  Security: 2 concerns        │                                     │
│  ⚠ Workload Identity off     │                                     │
│  ⚠ Shielded Nodes disabled   │                                     │
│                              │                                     │
│  Upgrade: v1.30.1 available  │                                     │
│  Next maintenance: Mar 20    │                                     │
│                              │                                     │
├──────────────────────────────┴─────────────────────────────────────┤
│  🔮 Gemini Insights                                                │
│  🔴 redis-0 OOMKilled 3x in 10min — memory limit 256Mi too low    │
│  🟡 api-gateway restarts trending up: 2 in last 5 minutes         │
│  🔵 nginx-web scaled 3→5 — CPU dropped from 78% to 31%            │
├────────────────────────────────────────────────────────────────────┤
│  > _                                                    [Ctrl+C]   │
└────────────────────────────────────────────────────────────────────┘
```

## Architecture

See [architecture.mermaid](architecture.mermaid) for the full diagram.

```
┌─────────────────────────────────────────────────┐
│          Rust TUI (ratatui + tokio)             │
│                                                  │
│  ┌───────────┐  ┌───────────┐  ┌─────────────┐ │
│  │ Chat (L)  │  │ Cluster(R)│  │ Insights(B) │ │
│  └─────┬─────┘  └─────┬─────┘  └──────┬──────┘ │
│        │               │               │        │
│  ┌─────┴───┐     ┌─────┴───────────────┴─────┐  │
│  │ Claude  │     │        Gemini             │  │
│  │ Client  │     │   Background Analyst      │  │
│  │(tool use│     │  (event stream → insights)│  │
│  └────┬────┘     └────────────┬──────────────┘  │
│       └────────┬──────────────┘                  │
│           gRPC │ (tonic)                         │
└────────────────┼─────────────────────────────────┘
                 │
┌────────────────┼─────────────────────────────────┐
│  Go K8s/GKE Agent (client-go + cloud.google.com) │
│                │                                  │
│  ┌─────────────┴──────────────────────────────┐  │
│  │         gRPC Server (grpc-go)              │  │
│  ├────────────────────┬───────────────────────┤  │
│  │  K8s API Tools (7) │  GKE API Tools (6)    │  │
│  │  GetPods           │  GetGKEClusterInfo    │  │
│  │  GetEvents         │  GetNodePools         │  │
│  │  GetLogs           │  GetUpgradeInfo       │  │
│  │  DescribeResource  │  GetMaintenanceWindows│  │
│  │  ScaleDeployment   │  GetSecurityPosture   │  │
│  │  RollbackDeployment│  GetClusterOperations │  │
│  │  GetClusterOverview│                       │  │
│  └─────────┬──────────┴──────────┬────────────┘  │
│       client-go            cloud.google.com/go   │
│            │                      │               │
│     K8s API Server    container.googleapis.com    │
└────────────────────────────────────────────────── ┘
```

## Tech Stack

| Component | Language | Key Libraries |
|-----------|----------|--------------|
| TUI + AI orchestration | **Rust** | `ratatui`, `tonic`, `reqwest`, `tokio`, `serde` |
| K8s + GKE agent | **Go** | `client-go`, `grpc-go`, `cloud.google.com/go/container` |
| Interactive reasoning | **Claude API** | Tool use (13 tools) via Messages API |
| Background monitoring | **Gemini API** | Event stream analysis via GenerateContent |
| IPC | **gRPC** | Shared `.proto` definition |

## Project Structure

```
kcp-copilot/
├── proto/
│   └── kcp.proto                 # Shared gRPC: K8s + GKE + streaming RPCs
├── agent/                        # Go K8s/GKE agent
│   ├── go.mod
│   ├── main.go                   # gRPC server + K8s + GKE client init
│   └── server/
│       ├── server.go             # K8s API tool implementations
│       └── gke.go                # GKE API tool implementations
├── tui/                          # Rust TUI
│   ├── Cargo.toml
│   ├── build.rs                  # Proto compilation (tonic-build)
│   └── src/
│       ├── main.rs               # Entry point
│       ├── app.rs                # App state, event loop, Gemini consumer
│       ├── ui.rs                 # Three-panel layout + Gemini insights
│       ├── claude.rs             # Claude tool-use loop (13 tools)
│       ├── gemini.rs             # Background analyst (event → insight)
│       └── agent.rs              # gRPC client (K8s + GKE methods)
├── architecture.mermaid          # Architecture diagram
├── KCP-Copilot-Spec.docx        # Full spec + demo script
├── Makefile                      # Build + demo orchestration
└── README.md
```

## Quick Start

```bash
# Prerequisites: Go 1.22+, Rust 1.75+, protoc, ANTHROPIC_API_KEY

# 1. Build everything
make all

# 2. Deploy demo workloads (intentionally broken for showcase)
make demo-cluster

# 3. Run (starts Go agent + Rust TUI)
export ANTHROPIC_API_KEY="sk-ant-..."
export GEMINI_API_KEY="..."  # optional, enables proactive insights
make run -- --project=my-project --location=us-central1 --cluster=my-cluster

# 4. Try these:
#   "why is redis failing?"
#   "is my cluster healthy and secure?"
#   "when is the next maintenance window?"
#   "scale api-gateway to 5 replicas"
#   "what operations are running on my cluster?"

# 5. Cleanup
make demo-cleanup
```

## Why This Matters

**For developers**: One terminal replaces kubectl + Cloud Console + memory. Ask questions in English, get answers that combine workload state with platform state.

**For the GKE team**: Demonstrates that the GKE API surface is rich enough to power AI-driven developer tools. Every GKE-specific tool (versions, node pools, security, maintenance, operations) provides data that kubectl simply cannot access.

**As a technical showcase**: Rust and Go working together via gRPC, two AI models with complementary roles, real-time TUI with concurrent async tasks — all in a weekend hack.
