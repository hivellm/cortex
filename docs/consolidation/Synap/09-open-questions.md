# Synap — Open Questions

## Operational

1. **Cluster Mode**: Phase 4 planned but not yet implemented. Current model is master-replica only (no peer-to-peer cluster sharding).
   
2. **GUI Dashboard**: Phase 4 planned. Current monitoring via REST endpoints + Prometheus only.

3. **Helm Charts**: Kubernetes deployment templates exist but coverage/maturity unclear. Verify against production HiveLLM deployments.

## Performance

4. **Stream Publish Latency**: Target < 1ms not yet publicly benchmarked. Current benchmarks show 2.3 GiB/s throughput; latency p99 unclear.

5. **SIMD Optimization Scope**: BITCOUNT, BITOP, PFMERGE have SIMD acceleration. What about other hotspots (SET, GET under contention)?

## Cortex Integration

6. **Consumer Group Rebalancing**: Synap supports Kafka-style consumer groups. Does Cortex need sticky assignment or will range assignment suffice?

7. **Backpressure Handling**: If ingest pipeline publishes faster than workers can consume, how does Synap queue backpressure propagate? Tested?

## Security

8. **Audit Log Retention**: Audit logging is implemented. What is the retention policy? Does it persist across server restarts?

9. **RBAC Granularity**: Fine-grained permissions exist. Should Cortex define a standard permission profile (e.g., read-only ingest → read all streams)?

## Compatibility

10. **RESP3 Coverage**: Not all Synap commands map to RESP3. Document which commands are unavailable on `:6379` vs synap://.

11. **SDK Parity**: TypeScript, Python, Rust SDKs all support synap://, resp3://, http://. Are breaking changes for transport selection ever planned?

## Monitoring

12. **Alerting**: Prometheus metrics exist. Are predefined alert rules (replication lag > 10s, queue depth > threshold) provided?

## Deployment

13. **Multi-Region**: Master-replica replication is single-region. How to handle cross-region failover or read replicas in another region (network latency)?
