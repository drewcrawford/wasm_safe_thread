//! A library that provides std::thread-like API across wasm and std platforms.

extern crate alloc;

#[cfg(not(target_arch = "wasm32"))]
mod stdlib;
#[cfg(target_arch = "wasm32")]
mod wasm;
mod test_executor;

#[cfg(not(target_arch = "wasm32"))]
use stdlib as backend;
#[cfg(target_arch = "wasm32")]
use wasm as backend;

use std::io;
use std::num::NonZeroUsize;
use std::time::Duration;

pub use backend::{AccessError, Builder, JoinHandle, LocalKey, Thread, ThreadId};

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

    #[test]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(target_arch = "wasm32", should_panic)] //can't join from the main thread
    fn test_spawn_and_join() {
        let handle = spawn(|| 42);
        let result = handle.join().unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn zzzpanic_hook() {
        console_error_panic_hook::set_once();

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

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    async fn test_spawn_and_join_async() {
        let handle = spawn(|| 42);
        let result = handle.join_async().await.unwrap();
        assert_eq!(result, 42);
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

            // Main thread should have name "main"
            let main_thread = current();
            assert_eq!(main_thread.name(), Some("main"));

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

    #[test]
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

    #[test]
    fn test_spawn_hook_single() {
        use std::sync::{Arc, Mutex};

        let order = Arc::new(Mutex::new(Vec::new()));
        let order_clone = order.clone();
        let order_main = order.clone();

        let handle = Builder::new()
            .spawn_hook(move || {
                order_clone.lock().unwrap().push(1);
            })
            .spawn(move || {
                order_main.lock().unwrap().push(2);
            })
            .unwrap();

        handle.join().unwrap();

        let result = order.lock().unwrap();
        assert_eq!(*result, vec![1, 2], "hook should run before main function");
    }

    #[test]
    fn test_spawn_hook_multiple_in_order() {
        use std::sync::{Arc, Mutex};

        let order = Arc::new(Mutex::new(Vec::new()));
        let o1 = order.clone();
        let o2 = order.clone();
        let o3 = order.clone();
        let o_main = order.clone();

        let handle = Builder::new()
            .spawn_hook(move || {
                o1.lock().unwrap().push(1);
            })
            .spawn_hook(move || {
                o2.lock().unwrap().push(2);
            })
            .spawn_hook(move || {
                o3.lock().unwrap().push(3);
            })
            .spawn(move || {
                o_main.lock().unwrap().push(100);
            })
            .unwrap();

        handle.join().unwrap();

        let result = order.lock().unwrap();
        assert_eq!(
            *result,
            vec![1, 2, 3, 100],
            "hooks should run in order before main function"
        );
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

}
