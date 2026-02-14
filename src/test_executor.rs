// SPDX-License-Identifier: MIT OR Apache-2.0
use std::future::Future;

#[macro_export]
macro_rules! async_test {
    (async fn $name:ident() $body:block) => {
        #[cfg(target_arch = "wasm32")]
        #[wasm_bindgen_test::wasm_bindgen_test]
        async fn $name() $body

        #[cfg(not(target_arch = "wasm32"))]
        #[test]
        fn $name() {
            $crate::test_executor::spawn(async $body)
        }
    };
}

/// Runs a future to completion, blocking the current thread until it's done.
///
/// # Platform behavior
///
/// - **Native**: Uses a simple polling loop with thread yielding.
/// - **WASM (in worker context)**: Spawns a child worker that runs the future using
///   `wasm_bindgen_futures::spawn_local`, which properly integrates with the JS event loop.
///   The calling thread blocks on `Atomics.wait` until the result is ready.
///
/// # Requirements
///
/// On WASM, this function requires:
/// - Being called from a worker thread (not the main browser thread), since it uses `Atomics.wait`
/// - The future and its output must be `Send + 'static`
///
/// Use `wasm_bindgen_test_configure!(run_in_dedicated_worker)` in doctests to ensure
/// the test runs in a worker context.
///
/// # Panics
///
/// On WASM main thread, this will spin forever waiting for the result (same as before),
/// because `Atomics.wait` is not available there.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn<F, T>(future: F) -> T
where
    F: Future<Output = T>,
    T: Send + 'static,
{
    use std::pin::pin;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    static NOOP_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &NOOP_WAKER_VTABLE),
        |_| (),
        |_| (),
        |_| (),
    );

    let mut f = pin!(future);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &NOOP_WAKER_VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Pending => {
                std::thread::yield_now();
            }
            Poll::Ready(r) => return r,
        }
    }
}

/// WASM implementation that spawns a worker to run the future with proper event loop integration.
#[cfg(target_arch = "wasm32")]
pub fn spawn<F, T>(future: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    use crate::mpsc::channel;

    let (tx, rx) = channel();

    // Spawn a worker thread to run the future
    crate::spawn(move || {
        // Queue the async work on this worker's JS event loop
        wasm_bindgen_futures::spawn_local(async move {
            let result = future.await;
            // Send result back to parent thread
            let _ = tx.send_sync(result);
        });
        // The closure returns, but the worker stays alive to process the queued async work
    });

    // Block waiting for the result (uses Atomics.wait in worker context)
    rx.recv_sync()
        .expect("worker thread panicked or was terminated")
}
