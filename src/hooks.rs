//! Global spawn hook registration system.
//!
//! Spawn hooks are functions that run at the beginning of every spawned thread,
//! before the main thread function. Hooks run in registration order.

use std::sync::Arc;
use wasm_safe_mutex::Mutex;

type SpawnHookFn = Arc<dyn Fn() + Send + Sync + 'static>;

static SPAWN_HOOKS: Mutex<Vec<(String, SpawnHookFn)>> = Mutex::new(Vec::new());

/// Registers a global spawn hook with the given name.
///
/// If a hook with this name already exists, it is replaced while maintaining
/// its original position in the execution order.
///
/// # Example
///
/// ```
/// use wasm_safe_thread::register_spawn_hook;
///
/// register_spawn_hook("my_hook", || {
///     println!("Thread starting!");
/// });
/// ```
pub fn register_spawn_hook<F>(name: impl Into<String>, hook: F)
where
    F: Fn() + Send + Sync + 'static,
{
    let name = name.into();
    let hook: SpawnHookFn = Arc::new(hook);

    let mut hooks = SPAWN_HOOKS.lock_sync();

    // Check if hook with this name already exists
    if let Some(pos) = hooks.iter().position(|(n, _)| n == &name) {
        // Replace in place to maintain order
        hooks[pos] = (name, hook);
    } else {
        hooks.push((name, hook));
    }
}

/// Removes a spawn hook by name.
///
/// Returns `true` if a hook was removed, `false` if no hook with that name existed.
pub fn remove_spawn_hook(name: &str) -> bool {
    let mut hooks = SPAWN_HOOKS.lock_sync();
    if let Some(pos) = hooks.iter().position(|(n, _)| n == name) {
        hooks.remove(pos);
        true
    } else {
        false
    }
}

/// Removes all registered spawn hooks.
pub fn clear_spawn_hooks() {
    let mut hooks = SPAWN_HOOKS.lock_sync();
    hooks.clear();
}

/// Runs all registered spawn hooks in order.
///
/// This is called internally by the spawn implementations before running
/// the main thread function.
pub(crate) fn run_spawn_hooks() {
    // Clone the hooks so we don't hold the lock while calling them
    // This prevents deadlock if a hook tries to register another hook
    let hooks: Vec<SpawnHookFn> = {
        let guard = SPAWN_HOOKS.lock_sync();
        guard.iter().map(|(_, hook)| Arc::clone(hook)).collect()
    };

    for hook in hooks {
        hook();
    }
}
