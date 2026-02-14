// SPDX-License-Identifier: MIT OR Apache-2.0
use super::RwLock;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
use crate as thread;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

#[test]
fn test_lock_try() {
    let mutex = RwLock::new(0);
    let lock = mutex.try_lock_read();
    assert!(lock.is_ok());
    assert_eq!(lock.as_ref().unwrap().deref(), &0);
    let lock2 = mutex.try_lock_read();
    assert!(lock2.is_ok());
    assert_eq!(lock2.as_ref().unwrap().deref(), &0);

    drop(lock2);
    //fail to acquire write lock
    let lock2 = mutex.try_lock_write();
    assert!(!lock2.is_ok());

    drop(lock);
    let mut write_lock = mutex.try_lock_write();
    assert!(write_lock.is_ok());
    assert_eq!(write_lock.as_ref().unwrap().deref(), &0);

    *write_lock.as_mut().unwrap().deref_mut() = 2;
    assert_eq!(write_lock.as_ref().unwrap().deref(), &2);

    //fail to acquire a new read lock
    let read_lock = mutex.try_lock_read();
    assert!(!read_lock.is_ok());

    drop(write_lock);
    //new read lock
    let read_lock = mutex.try_lock_read();
    assert!(read_lock.is_ok());
    assert_eq!(read_lock.as_ref().unwrap().deref(), &2);
}

#[test]
fn test_lock_spin() {
    let mutex = RwLock::new(0);
    let lock = mutex.lock_spin_read();
    drop(lock);

    let lock = mutex.lock_spin_write();
    drop(lock);
}

#[test]
fn test_lock_block() {
    let mutex = Arc::new(RwLock::new(0));
    let lock = mutex.lock_block_read();
    assert_eq!(lock.deref(), &0);
    let lock2 = mutex.lock_block_read();
    assert_eq!(lock.deref(), &0);
    drop(lock2);

    let (tx, rx) = std::sync::mpsc::channel();
    let mutex_clone = mutex.clone();
    thread::spawn(move || {
        //indicate thread came up
        tx.send(()).unwrap();
        let lock = mutex_clone.lock_block_write();
        tx.send(()).unwrap();
        thread::sleep(Duration::from_millis(25));
        drop(lock);
    });
    //wait for thread up msg
    rx.recv().unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(10)).is_err());
    drop(lock); //thread should now acquire lock
    rx.recv().unwrap(); //wait for thread to acquire lock
    let time = std::time::Instant::now();
    mutex.lock_block_read();
    //ensure time took >50ms
    assert!(time.elapsed() > Duration::from_millis(10));
}

#[test_executors::async_test]
async fn test_async() {
    let mutex = Arc::new(RwLock::new(0));
    let lock = mutex.lock_async_read().await;
    assert_eq!(lock.deref(), &0);
    drop(lock);
    let lock = mutex.lock_async_write().await;
    assert_eq!(lock.deref(), &0);
    drop(lock);
}

#[test]
fn test_sync() {
    let mutex = Arc::new(RwLock::new(0));
    let lock = mutex.lock_sync_read();
    assert_eq!(lock.deref(), &0);
    drop(lock);
    let lock = mutex.lock_sync_write();
    assert_eq!(lock.deref(), &0);
    drop(lock);
}
