package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"net"
	"path/filepath"

	container "cloud.google.com/go/container/apiv1"
	pb "github.com/your-org/kcp-copilot/agent/pb"
	"github.com/your-org/kcp-copilot/agent/server"
	"google.golang.org/grpc"
	"google.golang.org/grpc/reflection"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/tools/clientcmd"
	"k8s.io/client-go/util/homedir"
)

func main() {
	port := flag.Int("port", 50051, "gRPC server port")
	kubeconfig := flag.String("kubeconfig", "", "path to kubeconfig (defaults to ~/.kube/config)")
	project := flag.String("project", "", "GCP project ID (required for GKE API)")
	location := flag.String("location", "", "GKE cluster location, e.g. us-central1 (required)")
	cluster := flag.String("cluster", "", "GKE cluster name (required for GKE API)")
	flag.Parse()

	// ─── Build K8s client ──────────────────────────────────
	if *kubeconfig == "" {
		if home := homedir.HomeDir(); home != "" {
			*kubeconfig = filepath.Join(home, ".kube", "config")
		}
	}

	config, err := clientcmd.BuildConfigFromFlags("", *kubeconfig)
	if err != nil {
		log.Fatalf("Failed to build kubeconfig: %v", err)
	}

	clientset, err := kubernetes.NewForConfig(config)
	if err != nil {
		log.Fatalf("Failed to create K8s client: %v", err)
	}

	// ─── Build GKE API client ──────────────────────────────
	ctx := context.Background()
	gkeClient, err := container.NewClusterManagerClient(ctx)
	if err != nil {
		log.Fatalf("Failed to create GKE ClusterManager client: %v", err)
	}
	defer gkeClient.Close()

	if *project == "" || *location == "" || *cluster == "" {
		log.Println("WARNING: --project, --location, and --cluster flags are required for GKE API tools.")
		log.Println("K8s API tools will work, but GKE-specific tools will return errors.")
	}

	clusterPath := fmt.Sprintf("projects/%s/locations/%s/clusters/%s", *project, *location, *cluster)

	// ─── Start gRPC server ─────────────────────────────────
	lis, err := net.Listen("tcp", fmt.Sprintf(":%d", *port))
	if err != nil {
		log.Fatalf("Failed to listen: %v", err)
	}

	grpcServer := grpc.NewServer()
	agentServer := server.NewKcpAgentServer(clientset, gkeClient, clusterPath, *project, *location)
	pb.RegisterKcpAgentServer(grpcServer, agentServer)
	reflection.Register(grpcServer)

	log.Printf("KCP Agent listening on :%d", *port)
	log.Printf("  K8s API:  connected via kubeconfig")
	log.Printf("  GKE API:  cluster=%s", clusterPath)
	if err := grpcServer.Serve(lis); err != nil {
		log.Fatalf("Failed to serve: %v", err)
	}
}
