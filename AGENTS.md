# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
scripts/check_all          # Run all checks: fmt, check, clippy, tests, docs
scripts/tests              # Run tests (native + wasm32)
scripts/check              # Compile check (native + wasm32)
scripts/clippy             # Clippy (native + wasm32)
scripts/docs               # Build docs (native + wasm32)
scripts/fmt                # Check formatting

scripts/native/tests       # Native tests only
scripts/wasm32/tests       # WASM tests only (requires nightly)
```

Platform-specific scripts are in `scripts/native/` and `scripts/wasm32/`. The wasm32 scripts source `scripts/wasm32/_env` which sets up required RUSTFLAGS from `.cargo/config.toml`.

## Architecture

This crate provides a unified `std::thread`-like API that works on both native and WASM platforms. The core abstraction is a backend module pattern:

- `src/lib.rs` - Public API that delegates to the active backend
- `src/stdlib.rs` - Native backend (wraps `std::thread`)
- `src/wasm.rs` - WASM backend (Web Workers via `wasm-bindgen`)
- `src/wasm/wasm_utils.rs` - Low-level JS utilities (atomics, parking, environment detection)
- `src/hooks.rs` - Global spawn hook registry (runs callbacks at thread start)
- `src/test_executor.rs` - Test infrastructure with `async_test!` macro

Backend selection at compile time:
```rust
#[cfg(not(target_arch = "wasm32"))]
use stdlib as backend;
#[cfg(target_arch = "wasm32")]
use wasm as backend;
```

## Key Types

Both backends implement the same types with matching APIs:
- `Thread`, `ThreadId` - Thread handle and identifier
- `JoinHandle<T>` - Handle for joining spawned threads
- `Builder` - Thread configuration builder
- `LocalKey<T>` - Thread-local storage key

## WASM Implementation Details

The WASM backend spawns Web Workers with inline JavaScript (via `wasm_bindgen(inline_js)`). Key mechanisms:

- **Worker spawning**: `spawn_with_shared_module()` creates workers that share the WASM module and memory
- **Result passing**: Uses `wasm_safe_mutex::mpsc` channels for cross-thread communication
- **Exit coordination**: `exit_state` is a reference-counted atomic `[exit_code, ref_count]` that tracks worker completion
- **Panic handling**: Custom panic hook sends error through channel before abort
- **Async task tracking**: `task_begin()`/`task_finished()` track pending `spawn_local` tasks; worker waits for all to complete

Main thread restrictions (browser):
- `join()` panics - use `join_async()` instead
- `park()`/`park_timeout()` unavailable - `Atomics.wait` doesn't work on main thread

## Testing

Use the `async_test!` macro for cross-platform async tests:
```rust
crate::async_test! {
    async fn my_test() {
        let handle = spawn(|| 42);
        let result = handle.join_async().await.unwrap();
        assert_eq!(result, 42);
    }
}
```

This expands to:
- Native: `#[test]` with `test_executor::spawn()` polling loop
- WASM: `#[wasm_bindgen_test]` async test

Tests that need synchronous `join()` on WASM must run in a worker context (not main thread).
