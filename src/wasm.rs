//! WebAssembly backend - placeholder implementation

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen(inline_js = r#"
const SELF_URL = import.meta.url;

function normUrl(u) {
  // Strip trailing ":line:col" if present
  let s = u.replace(/\):\d+:\d+$/, ")"); // paranoia
  s = s.replace(/:\d+:\d+$/, "");
  return s.replace(/[)\s]+$/, "");
}

function collectUrlsFromStack(stack) {
  const out = [];
  const lines = (stack || "").split(/\r?\n/);

  for (const line of lines) {
    // Firefox/Safari: "name@URL:line:col" or "@URL:line:col"
    let m = line.match(/@(\S+):\d+:\d+/);
    if (m && m[1]) out.push(normUrl(m[1]));

    // Chrome: "(URL:line:col)"
    m = line.match(/\((\S+):\d+:\d+\)/);
    if (m && m[1]) out.push(normUrl(m[1]));

    // Node style paths also show up sometimes; ignore here (node handled elsewhere)
  }
  return out;
}

function discoverShimUrl() {
  const stack = (new Error()).stack || "";
  const urls = collectUrlsFromStack(stack);

  // Filter out our own inline snippet module (and other snippet modules if you want)
  const filtered = urls.filter(u => u && u !== SELF_URL);

  // Prefer wasm-bindgen-test harness module if present
  for (const u of filtered) {
    if (u.endsWith("/wasm-bindgen-test") || u.includes("/wasm-bindgen-test?")) return u;
  }

  // Otherwise: first non-snippet http(s) URL
  for (const u of filtered) {
    if ((u.startsWith("http://") || u.startsWith("https://")) && !u.includes("/snippets/")) return u;
  }

  // Otherwise just the first remaining candidate
  return filtered[0] || "";
}

// --- wasm-bindgen-test / wasm-bindgen export-shape handling ---

function pickExportContainer(m) {
  // wasm-bindgen can put exports directly on the module namespace (m),
  // or on the default export (function/object), or on `wasm_bindgen`.
  if (m && typeof m.wasm_bindgen === "function") return m.wasm_bindgen;
  if (m && typeof m.default === "function") return m.default;
  if (m && m.default && typeof m.default === "object") return m.default;
  return m;
}

function pickInit(m) {
  // Common ESM: default export is init()
  if (m && typeof m.default === "function") return m.default;

  // wasm-bindgen-test / some targets: __wbg_init named export
  if (m && typeof m.__wbg_init === "function") return m.__wbg_init;

  // Node dynamic-import of CJS: namespace.default is module.exports object
  if (m && m.default && typeof m.default.__wbg_init === "function") return m.default.__wbg_init;
  if (m && m.default && typeof m.default.default === "function") return m.default.default;

  // Older global-style: wasm_bindgen function is the init
  if (m && typeof m.wasm_bindgen === "function") return m.wasm_bindgen;
  if (m && m.default && typeof m.default.wasm_bindgen === "function") return m.default.wasm_bindgen;

  return null;
}

function pickEntry(m, name) {
  if (m && typeof m[name] === "function") return m[name];

  const c = pickExportContainer(m);
  if (c && typeof c[name] === "function") return c[name];

  // Sometimes on Node/CJS interop exports live under m.default
  if (m && m.default && typeof m.default[name] === "function") return m.default[name];

  return null;
}

async function loadShim(spec, entryName) {
  const m = await import(spec);
  const init = pickInit(m);
  const entry = pickEntry(m, entryName);

  if (!init || !entry) {
    const keys = m ? Object.keys(m) : [];
    const defKeys = (m && m.default && (typeof m.default === "object" || typeof m.default === "function"))
      ? Object.keys(m.default)
      : [];
    throw new Error(
      "Could not find init() and/or " + entryName +
      " in shim module. keys=" + JSON.stringify(keys) +
      " defaultKeys=" + JSON.stringify(defKeys) +
      " spec=" + spec
    );
  }

  return { init, entry };
}

// --- Worker script generation ---

function makeCommonHelperScript(entryName) {
  return `
    function pickExportContainer(m) {
      if (m && typeof m.wasm_bindgen === "function") return m.wasm_bindgen;
      if (m && typeof m.default === "function") return m.default;
      if (m && m.default && typeof m.default === "object") return m.default;
      return m;
    }

    function pickInit(m) {
      if (m && typeof m.default === "function") return m.default;
      if (m && typeof m.__wbg_init === "function") return m.__wbg_init;
      if (m && m.default && typeof m.default.__wbg_init === "function") return m.default.__wbg_init;
      if (m && m.default && typeof m.default.default === "function") return m.default.default;
      if (m && typeof m.wasm_bindgen === "function") return m.wasm_bindgen;
      if (m && m.default && typeof m.default.wasm_bindgen === "function") return m.default.wasm_bindgen;
      return null;
    }

    function pickEntry(m, name) {
      if (m && typeof m[name] === "function") return m[name];
      const c = pickExportContainer(m);
      if (c && typeof c[name] === "function") return c[name];
      if (m && m.default && typeof m.default[name] === "function") return m.default[name];
      return null;
    }

    async function loadShim(spec) {
      const m = await import(spec);
      const init = pickInit(m);
      const entry = pickEntry(m, ${JSON.stringify(entryName)});

      if (!init || !entry) {
        const keys = m ? Object.keys(m) : [];
        const defKeys = (m && m.default && (typeof m.default === "object" || typeof m.default === "function"))
          ? Object.keys(m.default)
          : [];
        throw new Error(
          "Could not find init() and/or ${entryName} in shim module. " +
          "keys=" + JSON.stringify(keys) +
          " defaultKeys=" + JSON.stringify(defKeys) +
          " spec=" + spec
        );
      }
      return { init, entry };
    }
  `;
}

function makeBrowserWorkerScript(spec, entryName) {
  const helpers = makeCommonHelperScript(entryName);
  return `
    ${helpers}

    let cached;
    async function get() { return cached || (cached = loadShim(${JSON.stringify(spec)})); }

    self.onmessage = (e) => {
      (async () => {
        const [module, memory, work] = e.data;
        const { init, entry } = await get();
        await init(module, memory);
        entry(work);
        close();
      })().catch(err => {
        console.log(err);
        setTimeout(() => { throw err; });
        throw err;
      });
    };
  `;
}

function makeNodeWorkerScript(spec, entryName) {
  const helpers = makeCommonHelperScript(entryName);
  return `
    import { parentPort } from "node:worker_threads";
    ${helpers}

    let cached;
    async function get() { return cached || (cached = loadShim(${JSON.stringify(spec)})); }

    parentPort.on("message", (msg) => {
      (async () => {
        const [module, memory, work] = msg;
        const { init, entry } = await get();
        await init(module, memory);
        entry(work);
        parentPort.close();
      })().catch(err => {
        console.log(err);
        setTimeout(() => { throw err; });
        throw err;
      });
    });
  `;
}


function spawnWorkerUniversal(shimUrl, module, memory, work, entryName) {
  // Browser path
  if (typeof globalThis.Worker === "function") {
    const spec = shimUrl; // in browser, "/wasm-bindgen-test" or "http://..." is a valid module specifier
    const src = makeBrowserWorkerScript(spec, entryName);

    const blob = new Blob([src], { type: "text/javascript" });
    const url = URL.createObjectURL(blob);
    const w = new Worker(url, { type: "module" });
    URL.revokeObjectURL(url);

    w.postMessage([module, memory, work]);

    return {
      postMessage: (msg) => w.postMessage(msg),
      terminate: () => w.terminate(),
      setOnMessage: (cb) => { w.onmessage = (e) => cb(e.data); },
    };
  }

  // Node path (NO require; queue until imports resolve)
  if (typeof process !== "undefined" && process.versions && process.versions.node) {
    let ready = (async () => {
      const { Worker } = await import("node:worker_threads");
      const { pathToFileURL } = await import("node:url");

      // Node shimUrl is likely an absolute path like "/private/.../wasm-bindgen-test.js"
      const spec = (/^(https?:|file:|data:|blob:)/.test(shimUrl))
        ? shimUrl
        : pathToFileURL(shimUrl).href;

      const src = makeNodeWorkerScript(spec, entryName);
      const w = new Worker(src, { eval: true, type: "module" });

      w.postMessage([module, memory, work]);
      return w;
    })();

    let onMsg = null;

    return {
      postMessage: (msg) => { ready.then(w => w.postMessage(msg)); },
      terminate: () => { ready.then(w => w.terminate()); },
      setOnMessage: (cb) => {
        onMsg = (data) => cb(data);
        ready.then(w => w.on("message", onMsg));
      },
    };
  }

  throw new Error("No Worker (browser) or worker_threads (Node) available");
}

export function wasm_safe_thread_spawn_worker(work, module, memory) {
  const shim = discoverShimUrl();
  if (!shim) throw new Error("Could not discover shim URL via stack trace");

  // Hardcode the Rust export name you want to call inside worker
  const entryName = "wasm_safe_thread_entry_point";
  return spawnWorkerUniversal(shim, module, memory, work, entryName);
}
"#)]
extern "C" {
    fn wasm_safe_thread_spawn_worker(work: JsValue, module: JsValue, memory: JsValue) -> WorkerLike;

    type WorkerLike;
    #[wasm_bindgen(method, js_name = postMessage)]
    fn post_message(this: &WorkerLike, msg: &JsValue);
    #[wasm_bindgen(method)]
    fn terminate(this: &WorkerLike);
    #[wasm_bindgen(method, js_name = setOnMessage)]
    fn set_on_message(this: &WorkerLike, cb: &js_sys::Function);
}



#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn console_log_js(v: &JsValue);
}

fn log_str(s: &str) {
    console_log_js(&JsValue::from_str(s));
}


pub struct WorkerHandle {
    worker: WorkerLike,
    _onmessage: Closure<dyn FnMut(JsValue)>,
}

impl WorkerHandle {
    pub fn post(&self, msg: &JsValue) {
        self.worker.post_message(msg);
    }
    pub fn terminate(self) {
        self.worker.terminate();
    }
}

pub fn spawn_with_shared_module(work: JsValue, mut on_msg: impl FnMut(JsValue) + 'static) -> WorkerHandle {
    // Note: wasm-bindgen exposes these in both browser and node hosts.
    let worker = wasm_safe_thread_spawn_worker(work, wasm_bindgen::module(), wasm_bindgen::memory());

    let cb = Closure::wrap(Box::new(move |data: JsValue| {
        on_msg(data);
    }) as Box<dyn FnMut(JsValue)>);

    worker.set_on_message(cb.as_ref().unchecked_ref());

    WorkerHandle { worker, _onmessage: cb }
}

#[wasm_bindgen]
pub fn wasm_safe_thread_entry_point(work: JsValue) {
    log_str("hi");
    // `work` can be a pointer, an index into a table, etc.
    // For now:
    let _ = work;
}

/// A thread local storage key which owns its contents.
pub struct LocalKey<T: 'static> {
    _marker: PhantomData<T>,
}

impl<T: 'static> LocalKey<T> {
    /// Creates a new `LocalKey`.
    #[doc(hidden)]
    pub const fn new(_init: fn() -> T) -> Self {
        LocalKey {
            _marker: PhantomData,
        }
    }

    /// Acquires a reference to the value in this TLS key.
    pub fn with<F, R>(&'static self, _f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        todo!("wasm LocalKey::with")
    }

    /// Acquires a reference to the value in this TLS key.
    ///
    /// Returns `Err(AccessError)` if the key is being destroyed or was already destroyed.
    pub fn try_with<F, R>(&'static self, _f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        todo!("wasm LocalKey::try_with")
    }
}

impl<T: 'static> fmt::Debug for LocalKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalKey").finish_non_exhaustive()
    }
}

// SAFETY: LocalKey is Sync because each thread accesses its own storage.
// The key itself is just an accessor, not the actual storage.
unsafe impl<T: 'static> Sync for LocalKey<T> {}

/// An error returned by [`LocalKey::try_with`].
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
}

impl<T> JoinHandle<T> {
    /// Waits for the thread to finish and returns its result.
    pub fn join(self) -> Result<T, Box<dyn std::any::Any + Send + 'static>> {
        self.receiver.recv_sync().map_err(|e| Box::new(e) as Box<dyn std::any::Any + Send + 'static>)
    }

    /// Gets the thread associated with this handle.
    pub fn thread(&self) -> &Thread {
        todo!("wasm JoinHandle::thread")
    }

    /// Checks if the thread has finished running.
    pub fn is_finished(&self) -> bool {
        todo!("wasm JoinHandle::is_finished")
    }
}

/// A handle to a thread.
#[derive(Clone)]
pub struct Thread {
    _private: (),
}

impl Thread {
    /// Gets the thread's unique identifier.
    pub fn id(&self) -> ThreadId {
        todo!("wasm Thread::id")
    }

    /// Gets the thread's name.
    pub fn name(&self) -> Option<&str> {
        todo!("wasm Thread::name")
    }

    /// Atomically makes the handle's token available if it is not already.
    pub fn unpark(&self) {
        todo!("wasm Thread::unpark")
    }
}

/// A unique identifier for a running thread.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ThreadId(u64);

/// A builder for configuring and spawning threads.
pub struct Builder {
    _name: Option<String>,
    _stack_size: Option<usize>,
    _spawn_hooks: Vec<Box<dyn FnOnce() + Send + 'static>>,
}

impl Builder {
    /// Creates a new thread builder.
    pub fn new() -> Self {
        Builder {
            _name: None,
            _stack_size: None,
            _spawn_hooks: Vec::new(),
        }
    }

    /// Sets the name of the thread.
    pub fn name(mut self, name: String) -> Self {
        self._name = Some(name);
        self
    }

    /// Sets the stack size for the new thread.
    pub fn stack_size(mut self, size: usize) -> Self {
        self._stack_size = Some(size);
        self
    }

    /// Registers a hook to run at the beginning of the spawned thread.
    ///
    /// Multiple hooks can be registered and they will run in the order they were added.
    pub fn spawn_hook<H>(mut self, hook: H) -> Self
    where
        H: FnOnce() + Send + 'static,
    {
        self._spawn_hooks.push(Box::new(hook));
        self
    }

    /// Spawns a new thread with this builder's configuration.
    pub fn spawn<F, T>(self, _f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        todo!("wasm Builder::spawn")
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawns a new thread, returning a JoinHandle for it.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (send,recv) = wasm_safe_mutex::mpsc::channel();
    let closure = || {
        let result = f();
        send.send_sync(result).unwrap();
    };


    // You must provide *absolute* URLs here (served by your app)
    spawn_with_shared_module(3.into(), |foo| {
        log_str("on message");
    });
    JoinHandle {
        receiver: recv,
    }
}

/// Gets a handle to the thread that invokes it.
pub fn current() -> Thread {
    todo!("wasm current")
}

/// Puts the current thread to sleep for at least the specified duration.
pub fn sleep(_dur: Duration) {
    todo!("wasm sleep")
}

/// Cooperatively gives up a timeslice to the OS scheduler.
pub fn yield_now() {
    todo!("wasm yield_now")
}

/// Blocks unless or until the current thread's token is made available.
pub fn park() {
    todo!("wasm park")
}

/// Blocks unless or until the current thread's token is made available
/// or the specified duration has been reached.
pub fn park_timeout(_dur: Duration) {
    todo!("wasm park_timeout")
}

/// Returns an estimate of the default amount of parallelism a program should use.
pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    todo!("wasm available_parallelism")
}
