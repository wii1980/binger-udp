# ADR-001: Platform Abstraction Strategy

**Status**: Accepted  
**Date**: 2025-06-03  
**Author**: binger-udp maintainer  

## Context

binger-udp needs to use different system calls on each platform to achieve
optimal batch UDP I/O:

| Platform | Optimal send | Optimal recv |
|----------|-------------|--------------|
| Linux | `sendmmsg` / `sendmsg + GSO` | `recvmmsg` |
| macOS | `sendmsg_x` (private API) | `recvmsg_x` (private API) |
| Windows | `WSASendMsg` | `WSARecvMsg` (extension function) |
| Other | `sendto` (loop) | `recvfrom` (loop) |

## Decision

Use a **compile-time-selected module** via `cfg` attributes in
`src/platform/mod.rs`. Each platform backend exports two public functions:

- `try_send_batch(fd, batch) -> io::Result<usize>`
- `try_recv_batch(fd, batch) -> io::Result<usize>`

Selection priority (first match wins):

1. `target_os = "linux"` && `not(feature = "miri-safe")` → `linux.rs`
2. `target_os = "macos"` && `not(feature = "miri-safe")` → `macos.rs`
3. `target_os = "windows"` && `not(feature = "miri-safe")` → `windows.rs`
4. Otherwise → `fallback.rs` (zero `unsafe`, Miri-compatible)

## Consequences

- Adding a new platform backend is a single new file + one `cfg` arm in `mod.rs`.
- Runtime detection (macOS dlsym, Windows WSAIoctl) is encapsulated within each
  backend; the caller never sees it.
- The `miri-safe` feature forces fallback, guaranteeing Miri-clean runs.
- Backends that support runtime detection also provide a `sendto`/`recvfrom`
  fallback path when the advanced syscall is unavailable.

## Rejected alternatives

### Runtime vtable dispatch
Would require `dyn` pointers, preclude inlining, and add complexity for zero
benefit — the platform is always known at compile time.

### Feature-flag selection per platform
Requires the user to manually select a backend. Automatic detection is reliable
and simpler.
