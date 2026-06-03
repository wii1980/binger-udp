# ADR-003: Adaptive Batching

**Status**: Accepted  
**Date**: 2025-06-03  
**Author**: binger-udp maintainer  

## Context

Batch size is a trade-off:

- **Large batches** → fewer syscalls, higher throughput, but higher latency
  if the batch takes time to fill and the socket is congested.
- **Small batches** → lower latency, more syscalls, lower throughput.

A static batch size cannot be optimal under all load conditions. The library
should dynamically adjust.

## Decision

Add an optional **adaptive batching** mode behind `Config::with_adaptive_batching(true)`.

When enabled, an `AdaptiveState` struct tracks:

- Total send attempts since last adjustment.
- `WouldBlock` events (socket buffer full, caller needs to wait).

Every 100 ms, the adapter adjusts:

- **WouldBlock rate > 30%** → halve the batch size (down to minimum 1).
- **WouldBlock rate < 10%** → increase batch size by 50% (up to maximum 1024).

The effective batch size is read via `BingerUdp::recommended_batch_size()`.

## Consequences

- Adaptive batching only adjusts in the async retry loop (`send_batch`,
  `recv_batch`), not in the direct `try_send_batch` / `try_recv_batch` calls.
- Lock contention on the `Mutex<AdaptiveState>` is negligible — adjustments
  happen at most every 100 ms, and the lock is held for microseconds.
- The 30%/10% thresholds and 100 ms interval are hardcoded but sensible for
  most workloads. Future versions may expose them in `Config`.

## Rejected alternatives

### PID controller
More complex to tune, harder to reason about, and unlikely to outperform the
simple threshold approach for this use case.

### ECN-based backpressure
Requires kernel support and socket options not universally available.
WouldBlock is the universal backpressure signal.
