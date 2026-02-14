// SPDX-License-Identifier: MIT OR Apache-2.0
#![cfg_attr(nightly_rustc, feature(internal_output_capture))]
//! A unified cross-platform `std::thread` + `std::sync` replacement for native + wasm32.
//!
//! ![logo](https://github.com/drewcrawford/wasm_safe_thread/raw/main/art/logo.png)
//!
//! This crate provides a unified threading API and synchronization primitives that work across both
//! WebAssembly and native platforms. In practice, you can treat it as a cross-platform replacement
//! for much of `std::thread` plus key `std::sync` primitives. Unlike similar crates, it's designed
//! from the ground up to handle the async realities of browser environments.
//!
//! # Synchronization primitives
//!
//! Alongside thread APIs, this crate includes WebAssembly-safe synchronization primitives:
//!
//! - [`Mutex`]
//! - [`rwlock::RwLock`]
//! - [`condvar::Condvar`]
//! - [`spinlock::Spinlock`]
//! - [`mpsc`] channels
//!
//! These APIs are usable on their own; you do not need to spawn threads with this crate to use
//! [`Mutex`], [`RwLock`](rwlock::RwLock), [`Condvar`](condvar::Condvar), or [`mpsc`].
//!
//! These primitives adapt their behavior to the runtime:
//!
//! - **Native**: uses thread parking for efficient blocking
//! - **WASM worker**: uses `Atomics.wait`-based blocking when available
//! - **WASM main thread**: falls back to non-blocking/spin strategies to avoid panics
//!
//! ## Sync Example
//!
//! ```
//! use wasm_safe_thread::Mutex;
//!
//! let data = Mutex::new(41);
//! *data.lock_sync() += 1;
//! assert_eq!(*data.lock_sync(), 42);
//! ```
//!
//! ## Channel Example
//!
//! ```
//! use wasm_safe_thread::mpsc::channel;
//!
//! let (tx, rx) = channel();
//! tx.send_sync(5).unwrap();
//! assert_eq!(rx.recv_sync().unwrap(), 5);
//! ```
//!
//! # Threading primitives
//!
//! In addition to synchronization primitives, this crate provides a `std::thread`-like API:
//! [`spawn()`], [`Builder`], [`JoinHandle`], [`park()`], [`Thread::unpark()`], thread locals,
//! and spawn hooks.
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
//! | **Dependencies** | wasm-bindgen, js-sys, continue | web-sys (many features), futures crate |
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
//! - `wasm_safe_thread` uses its built-in `mpsc` channels with async `recv_async()`
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
//! # if cfg!(target_arch="wasm32") { return; } //join() not reliable here
//! use wasm_safe_thread as thread;
//!
//! // Spawn a thread
//! let handle = thread::spawn(|| {
//!     println!("Hello from a worker!");
//!     42
//! });
//!
//! // Wait for the thread to complete
//! // Synchronous join (works on native and some browser context - but not reliably!)
//! let result = handle.join().unwrap();
//! assert_eq!(result, 42);
//! ```
//!
//! # API
//!
//! ## Thread spawning
//!
//! ```
//! use wasm_safe_thread::{spawn, spawn_named, Builder};
//!
//! // Simple spawn
//! let handle = spawn(|| "result");
//!
//! // Convenience function for named threads
//! let handle = spawn_named("my-worker", || "result").unwrap();
//!
//! // Builder pattern for more options
//! let handle = Builder::new()
//!     .name("my-worker".to_string())
//!     .spawn(|| "result")
//!     .unwrap();
//! ```
//!
//! ## Joining threads
//!
//! ```no_run
//! # if cfg!(target_arch="wasm32") { return; } //join() not reliable here
//! use wasm_safe_thread::spawn;
//!
//! // Synchronous join (works on native and some browser context - but not reliably!)
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
//! if cfg!(target_arch="wasm32") { return } //join not reliable on wasm
//! use wasm_safe_thread::{spawn, park, park_timeout};
//! use std::time::Duration;
//!
//! let handle = spawn(|| {
//!     // Park/unpark (from background threads)
//!     park_timeout(Duration::from_millis(10)); // Wait with timeout
//! });
//! handle.thread().unpark();  // Wake parked thread
//! handle.join().unwrap();  // join() is not reliable on wasm and should be avoided
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
//! ## Async task tracking (WASM)
//!
//! When spawning async tasks inside a worker thread using `wasm_bindgen_futures::spawn_local`,
//! you must notify the runtime so the worker waits for tasks to complete before exiting:
//!
//! ```
//! # #[cfg(target_arch = "wasm32")]
//! # {
//! use wasm_safe_thread::{task_begin, task_finished};
//!
//! task_begin();
//! wasm_bindgen_futures::spawn_local(async {
//!     // ... async work ...
//!     task_finished();
//! });
//! # }
//! ```
//!
//! These functions are no-ops on native platforms, so you can use them unconditionally
//! in cross-platform code.
//!
//! # WASM Limitations
//!
//! ## Main thread restrictions
//!
//! The browser main thread cannot use blocking APIs:
//!
//! - [`JoinHandle::join()`] - Use [`JoinHandle::join_async()`] instead
//! - [`park()`] / [`park_timeout()`] - Only works from background threads
//! - `Mutex::lock()` from std - Use `wasm_safe_thread::Mutex` instead
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

pub mod condvar;
pub mod guard;
mod hooks;
pub mod mpsc;
pub mod mutex;
pub mod rwlock;
pub mod spinlock;
#[cfg(not(target_arch = "wasm32"))]
mod stdlib;
#[cfg(test)]
mod sync_tests;
#[doc(hidden)]
pub mod test_executor;
#[cfg(target_arch = "wasm32")]
mod wasm;
mod wasm_support;

#[cfg(not(target_arch = "wasm32"))]
use stdlib as backend;
#[cfg(target_arch = "wasm32")]
use wasm as backend;

use std::io;
use std::num::NonZeroUsize;
use std::time::Duration;

pub use backend::yield_to_event_loop_async;
pub use backend::{AccessError, Builder, JoinHandle, LocalKey, Thread, ThreadId};
pub use backend::{task_begin, task_finished};
pub use guard::Guard;
pub use hooks::{clear_spawn_hooks, register_spawn_hook, remove_spawn_hook};
pub use mutex::{Mutex, NotAvailable};

const CONSOLE_REDIRECT_HOOK_NAME: &str = "wasm_safe_thread::println_eprintln_console_redirect";

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

/// Redirects `println!`/`eprintln!` for the current thread to JavaScript console output.
///
/// On `wasm32` + nightly this installs redirection for the calling thread.
/// On other targets/toolchains this is a no-op.
pub fn redirect_println_eprintln_to_console_current_thread() {
    backend::redirect_println_eprintln_to_console_current_thread_impl();
}

/// Installs a global spawn hook that redirects `println!`/`eprintln!` to JavaScript console output.
///
/// On `wasm32` + nightly this causes each newly spawned thread to install redirection.
/// On other targets/toolchains this hook has no effect.
pub fn install_println_eprintln_console_hook() {
    register_spawn_hook(CONSOLE_REDIRECT_HOOK_NAME, || {
        redirect_println_eprintln_to_console_current_thread();
    });
}

#[cfg(test)]
mod tests;
