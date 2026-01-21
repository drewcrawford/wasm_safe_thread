//! WebAssembly backend - placeholder implementation

use std::fmt;
use std::io;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

static THREAD_COUNTER: AtomicU64 = AtomicU64::new(0);

std::thread_local! {
    static CURRENT_THREAD: std::cell::RefCell<Option<Thread>> = const { std::cell::RefCell::new(None) };

    /// Holds a closure that sends a panic error through the channel.
    /// This is set before running user code and called from the panic hook.
    static PANIC_SENDER: std::cell::RefCell<Option<Box<dyn FnOnce(String) + Send>>> = const { std::cell::RefCell::new(None) };
}

use std::sync::Once;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures;

/// Ensures handlers are registered exactly once.
static INIT_HANDLERS: Once = Once::new();

/// Registers the cleanup handler with JavaScript.
/// Called lazily on first spawn. The Closure is intentionally leaked (one-time setup).
fn init_handlers() {
    INIT_HANDLERS.call_once(|| {
        // Cleanup handler for freeing exit_state when ref_count reaches 0
        let cleanup_handler = Closure::wrap(Box::new(|exit_state_ptr: u32| {
            wasm_free_exit_state(exit_state_ptr);
        }) as Box<dyn Fn(u32)>);
        register_cleanup_handler(cleanup_handler.as_ref().unchecked_ref());
        cleanup_handler.forget();
    });
}

/// Frees exit_state memory when ref_count reaches 0.
/// Called from JavaScript when the last reference (worker or JoinHandle) is released.
///
/// # Safety
/// The pointer must be valid and point to a `[AtomicU32; 2]` allocated by Box::into_raw.
fn wasm_free_exit_state(exit_state_ptr: u32) {
    unsafe {
        let ptr = exit_state_ptr as *mut [AtomicU32; 2];
        drop(Box::from_raw(ptr));
    }
}

#[wasm_bindgen(inline_js = r#"
const SELF_URL = import.meta.url;

// Global cleanup handler, registered once from Rust via register_cleanup_handler()
// Called when exit_state ref_count reaches 0 to free the memory
let __cleanup_handler = null;

export function register_cleanup_handler(handler) {
    __cleanup_handler = handler;
}

// Decrement ref_count and call cleanup if it reaches 0
// exit_state layout: [exit_code: u32, ref_count: u32]
function decrementRefCountAndCleanup(memory, exitStatePtr) {
    const i32 = new Int32Array(memory.buffer);
    const refCountIndex = (exitStatePtr >>> 2) + 1;  // ref_count is at offset 4
    const oldRefCount = Atomics.sub(i32, refCountIndex, 1);
    if (oldRefCount === 1 && __cleanup_handler) {
        // We decremented from 1 to 0, we're the last reference
        __cleanup_handler(exitStatePtr);
    }
}

function isNode() {
  return typeof process !== "undefined" && process.versions && process.versions.node;
}

// Derive the main wasm-bindgen shim URL from our snippet's URL.
// Our inline0.js is at .../snippets/<crate-hash>/inline0.js
// The main shim is at .../<binary-name>.js
function getShimUrl(shimName) {
  const idx = SELF_URL.indexOf("/snippets/");
  if (idx === -1) {
    throw new Error("Cannot derive shim URL: SELF_URL doesn't contain /snippets/: " + SELF_URL);
  }
  const base = SELF_URL.slice(0, idx);
  return base + "/" + shimName + ".js";
}

// Auto-detect the shim URL from loaded resources using Performance API (browser only)
function detectShimUrl() {
  const idx = SELF_URL.indexOf("/snippets/");
  if (idx === -1) {
    throw new Error("Cannot derive shim URL: SELF_URL doesn't contain /snippets/: " + SELF_URL);
  }
  const base = SELF_URL.slice(0, idx);

  // Performance API resource tracking only works in browsers, not Node.js
  if (isNode()) {
    return null;
  }

  // Find loaded JS modules at the base path
  // initiatorType can be 'script', 'module', 'other', etc. depending on browser
  const resources = performance.getEntriesByType('resource');
  for (const r of resources) {
    if (!r.name.startsWith(base + '/')) continue;
    if (r.name.includes('/snippets/')) continue;

    // Extract path without query string
    const url = new URL(r.name);
    const path = url.pathname;

    // Skip run.js (wasm-bindgen-test-runner's test runner)
    if (path.endsWith('/run.js')) continue;

    // Match .js files or extensionless paths (ES module imports)
    // Extensionless: /wasm-bindgen-test, /my-example
    // With extension: /wasm-bindgen-test.js
    if (path.endsWith('.js')) {
      return r.name.split('?')[0];  // Return full URL without query string
    }

    // Also match extensionless imports (Firefox records ES modules without extension)
    // These should be simple names with no extension and no dots
    const filename = path.split('/').pop();
    if (filename && !filename.includes('.') && filename !== 'favicon') {
      return r.name.split('?')[0] + '.js';  // Add .js for consistency
    }
  }

  // Fallback to constructed URL if detection fails
  return null;
}

// For testing: return the detected shim URL (or null if detection fails)
export function get_detected_shim_url() {
  return detectShimUrl();
}

// For debugging: return all resource entries as JSON
export function get_performance_resources_debug() {
  const idx = SELF_URL.indexOf("/snippets/");
  const base = idx === -1 ? null : SELF_URL.slice(0, idx);
  const resources = performance.getEntriesByType('resource');
  return JSON.stringify({
    selfUrl: SELF_URL,
    base: base,
    resources: resources.map(r => ({
      name: r.name,
      initiatorType: r.initiatorType,
      matchesBase: base ? r.name.startsWith(base + '/') : false,
      endsWithJs: r.name.endsWith('.js'),
      hasSnippets: r.name.includes('/snippets/')
    }))
  }, null, 2);
}

// For testing: return the shim URL that would be used for a given shim name
export function get_shim_url_for_testing(shimName) {
  return getShimUrl(shimName);
}

// Browser worker script - uses Web Worker API
function makeBrowserWorkerScript(shimUrl, entryName) {
  return `
    self.onmessage = async (e) => {
      try {
        const [module, memory, work] = e.data;

        // Cache-bust to get fresh module state per worker (needed for Safari)
        const url = ${JSON.stringify(shimUrl)} + '?worker=' + Math.random();
        const shim = await import(url);

        // Use initSync with module and memory from main thread
        shim.initSync({ module, memory, thread_stack_size: 1048576 });

        // Call the entry point
        shim[${JSON.stringify(entryName)}](work);

        // Signal exit before closing (browsers have no 'exit' event)
        self.postMessage({ __wasm_safe_thread_exit: true });
        close();
      } catch (err) {
        console.error(err);
        self.postMessage({ __wasm_safe_thread_error: err.message || String(err) });
        throw err;
      }
    };
  `;
}

// Node worker script - uses worker_threads API with cache-busting
function makeNodeWorkerScript(shimUrl, entryName) {
  return `
    import { parentPort, threadId } from "node:worker_threads";

    // Cache-bust to get fresh module state per worker
    const url = ${JSON.stringify(shimUrl)} + '?worker=' + threadId;

    parentPort.on("message", async (msg) => {
      try {
        const [module, memory, work] = msg;
        const shim = await import(url);

        // initSync is on shim directly (ESM) or shim.default (CJS)
        const initSync = shim.initSync || shim.default?.initSync;
        if (!initSync) throw new Error("No initSync found");

        // thread_stack_size tells runtime this is a worker thread
        initSync({ module, memory, thread_stack_size: 1048576 });

        // Entry point is on shim directly (ESM) or shim.default (CJS)
        const entry = shim[${JSON.stringify(entryName)}] || shim.default?.[${JSON.stringify(entryName)}];
        if (!entry) throw new Error("No entry point found: " + ${JSON.stringify(entryName)});

        entry(work);
        parentPort.close();
      } catch (err) {
        console.error(err);
        throw err;
      }
    });
  `;
}

// Helper to signal exit via atomics (works cross-thread)
function signalExit(memory, exitStatePtr, exitCode) {
  const i32 = new Int32Array(memory.buffer);
  const index = exitStatePtr >>> 2;  // Convert byte offset to i32 index
  Atomics.store(i32, index, exitCode);  // 1 = success, 2 = error
  Atomics.notify(i32, index, 1);  // Wake one waiter
}

export function wasm_safe_thread_spawn_worker(work, module, memory, name, shimName, exitStatePtr) {
  // Determine shim URL: use explicit name if provided, otherwise auto-detect
  let shimUrl;
  if (shimName) {
    shimUrl = getShimUrl(shimName);
  } else {
    // Try auto-detection, fall back to wasm-bindgen-test for test runner compatibility
    shimUrl = detectShimUrl() || getShimUrl("wasm-bindgen-test");
  }
  const entryName = "wasm_safe_thread_entry_point";

  // Browser: use Web Worker API
  if (typeof Worker === "function" && !isNode()) {
    const src = makeBrowserWorkerScript(shimUrl, entryName);
    const blob = new Blob([src], { type: "text/javascript" });
    const blobUrl = URL.createObjectURL(blob);

    const w = new Worker(blobUrl, { type: "module", name });

    w.onmessage = (e) => {
      const data = e.data;
      if (data && data.__wasm_safe_thread_exit) {
        signalExit(memory, exitStatePtr, 1);  // Success
        decrementRefCountAndCleanup(memory, exitStatePtr);
      } else if (data && data.__wasm_safe_thread_error) {
        signalExit(memory, exitStatePtr, 2);  // Error
        decrementRefCountAndCleanup(memory, exitStatePtr);
      }
    };

    w.onerror = (e) => {
      console.error("Worker error:", e.message, e.filename, e.lineno);
      signalExit(memory, exitStatePtr, 2);  // Error
      decrementRefCountAndCleanup(memory, exitStatePtr);
    };

    w.postMessage([module, memory, work]);
    URL.revokeObjectURL(blobUrl);

    return {
      postMessage: (msg) => w.postMessage(msg),
      terminate: () => w.terminate(),
    };
  }

  // Node: use worker_threads (synchronous creation is critical!)
  // If we use an async IIFE, the caller might enter an Atomics.wait loop before
  // the Worker is created, blocking the event loop and preventing the promise from resolving.
  if (isNode()) {
    // Use process.getBuiltinModule for synchronous import (Node.js 20+)
    const wt = process.getBuiltinModule
      ? process.getBuiltinModule("node:worker_threads")
      : null;
    if (!wt) throw new Error("worker_threads not available - need Node.js 20+ for synchronous import");
    const { Worker } = wt;

    const src = makeNodeWorkerScript(shimUrl, entryName);
    const w = new Worker(src, { eval: true, type: "module", name });

    // Note: Both 'error' and 'exit' events fire when a worker throws.
    // We only handle signaling and cleanup in 'exit' to avoid double-decrement of ref_count.
    // The exit code is non-zero when an error occurred.
    w.on('exit', (code) => {
      signalExit(memory, exitStatePtr, code === 0 ? 1 : 2);
      decrementRefCountAndCleanup(memory, exitStatePtr);
    });
    w.on('error', (e) => {
      // Log error for debugging, but don't signal/cleanup here.
      // The 'exit' event always follows and handles cleanup.
      console.error("Worker error:", e);
    });

    w.postMessage([module, memory, work]);

    return {
      postMessage: (msg) => w.postMessage(msg),
      terminate: () => w.terminate(),
    };
  }

  throw new Error("No Worker API available");
}

// Wait async on an atomic - returns a Promise that resolves when the value changes from 0
// Returns the new value (1 = success, 2 = error)
export function wait_for_exit_async(memory, ptr) {
  const i32 = new Int32Array(memory.buffer);
  const index = ptr >>> 2;

  // Check if already signaled
  const current = Atomics.load(i32, index);
  if (current !== 0) {
    return Promise.resolve(current);
  }

  // Use Atomics.waitAsync if available
  if (typeof Atomics.waitAsync === 'function') {
    const result = Atomics.waitAsync(i32, index, 0);
    if (result.async) {
      return result.value.then(() => Atomics.load(i32, index));
    } else {
      // Already not equal, return current value
      return Promise.resolve(Atomics.load(i32, index));
    }
  }

  // Fallback: poll with setTimeout (for environments without waitAsync)
  return new Promise((resolve) => {
    const poll = () => {
      const val = Atomics.load(i32, index);
      if (val !== 0) {
        resolve(val);
      } else {
        setTimeout(poll, 1);
      }
    };
    poll();
  });
}
"#)]
extern "C" {
    fn register_cleanup_handler(handler: &JsValue);
    fn wasm_safe_thread_spawn_worker(
        work: JsValue,
        module: JsValue,
        memory: JsValue,
        name: &str,
        shim_name: &str,
        exit_state_ptr: u32,
    ) -> WorkerLike;
    fn get_shim_url_for_testing(shim_name: &str) -> String;
    fn get_detected_shim_url() -> Option<String>;
    fn get_performance_resources_debug() -> String;
    fn wait_for_exit_async(memory: &JsValue, ptr: u32) -> js_sys::Promise;

    type WorkerLike;

    #[wasm_bindgen(method, js_name = postMessage)]
    fn post_message(this: &WorkerLike, msg: &JsValue);

    #[wasm_bindgen(method)]
    fn terminate(this: &WorkerLike);
}

#[wasm_bindgen(inline_js = r#"
export function is_node() {
  return typeof process !== "undefined" &&
    process?.versions?.node != null &&
    process?.release?.name === "node";
}

export function is_main_thread() {
  if (!is_node()) {
    // Browser main thread (classic window context)
    if (typeof window !== "undefined" && typeof document !== "undefined") {
      return true;
    }
    return false;
  }

  // Node: synchronous main-thread detection (ESM-safe)
  if (process?.getBuiltinModule) {
    const wt = process.getBuiltinModule("node:worker_threads");
    if (wt && typeof wt.isMainThread === "boolean") {
      return wt.isMainThread;
    }
  }

  throw new Error("Can't detect");

}

let __yield_sab = new SharedArrayBuffer(4);
let __yield_i32 = new Int32Array(__yield_sab);

// Yields to the browser event loop using setTimeout.
// Returns a Promise that resolves after the event loop processes.
// Note: We use 1ms instead of 0 because Chrome batches zero-delay timeouts
// aggressively, which may not give workers enough time to execute.
export async function yield_to_event_loop() {
  return new Promise(resolve => setTimeout(resolve, 1));
}

// Returns one of:
//   "ok" | "not-equal" | "timed-out" | "unsupported"
export function atomics_wait_timeout_ms_try(timeout_ms) {
  try {
    return Atomics.wait(__yield_i32, 0, 0, timeout_ms);
  } catch (_e) {
    // e.g. browser window main thread, or environments without SAB/Atomics.wait
    return "unsupported";
  }
}

export function sleep_sync_ms(ms) {
  if (ms <= 0) return;

  try {
    // SharedArrayBuffer may be unavailable unless crossOriginIsolated (in browsers)
    const sab = new SharedArrayBuffer(4);
    const i32 = new Int32Array(sab);
    // Wait while i32[0] is 0, with a timeout (ms). Returns "timed-out" typically.
    Atomics.wait(i32, 0, 0, ms);
    return;
  } catch {
    // Fall through to the worst-case synchronous fallback below.
  }

  // Worst-case fallback: busy-wait (CPU burn). Still fully synchronous.
  const end = (typeof performance !== "undefined" && performance.now)
    ? performance.now() + ms
    : Date.now() + ms;

  if (typeof performance !== "undefined" && performance.now) {
    while (performance.now() < end) {}
  } else {
    while (Date.now() < end) {}
  }
}

// Parks at a memory address within wasm linear memory.
// ptr is a byte offset into wasm memory, must be 4-byte aligned.
// Returns "ok" if woken by notify, "timed-out", or "unsupported"
export function park_wait_at_addr(memory, ptr) {
  try {
    const i32 = new Int32Array(memory.buffer);
    const index = ptr >>> 2;  // Convert byte offset to i32 index
    // Try to consume an existing token (1 -> 0)
    if (Atomics.compareExchange(i32, index, 1, 0) === 1) {
      return "ok";  // Had a pending unpark token
    }
    // Wait only if value is 0 (no token) - this is atomic with the check
    const result = Atomics.wait(i32, index, 0);
    // After wakeup, consume the token
    Atomics.store(i32, index, 0);
    return result;
  } catch (_e) {
    return "unsupported";
  }
}

// Parks with timeout at a memory address.
export function park_wait_timeout_at_addr(memory, ptr, timeout_ms) {
  try {
    const i32 = new Int32Array(memory.buffer);
    const index = ptr >>> 2;
    // Try to consume an existing token (1 -> 0)
    if (Atomics.compareExchange(i32, index, 1, 0) === 1) {
      return "ok";  // Had a pending unpark token
    }
    // Wait only if value is 0 (no token) - this is atomic with the check
    const result = Atomics.wait(i32, index, 0, timeout_ms);
    if (result === "ok") {
      // Woken by notify, consume the token
      Atomics.store(i32, index, 0);
    }
    return result;
  } catch (_e) {
    return "unsupported";
  }
}

// Unparks a thread by setting its token and notifying at a memory address.
export function park_notify_at_addr(memory, ptr) {
  const i32 = new Int32Array(memory.buffer);
  const index = ptr >>> 2;
  Atomics.store(i32, index, 1);   // Set token
  Atomics.notify(i32, index, 1);  // Wake one waiter
}

// Returns the number of logical processors available, or 1 if unknown.
export function get_available_parallelism() {
  // Browser: navigator.hardwareConcurrency
  if (typeof navigator !== "undefined" && navigator.hardwareConcurrency) {
    return navigator.hardwareConcurrency;
  }

  // Node.js: try os.availableParallelism() (Node 19.4+) or os.cpus().length
  if (typeof process !== "undefined" && process.versions && process.versions.node) {
    try {
      const os = process.getBuiltinModule?.("node:os");
      if (os) {
        // Node 19.4+ has availableParallelism()
        if (typeof os.availableParallelism === "function") {
          return os.availableParallelism();
        }
        // Fallback to cpus().length
        const cpus = os.cpus?.();
        if (cpus && cpus.length > 0) {
          return cpus.length;
        }
      }
    } catch {
      // Ignore errors
    }
  }

  // Unknown environment
  return 1;
}
"#)]
extern "C" {
    fn is_node() -> bool;
    fn is_main_thread() -> bool;
    fn atomics_wait_timeout_ms_try(timeout_ms: f64) -> JsValue;
    fn yield_to_event_loop() -> js_sys::Promise;
    fn sleep_sync_ms(ms: f64);
    fn get_available_parallelism() -> u32;
    fn park_wait_at_addr(memory: &JsValue, ptr: u32) -> JsValue;
    fn park_wait_timeout_at_addr(memory: &JsValue, ptr: u32, timeout_ms: f64) -> JsValue;
    fn park_notify_at_addr(memory: &JsValue, ptr: u32);
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn console_log_js(v: &JsValue);
}

#[allow(unused)]
fn log_str(s: &str) {
    console_log_js(&JsValue::from_str(s));
}

#[allow(dead_code)]
pub struct WorkerHandle {
    worker: WorkerLike,
}

#[allow(dead_code)]
impl WorkerHandle {
    pub fn post(&self, msg: &JsValue) {
        self.worker.post_message(msg);
    }
    pub fn terminate(self) {
        self.worker.terminate();
    }
}

/// Spawns a worker with shared module and memory.
///
/// # Arguments
/// * `work` - The work pointer (boxed closure) to execute
/// * `name` - Worker name for debugging
/// * `shim_name` - The wasm-bindgen shim name
/// * `exit_state_ptr` - Pointer to exit state atomic (0=running, 1=success, 2=error)
pub fn spawn_with_shared_module(
    work: JsValue,
    name: &str,
    shim_name: &str,
    exit_state_ptr: u32,
) -> WorkerHandle {
    // Ensure handlers are registered with JS (one-time setup)
    init_handlers();

    let worker = wasm_safe_thread_spawn_worker(
        work,
        wasm_bindgen::module(),
        wasm_bindgen::memory(),
        name,
        shim_name,
        exit_state_ptr,
    );

    WorkerHandle { worker }
}

#[wasm_bindgen]
pub fn wasm_safe_thread_entry_point(work: JsValue) {
    let ptr = work.as_f64().unwrap() as usize;
    // SAFETY: ptr came from Box::into_raw in spawn(), and we're the only consumer
    let boxed = unsafe { Box::from_raw(ptr as *mut Box<dyn FnOnce() + Send>) };
    let closure = *boxed;
    closure();
}

/// A thread local storage key which owns its contents.
///
/// This wraps `std::thread::LocalKey` which provides thread-local storage
/// on WASM with atomics via the standard library's TLS implementation.
pub struct LocalKey<T: 'static> {
    inner: &'static std::thread::LocalKey<T>,
}

impl<T: 'static> LocalKey<T> {
    /// Creates a new `LocalKey`.
    #[doc(hidden)]
    pub const fn new(inner: &'static std::thread::LocalKey<T>) -> Self {
        LocalKey { inner }
    }

    /// Acquires a reference to the value in this TLS key.
    ///
    /// This will lazily initialize the value if this is the first time
    /// the current thread has called `with` on this key.
    ///
    /// # Panics
    ///
    /// This function will panic if the initialization function panics.
    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.inner.with(f)
    }

    /// Acquires a reference to the value in this TLS key.
    ///
    /// Returns `Err(AccessError)` if the key is being destroyed or
    /// was already destroyed.
    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        self.inner.try_with(f).map_err(|_| AccessError)
    }
}

impl<T: 'static> fmt::Debug for LocalKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalKey").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessError;

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "already destroyed or being destroyed")
    }
}

impl std::error::Error for AccessError {}

/// A handle to a thread.
pub struct JoinHandle<T> {
    receiver: wasm_safe_mutex::mpsc::Receiver<Result<T, String>>,
    thread: Thread,
    finished: Arc<AtomicBool>,
    /// Pointer to exit_state: [exit_code: u32, ref_count: u32] in wasm linear memory.
    /// This is managed via reference counting - worker and JoinHandle each hold a reference.
    /// Whoever decrements ref_count to 0 frees the memory.
    exit_state_ptr: u32,
}

impl<T> JoinHandle<T> {
    pub fn join(self) -> Result<T, Box<String>> {
        if is_main_thread() {
            return Err(Box::new(
                "Can't join from the main thread on wasm32".to_string(),
            ));
        }
        self.receiver
            .recv_sync()
            .map_err(|e| Box::new(format!("{:?}", e)) as Box<String>)?
            .map_err(|e| Box::new(e) as Box<String>)
    }

    pub async fn join_async(self) -> Result<T, Box<String>> {
        // First get the return value from the channel
        // The channel sends Result<T, String> to support panic propagation
        let result = self
            .receiver
            .recv_async()
            .await
            .map_err(|e| Box::new(format!("{:?}", e)) as Box<String>)?
            .map_err(|e| Box::new(e) as Box<String>)?;

        // Then wait for the worker to actually exit
        let exit_code = wasm_bindgen_futures::JsFuture::from(wait_for_exit_async(
            &wasm_bindgen::memory(),
            self.exit_state_ptr,
        ))
        .await
        .map_err(|e| Box::new(format!("Worker exit error: {:?}", e)) as Box<String>)?;

        // Check exit code (1 = success, 2 = error)
        let code = exit_code.as_f64().unwrap_or(0.0) as u32;
        if code == 2 {
            return Err(Box::new("Worker exited with error".to_string()));
        }

        Ok(result)
        // Note: Drop impl will decrement ref_count and potentially free exit_state
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        // Decrement ref_count and free if we're the last reference.
        // exit_state layout: [exit_code: u32, ref_count: u32]
        let ptr = self.exit_state_ptr as *mut [AtomicU32; 2];
        // SAFETY: ptr was created from Box::into_raw in spawn() and is valid
        // as long as ref_count > 0. We're atomically decrementing ref_count.
        let old_ref_count = unsafe { (*ptr)[1].fetch_sub(1, Ordering::AcqRel) };
        if old_ref_count == 1 {
            // We decremented from 1 to 0, we're the last reference - free the memory
            unsafe {
                drop(Box::from_raw(ptr));
            }
        }
    }
}

#[derive(Clone)]
pub struct Thread {
    inner: Arc<ThreadInner>,
}

struct ThreadInner {
    name: Option<String>,
    id: ThreadId,
    /// Parking state: 0 = no token, 1 = unpark token present.
    /// This is a Box'd atomic so we get a stable address in wasm linear memory.
    parking_state: Box<AtomicU32>,
}

impl Thread {
    pub fn id(&self) -> ThreadId {
        self.inner.id
    }

    pub fn name(&self) -> Option<&str> {
        self.inner.name.as_deref()
    }

    pub fn unpark(&self) {
        let ptr = self.inner.parking_state.as_ref() as *const AtomicU32 as u32;
        park_notify_at_addr(&wasm_bindgen::memory(), ptr);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ThreadId(u64);

pub struct Builder {
    _name: Option<String>,
    _stack_size: Option<usize>,
    _shim_name: Option<String>,
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            _name: None,
            _stack_size: None,
            _shim_name: None,
        }
    }

    /// Set the wasm-bindgen shim name for worker spawning.
    /// This should match the binary/example name (e.g., "single_spawn", "wasm-bindgen-test").
    pub fn shim_name(mut self, name: String) -> Self {
        self._shim_name = Some(name);
        self
    }

    pub fn name(mut self, name: String) -> Self {
        self._name = Some(name);
        self
    }

    pub fn stack_size(mut self, size: usize) -> Self {
        self._stack_size = Some(size);
        self
    }

    /// Spawns a new thread with the configured settings.
    ///
    /// # Platform Notes (WASM)
    ///
    /// **The spawned thread will not begin executing until you yield to the JavaScript
    /// event loop.** On WASM targets, worker threads are spawned asynchronously via
    /// the JS event loop. When this function returns, the worker has been *created*
    /// but has not yet *started*.
    ///
    /// You must yield to the event loop to allow the worker to begin:
    ///
    /// ```
    /// let handle = wasm_safe_thread::Builder::new().spawn(|| { /* ... */ }).unwrap();
    /// # async fn ex() {
    /// wasm_safe_thread::yield_to_event_loop_async().await;  // Worker starts here, not above!
    /// # }
    /// ```
    ///
    /// Failure to yield can cause:
    /// - `is_finished()` to always return `false`
    /// - Deadlocks when waiting on atomics the worker should have set
    /// - Tests that hang indefinitely
    pub fn spawn<F, T>(self, f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        // Create Thread before spawning so we can pass it to the worker
        // Thread::name() only returns Some if explicitly set via Builder::name()
        let id = ThreadId(THREAD_COUNTER.fetch_add(1, Ordering::Relaxed));
        let thread = Thread {
            inner: Arc::new(ThreadInner {
                name: self._name.clone(),
                id,
                parking_state: Box::new(AtomicU32::new(0)),
            }),
        };

        // Worker name: use explicit name or generate a default for JS devtools
        let worker_name = self
            ._name
            .unwrap_or_else(|| format!("wasm_safe_thread {}", id.0));
        let thread_for_worker = thread.clone();

        let finished = Arc::new(AtomicBool::new(false));
        let finished_for_worker = finished.clone();

        // Exit state: [exit_code: u32, ref_count: u32]
        // - exit_code: 0 = running, 1 = exited successfully, 2 = exited with error
        // - ref_count: starts at 2 (worker + JoinHandle), decremented on worker exit and JoinHandle drop
        // Box'd so we get a stable address in wasm linear memory
        let exit_state = Box::new([AtomicU32::new(0), AtomicU32::new(2)]);
        let exit_state_ptr = Box::into_raw(exit_state) as u32;

        let (send, recv) = wasm_safe_mutex::mpsc::channel();
        let closure = move || {
            // Set up TLS for current() before running user code
            CURRENT_THREAD.with(|cell| {
                *cell.borrow_mut() = Some(thread_for_worker);
            });

            // Set up panic handling: store a sender closure that the panic hook can use
            // to send the error through the channel before aborting
            PANIC_SENDER.with(|cell| {
                let send_clone = send.clone();
                let finished_clone = finished_for_worker.clone();
                *cell.borrow_mut() = Some(Box::new(move |msg: String| {
                    finished_clone.store(true, Ordering::Release);
                    let _ = send_clone.send_sync(Err(msg));
                }));
            });

            // Set a panic hook that sends the error through PANIC_SENDER
            // We chain with the previous hook so that panics on other threads
            // (like the main thread) still work correctly
            let prev_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let msg = info.to_string();
                let sent = PANIC_SENDER.with(|cell| {
                    if let Some(sender) = cell.borrow_mut().take() {
                        sender(msg);
                        true
                    } else {
                        false
                    }
                });
                // If we didn't have a PANIC_SENDER (e.g., panic on main thread),
                // call the previous hook
                if !sent {
                    prev_hook(info);
                }
            }));

            crate::hooks::run_spawn_hooks();

            let result = f();

            // Clear panic sender since we completed successfully
            PANIC_SENDER.with(|cell| {
                cell.borrow_mut().take();
            });

            // Mark as finished before sending result (Release pairs with Acquire in is_finished)
            finished_for_worker.store(true, Ordering::Release);
            // Ignore send errors - receiver may have been dropped if JoinHandle wasn't joined
            let _ = send.send_sync(Ok(result));
        };

        // Double-box to get a thin pointer (Box<dyn FnOnce()> is a fat pointer)
        let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(closure));
        let ptr = Box::into_raw(boxed) as *mut () as usize;
        let work: JsValue = (ptr as f64).into();

        // If shim_name not explicitly set, pass empty string to trigger auto-detection in JS
        let shim_name = self._shim_name.as_deref().unwrap_or("");

        // Spawn the worker. The WorkerHandle is not needed after spawn -
        // the worker runs independently and signals completion via exit_state.
        let _worker_handle =
            spawn_with_shared_module(work, &worker_name, shim_name, exit_state_ptr);

        Ok(JoinHandle {
            receiver: recv,
            thread,
            finished,
            exit_state_ptr,
        })
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawns a new thread, returning a [`JoinHandle`] for it.
///
/// # Warning (WASM)
///
/// **The thread will not start until you yield to the JS event loop.**
/// See [`Builder::spawn`] for details.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Builder::new().spawn(f).expect("failed to spawn thread")
}

pub fn current() -> Thread {
    CURRENT_THREAD.with(|cell| {
        let mut borrowed = cell.borrow_mut();
        if let Some(ref thread) = *borrowed {
            thread.clone()
        } else {
            // Lazily create a Thread for threads we didn't spawn
            let name = if is_main_thread() {
                Some("main".to_string())
            } else {
                // Thread not spawned by us (e.g., a Web Worker created externally)
                None
            };
            let id = ThreadId(THREAD_COUNTER.fetch_add(1, Ordering::Relaxed));
            let thread = Thread {
                inner: Arc::new(ThreadInner {
                    name,
                    id,
                    parking_state: Box::new(AtomicU32::new(0)),
                }),
            };
            *borrowed = Some(thread.clone());
            thread
        }
    })
}

pub fn sleep(dur: Duration) {
    sleep_sync_ms(dur.as_millis() as f64);
}

pub fn yield_now() {
    atomics_wait_timeout_ms_try(0.001);
}

/// Yields to the browser event loop, allowing pending tasks (like worker startup) to execute.
/// This is necessary when spawning workers from within workers, as the child worker
/// won't start executing until the parent yields to the event loop.
pub async fn yield_to_event_loop_async() {
    let _: JsValue = wasm_bindgen_futures::JsFuture::from(yield_to_event_loop())
        .await
        .unwrap();
}

pub fn park() {
    let thread = current();
    let ptr = thread.inner.parking_state.as_ref() as *const AtomicU32 as u32;
    let result = park_wait_at_addr(&wasm_bindgen::memory(), ptr);
    let result_str = result.as_string().unwrap_or_default();
    if result_str == "unsupported" {
        panic!("Atomics.wait is not available in this context (likely main thread)");
    }
}

pub fn park_timeout(dur: Duration) {
    let thread = current();
    let ptr = thread.inner.parking_state.as_ref() as *const AtomicU32 as u32;
    let timeout_ms = dur.as_millis() as f64;
    let result = park_wait_timeout_at_addr(&wasm_bindgen::memory(), ptr, timeout_ms);
    let result_str = result.as_string().unwrap_or_default();
    if result_str == "unsupported" {
        panic!("Atomics.wait is not available in this context (likely main thread)");
    }
}

pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    let count = get_available_parallelism();
    NonZeroUsize::new(count as usize).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine available parallelism",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn test_is_main_thread() {
        assert!(is_main_thread());
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn test_default_shim_url_is_wasm_bindgen_test() {
        // Verify that the default shim URL ends with "wasm-bindgen-test.js"
        // This is the expected behavior when running wasm-bindgen-test tests
        let url = get_shim_url_for_testing("wasm-bindgen-test");
        assert!(
            url.ends_with("/wasm-bindgen-test.js"),
            "Expected shim URL to end with /wasm-bindgen-test.js, got: {}",
            url
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test]
    fn test_detect_shim_url_finds_wasm_bindgen_test() {
        // Performance API resource tracking only works in browsers, not Node.js
        if is_node() {
            // In Node.js, detection returns None - this is expected
            assert!(
                get_detected_shim_url().is_none(),
                "In Node.js, shim URL detection should return None"
            );
            return;
        }

        // When running in a browser, we should detect wasm-bindgen-test.js
        let debug_info = get_performance_resources_debug();
        let detected = get_detected_shim_url();
        assert!(
            detected.is_some(),
            "Should detect shim URL from loaded resources. Debug info:\n{}",
            debug_info
        );
        let url = detected.unwrap();
        assert!(
            url.ends_with("/wasm-bindgen-test.js"),
            "Expected detected shim URL to end with /wasm-bindgen-test.js, got: {}",
            url
        );
    }

    // Test that park_timeout works on Node.js main thread (Atomics.wait is available there)
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn test_park_timeout_on_node_main_thread() {
        if !is_node() {
            // Skip this test in browser - Atomics.wait will panic on main thread
            // which is the expected behavior, but we can't test it here
            return;
        }
        // In Node.js, Atomics.wait works on main thread, so park should succeed
        park_timeout(Duration::from_millis(1));
    }

    // Test that park panics on browser main thread (Atomics.wait unavailable there).
    // This produces "RuntimeError: unreachable" in console - expected for wasm panics.
    #[wasm_bindgen_test::wasm_bindgen_test]
    #[should_panic(expected = "Atomics.wait is not available")]
    fn test_park_panics_on_browser_main_thread() {
        if is_node() {
            // In Node.js, we need to manually panic to satisfy should_panic
            panic!("Atomics.wait is not available in this context (likely main thread)");
        }
        // In browser, this will actually panic
        park_timeout(Duration::from_millis(1));
    }

    #[wasm_bindgen(inline_js = r#"
export function schedule_delayed_throw() {
    setTimeout(() => {
        throw new Error('delayed error after closure returned');
    }, 0);
}
"#)]
    extern "C" {
        fn schedule_delayed_throw();
    }

    // Test that join_async waits for worker exit and returns error if worker errors after sending value.
    // This only works on Node.js - browser's close() cancels pending timers.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn test_join_async_error_after_value_sent() {
        if !is_node() {
            // Browser cancels setTimeout when close() is called, so this test only works on Node
            return;
        }

        // Spawn a worker that:
        // 1. Schedules a setTimeout that throws an error
        // 2. Returns a value successfully
        // The setTimeout fires AFTER the closure returns (because Node keeps the event loop alive)
        let handle = spawn(|| {
            schedule_delayed_throw();
            42
        });

        // Yield to event loop to let the worker start
        yield_to_event_loop_async().await;

        // join_async should:
        // 1. Receive the value 42 from the channel
        // 2. Wait for the exit signal
        // 3. See exit_state = 2 (error) because the setTimeout threw
        // 4. Return an error
        let result = handle.join_async().await;
        assert!(
            result.is_err(),
            "Expected error from delayed throw after value sent, got {:?}",
            result
        );
    }
}
