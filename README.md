# wasm_safe_thread


![logo](art/logo.png)

A `std::thread` replacement for wasm32 with proper async integration.

This crate provides a unified threading API that works across both WebAssembly and native platforms. Unlike similar crates, it's designed from the ground up to handle the async realities of browser environments.

## Comparison with wasm_thread

[wasm_thread](https://crates.io/crates/wasm_thread) is a popular crate that aims to closely replicate `std::thread` on wasm targets. This section compares design goals and practical tradeoffs.

### Design goals

- `wasm_safe_thread`: async-first, unified API that works identically on native and wasm32, playing well with the browser event loop.
- `wasm_thread`: high `std::thread` compatibility with minimal changes to existing codebases (wasm32 only; native uses `std::thread` directly).

### Feature comparison

| Feature | wasm_safe_thread | wasm_thread |
|---------|------------------|-------------|
| **Native support** | Unified API (same code runs on native and wasm) | Re-exports `std::thread::*` on native |
| **Node.js support** | Yes, via `worker_threads` | Browser only |
| **Event loop integration** | `yield_to_event_loop_async()` for cooperative scheduling | No equivalent |
| **Spawn hooks** | Global hooks that run at thread start | Not available |
| **Parking primitives** | `park()`/`unpark()` on wasm workers | Not implemented |
| **Scoped threads** | Not implemented | `scope()` allows borrowing non-`'static` data |
| **std compatibility** | Custom `Thread`/`ThreadId` (similar API) | Re-exports `std::thread::{Thread, ThreadId}` |
| **Worker scripts** | Inline JS via `wasm_bindgen(inline_js)` | External JS files; `es_modules` feature for module workers |
| **wasm-pack targets** | ES modules (`web`) only | `web` and `no-modules` via feature flag |
| **Dependencies** | wasm-bindgen, js-sys, wasm_safe_mutex | web-sys (many features), futures crate |
| **Thread handle** | `thread()` returns `&Thread` | `thread()` is unimplemented (panics) |

### Shared capabilities

Both crates provide:
- `spawn()` and `Builder` for thread creation
- `join()` (blocking) and `join_async()` (async) for waiting on threads
- `is_finished()` for non-blocking completion checks
- Thread naming via `Builder::name()`

### Behavioral differences to know

- **Main-thread blocking:** both crates must avoid blocking APIs on the browser main thread; `join_async()` is the safe path.
- **Spawn timing:** wasm workers only run after the main thread yields back to the event loop.
- **Worker spawning model:** `wasm_thread` proxies worker spawning through the main thread; `wasm_safe_thread` spawns directly (simpler, but different model).

### Implementation differences (for maintainers)

**Result passing:**
- `wasm_safe_thread` uses `wasm_safe_mutex::mpsc` channels with async `recv_async()`
- `wasm_thread` uses `Arc<Packet<UnsafeCell>>` with a custom `Signal` primitive and `Waker` list

**Async waiting:**
- `wasm_safe_thread` wraps JavaScript Promises via `wasm-bindgen-futures::JsFuture`
- `wasm_thread` implements `futures::future::poll_fn` with manual `Waker` tracking

### When to use which

**Choose wasm_safe_thread when:**
- You need Node.js support (wasm_thread is browser-only)
- You want identical behavior on native and wasm (e.g., for testing)
- You need park/unpark synchronization primitives
- You need spawn hooks for initialization (logging, tracing, etc.)
- You prefer fewer dependencies and no external JS files
- You want an actively developed library with responsive issue/PR handling

**Choose wasm_thread when:**
- You need scoped threads for borrowing non-`'static` data
- You want maximum compatibility with `std::thread` types
- You need `no-modules` wasm-pack target support

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
wasm_safe_thread = "0.1"
```

Replace `use std::thread` with `use wasm_safe_thread as thread`:

```rust
use wasm_safe_thread as thread;

// Spawn a thread
let handle = thread::spawn(|| {
    println!("Hello from a worker!");
    42
});

// In async context, use join_async (works on both native and wasm)
let result = handle.join_async().await.unwrap();
assert_eq!(result, 42);
```

## API

### Thread spawning

```rust
use wasm_safe_thread::{spawn, Builder};

// Simple spawn
let handle = spawn(|| "result");

// Named thread with builder
let handle = Builder::new()
    .name("my-worker".to_string())
    .spawn(|| "result")
    .unwrap();
```

### Joining threads

```rust
// From async context (recommended, works everywhere)
let result = handle.join_async().await.unwrap();

// From background thread only (panics on wasm main thread)
let result = handle.join().unwrap();

// Non-blocking check
if handle.is_finished() {
    // Thread completed
}
```

### Thread operations

```rust
use wasm_safe_thread::{current, sleep, yield_now, park, park_timeout};
use std::time::Duration;

// Get current thread
let thread = current();
println!("Thread: {:?}", thread.name());

// Sleep
sleep(Duration::from_millis(100));

// Yield to scheduler
yield_now();

// Park/unpark (from background threads)
park();                                  // Wait for unpark
park_timeout(Duration::from_millis(100)); // Wait with timeout
thread.unpark();                         // Wake parked thread
```

### Event loop integration

```rust
use wasm_safe_thread::yield_to_event_loop_async;

// Yield to browser event loop (works on native too)
yield_to_event_loop_async().await;
```

### Thread local storage

```rust
use wasm_safe_thread::thread_local;
use std::cell::RefCell;

thread_local! {
    static COUNTER: RefCell<u32> = RefCell::new(0);
}

COUNTER.with(|c| {
    *c.borrow_mut() += 1;
});
```

### Spawn hooks

Register callbacks that run when any thread starts:

```rust
use wasm_safe_thread::{register_spawn_hook, remove_spawn_hook, clear_spawn_hooks};

// Register a hook
register_spawn_hook("my-hook", || {
    println!("Thread starting!");
});

// Hooks run in registration order, before the thread's main function

// Remove specific hook
remove_spawn_hook("my-hook");

// Clear all hooks
clear_spawn_hooks();
```

## WASM Limitations

### Main thread restrictions

The browser main thread cannot use blocking APIs:

- `join()` - Use `join_async().await` instead
- `park()` / `park_timeout()` - Only works from background threads
- `Mutex::lock()` from std - Use `wasm_safe_mutex` instead

### SharedArrayBuffer requirements

Threading requires `SharedArrayBuffer`, which needs these HTTP headers:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

See [Mozilla's documentation](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer) for details.

### Environment support

- **Browser**: Web Workers with shared memory
- **Node.js**: worker_threads module

## Building for WASM

Standard library must be rebuilt with atomics support:

```bash
# Install nightly and components
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly

# Build with atomics
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory' \
cargo +nightly build -Z build-std=std,panic_abort \
    --target wasm32-unknown-unknown
```

## License

Licensed under either of

 * Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
