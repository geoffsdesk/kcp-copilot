//! gRPC client wrapper for the Go K8s + GKE agent.

use anyhow::Result;
use tonic::transport::Channel;

// Generated from proto/kcp.proto by tonic-build
pub mod pb {
    tonic::include_proto!("kcp");
}

use pb::kcp_agent_client::KcpAgentClient;

/// Wrapper around the gRPC client with convenience methods.
#[derive(Clone)]
pub struct AgentClient {
    client: KcpAgentClient<Channel>,
}

/// Simplified cluster overview for the app layer.
pub struct ClusterOverview {
    pub namespaces: Vec<pb::NamespaceSummary>,
    pub nodes: Vec<pb::NodeInfo>,
    pub recent_warnings: Vec<pb::EventInfo>,
}

impl AgentClient {
    pub async fn connect(addr: &str) -> Result<Self> {
        let client = KcpAgentClient::connect(addr.to_string()).await?;
        Ok(Self { client })
    }

    // ═══════════════════════════════════════════════════════
    // Kubernetes API tools
    // ═══════════════════════════════════════════════════════

    pub async fn get_pods(
        &mut self,
        namespace: &str,
        label_selector: &str,
    ) -> Result<String> {
        let resp = self
            .client
            .get_pods(pb::GetPodsRequest {
                namespace: namespace.to_string(),
                label_selector: label_selector.to_string(),
                field_selector: String::new(),
            })
            .await?
            .into_inner();

        let mut output = String::new();
        for pod in &resp.pods {
            let status_icon = match pod.phase.as_str() {
                "Running" => "●",
                "Pending" => "◑",
                _ => "○",
            };
            let extra = if !pod.status_message.is_empty() {
                format!(" ({})", pod.status_message)
            } else {
                String::new()
            };
            output.push_str(&format!(
                "{} {}/{} — {}/{} ready, {} restarts, age {}{}\n",
                status_icon, pod.namespace, pod.name,
                pod.ready_containers, pod.total_containers,
                pod.restarts, pod.age, extra
            ));
        }

        if output.is_empty() {
            output = "No pods found matching criteria.".to_string();
        }

        Ok(output)
    }

    pub async fn get_events(
        &mut self,
        namespace: &str,
        involved_object: &str,
        limit: i32,
    ) -> Result<String> {
        let resp = self
            .client
            .get_events(pb::GetEventsRequest {
                namespace: namespace.to_string(),
                involved_object: involved_object.to_string(),
                limit,
            })
            .await?
            .into_inner();

        let mut output = String::new();
        for event in &resp.events {
            let icon = if event.r#type == "Warning" { "⚠" } else { "ℹ" };
            output.push_str(&format!(
                "{} [{}] {} — {} (x{}, last: {})\n",
                icon, event.reason, event.involved_object,
                event.message, event.count, event.last_seen
            ));
        }

        if output.is_empty() {
            output = "No events found.".to_string();
        }

        Ok(output)
    }

    pub async fn get_logs(
        &mut self,
        namespace: &str,
        pod: &str,
        container: &str,
        tail_lines: i32,
        previous: bool,
    ) -> Result<String> {
        let resp = self
            .client
            .get_logs(pb::GetLogsRequest {
                namespace: namespace.to_string(),
                pod: pod.to_string(),
                container: container.to_string(),
                tail_lines,
                previous,
            })
            .await?
            .into_inner();

        Ok(if resp.logs.is_empty() {
            "No logs available.".to_string()
        } else {
            resp.logs
        })
    }

    pub async fn describe_resource(
        &mut self,
        kind: &str,
        namespace: &str,
        name: &str,
    ) -> Result<String> {
        let resp = self
            .client
            .describe_resource(pb::DescribeResourceRequest {
                kind: kind.to_string(),
                namespace: namespace.to_string(),
                name: name.to_string(),
            })
            .await?
            .into_inner();

        Ok(resp.description)
    }

    pub async fn scale_deployment(
        &mut self,
        namespace: &str,
        name: &str,
        replicas: i32,
    ) -> Result<String> {
        let resp = self
            .client
            .scale_deployment(pb::ScaleDeploymentRequest {
                namespace: namespace.to_string(),
                name: name.to_string(),
                replicas,
            })
            .await?
            .into_inner();

        Ok(resp.message)
    }

    pub async fn rollback_deployment(
        &mut self,
        namespace: &str,
        name: &str,
        revision: i64,
    ) -> Result<String> {
        let resp = self
            .client
            .rollback_deployment(pb::RollbackDeploymentRequest {
                namespace: namespace.to_string(),
                name: name.to_string(),
                revision,
            })
            .await?
            .into_inner();

        Ok(resp.message)
    }

    pub async fn get_cluster_overview(&mut self) -> Result<ClusterOverview> {
        let resp = self
            .client
            .get_cluster_overview(pb::GetClusterOverviewRequest {})
            .await?
            .into_inner();

        Ok(ClusterOverview {
            namespaces: resp.namespaces,
            nodes: resp.nodes,
            recent_warnings: resp.recent_warnings,
        })
    }

    // ═══════════════════════════════════════════════════════
    // GKE Control Plane API tools
    // ═══════════════════════════════════════════════════════

    pub async fn get_gke_cluster_info(&mut self) -> Result<String> {
        let resp = self
            .client
            .get_gke_cluster_info(pb::GetGkeClusterInfoRequest {})
            .await?
            .into_inner();

        Ok(format!(
            "GKE Cluster: {}\n\
             Location: {}\n\
             Status: {}\n\
             Master Version: {}\n\
             Node Version: {}\n\
             Release Channel: {}\n\
             Autopilot: {}\n\
             VPA: {}\n\
             Datapath: {}\n\
             Nodes: {}\n\
             Network: {}/{}\n\
             Logging: {}\n\
             Monitoring: {}\n\
             Created: {}",
            resp.name, resp.location, resp.status,
            resp.current_master_version, resp.current_node_version,
            resp.release_channel,
            if resp.autopilot_enabled { "Enabled" } else { "Disabled" },
            if resp.vertical_pod_autoscaling { "Enabled" } else { "Disabled" },
            resp.datapath_provider,
            resp.total_node_count,
            resp.network, resp.subnetwork,
            resp.logging_service, resp.monitoring_service,
            resp.create_time,
        ))
    }

    pub async fn get_node_pools(&mut self) -> Result<String> {
        let resp = self
            .client
            .get_node_pools(pb::GetNodePoolsRequest {})
            .await?
            .into_inner();

        let mut output = String::new();
        for np in &resp.node_pools {
            let autoscale = if np.autoscaling_enabled {
                format!("autoscale {}-{}", np.autoscaling_min, np.autoscaling_max)
            } else {
                "fixed".to_string()
            };
            let spot = if np.spot_instances { " [Spot]" } else { "" };

            output.push_str(&format!(
                "Pool: {} ({})\n  Machine: {}, Disk: {} {}GB, Image: {}\n  \
                 Nodes: {}, {}{}\n  Version: {}, Status: {}\n  \
                 Upgrade: {} (surge={}, unavail={})\n  Zones: {}\n\n",
                np.name, np.status,
                np.machine_type, np.disk_type, np.disk_size_gb, np.image_type,
                np.current_node_count, autoscale, spot,
                np.version, np.status,
                np.upgrade_strategy, np.max_surge, np.max_unavailable,
                np.locations.join(", "),
            ));
        }

        if output.is_empty() {
            output = "No node pools found.".to_string();
        }
        Ok(output)
    }

    pub async fn get_upgrade_info(&mut self) -> Result<String> {
        let resp = self
            .client
            .get_upgrade_info(pb::GetUpgradeInfoRequest {})
            .await?
            .into_inner();

        let mut output = format!(
            "Current Version: {}\n\
             Release Channel: {}\n\
             Auto-Upgrade: {}\n\
             Status: {}\n",
            resp.current_version, resp.release_channel,
            if resp.auto_upgrade_enabled { "Enabled" } else { "Disabled" },
            resp.upgrade_status,
        );

        if !resp.version_skew_warning.is_empty() {
            output.push_str(&format!("WARNING: {}\n", resp.version_skew_warning));
        }

        if !resp.available_master_versions.is_empty() {
            output.push_str("\nAvailable Master Versions:\n");
            for (i, v) in resp.available_master_versions.iter().take(5).enumerate() {
                let default_marker = if v.is_default { " (default)" } else { "" };
                output.push_str(&format!("  {}. {}{}\n", i + 1, v.version, default_marker));
            }
        }

        Ok(output)
    }

    pub async fn get_maintenance_windows(&mut self) -> Result<String> {
        let resp = self
            .client
            .get_maintenance_windows(pb::GetMaintenanceWindowsRequest {})
            .await?
            .into_inner();

        let mut output = String::new();

        if let Some(w) = &resp.window {
            output.push_str(&format!(
                "Maintenance Window:\n  Start: {}\n  End: {}\n  Recurrence: {}\n",
                w.start_time, w.end_time,
                if w.recurrence.is_empty() { "N/A" } else { &w.recurrence },
            ));
        }

        if !resp.exclusions.is_empty() {
            output.push_str("\nExclusions:\n");
            for excl in &resp.exclusions {
                output.push_str(&format!(
                    "  {} — {} to {} (scope: {})\n",
                    excl.name, excl.start_time, excl.end_time, excl.scope,
                ));
            }
        }

        if !resp.next_maintenance.is_empty() {
            output.push_str(&format!("\n{}\n", resp.next_maintenance));
        }

        if output.is_empty() {
            output = "No maintenance window configured.".to_string();
        }
        Ok(output)
    }

    pub async fn get_security_posture(&mut self) -> Result<String> {
        let resp = self
            .client
            .get_security_posture(pb::GetSecurityPostureRequest {})
            .await?
            .into_inner();

        let check = |enabled: bool| if enabled { "Enabled" } else { "Disabled" };

        let mut output = format!(
            "Security Posture:\n\
             Workload Identity: {}\n\
             Binary Authorization: {} (mode: {})\n\
             Shielded Nodes: {}\n\
             Network Policy: {}\n\
             Datapath: {}\n\
             Intranode Visibility: {}\n\
             Secret Encryption: {}\n\
             Master Auth: {}\n",
            check(resp.workload_identity_enabled),
            check(resp.binary_authorization_enabled), resp.binary_auth_eval_mode,
            check(resp.shielded_nodes_enabled),
            check(resp.network_policy_enabled),
            resp.datapath_provider,
            check(resp.intranode_visibility),
            check(resp.secret_encryption_enabled),
            resp.master_auth_mode,
        );

        if !resp.security_issues.is_empty() {
            output.push_str("\nSecurity Concerns:\n");
            for issue in &resp.security_issues {
                output.push_str(&format!("  ⚠ {}\n", issue));
            }
        }

        Ok(output)
    }

    pub async fn get_cluster_operations(&mut self, active_only: bool) -> Result<String> {
        let resp = self
            .client
            .get_cluster_operations(pb::GetClusterOperationsRequest { active_only })
            .await?
            .into_inner();

        let mut output = String::new();
        for op in &resp.operations {
            output.push_str(&format!(
                "{} — {} [{}]{}\n  Started: {}{}\n",
                op.operation_type, op.status, op.name,
                if !op.progress_percent.is_empty() {
                    format!(" ({})", op.progress_percent)
                } else {
                    String::new()
                },
                op.start_time,
                if !op.end_time.is_empty() {
                    format!(", Ended: {}", op.end_time)
                } else {
                    String::new()
                },
            ));
            if !op.detail.is_empty() {
                output.push_str(&format!("  Detail: {}\n", op.detail));
            }
        }

        if output.is_empty() {
            output = if active_only {
                "No active operations.".to_string()
            } else {
                "No recent operations found.".to_string()
            };
        }
        Ok(output)
    }
}
