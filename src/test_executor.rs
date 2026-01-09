#[cfg(not(target_arch="wasm32"))]
use std::future::Future;
#[cfg(not(target_arch="wasm32"))]
use std::pin::pin;
#[cfg(not(target_arch="wasm32"))]

use std::task::{Context, Poll, RawWaker, Waker};
#[cfg(not(target_arch="wasm32"))]
use std::task::RawWakerVTable;

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

#[cfg(not(target_arch = "wasm32"))]
static NOOP_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    |_| RawWaker::new(std::ptr::null(), &NOOP_WAKER_VTABLE),
    |_| (),
    |_| (),
    |_| (),
);



//this is the world's worst 'executor'
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn<F: Future>(future: F) -> F::Output {
    let mut f = pin!(future);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &NOOP_WAKER_VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Pending => { std::thread::yield_now(); },
            Poll::Ready(r) => { return r; }
        }
    }
}