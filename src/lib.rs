//! A `std::thread` replacement for wasm32 with proper async integration.
//!
//! This crate provides a unified threading API that works across both WebAssembly and native platforms.
//! Unlike similar crates, it's designed from the ground up to handle the async realities of browser
//! environments.
//!
//! # Comparison with wasm_thread
//!
//! [wasm_thread](https://crates.io/crates/wasm_thread) is a popular crate that aims to closely
//! replicate `std::thread` on wasm targets. This section compares design goals and practical tradeoffs.
//!
//! ## Design goals
//!
//! - `wasm_safe_thread`: async-first, unified API that works identically on native and wasm32,
//!   playing well with the browser event loop.
//! - `wasm_thread`: high `std::thread` compatibility with minimal changes to existing codebases
//!   (wasm32 only; native uses `std::thread` directly).
//!
//! ## Feature comparison
//!
//! | Feature | wasm_safe_thread | wasm_thread |
//! |---------|------------------|-------------|
//! | **Native support** | Unified API (same code runs on native and wasm) | Re-exports `std::thread::*` on native |
//! | **Node.js support** | Yes, via `worker_threads` | Browser only |
//! | **Event loop integration** | [`yield_to_event_loop_async()`] for cooperative scheduling | No equivalent |
//! | **Spawn hooks** | Global hooks that run at thread start | Not available |
//! | **Parking primitives** | [`park()`]/[`Thread::unpark()`] on wasm workers | Not implemented |
//! | **Scoped threads** | Not implemented | `scope()` allows borrowing non-`'static` data |
//! | **std compatibility** | Custom [`Thread`]/[`ThreadId`] (similar API) | Re-exports `std::thread::{Thread, ThreadId}` |
//! | **Worker scripts** | Inline JS via `wasm_bindgen(inline_js)` | External JS files; `es_modules` feature for module workers |
//! | **wasm-pack targets** | ES modules (`web`) only | `web` and `no-modules` via feature flag |
//! | **Dependencies** | wasm-bindgen, js-sys, wasm_safe_mutex | web-sys (many features), futures crate |
//! | **Thread handle** | [`JoinHandle::thread()`] returns `&Thread` | `thread()` is unimplemented (panics) |
//!
//! ## Shared capabilities
//!
//! Both crates provide:
//! - [`spawn()`] and [`Builder`] for thread creation
//! - [`JoinHandle::join()`] (blocking) and [`JoinHandle::join_async()`] (async) for waiting on threads
//! - [`JoinHandle::is_finished()`] for non-blocking completion checks
//! - Thread naming via [`Builder::name()`]
//!
//! ## Behavioral differences to know
//!
//! - **Main-thread blocking:** both crates must avoid blocking APIs on the browser main thread;
//!   [`JoinHandle::join_async()`] is the safe path.
//! - **Spawn timing:** wasm workers only run after the main thread yields back to the event loop.
//! - **Worker spawning model:** `wasm_thread` proxies worker spawning through the main thread;
//!   `wasm_safe_thread` spawns directly (simpler, but different model).
//!
//! ## Implementation differences (for maintainers)
//!
//! **Result passing:**
//! - `wasm_safe_thread` uses `wasm_safe_mutex::mpsc` channels with async `recv_async()`
//! - `wasm_thread` uses `Arc<Packet<UnsafeCell>>` with a custom `Signal` primitive and `Waker` list
//!
//! **Async waiting:**
//! - `wasm_safe_thread` wraps JavaScript Promises via `wasm-bindgen-futures::JsFuture`
//! - `wasm_thread` implements `futures::future::poll_fn` with manual `Waker` tracking
//!
//! ## When to use which
//!
//! **Choose wasm_safe_thread when:**
//! - You need Node.js support (wasm_thread is browser-only)
//! - You want identical behavior on native and wasm (e.g., for testing)
//! - You need park/unpark synchronization primitives
//! - You need spawn hooks for initialization (logging, tracing, etc.)
//! - You prefer fewer dependencies and no external JS files
//! - You want an actively developed library with responsive issue/PR handling
//!
//! **Choose wasm_thread when:**
//! - You need scoped threads for borrowing non-`'static` data
//! - You want maximum compatibility with `std::thread` types
//! - You need `no-modules` wasm-pack target support
//!
//! # Usage
//!
//! Replace `use std::thread` with `use wasm_safe_thread as thread`:
//!
//! ```
//! # #[cfg(target_arch = "wasm32")]
//! # wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);
//! use wasm_safe_thread as thread;
//!
//! // Spawn a thread
//! let handle = thread::spawn(|| {
//!     println!("Hello from a worker!");
//!     42
//! });
//!
//! // Wait for the thread to complete
//! let result = handle.join().unwrap();
//! assert_eq!(result, 42);
//! ```
//!
//! # API
//!
//! ## Thread spawning
//!
//! ```
//! use wasm_safe_thread::{spawn, Builder};
//!
//! // Simple spawn
//! let handle = spawn(|| "result");
//!
//! // Named thread with builder
//! let handle = Builder::new()
//!     .name("my-worker".to_string())
//!     .spawn(|| "result")
//!     .unwrap();
//! ```
//!
//! ## Joining threads
//!
//! ```
//! # #[cfg(target_arch = "wasm32")]
//! # wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);
//! use wasm_safe_thread::spawn;
//!
//! // Synchronous join (works on native and wasm worker threads)
//! let handle = spawn(|| 42);
//! let result = handle.join().unwrap();
//! assert_eq!(result, 42);
//!
//! // Non-blocking check
//! let handle = spawn(|| 42);
//! if handle.is_finished() {
//!     // Thread completed
//! }
//! # drop(handle);
//! ```
//!
//! For async contexts, use `join_async`:
//!
//! ```compile_only
//! // In an async context (e.g., with wasm_bindgen_futures::spawn_local)
//! let result = handle.join_async().await.unwrap();
//! ```
//!
//! ## Thread operations
//!
//! ```
//! use wasm_safe_thread::{current, sleep, yield_now};
//! use std::time::Duration;
//!
//! // Get current thread
//! let thread = current();
//! println!("Thread: {:?}", thread.name());
//!
//! // Sleep
//! sleep(Duration::from_millis(10));
//!
//! // Yield to scheduler
//! yield_now();
//! ```
//!
//! Park/unpark works from background threads:
//!
//! ```
//! # #[cfg(target_arch = "wasm32")]
//! # wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_dedicated_worker);
//! use wasm_safe_thread::{spawn, park, park_timeout};
//! use std::time::Duration;
//!
//! let handle = spawn(|| {
//!     // Park/unpark (from background threads)
//!     park_timeout(Duration::from_millis(10)); // Wait with timeout
//! });
//! handle.thread().unpark();  // Wake parked thread
//! handle.join().unwrap();  // join() requires worker context on wasm
//! ```
//!
//! ## Event loop integration
//!
//! ```
//! # #[cfg(not(target_arch = "wasm32"))]
//! # fn main() {
//! use wasm_safe_thread::yield_to_event_loop_async;
//!
//! // Yield to browser event loop (works on native too)
//! # wasm_safe_thread::test_executor::spawn(async {
//! yield_to_event_loop_async().await;
//! # });
//! # }
//! # #[cfg(target_arch = "wasm32")]
//! # fn main() {} // JsFuture is !Send, tested separately via wasm_bindgen_test
//! ```
//!
//! ## Thread local storage
//!
//! ```
//! use wasm_safe_thread::thread_local;
//! use std::cell::RefCell;
//!
//! thread_local! {
//!     static COUNTER: RefCell<u32> = RefCell::new(0);
//! }
//!
//! COUNTER.with(|c| {
//!     *c.borrow_mut() += 1;
//! });
//! ```
//!
//! ## Spawn hooks
//!
//! Register callbacks that run when any thread starts:
//!
//! ```
//! use wasm_safe_thread::{register_spawn_hook, remove_spawn_hook, clear_spawn_hooks};
//!
//! // Register a hook
//! register_spawn_hook("my-hook", || {
//!     println!("Thread starting!");
//! });
//!
//! // Hooks run in registration order, before the thread's main function
//!
//! // Remove specific hook
//! remove_spawn_hook("my-hook");
//!
//! // Clear all hooks
//! clear_spawn_hooks();
//! ```
//!
//! # WASM Limitations
//!
//! ## Main thread restrictions
//!
//! The browser main thread cannot use blocking APIs:
//!
//! - [`JoinHandle::join()`] - Use [`JoinHandle::join_async()`] instead
//! - [`park()`] / [`park_timeout()`] - Only works from background threads
//! - `Mutex::lock()` from std - Use `wasm_safe_mutex` instead
//!
//! ## SharedArrayBuffer requirements
//!
//! Threading requires `SharedArrayBuffer`, which needs these HTTP headers:
//!
//! ```text
//! Cross-Origin-Opener-Policy: same-origin
//! Cross-Origin-Embedder-Policy: require-corp
//! ```
//!
//! See [Mozilla's documentation](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer) for details.
//!
//! ## Environment support
//!
//! - **Browser**: Web Workers with shared memory
//! - **Node.js**: worker_threads module
//!
//! # Building for WASM
//!
//! Standard library must be rebuilt with atomics support:
//!
//! ```bash
//! # Install nightly and components
//! rustup toolchain install nightly
//! rustup component add rust-src --toolchain nightly
//!
//! # Build with atomics
//! RUSTFLAGS='-C target-feature=+atomics,+bulk-memory' \
//! cargo +nightly build -Z build-std=std,panic_abort \
//!     --target wasm32-unknown-unknown
//! ```
//!

extern crate alloc;

mod hooks;
#[cfg(not(target_arch = "wasm32"))]
mod stdlib;
#[doc(hidden)]
pub mod test_executor;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
use stdlib as backend;
#[cfg(target_arch = "wasm32")]
use wasm as backend;

use std::io;
use std::num::NonZeroUsize;
use std::time::Duration;

pub use backend::yield_to_event_loop_async;
pub use backend::{AccessError, Builder, JoinHandle, LocalKey, Thread, ThreadId};
pub use hooks::{clear_spawn_hooks, register_spawn_hook, remove_spawn_hook};

/// Declare a new thread local storage key of type [`LocalKey`].
///
/// # Examples
///
/// ```
/// use wasm_safe_thread::thread_local;
/// use std::cell::RefCell;
///
/// thread_local! {
///     static FOO: RefCell<u32> = RefCell::new(1);
/// }
///
/// FOO.with(|f| {
///     assert_eq!(*f.borrow(), 1);
///     *f.borrow_mut() = 2;
/// });
/// ```
#[macro_export]
#[cfg(not(target_arch = "wasm32"))]
macro_rules! thread_local {
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => {
        std::thread_local! {
            $(#[$attr])* static INNER: $t = $init;
        }
        $(#[$attr])* $vis static $name: $crate::LocalKey<$t> = $crate::LocalKey::new(&INNER);
        $crate::thread_local!($($rest)*);
    };
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => {
        std::thread_local! {
            $(#[$attr])* static INNER: $t = $init;
        }
        $(#[$attr])* $vis static $name: $crate::LocalKey<$t> = $crate::LocalKey::new(&INNER);
    };
    () => {};
}

/// Declare a new thread local storage key of type [`LocalKey`].
#[macro_export]
#[cfg(target_arch = "wasm32")]
macro_rules! thread_local {
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr; $($rest:tt)*) => {
        std::thread_local! {
            $(#[$attr])* static INNER: $t = $init;
        }
        $(#[$attr])* $vis static $name: $crate::LocalKey<$t> = $crate::LocalKey::new(&INNER);
        $crate::thread_local!($($rest)*);
    };
    ($(#[$attr:meta])* $vis:vis static $name:ident: $t:ty = $init:expr) => {
        std::thread_local! {
            $(#[$attr])* static INNER: $t = $init;
        }
        $(#[$attr])* $vis static $name: $crate::LocalKey<$t> = $crate::LocalKey::new(&INNER);
    };
    () => {};
}

/// Spawns a new thread, returning a JoinHandle for it.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    backend::spawn(f)
}

/// Gets a handle to the thread that invokes it.
pub fn current() -> Thread {
    backend::current()
}

/// Puts the current thread to sleep for at least the specified duration.
pub fn sleep(dur: Duration) {
    backend::sleep(dur)
}

/// Cooperatively gives up a timeslice to the OS scheduler.
pub fn yield_now() {
    backend::yield_now()
}

/// Blocks unless or until the current thread's token is made available.
pub fn park() {
    backend::park()
}

/// Blocks unless or until the current thread's token is made available
/// or the specified duration has been reached.
pub fn park_timeout(dur: Duration) {
    backend::park_timeout(dur)
}

/// Returns an estimate of the default amount of parallelism a program should use.
pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    backend::available_parallelism()
}

/// A convenience function for spawning a thread with a name.
pub fn spawn_named<F, T>(name: impl Into<String>, f: F) -> io::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Builder::new().name(name.into()).spawn(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    // On wasm32, join() panics on the main thread (Atomics.wait unavailable).
    // This produces "RuntimeError: unreachable" in console - expected for wasm panics.
    #[test]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(target_arch = "wasm32", should_panic)]
    fn test_spawn_and_join() {
        let handle = spawn(|| 42);
        let result = handle.join().unwrap();
        assert_eq!(result, 42);
    }

    // try join from bg thread
    crate::async_test! {
        async fn join_bg() {

            let handle = Builder::new()
                    .name("join me".to_string())
                    .spawn(|| {})
                    .unwrap();

            let bg = Builder::new()
            .name("joining".to_string())
        .spawn(|| {
            handle.join().unwrap();
        }).unwrap();
        bg.join_async().await.unwrap();
        }
    }

    async_test! {
        async fn test_spawn_and_join_async() {
            let handle = spawn(|| 42);
            let result = handle.join_async().await.unwrap();
            assert_eq!(result, 42);
        }
    }

    async_test! {
        async fn test_builder() {
            let handle = Builder::new()
            .name("test-thread".to_string())
            .spawn(|| "hello")
            .unwrap();
            let result = handle.join_async().await.unwrap();
            assert_eq!(result, "hello");
        }
    }

    #[test]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn test_current_thread() {
        let _current = current();
    }

    // Test that current() returns correct thread info
    crate::async_test! {
        async fn test_current_in_spawned_thread() {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::Arc;

            // Verify we can get current thread (name varies by context)
            let main_thread = current();
            let _ = main_thread.name(); // Just verify it doesn't panic

            let saw_correct_name = Arc::new(AtomicBool::new(false));
            let saw_correct_name_clone = saw_correct_name.clone();

            let handle = Builder::new()
                .name("test-worker".to_string())
                .spawn(move || {
                    let thread = current();
                    if thread.name() == Some("test-worker") {
                        saw_correct_name_clone.store(true, Ordering::SeqCst);
                    }
                })
                .unwrap();

            handle.join_async().await.unwrap();

            assert!(saw_correct_name.load(Ordering::SeqCst),
                "spawned thread should see its own name via current()");
        }
    }

    #[test]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn test_yield_now() {
        yield_now();
    }

    async_test! {
        async fn test_yield_now_bg() {
            crate::Builder::new()
            .name("test-thread".to_string())
            .spawn(|| {
                yield_now();
            }).unwrap().join_async().await.unwrap();
        }
    }

    #[test]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn test_sleep() {
        #[cfg(not(target_arch = "wasm32"))]
        use std::time::Instant;
        #[cfg(target_arch = "wasm32")]
        use web_time::Instant;

        let start = Instant::now();
        sleep(Duration::from_millis(50));
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(50),
            "sleep should wait at least 50ms, but only waited {:?}",
            elapsed
        );
    }

    crate::async_test! {
        async fn sleep_bg() {
            #[cfg(not(target_arch = "wasm32"))]
            use std::time::Instant;
            #[cfg(target_arch = "wasm32")]
            use web_time::Instant;

            let start = Instant::now();
            let bg = Builder::new()
                .name("sleeping".to_string())
                .spawn(|| {
                    sleep(Duration::from_millis(50));
                })
                .unwrap();
            bg.join_async().await.unwrap();
            let elapsed = start.elapsed();
            assert!(elapsed >= Duration::from_millis(50), "sleep should wait at least 50ms, but only waited {:?}", elapsed);
        }
    }

    #[test]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn test_available_parallelism() {
        let parallelism = available_parallelism().unwrap();
        assert!(parallelism.get() >= 1);
    }

    #[test]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn test_thread_local() {
        use std::cell::Cell;

        thread_local! {
            static COUNTER: Cell<u32> = Cell::new(0);
        }

        COUNTER.with(|c| {
            assert_eq!(c.get(), 0);
            c.set(42);
        });

        COUNTER.with(|c| {
            assert_eq!(c.get(), 42);
        });
    }

    #[test]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn test_thread_local_try_with() {
        use std::cell::Cell;

        thread_local! {
            static VALUE: Cell<i32> = Cell::new(100);
        }

        let result = VALUE.try_with(|v| v.get());
        assert_eq!(result.unwrap(), 100);
    }

    // Test that TLS values are isolated per-thread
    crate::async_test! {
        async fn test_thread_local_isolation() {
            use std::cell::Cell;
            use std::sync::atomic::{AtomicU32, Ordering};
            use std::sync::Arc;

            thread_local! {
                static TLS_VALUE: Cell<u32> = Cell::new(0);
            }

            // Set main thread's TLS to 999
            TLS_VALUE.with(|v| v.set(999));

            // Spawned thread should see initial value (0), not main thread's value
            let worker_saw = Arc::new(AtomicU32::new(0));
            let worker_saw_clone = worker_saw.clone();

            let handle = Builder::new()
                .name("tls-isolation-test".to_string())
                .spawn(move || {
                    // Worker should see initial value, not main thread's 999
                    let initial = TLS_VALUE.with(|v| v.get());
                    worker_saw_clone.store(initial, Ordering::SeqCst);

                    // Modify worker's TLS
                    TLS_VALUE.with(|v| v.set(123));

                    // Verify worker sees its own modification
                    let after = TLS_VALUE.with(|v| v.get());
                    assert_eq!(after, 123, "worker should see its own TLS modification");
                })
                .unwrap();

            handle.join_async().await.unwrap();

            // Worker should have seen initial value (0)
            assert_eq!(worker_saw.load(Ordering::SeqCst), 0,
                "spawned thread should see initial TLS value, not main thread's value");

            // Main thread's TLS should be unchanged
            TLS_VALUE.with(|v| {
                assert_eq!(v.get(), 999, "main thread's TLS should be unchanged by worker");
            });
        }
    }

    // Note: These hook tests use unique hook names to avoid interference
    // when tests run in parallel. Each test only removes its own hooks.

    async_test! {
        async fn test_spawn_hook_single() {
            use std::sync::{Arc, Mutex};

            const HOOK_NAME: &str = "test_spawn_hook_single";

            let order = Arc::new(Mutex::new(Vec::new()));
            let order_hook = order.clone();
            let order_main = order.clone();

            register_spawn_hook(HOOK_NAME, move || {
                order_hook.lock().unwrap().push(1);
            });

            let handle = Builder::new()
                .spawn(move || {
                    order_main.lock().unwrap().push(2);
                })
                .unwrap();

            handle.join_async().await.unwrap();

            remove_spawn_hook(HOOK_NAME);

            let result = order.lock().unwrap();
            // Hook ran once, then main function
            assert!(result.contains(&1), "hook should have run");
            assert!(result.contains(&2), "main function should have run");
            // Verify hook ran before main (find positions)
            let hook_pos = result.iter().position(|&x| x == 1).unwrap();
            let main_pos = result.iter().position(|&x| x == 2).unwrap();
            assert!(hook_pos < main_pos, "hook should run before main function");
        }
    }

    async_test! {
        async fn test_spawn_hook_multiple_in_order() {
            use std::sync::{Arc, Mutex};

            const HOOK1: &str = "test_spawn_hook_multiple_1";
            const HOOK2: &str = "test_spawn_hook_multiple_2";
            const HOOK3: &str = "test_spawn_hook_multiple_3";

            let order = Arc::new(Mutex::new(Vec::new()));
            let o1 = order.clone();
            let o2 = order.clone();
            let o3 = order.clone();
            let o_main = order.clone();

            register_spawn_hook(HOOK1, move || {
                o1.lock().unwrap().push(1);
            });
            register_spawn_hook(HOOK2, move || {
                o2.lock().unwrap().push(2);
            });
            register_spawn_hook(HOOK3, move || {
                o3.lock().unwrap().push(3);
            });

            let handle = Builder::new()
                .spawn(move || {
                    o_main.lock().unwrap().push(100);
                })
                .unwrap();

            handle.join_async().await.unwrap();

            remove_spawn_hook(HOOK1);
            remove_spawn_hook(HOOK2);
            remove_spawn_hook(HOOK3);

            let result = order.lock().unwrap();
            // Find positions of our markers
            let pos1 = result.iter().position(|&x| x == 1);
            let pos2 = result.iter().position(|&x| x == 2);
            let pos3 = result.iter().position(|&x| x == 3);
            let pos_main = result.iter().position(|&x| x == 100);

            // All hooks and main should have run
            assert!(pos1.is_some(), "hook1 should have run");
            assert!(pos2.is_some(), "hook2 should have run");
            assert!(pos3.is_some(), "hook3 should have run");
            assert!(pos_main.is_some(), "main should have run");

            // Verify order: hooks in registration order, then main
            let pos1 = pos1.unwrap();
            let pos2 = pos2.unwrap();
            let pos3 = pos3.unwrap();
            let pos_main = pos_main.unwrap();
            assert!(pos1 < pos2, "hook1 should run before hook2");
            assert!(pos2 < pos3, "hook2 should run before hook3");
            assert!(pos3 < pos_main, "hook3 should run before main");
        }
    }

    async_test! {
        async fn test_remove_spawn_hook() {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::Arc;

            const HOOK1: &str = "test_remove_hook1";
            const HOOK2: &str = "test_remove_hook2";

            // Use flags instead of counters to be robust against parallel tests
            let hook1_ran = Arc::new(AtomicBool::new(false));
            let hook2_ran = Arc::new(AtomicBool::new(false));
            let h1 = hook1_ran.clone();
            let h2 = hook2_ran.clone();

            register_spawn_hook(HOOK1, move || {
                h1.store(true, Ordering::SeqCst);
            });
            register_spawn_hook(HOOK2, move || {
                h2.store(true, Ordering::SeqCst);
            });

            // Remove hook1
            assert!(remove_spawn_hook(HOOK1));
            assert!(!remove_spawn_hook("nonexistent"));

            // Reset flags before spawning
            hook1_ran.store(false, Ordering::SeqCst);
            hook2_ran.store(false, Ordering::SeqCst);

            let handle = spawn(|| {});
            handle.join_async().await.unwrap();

            // Check flags after join
            let h1_ran = hook1_ran.load(Ordering::SeqCst);
            let h2_ran = hook2_ran.load(Ordering::SeqCst);

            remove_spawn_hook(HOOK2);

            // hook1 was removed so shouldn't have run, hook2 should have run
            assert!(!h1_ran, "removed hook1 should not have run");
            assert!(h2_ran, "hook2 should have run");
        }
    }

    async_test! {
        async fn test_clear_spawn_hooks() {
            use std::sync::atomic::{AtomicU32, Ordering};
            use std::sync::Arc;

            const HOOK: &str = "test_clear_hook";

            let counter = Arc::new(AtomicU32::new(0));
            let c = counter.clone();

            register_spawn_hook(HOOK, move || {
                c.fetch_add(1, Ordering::SeqCst);
            });

            // Remove just this hook (not clear_spawn_hooks which would affect other tests)
            remove_spawn_hook(HOOK);

            let before = counter.load(Ordering::SeqCst);
            let handle = spawn(|| {});
            handle.join_async().await.unwrap();
            let after = counter.load(Ordering::SeqCst);

            // Our hook was removed so it shouldn't have incremented
            // (other hooks from parallel tests may run, but ours won't)
            assert_eq!(after, before, "removed hook should not run");
        }
    }

    async_test! {
        async fn test_replace_hook_same_name() {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::Arc;

            const HOOK: &str = "test_replace_same_name";

            // Use flags to be robust against parallel tests
            let first_hook_ran = Arc::new(AtomicBool::new(false));
            let second_hook_ran = Arc::new(AtomicBool::new(false));
            let f1 = first_hook_ran.clone();
            let f2 = second_hook_ran.clone();

            register_spawn_hook(HOOK, move || {
                f1.store(true, Ordering::SeqCst);
            });
            // Register with same name - should replace
            register_spawn_hook(HOOK, move || {
                f2.store(true, Ordering::SeqCst);
            });

            // Reset flags before spawning
            first_hook_ran.store(false, Ordering::SeqCst);
            second_hook_ran.store(false, Ordering::SeqCst);

            let handle = spawn(|| {});
            handle.join_async().await.unwrap();

            // Check flags after join
            let first_ran = first_hook_ran.load(Ordering::SeqCst);
            let second_ran = second_hook_ran.load(Ordering::SeqCst);

            remove_spawn_hook(HOOK);

            // Second hook replaced first, so only second should have run
            assert!(!first_ran, "first hook should have been replaced");
            assert!(second_ran, "second hook should have run");
        }
    }

    crate::async_test! {
        async fn closure_bomb() {
        Builder::new()
            .name("closure bomb".to_string())
            .spawn(|| {
                for i in 0..5 {
                    Builder::new()
                        .spawn(move || {
                            // Just do a simple computation, no allocations
                            let x = i * 2;
                            std::hint::black_box(x);
                        })
                        .unwrap();
                }
            })
            .unwrap();
        }
    }

    crate::async_test! {
        async fn single_spawn() {
            let handle = Builder::new()
                .name("single worker".to_string())
                .spawn(|| {
                    // Do nothing
                })
                .unwrap();
            handle.join_async().await.unwrap();
        }
    }

    crate::async_test! {
        async fn flat_spawn_parallel() {
            for i in 0..12 {
                Builder::new()
                    .name(format!("parallel worker {}", i))
                    .spawn(|| {})
                    .unwrap();
            }
        }
    }

    // Test that park/unpark works on background thread
    // Note: We can't spin-wait on the main thread in wasm (blocks event loop),
    // so we just unpark and rely on the token mechanism - if unpark happens
    // before park, the token is consumed and park returns immediately.
    crate::async_test! {
        async fn test_park_unpark_background() {
            let handle = Builder::new()
                .name("parker".to_string())
                .spawn(|| {
                    park();
                    42
                })
                .unwrap();

            // Unpark the thread - either wakes it or sets a token for when it parks
            handle.thread().unpark();

            // Join should succeed
            let result = handle.join_async().await.unwrap();
            assert_eq!(result, 42);
        }
    }

    // Test park_timeout on background thread
    crate::async_test! {
        async fn test_park_timeout_background() {
            #[cfg(not(target_arch = "wasm32"))]
            use std::time::Instant;
            #[cfg(target_arch = "wasm32")]
            use web_time::Instant;

            let handle = Builder::new()
                .name("park_timeout".to_string())
                .spawn(|| {
                    let start = Instant::now();
                    park_timeout(Duration::from_millis(100));
                    start.elapsed()
                })
                .unwrap();

            let elapsed = handle.join_async().await.unwrap();
            // Should have waited at least 100ms (but allow some slack)
            assert!(elapsed >= Duration::from_millis(90),
                "park_timeout should wait at least 90ms, but only waited {:?}", elapsed);
        }
    }

    // Test that unpark before park doesn't block
    crate::async_test! {
        async fn test_unpark_before_park() {
            let handle = Builder::new()
                .name("pre-unpark".to_string())
                .spawn(|| {
                    let thread = current();
                    // Unpark ourselves before parking
                    thread.unpark();
                    // This should return immediately since we have a token
                    park();
                    "done"
                })
                .unwrap();

            let result = handle.join_async().await.unwrap();
            assert_eq!(result, "done");
        }
    }

    // Test is_finished returns false before thread completes
    crate::async_test! {
        async fn test_is_finished_false_initially() {
            use std::sync::atomic::{AtomicBool, Ordering};
            use std::sync::Arc;

            let can_exit = Arc::new(AtomicBool::new(false));
            let can_exit_clone = can_exit.clone();

            let handle = Builder::new()
                .name("is_finished_test".to_string())
                .spawn(move || {
                    // Wait until main thread tells us to exit
                    while !can_exit_clone.load(Ordering::Acquire) {
                        sleep(Duration::from_millis(10));
                    }
                    42
                })
                .unwrap();

            // Thread is running, should not be finished yet
            assert!(!handle.is_finished(), "thread should not be finished while running");

            // Allow thread to exit
            can_exit.store(true, Ordering::Release);

            // Wait for completion
            let result = handle.join_async().await.unwrap();
            assert_eq!(result, 42);
        }
    }

    // Test is_finished returns true after thread completes
    crate::async_test! {
        async fn test_is_finished_true_after_complete() {
            let handle = Builder::new()
                .name("is_finished_complete".to_string())
                .spawn(|| {
                    // Return immediately
                    123
                })
                .unwrap();

            // Wait for completion via join
            let result = handle.join_async().await.unwrap();
            assert_eq!(result, 123);
            // Note: We can't check is_finished after join because join consumes the handle
        }
    }

    // Test is_finished by polling from async context with proper event loop yields
    crate::async_test! {
        async fn test_is_finished_without_join() {
            // Spawn a thread that returns quickly
            let handle = Builder::new()
                .name("is_finished_target".to_string())
                .spawn(|| {
                    42
                })
                .unwrap();

            // Yield to event loop to let the worker start
            crate::yield_to_event_loop_async().await;

            // Poll is_finished with async yields until it becomes true
            let mut attempts = 0;
            while !handle.is_finished() && attempts < 1_000 {
                crate::yield_to_event_loop_async().await;
                attempts += 1;
            }

            assert!(handle.is_finished(), "thread should be finished after polling");
            let result = handle.join_async().await.unwrap();
            assert_eq!(result, 42);
        }
    }

    // Test that yield_to_event_loop_async completes without hanging
    async_test! {
        async fn test_yield_to_event_loop_async() {
            // Basic test: should complete without hanging
            crate::yield_to_event_loop_async().await;

            // Multiple yields should work
            for _ in 0..10 {
                crate::yield_to_event_loop_async().await;
            }
        }
    }

    // Test that panics in spawned threads propagate to join_async as errors.
    // Note: On wasm32, this test produces "RuntimeError: unreachable" in the console output.
    // This is expected - wasm panics trigger an abort which manifests as this error.
    // The test still passes because the panic is caught and returned as an Err.
    async_test! {
        async fn test_thread_panic_propagates() {
            let handle = spawn(|| {
                panic!("test panic from thread!");
            });
            let result = handle.join_async().await;
            assert!(result.is_err(), "join_async should return Err when thread panics");
        }
    }
}
