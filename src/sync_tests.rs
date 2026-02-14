// SPDX-License-Identifier: MIT OR Apache-2.0
//! Tests for the wasm_safe_thread crate.

use crate::{Mutex, spinlock::Spinlock};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};
#[cfg(target_arch = "wasm32")]
use web_time::{Duration, Instant};

#[cfg(target_arch = "wasm32")]
use crate as thread;
use r#continue::continuation;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[test]
fn test_spinlock_basic() {
    let spinlock = Spinlock::new(42);
    let result = spinlock.with_mut(|data| {
        *data += 1;
        *data
    });
    assert_eq!(result, 43);
}

#[test_executors::async_test]
async fn test_spinlock_concurrent_access() {
    let spinlock = Arc::new(Spinlock::new(0));
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let spinlock = Arc::clone(&spinlock);
            let (c, r) = continuation();
            thread::spawn(move || {
                for _ in 0..100 {
                    spinlock.with_mut(|data| *data += 1);
                }
                c.send(());
            });
            r
        })
        .collect();

    for h in handles {
        h.await;
    }
    assert_eq!(spinlock.with_mut(|data| *data), 1000);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[test]
fn test_mutex_try_lock_success() {
    let mutex = Mutex::new(42);
    let guard = mutex.try_lock().unwrap();
    assert_eq!(*guard, 42);
}

#[test_executors::async_test]
async fn test_mutex_try_lock_contention() {
    //for the time being, wasm_thread only works in browser
    //see https://github.com/rustwasm/wasm-bindgen/issues/4534,
    //though we also need wasm_thread support.
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
    let mutex = Arc::new(Mutex::new(42));
    let guard = mutex.try_lock().unwrap();

    let mutex_clone = Arc::clone(&mutex);
    let (c, r) = continuation();
    thread::spawn(move || {
        let failed = mutex_clone.try_lock().is_err();
        c.send(failed);
    });

    let failed = r.await;
    assert!(failed);
    drop(guard);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[test]
fn test_mutex_lock_spin() {
    let mutex = Mutex::new(0);
    let mut guard = mutex.lock_spin();
    *guard = 42;
    drop(guard);

    let guard = mutex.lock_spin();
    assert_eq!(*guard, 42);
}

#[test_executors::async_test]
async fn test_mutex_lock_block() {
    //for the time being, wasm_thread only works in browser
    //see https://github.com/rustwasm/wasm-bindgen/issues/4534,
    //though we also need wasm_thread support.
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
    let mutex = Arc::new(Mutex::new(0));
    let mutex_clone = Arc::clone(&mutex);

    let (c, r) = continuation();
    thread::spawn(move || {
        let mut guard = mutex_clone.lock_block();
        *guard = 42;
        // Don't use thread::sleep in WASM as it calls Atomics.wait
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(10));
        c.send(());
    });

    // Wait for the spawned thread to complete first
    r.await;

    // Don't use thread::sleep in WASM as it calls Atomics.wait
    #[cfg(not(target_arch = "wasm32"))]
    thread::sleep(Duration::from_millis(5));

    let guard = mutex.lock_block();
    assert_eq!(*guard, 42);
}

#[test_executors::async_test]
async fn test_mutex_concurrent_increment() {
    //for the time being, wasm_thread only works in browser
    //see https://github.com/rustwasm/wasm-bindgen/issues/4534,
    //though we also need wasm_thread support.
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    let mutex = Arc::new(Mutex::new(0));
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let mutex = Arc::clone(&mutex);
            let (c, r) = continuation();
            thread::spawn(move || {
                for _ in 0..100 {
                    let mut guard = mutex.lock_spin();
                    *guard += 1;
                }
                c.send(());
            });
            r
        })
        .collect();

    for handle in handles {
        handle.await;
    }

    let guard = mutex.lock_spin();
    assert_eq!(*guard, 1000);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[test]
fn test_mutex_lock_async() {
    test_executors::spin_on(async {
        let mutex = Mutex::new(42);
        let guard = mutex.lock_async().await;
        assert_eq!(*guard, 42);
    });
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[test]
fn test_mutex_async_contention() {
    test_executors::spin_on(async {
        let mutex = Arc::new(Mutex::new(0));

        let mutex1 = Arc::clone(&mutex);
        let task1 = async move {
            let mut guard = mutex1.lock_async().await;
            *guard += 1;
            drop(guard);
        };

        let mutex2 = Arc::clone(&mutex);
        let task2 = async move {
            let mut guard = mutex2.lock_async().await;
            *guard += 10;
        };

        task1.await;
        task2.await;

        let guard = mutex.lock_async().await;
        assert_eq!(*guard, 11);
    });
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[test]
fn test_guard_drop_releases_lock() {
    let mutex = Arc::new(Mutex::new(42));
    {
        let _guard = mutex.lock_spin();
    }

    let guard = mutex.try_lock().unwrap();
    assert_eq!(*guard, 42);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[test]
fn test_mutex_lock_spin_timeout() {
    let mutex = Mutex::new(0);
    let deadline = Instant::now() + Duration::from_secs(1);

    // Test successful acquisition
    if let Some(mut guard) = mutex.lock_spin_timeout(deadline) {
        *guard = 42;
    } else {
        panic!("Failed to acquire lock");
    }

    assert_eq!(*mutex.lock_spin(), 42);
}

#[test_executors::async_test]
async fn test_mutex_lock_spin_timeout_fails() {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    let mutex = Arc::new(Mutex::new(0));
    let mutex_clone = Arc::clone(&mutex);

    // Lock it in another thread
    let (c, r) = continuation();
    thread::spawn(move || {
        let _guard = mutex_clone.lock_spin();
        c.send(());
        // Hold it for a bit
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_secs(1));
        #[cfg(target_arch = "wasm32")]
        {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(1) {
                std::hint::spin_loop();
            }
        }
    });

    r.await; // Wait for thread to acquire lock

    // Try to acquire with short timeout
    let deadline = Instant::now() + Duration::from_millis(10);
    let result = mutex.lock_spin_timeout(deadline);
    assert!(result.is_none());
}

#[test_executors::async_test]
async fn test_mutex_lock_block_timeout() {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    let mutex = Arc::new(Mutex::new(0));
    let mutex_clone = Arc::clone(&mutex);

    // Lock it in another thread
    let (c, r) = continuation();
    thread::spawn(move || {
        let _guard = mutex_clone.lock_block();
        c.send(());
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_secs(1));
        #[cfg(target_arch = "wasm32")]
        {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(1) {
                std::hint::spin_loop();
            }
        }
    });

    r.await;

    // Try to acquire with short timeout - must run in worker thread on WASM
    // because lock_block_timeout uses Atomics.wait which is forbidden on main thread
    let mutex_clone2 = Arc::clone(&mutex);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(10);
        let result = mutex_clone2.lock_block_timeout(deadline);
        assert!(result.is_none());
        c2.send(());
    });
    r2.await;

    // Wait for thread to release (approx)
    #[cfg(not(target_arch = "wasm32"))]
    thread::sleep(Duration::from_secs(1));
    #[cfg(target_arch = "wasm32")]
    {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(1) {
            // yield to event loop? no async sleep here.
            // just spin
            std::hint::spin_loop();
        }
    }

    // Should succeed now - must run in worker thread on WASM
    let mutex_clone3 = Arc::clone(&mutex);
    let (c3, r3) = continuation();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(1);
        if let Some(guard) = mutex_clone3.lock_block_timeout(deadline) {
            assert_eq!(*guard, 0);
        } else {
            panic!("Failed to acquire lock after release");
        }
        c3.send(());
    });
    r3.await;
}

#[test_executors::async_test]
async fn test_mutex_lock_sync_timeout() {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    let mutex = Arc::new(Mutex::new(0));
    let mutex_clone = Arc::clone(&mutex);

    // Lock it in another thread
    let (c, r) = continuation();
    thread::spawn(move || {
        let _guard = mutex_clone.lock_sync();
        c.send(());
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(500));
        #[cfg(target_arch = "wasm32")]
        {
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(500) {
                std::hint::spin_loop();
            }
        }
    });

    r.await;

    // Try to acquire with short timeout
    let deadline = Instant::now() + Duration::from_millis(10);
    let result = mutex.lock_sync_timeout(deadline);
    assert!(result.is_none());
}

#[test_executors::async_test]
async fn test_mutex_lock_async_timeout() {
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    let mutex = Arc::new(Mutex::new(0));

    // Test success
    let deadline = Instant::now() + Duration::from_secs(1);
    if let Some(guard) = mutex.lock_async_timeout(deadline).await {
        assert_eq!(*guard, 0);
    } else {
        panic!("Failed to acquire free lock");
    }

    let mutex_clone = Arc::clone(&mutex);
    // Lock it in another task
    let (c, r) = continuation();

    // Spawn a thread to hold the lock, because async tasks on same executor might not run in parallel if we block
    thread::spawn(move || {
        test_executors::spin_on(async {
            let _guard = mutex_clone.lock_async().await;
            c.send(());
            // Hold it
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(500) {
                // yield?
                r#continue::continuation::<()>().1.await; // never resolves, but yields? no, it blocks if we await it?
                // we just want to delay.
                // but we are in async.
                // spin loop is fine for test
                std::hint::spin_loop();
            }
        });
    });

    r.await;

    // Try to acquire with short timeout
    let deadline = Instant::now() + Duration::from_millis(10);
    let result = mutex.lock_async_timeout(deadline).await;
    assert!(result.is_none());
}
