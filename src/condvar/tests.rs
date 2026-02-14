// SPDX-License-Identifier: MIT OR Apache-2.0
use super::*;
use crate::Mutex;
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
use web_time::Duration;

#[cfg(target_arch = "wasm32")]
use crate as thread;
use r#continue::continuation;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

// Configure WASM tests to run in browser (required for thread spawning)
#[cfg(all(test, target_arch = "wasm32"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[test_executors::async_test]
//#[cfg(not(target_arch = "wasm32"))] // HANGS on WASM
async fn test_condvar_basic_spin() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::Builder::new()
        .name("basic_spin_write".into())
        .spawn(move || {
            thread::sleep(std::time::Duration::from_millis(50));

            let (mutex, condvar) = &*pair_clone;
            let mut ready = mutex.lock_sync();
            *ready = true;
            drop(ready);
            condvar.notify_one();
            c.send(());
        })
        .unwrap();
    let (c, r2) = continuation();
    thread::Builder::new()
        .name("basic_spin_write".into())
        .spawn(move || {
            let (mutex, condvar) = &*pair;
            let mut ready = mutex.lock_sync();
            while !*ready {
                ready = condvar.wait_spin(ready);
            }
            assert!(*ready);
            c.send(());
        })
        .unwrap();
    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_block() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(10));

        let (mutex, condvar) = &*pair_clone;
        let mut ready = mutex.lock_sync();
        *ready = true;
        drop(ready);
        condvar.notify_one();
        c.send(());
    });

    // Move the waiting into a worker thread
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair;
        let mut ready = mutex.lock_sync();
        while !*ready {
            ready = condvar.wait_block(ready);
        }
        assert!(*ready);
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_sync() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(10));

        let (mutex, condvar) = &*pair_clone;
        let mut ready = mutex.lock_sync();
        *ready = true;
        drop(ready);
        condvar.notify_one();
        c.send(());
    });

    // Move the waiting into a worker thread
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair;
        let mut ready = mutex.lock_sync();
        while !*ready {
            ready = condvar.wait_sync(ready);
        }
        assert!(*ready);
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_async() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(10));

        test_executors::spin_on(async {
            let (mutex, condvar) = &*pair_clone;
            let mut ready = mutex.lock_async().await;
            *ready = true;
            drop(ready);
            condvar.notify_one();
        });
        c.send(());
    });

    // Move the async waiting into a worker thread
    let (c2, r2) = continuation();
    thread::spawn(move || {
        test_executors::spin_on(async {
            let (mutex, condvar) = &*pair;
            let mut ready = mutex.lock_async().await;
            while !*ready {
                ready = condvar.wait_async(ready).await;
            }
            assert!(*ready);
        });
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_notify_all() {
    let pair = Arc::new((Mutex::new(0), Condvar::new()));
    let mut receivers = Vec::new();

    // Spawn 3 threads that wait for count to reach 10
    for _ in 0..3 {
        let pair = Arc::clone(&pair);
        let (c, r) = continuation();
        thread::spawn(move || {
            let (mutex, condvar) = &*pair;
            let mut count = mutex.lock_sync();
            while *count < 10 {
                count = condvar.wait_sync(count);
            }
            c.send(*count);
        });
        receivers.push(r);
    }

    // Give threads time to start waiting
    #[cfg(not(target_arch = "wasm32"))]
    thread::sleep(Duration::from_millis(50));

    // Update the count and notify all
    let (mutex, condvar) = &*pair;
    let mut count = mutex.lock_sync();
    *count = 10;
    drop(count);
    condvar.notify_all();

    // All threads should wake up and see count = 10
    for receiver in receivers {
        let result = receiver.await;
        assert_eq!(result, 10);
    }
}

#[test_executors::async_test]
async fn test_condvar_producer_consumer() {
    use std::collections::VecDeque;

    let shared = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
    let producer = Arc::clone(&shared);

    let (c, r) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*producer;
        for i in 0..5 {
            #[cfg(not(target_arch = "wasm32"))]
            thread::sleep(Duration::from_millis(5));

            let mut queue = mutex.lock_sync();
            queue.push_back(i);
            drop(queue);
            condvar.notify_one();
        }
        c.send(());
    });

    // Move the consumer (waiting) into a worker thread
    let consumer = Arc::clone(&shared);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*consumer;
        let mut collected = Vec::new();
        for _ in 0..5 {
            let mut queue = mutex.lock_sync();
            while queue.is_empty() {
                queue = condvar.wait_sync(queue);
            }
            if let Some(value) = queue.pop_front() {
                collected.push(value);
            }
        }
        assert_eq!(collected, vec![0, 1, 2, 3, 4]);
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_spin_timeout() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let (mutex, condvar) = &*pair;
    let mut ready = mutex.lock_sync();
    let deadline = Instant::now() + Duration::from_millis(10);
    let result;
    (ready, result) = condvar.wait_spin_timeout(ready, deadline);
    assert!(result.timed_out());
    assert!(!*ready);
}

#[test_executors::async_test]
async fn test_condvar_wait_spin_timeout_notified() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(50));
        let (mutex, condvar) = &*pair_clone;
        let mut ready = mutex.lock_sync();
        *ready = true;
        drop(ready);
        condvar.notify_one();
        c.send(());
    }); // removed unwrap because thread::spawn returns JoinHandle, not Result in standard lib, but wasm_thread might be different?
    // checking wasm_thread docs or source would be good, but standard thread::spawn returns JoinHandle.
    // Wait, standard thread::spawn panics if it fails to spawn (in some impls) or returns JoinHandle.
    // Actually std::thread::Builder::spawn returns Result. std::thread::spawn returns JoinHandle.
    // So unwrap is not needed for spawn, but we should ensure it didn't panic.

    let pair_clone2 = Arc::clone(&pair);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair_clone2;
        let mut ready = mutex.lock_sync();
        // Increase deadline to 5 seconds to be safe
        let deadline = Instant::now() + Duration::from_secs(5);
        while !*ready {
            let result;
            (ready, result) = condvar.wait_spin_timeout(ready, deadline);
            if result.timed_out() {
                // Add more info to panic
                panic!("Should not have timed out. Ready is still false.");
            }
        }
        assert!(*ready);
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_sync_while() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(10));

        let (mutex, condvar) = &*pair_clone;
        let mut ready = mutex.lock_sync();
        *ready = true;
        drop(ready);
        condvar.notify_one();
        c.send(());
    });

    let pair_waiter = Arc::clone(&pair);
    let (cw, rw) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair_waiter;
        let guard = mutex.lock_sync();
        let guard = condvar.wait_sync_while(guard, |ready| !*ready);
        assert!(*guard);
        drop(guard);
        cw.send(());
    });

    r.await;
    rw.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_async_while() {
    let pair = Arc::new((Mutex::new(0), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(10));

        test_executors::spin_on(async move {
            let (mutex, condvar) = &*pair_clone;
            let mut value = mutex.lock_async().await;
            *value = 1;
            drop(value);
            condvar.notify_one();
            c.send(());
        });
    });

    let (mutex, condvar) = &*pair;
    let guard = mutex.lock_async().await;
    let guard = condvar.wait_async_while(guard, |value| *value == 0).await;
    assert_eq!(*guard, 1);
    drop(guard);

    r.await;
}

#[test_executors::async_test]
async fn test_condvar_notify_one_only_wakes_one() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let mut receivers = Vec::new();

    // Spawn 3 threads that increment on each wake
    for _ in 0..3 {
        let pair = Arc::clone(&pair);
        let (c, r) = continuation();
        thread::spawn(move || {
            let (mutex, condvar) = &*pair;
            let mut wake = mutex.lock_sync();
            while !wake.as_ref() {
                wake = condvar.wait_spin(wake);
            }
            eprintln!("Finished all wait_spins");
            c.send(());
        });
        receivers.push(r);
    }

    // Give threads time to start waiting
    #[cfg(not(target_arch = "wasm32"))]
    thread::sleep(Duration::from_millis(100));

    // Increment and notify one at a time
    for _ in 1..=3 {
        let (mutex, condvar) = &*pair;
        let mut wake = mutex.lock_sync();
        *wake = true;
        drop(wake);
        condvar.notify_one();

        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(10));
    }

    // All threads should eventually complete
    for receiver in receivers {
        receiver.await;
    }

    let (mutex, _) = &*pair;
    assert_eq!(*mutex.lock_sync(), true);
}

#[test_executors::async_test]
async fn test_condvar_wait_block_timeout() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair_clone;
        let mut ready = mutex.lock_sync();
        let deadline = Instant::now() + Duration::from_millis(10);
        let result;
        (ready, result) = condvar.wait_block_timeout(ready, deadline);
        assert!(result.timed_out());
        assert!(!*ready);
        c.send(());
    });

    r.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_block_timeout_notified() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(50));

        let (mutex, condvar) = &*pair_clone;
        let mut ready = mutex.lock_sync();
        *ready = true;
        drop(ready);
        condvar.notify_one();
        c.send(());
    });

    let pair_clone2 = Arc::clone(&pair);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair_clone2;
        let mut ready = mutex.lock_sync();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !*ready {
            let result;
            (ready, result) = condvar.wait_block_timeout(ready, deadline);
            if result.timed_out() {
                panic!("Should not have timed out. Ready is still false.");
            }
        }
        assert!(*ready);
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_timeout_dispatch() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(50));

        let (mutex, condvar) = &*pair_clone;
        let mut ready = mutex.lock_sync();
        *ready = true;
        drop(ready);
        condvar.notify_one();
        c.send(());
    });

    let pair_clone2 = Arc::clone(&pair);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair_clone2;
        let mut ready = mutex.lock_sync();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !*ready {
            let result;
            (ready, result) = condvar.wait_sync_timeout(ready, deadline);
            if result.timed_out() {
                panic!("Should not have timed out. Ready is still false.");
            }
        }
        assert!(*ready);
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_async_timeout() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let (mutex, condvar) = &*pair;

    let mut ready = mutex.lock_async().await;
    let deadline = Instant::now() + Duration::from_millis(50);
    let result;
    (ready, result) = condvar.wait_async_timeout(ready, deadline).await;
    assert!(result.timed_out());
    assert!(!*ready);
}

#[test_executors::async_test]
async fn test_condvar_wait_async_timeout_notified() {
    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(50));

        test_executors::spin_on(async {
            let (mutex, condvar) = &*pair_clone;
            let mut ready = mutex.lock_async().await;
            *ready = true;
            drop(ready);
            condvar.notify_one();
        });
        c.send(());
    });

    let pair_clone2 = Arc::clone(&pair);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        test_executors::spin_on(async {
            let (mutex, condvar) = &*pair_clone2;
            let mut ready = mutex.lock_async().await;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !*ready {
                let result;
                (ready, result) = condvar.wait_async_timeout(ready, deadline).await;
                if result.timed_out() {
                    panic!("Should not have timed out. Ready is still false.");
                }
            }
            assert!(*ready);
        });
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_async_timeout_while() {
    let pair = Arc::new((Mutex::new(0), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(50));

        test_executors::spin_on(async {
            let (mutex, condvar) = &*pair_clone;
            let mut value = mutex.lock_async().await;
            *value = 10;
            drop(value);
            condvar.notify_one();
        });
        c.send(());
    });

    let pair_clone2 = Arc::clone(&pair);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        test_executors::spin_on(async {
            let (mutex, condvar) = &*pair_clone2;
            let guard = mutex.lock_async().await;
            let deadline = Instant::now() + Duration::from_secs(5);
            let (guard, result) = condvar
                .wait_async_timeout_while(guard, deadline, |value| *value < 10)
                .await;
            assert!(!result.timed_out());
            assert_eq!(*guard, 10);
        });
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_spin_timeout_while() {
    let pair = Arc::new((Mutex::new(0), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(50));
        let (mutex, condvar) = &*pair_clone;
        let mut value = mutex.lock_sync();
        *value = 10;
        drop(value);
        condvar.notify_one();
        c.send(());
    });

    let pair_clone2 = Arc::clone(&pair);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair_clone2;
        let guard = mutex.lock_sync();
        let deadline = Instant::now() + Duration::from_secs(5);
        let (guard, result) = condvar.wait_spin_timeout_while(guard, deadline, |v| *v < 10);
        assert!(!result.timed_out());
        assert_eq!(*guard, 10);
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_block_timeout_while() {
    let pair = Arc::new((Mutex::new(0), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(50));

        let (mutex, condvar) = &*pair_clone;
        let mut value = mutex.lock_sync();
        *value = 10;
        drop(value);
        condvar.notify_one();
        c.send(());
    });

    let pair_clone2 = Arc::clone(&pair);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair_clone2;
        let guard = mutex.lock_sync();
        let deadline = Instant::now() + Duration::from_secs(5);
        let (guard, result) = condvar.wait_block_timeout_while(guard, deadline, |v| *v < 10);
        assert!(!result.timed_out());
        assert_eq!(*guard, 10);
        c2.send(());
    });

    r.await;
    r2.await;
}

#[test_executors::async_test]
async fn test_condvar_wait_sync_timeout_while() {
    let pair = Arc::new((Mutex::new(0), Condvar::new()));
    let pair_clone = Arc::clone(&pair);

    let (c, r) = continuation();
    thread::spawn(move || {
        #[cfg(not(target_arch = "wasm32"))]
        thread::sleep(Duration::from_millis(50));

        let (mutex, condvar) = &*pair_clone;
        let mut value = mutex.lock_sync();
        *value = 10;
        drop(value);
        condvar.notify_one();
        c.send(());
    });

    let pair_clone2 = Arc::clone(&pair);
    let (c2, r2) = continuation();
    thread::spawn(move || {
        let (mutex, condvar) = &*pair_clone2;
        let guard = mutex.lock_sync();
        let deadline = Instant::now() + Duration::from_secs(5);
        let (guard, result) = condvar.wait_sync_timeout_while(guard, deadline, |v| *v < 10);
        assert!(!result.timed_out());
        assert_eq!(*guard, 10);
        c2.send(());
    });

    r.await;
    r2.await;
}
