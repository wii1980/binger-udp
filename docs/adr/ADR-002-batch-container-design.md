# ADR-002: Batch Container Design

**Status**: Accepted  
**Date**: 2025-06-03  
**Author**: binger-udp maintainer  

## Context

The library needs a batch abstraction that supports:

1. **Compile-time capacity** — stack allocation for small batches (≤64),
   heap fallback for larger batches.
2. **Connectionless and connected modes** — batches can carry per-packet
   destination addresses or omit them.
3. **Zero-alloc hot path** — no allocation during `send_batch` / `recv_batch`.
4. **Reusable** — clear and refill without reallocation.
5. **Deref to raw type** — allow generic code that works with any capacity.

## Decision

Use a **two-layer design** with `const` generics:

```
SendBatch<const N: usize>  ──DerefMut──>  SendBatchRaw
RecvBatch<const N: usize>  ──DerefMut──>  RecvBatchRaw
```

- `SendBatch<N>` / `RecvBatch<N>` are the primary user-facing types.
- They wrap and delegate to the dynamically-sized raw types.
- `BingerUdp::send_batch` / `try_send_batch` accept `&mut SendBatchRaw` —
  so any `SendBatch<N>` works via auto-deref.
- `SendSlot` stores `(data_ptr, data_len, Option<SocketAddr>)` — raw
  pointers avoid copying data into the batch.

## Consequences

- The const generic `N` has zero runtime cost — the wrapping is a single-field
  struct with no overhead.
- Users CAN use `SendBatchRaw` directly if they need dynamic capacity
  (rare) — it's `pub` but documented as "most users should use `SendBatch<N>`".
- `SendSlot` stores raw pointers, which introduces a lifetime contract:
  pushed data must outlive the `send_batch()` call. This is documented on
  `push()` and in examples.
- `RecvSlot` owns its buffer (`Vec<u8>`), so no lifetime issues on the recv
  side.

## Rejected alternatives

### Const-generic only (no raw type)
Would prevent functions from accepting batches of any capacity. Every
`send_batch` call would need to be monomorphized for the specific `N`.

### `Box<[T]>`-based dynamic dispatch
Adds a heap indirection on every access with no benefit over `Vec<T>`.

### `unsafe` internal cursor
Some batch-optimized C libraries advance a cursor on partial send to avoid
re-sending. We chose to return `Ok(sent)` with a partial count and let the
caller retry, which is simpler and composable with async readiness.
