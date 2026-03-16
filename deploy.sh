#!/bin/bash
set -euo pipefail

# ═══════════════════════════════════════════════════════════════
# KCP Copilot — Full Deployment to GKE
# Project: first-cascade-490202-e3
# Cluster: sympozium-cluster (us-central1-a)
# ═══════════════════════════════════════════════════════════════

PROJECT_ID="first-cascade-490202-e3"
REGION="us-central1"
ZONE="us-central1-a"
CLUSTER="sympozium-cluster"
REGISTRY="us-docker.pkg.dev/${PROJECT_ID}/sympozium"
NAMESPACE="sympozium-showcase"

echo "═══════════════════════════════════════════════════════════"
echo " KCP Copilot Deployment"
echo "═══════════════════════════════════════════════════════════"

# ─── Step 1: Enable GKE control plane metrics ─────────────────
echo ""
echo "▶ Step 1: Enabling GKE control plane metrics..."
gcloud container clusters update ${CLUSTER} \
  --zone=${ZONE} \
  --project=${PROJECT_ID} \
  --monitoring=SYSTEM,API_SERVER,SCHEDULER,CONTROLLER_MANAGER

# ─── Step 2: Create GCP service account for Workload Identity ─
echo ""
echo "▶ Step 2: Setting up Workload Identity for kcp-agent..."

# Create GCP SA (ignore error if exists)
gcloud iam service-accounts create kcp-agent \
  --display-name="KCP Copilot Agent" \
  --project=${PROJECT_ID} 2>/dev/null || echo "  SA already exists"

# Grant monitoring viewer (for GMP queries)
gcloud projects add-iam-policy-binding ${PROJECT_ID} \
  --member="serviceAccount:kcp-agent@${PROJECT_ID}.iam.gserviceaccount.com" \
  --role="roles/monitoring.viewer" \
  --condition=None --quiet

# Grant container viewer (for GKE API)
gcloud projects add-iam-policy-binding ${PROJECT_ID} \
  --member="serviceAccount:kcp-agent@${PROJECT_ID}.iam.gserviceaccount.com" \
  --role="roles/container.viewer" \
  --condition=None --quiet

# Bind KSA → GSA for Workload Identity
gcloud iam service-accounts add-iam-policy-binding \
  kcp-agent@${PROJECT_ID}.iam.gserviceaccount.com \
  --role="roles/iam.workloadIdentityUser" \
  --member="serviceAccount:${PROJECT_ID}.svc.id.goog[${NAMESPACE}/kcp-agent]" \
  --project=${PROJECT_ID} --quiet

echo "  ✓ Workload Identity configured"

# ─── Step 3: Build and push container image ────────────────────
echo ""
echo "▶ Step 3: Building kcp-agent container image..."
cd "$(dirname "$0")"

gcloud builds submit \
  --project=${PROJECT_ID} \
  --tag="${REGISTRY}/kcp-agent:latest" \
  --dockerfile=Dockerfile.agent \
  .

echo "  ✓ Image pushed to ${REGISTRY}/kcp-agent:latest"

# ─── Step 4: Deploy Grafana control plane dashboard ───────────
echo ""
echo "▶ Step 4: Deploying GKE control plane Grafana dashboard..."

# Get cluster credentials
gcloud container clusters get-credentials ${CLUSTER} \
  --zone=${ZONE} --project=${PROJECT_ID}

# The existing grafana-dashboards ConfigMap holds all dashboards.
# Recreate it with BOTH the original overview + new control plane dashboard.
DASHBOARD_DIR="../sympozium-gcp/config/observability"
kubectl create configmap grafana-dashboards \
  --from-file=sympozium-overview.json=${DASHBOARD_DIR}/grafana-dashboard.json \
  --from-file=gke-control-plane.json=${DASHBOARD_DIR}/gke-control-plane-dashboard.json \
  -n ${NAMESPACE} \
  --dry-run=client -o yaml | kubectl apply -f -

echo "  ✓ Dashboard ConfigMap updated (overview + control plane)"

# Restart Grafana to pick up the new dashboard
kubectl rollout restart deployment/grafana -n ${NAMESPACE}
kubectl rollout status deployment/grafana -n ${NAMESPACE} --timeout=60s
echo "  ✓ Grafana restarted"

# ─── Step 5: Deploy KCP Agent ─────────────────────────────────
echo ""
echo "▶ Step 5: Deploying KCP Agent to GKE..."
kubectl apply -f k8s/kcp-agent.yaml

echo ""
echo "▶ Waiting for rollout..."
kubectl rollout status deployment/kcp-agent -n ${NAMESPACE} --timeout=120s

# ─── Step 6: Verify ──────────────────────────────────────────
echo ""
echo "▶ Step 6: Verifying deployment..."
kubectl get pods -n ${NAMESPACE} -l app=kcp-agent
echo ""
kubectl logs -n ${NAMESPACE} -l app=kcp-agent --tail=10

echo ""
echo "═══════════════════════════════════════════════════════════"
echo " ✓ KCP Copilot deployed successfully!"
echo ""
echo " To connect the TUI locally:"
echo "   kubectl port-forward svc/kcp-agent 50051:50051 -n ${NAMESPACE}"
echo "   cd tui && cargo run"
echo ""
echo " To query metrics via grpcurl:"
echo "   kubectl port-forward svc/kcp-agent 50051:50051 -n ${NAMESPACE}"
echo "   grpcurl -plaintext localhost:50051 kcp.KcpAgent/GetClusterOverview"
echo "═══════════════════════════════════════════════════════════"
