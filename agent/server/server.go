package server

import (
	"context"
	"fmt"
	"sort"
	"strings"
	"time"

	container "cloud.google.com/go/container/apiv1"
	pb "github.com/your-org/kcp-copilot/agent/pb"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
)

type KcpAgentServer struct {
	pb.UnimplementedKcpAgentServer
	clientset   *kubernetes.Clientset
	gkeClient   *container.ClusterManagerClient
	clusterPath string // projects/{project}/locations/{location}/clusters/{cluster}
	projectID   string
	location    string
}

func NewKcpAgentServer(
	clientset *kubernetes.Clientset,
	gkeClient *container.ClusterManagerClient,
	clusterPath, projectID, location string,
) *KcpAgentServer {
	return &KcpAgentServer{
		clientset:   clientset,
		gkeClient:   gkeClient,
		clusterPath: clusterPath,
		projectID:   projectID,
		location:    location,
	}
}

// ═══════════════════════════════════════════════════════════
// Kubernetes API Implementations
// ═══════════════════════════════════════════════════════════

// ─── GetPods ───────────────────────────────────────────────

func (s *KcpAgentServer) GetPods(ctx context.Context, req *pb.GetPodsRequest) (*pb.GetPodsResponse, error) {
	ns := req.Namespace

	opts := metav1.ListOptions{
		LabelSelector: req.LabelSelector,
		FieldSelector: req.FieldSelector,
	}

	pods, err := s.clientset.CoreV1().Pods(ns).List(ctx, opts)
	if err != nil {
		return nil, fmt.Errorf("failed to list pods: %w", err)
	}

	var result []*pb.PodInfo
	for _, pod := range pods.Items {
		ready, total := containerReadiness(pod.Status.ContainerStatuses)
		restarts := totalRestarts(pod.Status.ContainerStatuses)

		result = append(result, &pb.PodInfo{
			Name:            pod.Name,
			Namespace:       pod.Namespace,
			Phase:           string(pod.Status.Phase),
			ReadyContainers: int32(ready),
			TotalContainers: int32(total),
			Restarts:        int32(restarts),
			Age:             humanDuration(time.Since(pod.CreationTimestamp.Time)),
			Node:            pod.Spec.NodeName,
			StatusMessage:   podStatusReason(pod),
		})
	}

	return &pb.GetPodsResponse{Pods: result}, nil
}

// ─── GetEvents ─────────────────────────────────────────────

func (s *KcpAgentServer) GetEvents(ctx context.Context, req *pb.GetEventsRequest) (*pb.GetEventsResponse, error) {
	ns := req.Namespace
	opts := metav1.ListOptions{}

	if req.InvolvedObject != "" {
		parts := strings.SplitN(req.InvolvedObject, "/", 2)
		if len(parts) == 2 {
			opts.FieldSelector = fmt.Sprintf("involvedObject.kind=%s,involvedObject.name=%s", parts[0], parts[1])
		}
	}

	events, err := s.clientset.CoreV1().Events(ns).List(ctx, opts)
	if err != nil {
		return nil, fmt.Errorf("failed to list events: %w", err)
	}

	sort.Slice(events.Items, func(i, j int) bool {
		return events.Items[i].LastTimestamp.After(events.Items[j].LastTimestamp.Time)
	})

	limit := int(req.Limit)
	if limit <= 0 {
		limit = 20
	}
	if limit > len(events.Items) {
		limit = len(events.Items)
	}

	var result []*pb.EventInfo
	for _, e := range events.Items[:limit] {
		result = append(result, &pb.EventInfo{
			Type:           e.Type,
			Reason:         e.Reason,
			Message:        e.Message,
			InvolvedObject: fmt.Sprintf("%s/%s", e.InvolvedObject.Kind, e.InvolvedObject.Name),
			FirstSeen:      e.FirstTimestamp.Format(time.RFC3339),
			LastSeen:       e.LastTimestamp.Format(time.RFC3339),
			Count:          e.Count,
		})
	}

	return &pb.GetEventsResponse{Events: result}, nil
}

// ─── GetLogs ───────────────────────────────────────────────

func (s *KcpAgentServer) GetLogs(ctx context.Context, req *pb.GetLogsRequest) (*pb.GetLogsResponse, error) {
	tailLines := int64(req.TailLines)
	if tailLines <= 0 {
		tailLines = 50
	}

	opts := &corev1.PodLogOptions{
		Container: req.Container,
		TailLines: &tailLines,
		Previous:  req.Previous,
	}

	result := s.clientset.CoreV1().Pods(req.Namespace).GetLogs(req.Pod, opts)
	logs, err := result.Do(ctx).Raw()
	if err != nil {
		return nil, fmt.Errorf("failed to get logs for %s/%s: %w", req.Namespace, req.Pod, err)
	}

	return &pb.GetLogsResponse{Logs: string(logs)}, nil
}

// ─── ScaleDeployment ───────────────────────────────────────

func (s *KcpAgentServer) ScaleDeployment(ctx context.Context, req *pb.ScaleDeploymentRequest) (*pb.ScaleDeploymentResponse, error) {
	deploy, err := s.clientset.AppsV1().Deployments(req.Namespace).Get(ctx, req.Name, metav1.GetOptions{})
	if err != nil {
		return nil, fmt.Errorf("failed to get deployment %s/%s: %w", req.Namespace, req.Name, err)
	}

	previousReplicas := *deploy.Spec.Replicas
	replicas := req.Replicas
	deploy.Spec.Replicas = &replicas

	_, err = s.clientset.AppsV1().Deployments(req.Namespace).Update(ctx, deploy, metav1.UpdateOptions{})
	if err != nil {
		return nil, fmt.Errorf("failed to scale deployment: %w", err)
	}

	return &pb.ScaleDeploymentResponse{
		Success:          true,
		PreviousReplicas: previousReplicas,
		NewReplicas:      replicas,
		Message:          fmt.Sprintf("Scaled %s from %d to %d replicas", req.Name, previousReplicas, replicas),
	}, nil
}

// ─── GetClusterOverview ────────────────────────────────────

func (s *KcpAgentServer) GetClusterOverview(ctx context.Context, req *pb.GetClusterOverviewRequest) (*pb.GetClusterOverviewResponse, error) {
	pods, err := s.clientset.CoreV1().Pods("").List(ctx, metav1.ListOptions{})
	if err != nil {
		return nil, fmt.Errorf("failed to list pods: %w", err)
	}

	nsMap := make(map[string]*pb.NamespaceSummary)
	for _, pod := range pods.Items {
		ns := pod.Namespace
		if _, ok := nsMap[ns]; !ok {
			nsMap[ns] = &pb.NamespaceSummary{Namespace: ns}
		}
		summary := nsMap[ns]
		summary.TotalPods++

		switch pod.Status.Phase {
		case corev1.PodRunning:
			summary.RunningPods++
		case corev1.PodFailed:
			summary.FailedPods++
			summary.Issues = append(summary.Issues, fmt.Sprintf("%s: %s", pod.Name, podStatusReason(pod)))
		case corev1.PodPending:
			summary.PendingPods++
		}

		for _, cs := range pod.Status.ContainerStatuses {
			if cs.State.Waiting != nil && cs.State.Waiting.Reason == "CrashLoopBackOff" {
				summary.Issues = append(summary.Issues,
					fmt.Sprintf("%s: CrashLoopBackOff (%d restarts)", pod.Name, cs.RestartCount))
			}
		}
	}

	var namespaces []*pb.NamespaceSummary
	for _, ns := range nsMap {
		namespaces = append(namespaces, ns)
	}

	nodes, err := s.clientset.CoreV1().Nodes().List(ctx, metav1.ListOptions{})
	if err != nil {
		return nil, fmt.Errorf("failed to list nodes: %w", err)
	}

	var nodeInfos []*pb.NodeInfo
	for _, node := range nodes.Items {
		ready := false
		for _, cond := range node.Status.Conditions {
			if cond.Type == corev1.NodeReady && cond.Status == corev1.ConditionTrue {
				ready = true
			}
		}
		nodeInfos = append(nodeInfos, &pb.NodeInfo{
			Name:    node.Name,
			Ready:   ready,
			Version: node.Status.NodeInfo.KubeletVersion,
		})
	}

	events, err := s.clientset.CoreV1().Events("").List(ctx, metav1.ListOptions{
		FieldSelector: "type=Warning",
	})
	if err != nil {
		return nil, fmt.Errorf("failed to list events: %w", err)
	}

	sort.Slice(events.Items, func(i, j int) bool {
		return events.Items[i].LastTimestamp.After(events.Items[j].LastTimestamp.Time)
	})

	limit := 5
	if limit > len(events.Items) {
		limit = len(events.Items)
	}
	var warnings []*pb.EventInfo
	for _, e := range events.Items[:limit] {
		warnings = append(warnings, &pb.EventInfo{
			Type:           e.Type,
			Reason:         e.Reason,
			Message:        e.Message,
			InvolvedObject: fmt.Sprintf("%s/%s", e.InvolvedObject.Kind, e.InvolvedObject.Name),
			LastSeen:       e.LastTimestamp.Format(time.RFC3339),
			Count:          e.Count,
		})
	}

	return &pb.GetClusterOverviewResponse{
		Namespaces:     namespaces,
		Nodes:          nodeInfos,
		RecentWarnings: warnings,
	}, nil
}

// ═══════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════

func containerReadiness(statuses []corev1.ContainerStatus) (int, int) {
	ready, total := 0, len(statuses)
	for _, cs := range statuses {
		if cs.Ready {
			ready++
		}
	}
	return ready, total
}

func totalRestarts(statuses []corev1.ContainerStatus) int {
	total := 0
	for _, cs := range statuses {
		total += int(cs.RestartCount)
	}
	return total
}

func podStatusReason(pod corev1.Pod) string {
	for _, cs := range pod.Status.ContainerStatuses {
		if cs.State.Waiting != nil && cs.State.Waiting.Reason != "" {
			return cs.State.Waiting.Reason
		}
		if cs.State.Terminated != nil && cs.State.Terminated.Reason != "" {
			return cs.State.Terminated.Reason
		}
	}
	return ""
}

func humanDuration(d time.Duration) string {
	if d < time.Minute {
		return fmt.Sprintf("%ds", int(d.Seconds()))
	}
	if d < time.Hour {
		return fmt.Sprintf("%dm", int(d.Minutes()))
	}
	if d < 24*time.Hour {
		return fmt.Sprintf("%dh%dm", int(d.Hours()), int(d.Minutes())%60)
	}
	days := int(d.Hours()) / 24
	hours := int(d.Hours()) % 24
	return fmt.Sprintf("%dd%dh", days, hours)
}
