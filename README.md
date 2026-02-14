# wasm_safe_thread


![logo](art/logo.png)

A `std::thread` + `std::sync` replacement for wasm32 with proper async integration.

This crate provides a unified threading API and synchronization primitives that work across both WebAssembly and native platforms. In practice, you can treat it as a cross-platform replacement for much of `std::thread` plus key `std::sync` primitives.

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

Replace `use std::thread` (and relevant `std::sync` primitives) with `wasm_safe_thread` APIs:

```rust
use wasm_safe_thread as thread;

// Spawn a thread
let handle = thread::spawn(|| {
    println!("Hello from a worker!");
    42
});

// Wait for the thread to complete
let result = handle.join().unwrap();
assert_eq!(result, 42);
```

## API

### Thread spawning

```rust
use wasm_safe_thread::{spawn, spawn_named, Builder};

// Simple spawn
let handle = spawn(|| "result");

// Convenience function for named threads
let handle = spawn_named("my-worker", || "result").unwrap();

// Builder pattern for more options
let handle = Builder::new()
    .name("my-worker".to_string())
    .spawn(|| "result")
    .unwrap();
```

### Joining threads

```rust
use wasm_safe_thread::spawn;

// Synchronous join (works on native and wasm worker threads)
let handle = spawn(|| 42);
let result = handle.join().unwrap();
assert_eq!(result, 42);

// Non-blocking check
let handle = spawn(|| 42);
if handle.is_finished() {
    // Thread completed
}
```

For async contexts, use `join_async`:

```rust
// In an async context (e.g., with wasm_bindgen_futures::spawn_local)
let result = handle.join_async().await.unwrap();
```

### Thread operations

```rust
use wasm_safe_thread::{current, sleep, yield_now};
use std::time::Duration;

// Get current thread
let thread = current();
println!("Thread: {:?}", thread.name());

// Sleep
sleep(Duration::from_millis(10));

// Yield to scheduler
yield_now();
```

Park/unpark works from background threads:

```rust
use wasm_safe_thread::{spawn, park, park_timeout};
use std::time::Duration;

let handle = spawn(|| {
    // Park/unpark (from background threads)
    park_timeout(Duration::from_millis(10)); // Wait with timeout
});
handle.thread().unpark();  // Wake parked thread
handle.join().unwrap();  // join() requires worker context on wasm
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

### Async task tracking (WASM)

When spawning async tasks inside a worker thread using `wasm_bindgen_futures::spawn_local`,
you must notify the runtime so the worker waits for tasks to complete before exiting:

```rust
use wasm_safe_thread::{task_begin, task_finished};

task_begin();
wasm_bindgen_futures::spawn_local(async {
    // ... async work ...
    task_finished();
});
```

These functions are no-ops on native platforms, so you can use them unconditionally
in cross-platform code.

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
