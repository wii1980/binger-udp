# Contributing to binger-udp

Thank you for your interest in contributing! This document provides guidelines
for development, testing, and submitting changes.

## Quick start

```bash
# Build
cargo build

# Run tests (default features: tokio)
cargo test

# Run all tests (all platforms, all features)
cargo test --all-features

# Clippy (must pass — CI enforces it)
cargo clippy --all-features -- -D warnings

# Formatting (CI enforces it)
cargo fmt -- --check

# Documentation
cargo doc --no-deps --all-features --open
```

## Development workflow

1. **Fork and branch** from `main`.
2. **Make your changes** — keep commits atomic and well-described.
3. **Run the full test matrix** before submitting:

   ```bash
   cargo test --all-features
   cargo clippy --all-features -- -D warnings
   cargo fmt -- --check
   cargo build --all-features
   ```

4. **Submit a pull request** against `main`.

> **Note**: CI runs the full matrix (Linux / macOS / Windows, stable Rust, MSRV
> 1.75, clippy, fmt, doc). Make sure your PR passes before requesting review.

## Code style

### General

- Follow `clippy::pedantic` — exceptions are listed in `Cargo.toml` under
  `[lints.clippy]` with safety justifications.
- No `as any`, `@ts-ignore`, or equivalent suppression patterns (this is Rust!).
- Keep `unsafe` confined to platform modules — the public API MUST be 100% safe.
- Every `unsafe` block MUST have a `// SAFETY:` comment explaining the
  invariants that justify it.

### Error handling

- Use `thiserror` for library error types.
- Use `io::Result` for system-call wrappers (not `anyhow` or `Box<dyn Error>`).
- Propagate OS errors transparently via `From<io::Error>`.

### Feature gates

- Platform-specific code MUST be `#[cfg]`-gated.
- Linux-only features (GSO, GRO, pacing, busy-poll, timestamping, pktinfo)
  MUST use `#[cfg(all(target_os = "linux", feature = "..."))]`.
- The `miri-safe` feature MUST disable all platform-optimized backends and use
  only the pure-Rust fallback.
- New feature flags should follow the convention: short, lowercase, kebab-case.

### Testing

- Unit tests live in `#[cfg(test)] mod tests` blocks inside each source file.
- Integration tests live in `tests/` and use `#[tokio::test]` for async tests.
- Benchmarks live in `benches/` and use `criterion`.
- All new public API functions MUST have doc-tests (compile-checked via
  `rustdoc`).
- Aim for: unit tests for edge cases + doc-tests for API examples + integration
  tests for end-to-end I/O.

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add support for SO_TIMESTAMPNS on Linux
fix: handle EINTR in recvmmsg retry loop
docs: add KCP integration example
refactor: extract sockaddr encoding into shared module
```

Do NOT add AI agent co-author or attribution trailers.

## Feature flags

| Flag | Default | Description |
|------|---------|-------------|
| `tokio` | yes | Tokio async runtime integration |
| `gso` | no | Linux Generic Segmentation Offload |
| `gro` | no | Linux Generic Receive Offload |
| `busy-poll` | no | Linux `SO_BUSY_POLL` |
| `pacing` | no | Linux `SO_MAX_PACING_RATE` |
| `metrics` | no | Atomic counter instrumentation |
| `timestamping` | no | Linux `SO_TIMESTAMPNS` |
| `pktinfo` | no | Linux `IP_PKTINFO` / `IPV6_RECVPKTINFO` |
| `miri-safe` | no | Miri-compatible pure-Rust fallback |

## Questions?

Open a [GitHub Discussion](https://github.com/wii1980/binger-udp/discussions)
or file an issue. For architecture decisions, see `docs/adr/`.
