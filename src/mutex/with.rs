// SPDX-License-Identifier: MIT OR Apache-2.0
use super::Mutex;

pub(crate) fn with_sync<T, R, F: FnOnce(&T) -> R>(mutex: &Mutex<T>, f: F) -> R {
    let guard = mutex.lock_sync();
    f(&guard)
}

pub(crate) fn with_mut_sync<T, R, F: FnOnce(&mut T) -> R>(mutex: &Mutex<T>, f: F) -> R {
    let mut guard = mutex.lock_sync();
    f(&mut guard)
}

pub(crate) async fn with_async<T, R, F: FnOnce(&T) -> R>(mutex: &Mutex<T>, f: F) -> R {
    let guard = mutex.lock_async().await;
    f(&guard)
}

pub(crate) async fn with_mut_async<T, R, F: FnOnce(&mut T) -> R>(mutex: &Mutex<T>, f: F) -> R {
    let mut guard = mutex.lock_async().await;
    f(&mut guard)
}
