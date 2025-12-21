//! WebAssembly backend - placeholder implementation

use std::fmt;
use std::io;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::num::NonZeroUsize;
use std::time::Duration;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[wasm_bindgen]
extern "C" {
    // --- Worker ---
    type Worker;

    #[wasm_bindgen(constructor, js_class = "Worker")]
    fn new(url: &str, options: &JsValue) -> Worker;

    #[wasm_bindgen(method, js_name = postMessage)]
    fn post_message(this: &Worker, msg: &JsValue);

    #[wasm_bindgen(method)]
    fn terminate(this: &Worker);

    #[wasm_bindgen(method, setter)]
    fn set_onmessage(this: &Worker, cb: Option<&js_sys::Function>);

    // --- Blob ---
    type Blob;

    // new Blob(parts, options)
    #[wasm_bindgen(constructor, js_class = "Blob")]
    fn new(parts: &JsValue, options: &JsValue) -> Blob;

    // --- URL ---
    #[wasm_bindgen(js_namespace = URL, js_name = createObjectURL)]
    fn create_object_url(blob: &Blob) -> String;

    #[wasm_bindgen(js_namespace = URL, js_name = revokeObjectURL)]
    fn revoke_object_url(url: &str);

    // --- console.log (optional) ---
    #[wasm_bindgen(js_namespace = console, js_name = log)]
    fn console_log(a: &JsValue);
}

fn log_str(s: &str) {
    console_log(&JsValue::from_str(s));
}

fn blob_url_from_js_source(js_source: &str) -> String {
    // parts = [ "....js source..." ]
    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(js_source));

    // options = { type: "text/javascript" }
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(
        &opts,
        &JsValue::from_str("type"),
        &JsValue::from_str("text/javascript"),
    )
        .unwrap();

    let blob = Blob::new(&parts.into(), &opts.into());
    create_object_url(&blob)
}

pub struct WorkerHandle {
    worker: Worker,
    _onmessage: Closure<dyn FnMut(JsValue)>,
    _url: String,
}

impl WorkerHandle {
    pub fn post(&self, msg: &JsValue) {
        self.worker.post_message(msg);
    }

    pub fn terminate(self) {
        self.worker.terminate();
        // drop -> callback dropped; url revoked in Drop
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        // URL is only needed for initial load; safe to revoke after creation.
        revoke_object_url(&self._url);
    }
}

fn spawn_module_worker_from_source(
    name: &str,
    js_source: &str,
    mut on_msg: impl FnMut(JsValue) + 'static,
) -> WorkerHandle {
    let url = blob_url_from_js_source(js_source);

    // options = { type: "module" }
    let options = js_sys::Object::new();
    js_sys::Reflect::set(
        &options,
        &JsValue::from_str("type"),
        &JsValue::from_str("module"),
    )
        .unwrap();
    js_sys::Reflect::set(
        &options,
        &JsValue::from_str("name"),
        &JsValue::from_str(&name),
    )
        .unwrap();

    let worker = Worker::new(&url, &options.into());

    let cb = Closure::wrap(Box::new(move |data: JsValue| {
        on_msg(data);
    }) as Box<dyn FnMut(JsValue)>);

    worker.set_onmessage(Some(cb.as_ref().unchecked_ref()));

    WorkerHandle {
        worker,
        _onmessage: cb,
        _url: url,
    }
}

fn make_worker_script_url(shim_url: &str) -> String {
    // Module worker: import init + your entrypoint from the shim URL we discovered.
    let script = format!(
        r#"
import init, {{ wasm_safe_thread_entry_point }} from "{shim_url}";

self.onmessage = (event) => {{
  let [module, memory, work] = event.data;

  init(module, memory).catch(err => {{
    console.log(err);
    setTimeout(() => {{ throw err; }});
    throw err;
  }}).then(() => {{
    wasm_safe_thread_entry_point(work);
    close();
  }});
}};
"#
    );

    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(&script));

    let opts = js_sys::Object::new();
    js_sys::Reflect::set(&opts, &"type".into(), &"text/javascript".into()).unwrap();

    let blob = Blob::new(&parts.into(), &opts.into());
    create_object_url(&blob)
}
fn get_wasm_bindgen_shim_script_path() -> String {
// Returns the first captured URL in the stack trace.
let js = r#"
(function script_path() {
  try { throw new Error(); }
  catch (e) {
    let parts = (e.stack || "").match(/(?:\(|@)(\S+):\d+:\d+/);
    return parts && parts[1] ? parts[1] : "";
  }
})()
"#;

js_sys::eval(js).unwrap().as_string().unwrap_or_default()
}

fn spawn_module_worker_with_shared_module(work: JsValue) -> Worker {
    let shim_url = get_wasm_bindgen_shim_script_path();
    if shim_url.is_empty() {
        panic!("Could not discover wasm-bindgen shim URL via stack trace");
    }
    log_str(&format!("shim url: {shim_url}"));

    let worker_script_url = make_worker_script_url(&shim_url);

    // options = { type: "module" }
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &"type".into(), &"module".into()).unwrap();

    let worker = Worker::new(&worker_script_url, &options.into());

    // Script URL only needed for construction; revoke immediately.
    revoke_object_url(&worker_script_url);

    // Send [module, memory, work] exactly like wasm_thread.
    // These are provided by wasm-bindgen at runtime.
    let arr = js_sys::Array::new();
    arr.push(&wasm_bindgen::module());
    arr.push(&wasm_bindgen::memory());
    arr.push(&work);

    worker.post_message(&arr.into());
    worker
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
    _marker: std::marker::PhantomData<T>,
}

impl<T> JoinHandle<T> {
    /// Waits for the thread to finish and returns its result.
    pub fn join(self) -> Result<T, Box<dyn std::any::Any + Send + 'static>> {
        panic!("join not implemented");
        // log_str("todo: join");
        // //shitty UB
        // unsafe { MaybeUninit::zeroed().assume_init() }
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
pub fn spawn<F, T>(_f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{


    // You must provide *absolute* URLs here (served by your app)
    spawn_module_worker_with_shared_module(wasm_bindgen::module());
    JoinHandle {
        _marker: PhantomData,
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
