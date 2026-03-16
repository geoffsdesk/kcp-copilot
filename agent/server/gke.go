package server

import (
	"context"
	"fmt"
	"strings"

	containerpb "cloud.google.com/go/container/apiv1/containerpb"
	pb "github.com/your-org/kcp-copilot/agent/pb"
)

// ═══════════════════════════════════════════════════════════
// GKE Control Plane API Implementations
// These call container.googleapis.com — data not available
// via the standard K8s API.
// ═══════════════════════════════════════════════════════════

// ─── GetGKEClusterInfo ─────────────────────────────────────

func (s *KcpAgentServer) GetGKEClusterInfo(ctx context.Context, req *pb.GetGKEClusterInfoRequest) (*pb.GetGKEClusterInfoResponse, error) {
	cluster, err := s.gkeClient.GetCluster(ctx, &containerpb.GetClusterRequest{
		Name: s.clusterPath,
	})
	if err != nil {
		return nil, fmt.Errorf("GKE API: failed to get cluster: %w", err)
	}

	releaseChannel := "UNSPECIFIED"
	if cluster.ReleaseChannel != nil {
		releaseChannel = cluster.ReleaseChannel.Channel.String()
	}

	autopilot := false
	if cluster.Autopilot != nil {
		autopilot = cluster.Autopilot.Enabled
	}

	vpa := false
	if cluster.VerticalPodAutoscaling != nil {
		vpa = cluster.VerticalPodAutoscaling.Enabled
	}

	datapathProvider := "LEGACY"
	if cluster.NetworkConfig != nil {
		datapathProvider = cluster.NetworkConfig.DatapathProvider.String()
	}

	return &pb.GetGKEClusterInfoResponse{
		Name:                    cluster.Name,
		Location:                cluster.Location,
		CurrentMasterVersion:    cluster.CurrentMasterVersion,
		CurrentNodeVersion:      cluster.CurrentNodeVersion,
		ReleaseChannel:          releaseChannel,
		Status:                  cluster.Status.String(),
		Network:                 cluster.Network,
		Subnetwork:              cluster.Subnetwork,
		Endpoint:                cluster.Endpoint,
		AutopilotEnabled:        autopilot,
		VerticalPodAutoscaling:  vpa,
		DatapathProvider:        datapathProvider,
		LoggingService:          cluster.LoggingService,
		MonitoringService:       cluster.MonitoringService,
		TotalNodeCount:          int32(cluster.CurrentNodeCount),
		CreateTime:              cluster.CreateTime,
	}, nil
}

// ─── GetNodePools ──────────────────────────────────────────

func (s *KcpAgentServer) GetNodePools(ctx context.Context, req *pb.GetNodePoolsRequest) (*pb.GetNodePoolsResponse, error) {
	cluster, err := s.gkeClient.GetCluster(ctx, &containerpb.GetClusterRequest{
		Name: s.clusterPath,
	})
	if err != nil {
		return nil, fmt.Errorf("GKE API: failed to get cluster for node pools: %w", err)
	}

	var pools []*pb.NodePoolInfo
	for _, np := range cluster.NodePools {
		info := &pb.NodePoolInfo{
			Name:             np.Name,
			MachineType:      np.Config.MachineType,
			DiskType:         np.Config.DiskType,
			DiskSizeGb:       np.Config.DiskSizeGb,
			ImageType:        np.Config.ImageType,
			InitialNodeCount: np.InitialNodeCount,
			Version:          np.Version,
			Status:           np.Status.String(),
			Locations:        np.Locations,
		}

		if np.Autoscaling != nil {
			info.AutoscalingEnabled = np.Autoscaling.Enabled
			info.AutoscalingMin = np.Autoscaling.MinNodeCount
			info.AutoscalingMax = np.Autoscaling.MaxNodeCount
		}

		if np.Config.Spot {
			info.SpotInstances = true
		}

		// Count current nodes across all instance groups
		info.CurrentNodeCount = int32(len(np.InstanceGroupUrls))

		if np.UpgradeSettings != nil {
			info.MaxSurge = int32(np.UpgradeSettings.MaxSurge)
			info.MaxUnavailable = int32(np.UpgradeSettings.MaxUnavailable)
			if np.UpgradeSettings.Strategy != nil {
				info.UpgradeStrategy = np.UpgradeSettings.Strategy.String()
			}
		}

		pools = append(pools, info)
	}

	return &pb.GetNodePoolsResponse{NodePools: pools}, nil
}

// ─── GetUpgradeInfo ────────────────────────────────────────

func (s *KcpAgentServer) GetUpgradeInfo(ctx context.Context, req *pb.GetUpgradeInfoRequest) (*pb.GetUpgradeInfoResponse, error) {
	cluster, err := s.gkeClient.GetCluster(ctx, &containerpb.GetClusterRequest{
		Name: s.clusterPath,
	})
	if err != nil {
		return nil, fmt.Errorf("GKE API: failed to get cluster: %w", err)
	}

	// Get available server config (versions)
	parentPath := fmt.Sprintf("projects/%s/locations/%s", s.projectID, s.location)
	serverConfig, err := s.gkeClient.GetServerConfig(ctx, &containerpb.GetServerConfigRequest{
		Name: parentPath,
	})
	if err != nil {
		return nil, fmt.Errorf("GKE API: failed to get server config: %w", err)
	}

	resp := &pb.GetUpgradeInfoResponse{
		CurrentVersion: cluster.CurrentMasterVersion,
	}

	if cluster.ReleaseChannel != nil {
		resp.ReleaseChannel = cluster.ReleaseChannel.Channel.String()
	}

	// Find available versions from server config
	for _, v := range serverConfig.ValidMasterVersions {
		isDefault := v == serverConfig.DefaultClusterVersion
		resp.AvailableMasterVersions = append(resp.AvailableMasterVersions, &pb.AvailableVersion{
			Version:   v,
			IsDefault: isDefault,
		})
	}

	for _, v := range serverConfig.ValidNodeVersions {
		resp.AvailableNodeVersions = append(resp.AvailableNodeVersions, &pb.AvailableVersion{
			Version: v,
		})
	}

	// Check for version skew
	if cluster.CurrentMasterVersion != cluster.CurrentNodeVersion {
		resp.VersionSkewWarning = fmt.Sprintf(
			"Master (%s) and node (%s) versions differ. Consider upgrading nodes.",
			cluster.CurrentMasterVersion, cluster.CurrentNodeVersion,
		)
	}

	// Check auto-upgrade on node pools
	autoUpgrade := true
	for _, np := range cluster.NodePools {
		if np.Management == nil || !np.Management.AutoUpgrade {
			autoUpgrade = false
			break
		}
	}
	resp.AutoUpgradeEnabled = autoUpgrade

	// Determine upgrade status
	if len(resp.AvailableMasterVersions) > 0 &&
		resp.AvailableMasterVersions[0].Version != cluster.CurrentMasterVersion {
		resp.UpgradeStatus = "UPGRADE_AVAILABLE"
	} else {
		resp.UpgradeStatus = "UP_TO_DATE"
	}

	return resp, nil
}

// ─── GetMaintenanceWindows ─────────────────────────────────

func (s *KcpAgentServer) GetMaintenanceWindows(ctx context.Context, req *pb.GetMaintenanceWindowsRequest) (*pb.GetMaintenanceWindowsResponse, error) {
	cluster, err := s.gkeClient.GetCluster(ctx, &containerpb.GetClusterRequest{
		Name: s.clusterPath,
	})
	if err != nil {
		return nil, fmt.Errorf("GKE API: failed to get cluster: %w", err)
	}

	resp := &pb.GetMaintenanceWindowsResponse{}

	if cluster.MaintenancePolicy != nil && cluster.MaintenancePolicy.Window != nil {
		w := cluster.MaintenancePolicy.Window

		if w.GetDailyMaintenanceWindow() != nil {
			dmw := w.GetDailyMaintenanceWindow()
			resp.Window = &pb.MaintenanceWindow{
				StartTime: dmw.StartTime,
				EndTime:   dmw.Duration, // DailyMaintenanceWindow has Duration, not EndTime
			}
		}

		if w.GetRecurringWindow() != nil {
			rw := w.GetRecurringWindow()
			resp.Window = &pb.MaintenanceWindow{
				StartTime:  rw.Window.StartTime.AsTime().String(),
				EndTime:    rw.Window.EndTime.AsTime().String(),
				Recurrence: rw.Recurrence,
			}
		}

		for name, excl := range w.MaintenanceExclusions {
			exclusion := &pb.MaintenanceExclusion{
				Name:      name,
				StartTime: excl.StartTime.AsTime().String(),
				EndTime:   excl.EndTime.AsTime().String(),
			}
			resp.Exclusions = append(resp.Exclusions, exclusion)
		}
	} else {
		resp.NextMaintenance = "No maintenance window configured — GKE may perform maintenance at any time."
	}

	return resp, nil
}

// ─── GetSecurityPosture ────────────────────────────────────

func (s *KcpAgentServer) GetSecurityPosture(ctx context.Context, req *pb.GetSecurityPostureRequest) (*pb.GetSecurityPostureResponse, error) {
	cluster, err := s.gkeClient.GetCluster(ctx, &containerpb.GetClusterRequest{
		Name: s.clusterPath,
	})
	if err != nil {
		return nil, fmt.Errorf("GKE API: failed to get cluster: %w", err)
	}

	resp := &pb.GetSecurityPostureResponse{}

	// Workload Identity
	if cluster.WorkloadIdentityConfig != nil && cluster.WorkloadIdentityConfig.WorkloadPool != "" {
		resp.WorkloadIdentityEnabled = true
	}

	// Binary Authorization
	if cluster.BinaryAuthorization != nil {
		resp.BinaryAuthorizationEnabled = cluster.BinaryAuthorization.Enabled
		resp.BinaryAuthEvalMode = cluster.BinaryAuthorization.EvaluationMode.String()
	}

	// Shielded Nodes
	if cluster.ShieldedNodes != nil {
		resp.ShieldedNodesEnabled = cluster.ShieldedNodes.Enabled
	}

	// Network Policy
	if cluster.NetworkPolicy != nil {
		resp.NetworkPolicyEnabled = cluster.NetworkPolicy.Enabled
	}

	// Dataplane V2
	if cluster.NetworkConfig != nil {
		resp.DatapathProvider = cluster.NetworkConfig.DatapathProvider.String()
		if cluster.NetworkConfig.EnableIntraNodeVisibility {
			resp.IntranodeVisibility = true
		}
	}

	// Database encryption (secrets at rest)
	if cluster.DatabaseEncryption != nil && cluster.DatabaseEncryption.State == containerpb.DatabaseEncryption_ENCRYPTED {
		resp.SecretEncryptionEnabled = true
	}

	// Master auth
	if cluster.MasterAuth != nil {
		if cluster.MasterAuth.Username == "" {
			resp.MasterAuthMode = "Certificate/IAM only (password auth disabled)"
		} else {
			resp.MasterAuthMode = "Basic auth enabled (not recommended)"
			resp.SecurityIssues = append(resp.SecurityIssues, "Basic authentication is enabled — consider disabling for production")
		}
	}

	// Flag security concerns
	if !resp.WorkloadIdentityEnabled {
		resp.SecurityIssues = append(resp.SecurityIssues, "Workload Identity is not enabled — pods may use node's service account")
	}
	if !resp.ShieldedNodesEnabled {
		resp.SecurityIssues = append(resp.SecurityIssues, "Shielded GKE Nodes are not enabled")
	}
	if !resp.NetworkPolicyEnabled && !strings.Contains(resp.DatapathProvider, "ADVANCED") {
		resp.SecurityIssues = append(resp.SecurityIssues, "Neither Network Policy nor Dataplane V2 is enabled — no pod-level network isolation")
	}

	return resp, nil
}

// ─── GetClusterOperations ──────────────────────────────────

func (s *KcpAgentServer) GetClusterOperations(ctx context.Context, req *pb.GetClusterOperationsRequest) (*pb.GetClusterOperationsResponse, error) {
	parentPath := fmt.Sprintf("projects/%s/locations/%s", s.projectID, s.location)

	resp, err := s.gkeClient.ListOperations(ctx, &containerpb.ListOperationsRequest{
		Parent: parentPath,
	})
	if err != nil {
		return nil, fmt.Errorf("GKE API: failed to list operations: %w", err)
	}

	var ops []*pb.ClusterOperation
	for _, op := range resp.Operations {
		// Filter to this cluster's operations
		if !strings.Contains(op.TargetLink, s.clusterPath) && op.TargetLink != "" {
			continue
		}

		if req.ActiveOnly && op.Status == containerpb.Operation_DONE {
			continue
		}

		pbOp := &pb.ClusterOperation{
			Name:            op.Name,
			OperationType:   op.OperationType.String(),
			Status:          op.Status.String(),
			Detail:          op.Detail,
			StartTime:       op.StartTime,
			EndTime:         op.EndTime,
			TargetResource:  op.TargetLink,
		}

		if op.Progress != nil {
			for _, metric := range op.Progress.Metrics {
				if metric.Name == "progress" {
					pbOp.ProgressPercent = fmt.Sprintf("%d%%", metric.GetIntValue())
				}
			}
		}

		ops = append(ops, pbOp)
	}

	return &pb.GetClusterOperationsResponse{Operations: ops}, nil
}
