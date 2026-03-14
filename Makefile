.PHONY: all proto agent tui run clean demo-cluster

# ─── Build Everything ───────────────────────────────────────

all: proto agent tui

# ─── Proto Compilation ──────────────────────────────────────

proto:
	@echo "==> Generating Go protobuf code..."
	protoc --go_out=agent/pb --go_opt=paths=source_relative \
	       --go-grpc_out=agent/pb --go-grpc_opt=paths=source_relative \
	       proto/kcp.proto
	@echo "==> Rust proto compilation happens at cargo build time (build.rs)"

# ─── Go Agent ───────────────────────────────────────────────

agent:
	@echo "==> Building Go K8s agent..."
	cd agent && go build -o ../bin/kcp-agent .

# ─── Rust TUI ───────────────────────────────────────────────

tui:
	@echo "==> Building Rust TUI..."
	cd tui && cargo build --release
	cp tui/target/release/kcp-copilot bin/

# ─── Run ────────────────────────────────────────────────────

run: all
	@echo "==> Starting KCP Agent on :50051..."
	./bin/kcp-agent &
	@sleep 1
	@echo "==> Starting KCP Copilot TUI..."
	ANTHROPIC_API_KEY=$${ANTHROPIC_API_KEY} ./bin/kcp-copilot

# ─── Demo Cluster Setup ────────────────────────────────────

demo-cluster:
	@echo "==> Setting up demo workloads for impressive demo..."
	# Healthy deployment
	kubectl create deployment nginx-web --image=nginx:1.25 --replicas=3 2>/dev/null || true
	# Deliberately broken deployment (bad image tag → CrashLoopBackOff)
	kubectl create deployment redis-cache --image=redis:nonexistent-tag --replicas=1 2>/dev/null || true
	# Scalable API
	kubectl create deployment api-gateway --image=nginx:1.25 --replicas=2 2>/dev/null || true
	kubectl expose deployment api-gateway --port=80 --type=ClusterIP 2>/dev/null || true
	# OOMKill candidate (set impossibly low memory limit)
	kubectl run memory-hog --image=nginx:1.25 --restart=Always \
		--overrides='{"spec":{"containers":[{"name":"memory-hog","image":"nginx:1.25","resources":{"limits":{"memory":"4Mi"}}}]}}' \
		2>/dev/null || true
	@echo ""
	@echo "Demo cluster ready! Try these in KCP Copilot:"
	@echo "  • 'what pods are failing?'"
	@echo "  • 'why is redis-cache crashlooping?'"
	@echo "  • 'check the memory-hog pod'"
	@echo "  • 'scale api-gateway to 5 replicas'"

demo-cleanup:
	kubectl delete deployment nginx-web redis-cache api-gateway 2>/dev/null || true
	kubectl delete pod memory-hog 2>/dev/null || true
	kubectl delete service api-gateway 2>/dev/null || true

# ─── Clean ──────────────────────────────────────────────────

clean:
	rm -rf bin/
	cd tui && cargo clean
	cd agent && rm -f kcp-agent
