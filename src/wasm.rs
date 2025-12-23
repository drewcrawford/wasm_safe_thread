//! WebAssembly backend - placeholder implementation

use std::fmt;
use std::io;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

static THREAD_COUNTER: AtomicU64 = AtomicU64::new(0);

std::thread_local! {
    static CURRENT_THREAD: std::cell::RefCell<Option<Thread>> = const { std::cell::RefCell::new(None) };
}

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen(
    inline_js = r#"
const SELF_URL = import.meta.url;

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

// Browser worker script - uses Web Worker API
function makeBrowserWorkerScript(shimUrl, entryName) {
  return `
    self.onmessage = async (e) => {
      try {
        const [module, memory, work] = e.data;
        console.log("Worker received:", { module, memory, work });
        console.log("Memory buffer:", memory.buffer);
        console.log("Memory buffer byteLength:", memory.buffer.byteLength);
        console.log("Is SharedArrayBuffer:", memory.buffer instanceof SharedArrayBuffer);

        // Cache-bust to get fresh module state per worker (needed for Safari)
        const url = ${JSON.stringify(shimUrl)} + '?worker=' + Math.random();
        const shim = await import(url);
        console.log("Shim loaded, calling initSync");

        // Use initSync with module and memory from main thread
        shim.initSync({ module, memory, thread_stack_size: 1048576 });
        console.log("initSync complete");

        // Call the entry point
        shim[${JSON.stringify(entryName)}](work);
        close();
      } catch (err) {
        console.error(err);
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

export function wasm_safe_thread_spawn_worker(work, module, memory, name, shimName) {
  const shimUrl = getShimUrl(shimName);
  console.log("Shim URL:", shimUrl);
  const entryName = "wasm_safe_thread_entry_point";

  // Browser: use Web Worker API
  if (typeof Worker === "function" && !isNode()) {
    const src = makeBrowserWorkerScript(shimUrl, entryName);
    const blob = new Blob([src], { type: "text/javascript" });
    const blobUrl = URL.createObjectURL(blob);
    const w = new Worker(blobUrl, { type: "module", name });
    URL.revokeObjectURL(blobUrl);
    w.postMessage([module, memory, work]);

    return {
      postMessage: (msg) => w.postMessage(msg),
      terminate: () => w.terminate(),
      setOnMessage: (cb) => { w.onmessage = (e) => cb(e.data); },
    };
  }

  // Node: use worker_threads
  if (isNode()) {
    const ready = (async () => {
      const { Worker } = await import("node:worker_threads");
      const src = makeNodeWorkerScript(shimUrl, entryName);
      const w = new Worker(src, { eval: true, type: "module", name });
      w.postMessage([module, memory, work]);
      return w;
    })();

    return {
      postMessage: (msg) => ready.then(w => w.postMessage(msg)),
      terminate: () => ready.then(w => w.terminate()),
      setOnMessage: (cb) => ready.then(w => w.on("message", cb)),
    };
  }

  throw new Error("No Worker API available");
}
"#
)]
extern "C" {
    fn wasm_safe_thread_spawn_worker(work: JsValue, module: JsValue, memory: JsValue, name: &str, shim_name: &str) -> WorkerLike;

    type WorkerLike;

    #[wasm_bindgen(method, js_name = postMessage)]
    fn post_message(this: &WorkerLike, msg: &JsValue);

    #[wasm_bindgen(method)]
    fn terminate(this: &WorkerLike);

    #[wasm_bindgen(method, js_name = setOnMessage)]
    fn set_on_message(this: &WorkerLike, cb: &js_sys::Function);
}

#[wasm_bindgen(
    inline_js = r#"
export function is_main_thread() {

const isNode =
    typeof process !== "undefined" &&
    process?.versions?.node != null &&
    process?.release?.name === "node";

  if (!isNode) {
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
"#
)]
extern "C" {
    fn is_main_thread() -> bool;
    fn atomics_wait_timeout_ms_try(timeout_ms: f64) -> JsValue;
    fn sleep_sync_ms(ms: f64);
    fn get_available_parallelism() -> u32;
}




#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn console_log_js(v: &JsValue);
}

fn log_str(s: &str) {
    console_log_js(&JsValue::from_str(s));
}

#[allow(dead_code)]
pub struct WorkerHandle {
    worker: WorkerLike,
    _onmessage: Closure<dyn FnMut(JsValue)>,
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

pub fn spawn_with_shared_module(work: JsValue, name: &str, shim_name: &str, mut on_msg: impl FnMut(JsValue) + 'static) -> WorkerHandle {
    let worker = wasm_safe_thread_spawn_worker(work, wasm_bindgen::module(), wasm_bindgen::memory(), name, shim_name);

    let cb = Closure::wrap(Box::new(move |data: JsValue| {
        on_msg(data);
    }) as Box<dyn FnMut(JsValue)>);

    worker.set_on_message(cb.as_ref().unchecked_ref());

    WorkerHandle {
        worker,
        _onmessage: cb,
    }
}

#[wasm_bindgen]
pub fn wasm_safe_thread_entry_point(work: JsValue) {
    log_str("entry_point: start");

    let ptr = work.as_f64().unwrap() as usize;
    log_str(&format!("entry_point: ptr = {:#x}", ptr));

    // SAFETY: ptr came from Box::into_raw in spawn(), and we're the only consumer
    log_str("entry_point: about to Box::from_raw");
    let boxed = unsafe { Box::from_raw(ptr as *mut Box<dyn FnOnce() + Send>) };
    log_str("entry_point: got boxed");
    let closure = *boxed;
    log_str("entry_point: got closure, about to call");
    closure();
    log_str("entry_point: closure complete");
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
    receiver: wasm_safe_mutex::mpsc::Receiver<T>,
    thread: Thread,
}

impl<T> JoinHandle<T> {
    pub fn join(self) -> Result<T, Box<String>> {
        if is_main_thread() {
            return Err(Box::new("Can't join from the main thread on wasm32".to_string()))
        }
        self.receiver
            .recv_sync()
            .map_err(|e| Box::new(format!("{:?}", e)) as Box<String>)
    }

    pub async fn join_async(self) -> Result<T, Box<String>> {
        self.receiver
            .recv_async()
            .await
            .map_err(|e| Box::new(format!("{:?}", e)) as Box<String>)
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    pub fn is_finished(&self) -> bool {
        todo!("wasm JoinHandle::is_finished")
    }
}

#[derive(Clone)]
pub struct Thread {
    inner: Arc<ThreadInner>,
}

struct ThreadInner {
    name: Option<String>,
}

impl Thread {
    pub fn id(&self) -> ThreadId {
        todo!("wasm Thread::id")
    }

    pub fn name(&self) -> Option<&str> {
        self.inner.name.as_deref()
    }

    pub fn unpark(&self) {
        todo!("wasm Thread::unpark")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ThreadId(u64);

pub struct Builder {
    _name: Option<String>,
    _stack_size: Option<usize>,
    _spawn_hooks: Vec<Box<dyn FnOnce() + Send + 'static>>,
    _shim_name: Option<String>,
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            _name: None,
            _stack_size: None,
            _spawn_hooks: Vec::new(),
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

    pub fn spawn_hook<H>(mut self, hook: H) -> Self
    where
        H: FnOnce() + Send + 'static,
    {
        self._spawn_hooks.push(Box::new(hook));
        self
    }

    pub fn spawn<F, T>(self, f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        // Worker name: use explicit name or generate a default for JS devtools
        let worker_name = self._name.clone().unwrap_or_else(|| {
            let id = THREAD_COUNTER.fetch_add(1, Ordering::Relaxed);
            format!("wasm_safe_thread {}", id)
        });

        // Create Thread before spawning so we can pass it to the worker
        // Thread::name() only returns Some if explicitly set via Builder::name()
        let thread = Thread {
            inner: Arc::new(ThreadInner { name: self._name }),
        };
        let thread_for_worker = thread.clone();

        let (send, recv) = wasm_safe_mutex::mpsc::channel();
        let closure = move || {
            // Set up TLS for current() before running user code
            CURRENT_THREAD.with(|cell| {
                *cell.borrow_mut() = Some(thread_for_worker);
            });

            let result = f();
            // Ignore send errors - receiver may have been dropped if JoinHandle wasn't joined
            let _ = send.send_sync(result);
        };

        // Double-box to get a thin pointer (Box<dyn FnOnce()> is a fat pointer)
        let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(closure));
        let ptr = Box::into_raw(boxed) as *mut () as usize;
        let work: JsValue = (ptr as f64).into();

        // Default shim name to "wasm-bindgen-test" for tests - override with Builder::shim_name() for examples/binaries
        let shim_name = self._shim_name.as_deref().unwrap_or("wasm-bindgen-test");

        // The on_msg callback isn't used yet; Worker closes itself after running entrypoint.
        spawn_with_shared_module(work, &worker_name, shim_name, |_| {
            log_str("on message");
        });

        Ok(JoinHandle {
            receiver: recv,
            thread,
        })
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

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
            let thread = Thread {
                inner: Arc::new(ThreadInner { name }),
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

pub fn park() {
    todo!("wasm park")
}

pub fn park_timeout(_dur: Duration) {
    todo!("wasm park_timeout")
}

pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    let count = get_available_parallelism();
    NonZeroUsize::new(count as usize).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "could not determine available parallelism")
    })
}


#[cfg(test)] mod tests {
    #[wasm_bindgen_test::wasm_bindgen_test] fn is_main_thread() {
        assert!(super::is_main_thread());
    }


}