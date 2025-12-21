//! WebAssembly backend - placeholder implementation

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::num::NonZeroUsize;
use std::time::Duration;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen(
    inline_js = r#"
const SELF_URL = import.meta.url;

function stripLineCol(s) {
  return s.replace(/:\d+:\d+$/, "").replace(/[)\s]+$/, "");
}

function urlsFromStack(stack) {
  const out = [];
  for (const line of (stack || "").split(/\r?\n/)) {
    // Firefox/Safari: "name@URL:line:col" or "@URL:line:col"
    let m = line.match(/@(\S+):\d+:\d+/);
    if (m && m[1]) out.push(stripLineCol(m[1]));

    // Chrome/Node ESM: "(URL:line:col)"
    m = line.match(/\((\S+):\d+:\d+\)/);
    if (m && m[1]) out.push(stripLineCol(m[1]));

    // Node sometimes: "at file:///...:line:col" (no parens)
    m = line.match(/\s(file:\/\/\/\S+):\d+:\d+$/);
    if (m && m[1]) out.push(stripLineCol(m[1]));

    // Node CJS sometimes: "at /abs/path/file.js:line:col"
    m = line.match(/\s(\/\S+):\d+:\d+$/);
    if (m && m[1]) out.push(stripLineCol(m[1]));
  }
  return out;
}

function isNode() {
  return typeof process !== "undefined" && process.versions && process.versions.node;
}

function deriveHarnessFromSelfUrl(selfUrl) {
  // Works for:
  //   file:///.../snippets/<crate-hash>/inline0.js  -> file:///.../wasm-bindgen-test.js
  //   http://.../snippets/<crate-hash>/inline0.js   -> http://.../wasm-bindgen-test
  const idx = selfUrl.indexOf("/snippets/");
  if (idx === -1) return "";

  const base = selfUrl.slice(0, idx);
  return isNode()
    ? base + "/wasm-bindgen-test.js"
    : base + "/wasm-bindgen-test";
}

function discoverShimUrl() {
  const stack = (new Error()).stack || "";
  const urls = urlsFromStack(stack);

  // 1) Prefer the harness if it appears anywhere
  for (const u of urls) {
    if (u.includes("wasm-bindgen-test")) return u;
  }

  // 2) Node: derive harness from our own snippet location
  if (isNode()) {
    const derived = deriveHarnessFromSelfUrl(SELF_URL);
    if (derived) return derived;
  }

  // 3) Browser: skip self (so we don’t pick inline0.js), prefer non-snippet URL
  const filtered = urls.filter(u => u && u !== SELF_URL);
  for (const u of filtered) {
    if ((u.startsWith("http://") || u.startsWith("https://")) && !u.includes("/snippets/")) return u;
  }

  // 4) Last resort: something (even self) so we fail later with good debug
  return filtered[0] || SELF_URL || "";
}

// --- Worker script generation ---
//
// CRITICAL: For your "pass a raw pointer into shared wasm memory" approach to work,
// the worker MUST be initialized with the exact (module, memory) pair passed from the parent.
// Therefore we do NOT treat init as optional here. If we can't find a compatible init, we throw.

function makeCommonHelperScript(entryName) {
  return `
    function pickInitStrict(m) {
      // wasm-bindgen-test (sometimes): named __wbg_init
      if (m && typeof m.__wbg_init === "function") return m.__wbg_init;

      // wasm-pack/web style: default export is init
      if (m && typeof m.default === "function") return m.default;

      // Node ESM importing a CJS harness: namespace.default is module.exports object
      if (m && m.default && typeof m.default.__wbg_init === "function") return m.default.__wbg_init;
      if (m && m.default && typeof m.default.default === "function") return m.default.default;
      if (m && m.default && typeof m.default === "function") return m.default;

      // Older global-style
      if (m && typeof m.wasm_bindgen === "function") return m.wasm_bindgen;
      if (m && m.default && typeof m.default.wasm_bindgen === "function") return m.default.wasm_bindgen;

      return null;
    }

    function pickEntry(m, name) {
      if (m && typeof m[name] === "function") return m[name];
      if (m && m.default && typeof m.default[name] === "function") return m.default[name];
      return null;
    }

    async function loadShim(spec) {
      const m = await import(spec);

      const init = pickInitStrict(m);
      const entry = pickEntry(m, ${JSON.stringify(entryName)});

      const keys = m ? Object.keys(m) : [];
      const defKeys = (m && m.default && (typeof m.default === "object" || typeof m.default === "function"))
        ? Object.keys(m.default)
        : [];

      if (!entry) {
        throw new Error(
          "Missing entrypoint ${entryName} in shim module. " +
          "keys=" + JSON.stringify(keys) +
          " defaultKeys=" + JSON.stringify(defKeys) +
          " spec=" + spec
        );
      }

      if (!init) {
        // For THIS library, we require an init we can call with (module, memory),
        // otherwise the raw-pointer scheme is not reliable.
        throw new Error(
          "Missing wasm-bindgen init function in shim module (need init(module, memory)). " +
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

        // STRICT: do not swallow errors; we need shared (module, memory) to be wired up.
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

        // STRICT: do not swallow errors
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
    const spec = shimUrl; // "/wasm-bindgen-test" or "http://..." is valid in browser
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
  if (isNode()) {
    let ready = (async () => {
      const { Worker } = await import("node:worker_threads");
      const { pathToFileURL } = await import("node:url");

      const spec = (/^(https?:|file:|data:|blob:)/.test(shimUrl))
        ? shimUrl
        : pathToFileURL(shimUrl).href;

      const src = makeNodeWorkerScript(spec, entryName);
      const w = new Worker(src, { eval: true, type: "module" });

      w.postMessage([module, memory, work]);
      return w;
    })();

    return {
      postMessage: (msg) => { ready.then(w => w.postMessage(msg)); },
      terminate: () => { ready.then(w => w.terminate()); },
      setOnMessage: (cb) => {
        ready.then(w => w.on("message", (data) => cb(data)));
      },
    };
  }

  throw new Error("No Worker (browser) or worker_threads (Node) available");
}

export function wasm_safe_thread_spawn_worker(work, module, memory) {
  const shim = discoverShimUrl();
  if (!shim) throw new Error("Could not discover shim URL via stack trace");

  const entryName = "wasm_safe_thread_entry_point";
  return spawnWorkerUniversal(shim, module, memory, work, entryName);
}
"#
)]
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
    let worker = wasm_safe_thread_spawn_worker(work, wasm_bindgen::module(), wasm_bindgen::memory());

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
    log_str("hi");

    let ptr = work.as_f64().unwrap() as usize;

    // SAFETY: ptr came from Box::into_raw in spawn(), and we're the only consumer
    let boxed = unsafe { Box::from_raw(ptr as *mut Box<dyn FnOnce() + Send>) };
    let closure = *boxed;
    closure();
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

    pub fn with<F, R>(&'static self, _f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        todo!("wasm LocalKey::with")
    }

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

unsafe impl<T: 'static> Sync for LocalKey<T> {}

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
    pub fn join(self) -> Result<T, Box<String>> {
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
        todo!("wasm JoinHandle::thread")
    }

    pub fn is_finished(&self) -> bool {
        todo!("wasm JoinHandle::is_finished")
    }
}

#[derive(Clone)]
pub struct Thread {
    _private: (),
}

impl Thread {
    pub fn id(&self) -> ThreadId {
        todo!("wasm Thread::id")
    }

    pub fn name(&self) -> Option<&str> {
        todo!("wasm Thread::name")
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
}

impl Builder {
    pub fn new() -> Self {
        Builder {
            _name: None,
            _stack_size: None,
            _spawn_hooks: Vec::new(),
        }
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

pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (send, recv) = wasm_safe_mutex::mpsc::channel();
    let closure = move || {
        let result = f();
        send.send_sync(result).unwrap();
    };

    // Double-box to get a thin pointer (Box<dyn FnOnce()> is a fat pointer)
    let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(closure));
    let ptr = Box::into_raw(boxed) as *mut () as usize;
    let work: JsValue = (ptr as f64).into();

    // The on_msg callback isn't used yet; Worker closes itself after running entrypoint.
    spawn_with_shared_module(work, |_| {
        log_str("on message");
    });

    JoinHandle { receiver: recv }
}

pub fn current() -> Thread {
    todo!("wasm current")
}

pub fn sleep(_dur: Duration) {
    todo!("wasm sleep")
}

pub fn yield_now() {
    todo!("wasm yield_now")
}

pub fn park() {
    todo!("wasm park")
}

pub fn park_timeout(_dur: Duration) {
    todo!("wasm park_timeout")
}

pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    todo!("wasm available_parallelism")
}
