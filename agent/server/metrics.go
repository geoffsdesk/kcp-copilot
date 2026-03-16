package server

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"time"

	pb "github.com/your-org/kcp-copilot/agent/pb"
)

// QueryMetrics handles PromQL queries against Prometheus or GMP frontend.
func (s *KcpAgentServer) QueryMetrics(ctx context.Context, req *pb.QueryMetricsRequest) (*pb.QueryMetricsResponse, error) {
	if s.prometheusURL == "" {
		return nil, fmt.Errorf("prometheus URL not configured")
	}

	if req.Query == "" {
		return nil, fmt.Errorf("query is required")
	}

	// Determine if this is a range query or instant query
	isRangeQuery := req.Start != "" && req.End != ""

	var respData prometheusResponse
	var err error

	if isRangeQuery {
		respData, err = s.queryRange(ctx, req)
	} else {
		respData, err = s.queryInstant(ctx, req)
	}

	if err != nil {
		return nil, err
	}

	// Parse the response based on result type
	samples, resultType := s.parsePrometheusResponse(respData)

	// Marshal raw response for reference
	rawJSON, _ := json.Marshal(respData)

	return &pb.QueryMetricsResponse{
		ResultType: resultType,
		Samples:    samples,
		RawJson:    string(rawJSON),
	}, nil
}

// queryInstant performs an instant query against /api/v1/query
func (s *KcpAgentServer) queryInstant(ctx context.Context, req *pb.QueryMetricsRequest) (prometheusResponse, error) {
	params := url.Values{}
	params.Set("query", req.Query)

	if req.Time != "" {
		params.Set("time", req.Time)
	}

	return s.doPrometheusRequest(ctx, "/api/v1/query", params)
}

// queryRange performs a range query against /api/v1/query_range
func (s *KcpAgentServer) queryRange(ctx context.Context, req *pb.QueryMetricsRequest) (prometheusResponse, error) {
	params := url.Values{}
	params.Set("query", req.Query)
	params.Set("start", req.Start)
	params.Set("end", req.End)

	if req.Step == "" {
		params.Set("step", "60s") // default step
	} else {
		params.Set("step", req.Step)
	}

	return s.doPrometheusRequest(ctx, "/api/v1/query_range", params)
}

// doPrometheusRequest makes the HTTP request to Prometheus
func (s *KcpAgentServer) doPrometheusRequest(ctx context.Context, path string, params url.Values) (prometheusResponse, error) {
	fullURL := s.prometheusURL + path + "?" + params.Encode()

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, fullURL, nil)
	if err != nil {
		return prometheusResponse{}, fmt.Errorf("failed to create request: %w", err)
	}

	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return prometheusResponse{}, fmt.Errorf("failed to query prometheus: %w", err)
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return prometheusResponse{}, fmt.Errorf("failed to read response: %w", err)
	}

	if resp.StatusCode != http.StatusOK {
		return prometheusResponse{}, fmt.Errorf("prometheus returned status %d: %s", resp.StatusCode, string(body))
	}

	var result prometheusResponse
	if err := json.Unmarshal(body, &result); err != nil {
		return prometheusResponse{}, fmt.Errorf("failed to parse response: %w", err)
	}

	if result.Status != "success" {
		return prometheusResponse{}, fmt.Errorf("prometheus query failed: %s", result.Error)
	}

	return result, nil
}

// parsePrometheusResponse converts Prometheus response to our protobuf format
func (s *KcpAgentServer) parsePrometheusResponse(respData prometheusResponse) ([]*pb.MetricSample, string) {
	var samples []*pb.MetricSample

	switch respData.Data.ResultType {
	case "vector":
		// Instant query: returns vector of current values
		var results []prometheusMetric
		if err := json.Unmarshal(respData.Data.Result, &results); err == nil {
			for _, result := range results {
				sample := &pb.MetricSample{
					Labels: result.Metric,
					Value:  parseValue(result.Value),
				}
				samples = append(samples, sample)
			}
		}

	case "matrix":
		// Range query: returns time series of values
		var results []prometheusMetric
		if err := json.Unmarshal(respData.Data.Result, &results); err == nil {
			for _, result := range results {
				values, ok := result.Values.([]interface{})
				if !ok {
					continue
				}
				for _, point := range values {
					timestamp, value := parsePoint(point)
					sample := &pb.MetricSample{
						Labels:    result.Metric,
						Value:     value,
						Timestamp: timestamp,
					}
					samples = append(samples, sample)
				}
			}
		}

	case "scalar":
		// Scalar result: single value
		var scalarResult []interface{}
		if err := json.Unmarshal(respData.Data.Result, &scalarResult); err == nil {
			timestamp, value := parsePoint(scalarResult)
			sample := &pb.MetricSample{
				Labels:    map[string]string{},
				Value:     value,
				Timestamp: timestamp,
			}
			samples = append(samples, sample)
		}
	}

	return samples, respData.Data.ResultType
}

// parseValue extracts numeric value from [string, string] tuple
func parseValue(val interface{}) float64 {
	if arr, ok := val.([]interface{}); ok && len(arr) >= 2 {
		if numStr, ok := arr[1].(string); ok {
			var result float64
			fmt.Sscanf(numStr, "%f", &result)
			return result
		}
	}
	return 0.0
}

// parsePoint extracts timestamp and value from [timestamp, value] tuple
func parsePoint(point interface{}) (string, float64) {
	if arr, ok := point.([]interface{}); ok && len(arr) >= 2 {
		var timestamp string
		if ts, ok := arr[0].(float64); ok {
			timestamp = time.Unix(int64(ts), 0).UTC().Format(time.RFC3339)
		}
		value := 0.0
		if valStr, ok := arr[1].(string); ok {
			fmt.Sscanf(valStr, "%f", &value)
		}
		return timestamp, value
	}
	return "", 0.0
}

// Prometheus API response structures

type prometheusResponse struct {
	Status string            `json:"status"`
	Data   prometheusData    `json:"data"`
	Error  string            `json:"error,omitempty"`
	Errors []string          `json:"errors,omitempty"`
}

type prometheusData struct {
	ResultType string            `json:"resultType"`
	Result     json.RawMessage   `json:"result"`
}

type prometheusMetric struct {
	Metric map[string]string `json:"metric"`
	Value  interface{}       `json:"value,omitempty"`
	Values interface{}       `json:"values,omitempty"`
}
