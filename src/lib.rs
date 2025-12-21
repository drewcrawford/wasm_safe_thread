//! A library that provides std::thread-like API across wasm and std platforms.

#[cfg(not(target_arch = "wasm32"))]
mod stdlib;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
use stdlib as backend;
#[cfg(target_arch = "wasm32")]
use wasm as backend;

use std::io;
use std::num::NonZeroUsize;
use std::time::Duration;

pub use backend::{Builder, JoinHandle, Thread, ThreadId};

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

    #[test]
    fn test_spawn_and_join() {
        let handle = spawn(|| 42);
        let result = handle.join().unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_builder() {
        let handle = Builder::new()
            .name("test-thread".to_string())
            .spawn(|| "hello")
            .unwrap();
        let result = handle.join().unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_current_thread() {
        let _current = current();
    }

    #[test]
    fn test_yield_now() {
        yield_now();
    }

    #[test]
    fn test_sleep() {
        sleep(Duration::from_millis(1));
    }

    #[test]
    fn test_available_parallelism() {
        let parallelism = available_parallelism().unwrap();
        println!("available_parallelism: {}", parallelism);
        assert!(parallelism.get() >= 1);
    }
}
