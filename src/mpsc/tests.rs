// SPDX-License-Identifier: MIT OR Apache-2.0
use super::*;
use std::thread;
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(target_arch = "wasm32")]
use web_time::Instant;

#[test]
fn test_send_recv_spin() {
    let (tx, rx) = channel();
    tx.send_spin(1).unwrap();
    assert_eq!(rx.recv_spin(), Ok(1));
}

#[test]
fn test_send_recv_block() {
    let (tx, rx) = channel();
    tx.send_block(1).unwrap();
    assert_eq!(rx.recv_block(), Ok(1));
}

#[test]
fn test_send_recv_sync() {
    let (tx, rx) = channel();
    tx.send_sync(1).unwrap();
    assert_eq!(rx.recv_sync(), Ok(1));
}

#[test]
fn test_send_recv_async() {
    test_executors::spin_on(async {
        let (tx, rx) = channel();
        tx.send_async(1).await.unwrap();
        assert_eq!(rx.recv_async().await, Ok(1));
    });
}

#[test]
fn test_multiple_senders() {
    let (tx, rx) = channel();
    let tx2 = tx.clone();
    tx.send_sync(1).unwrap();
    tx2.send_sync(2).unwrap();
    assert_eq!(rx.recv_sync(), Ok(1));
    assert_eq!(rx.recv_sync(), Ok(2));
}

#[test]
fn test_ordering() {
    let (tx, rx) = channel();
    tx.send_sync(1).unwrap();
    tx.send_sync(2).unwrap();
    assert_eq!(rx.recv_sync(), Ok(1));
    assert_eq!(rx.recv_sync(), Ok(2));
}

#[test]
fn test_blocking_behavior() {
    let (tx, rx) = channel();
    let t = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        tx.send_block(42).unwrap();
    });
    assert_eq!(rx.recv_block(), Ok(42));
    t.join().unwrap();
}

#[test]
fn test_try_recv() {
    let (tx, rx) = channel();
    assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    tx.send_sync(1).unwrap();
    assert_eq!(rx.try_recv(), Ok(1));
    assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
}

#[test]
fn test_recv_timeout() {
    let (tx, rx) = channel();
    let deadline = Instant::now() + Duration::from_millis(100);
    assert_eq!(
        rx.recv_sync_timeout(deadline),
        Err(RecvTimeoutError::Timeout)
    );
    tx.send_sync(1).unwrap();
    assert_eq!(
        rx.recv_sync_timeout(Instant::now() + Duration::from_secs(1)),
        Ok(1)
    );
    assert_eq!(
        rx.recv_sync_timeout(Instant::now() + Duration::from_millis(10)),
        Err(RecvTimeoutError::Timeout)
    );
}

#[test]
fn test_recv_spin_timeout() {
    let (tx, rx) = channel();
    tx.send_spin(1).unwrap();
    assert_eq!(
        rx.recv_spin_timeout(Instant::now() + Duration::from_secs(1)),
        Ok(1)
    );
    assert_eq!(
        rx.recv_spin_timeout(Instant::now() + Duration::from_millis(10)),
        Err(RecvTimeoutError::Timeout)
    );
}

#[test]
fn test_recv_block_timeout() {
    let (tx, rx) = channel();
    tx.send_block(1).unwrap();
    assert_eq!(
        rx.recv_block_timeout(Instant::now() + Duration::from_secs(1)),
        Ok(1)
    );
    assert_eq!(
        rx.recv_block_timeout(Instant::now() + Duration::from_millis(10)),
        Err(RecvTimeoutError::Timeout)
    );
}

#[test]
fn test_recv_async_timeout() {
    test_executors::spin_on(async {
        let (tx, rx) = channel();
        tx.send_async(1).await.unwrap();
        assert_eq!(
            rx.recv_async_timeout(Instant::now() + Duration::from_secs(1))
                .await,
            Ok(1)
        );
        assert_eq!(
            rx.recv_async_timeout(Instant::now() + Duration::from_millis(10))
                .await,
            Err(RecvTimeoutError::Timeout)
        );
    });
}

#[test]
fn test_debug() {
    let (tx, rx) = channel::<i32>();
    assert_eq!(format!("{:?}", tx), "Sender");
    assert_eq!(format!("{:?}", rx), "Receiver");
}

#[test]
fn test_into_iter() {
    let (tx, rx) = channel();
    let t = thread::spawn(move || {
        tx.send_sync(1).unwrap();
        tx.send_sync(2).unwrap();
        tx.send_sync(3).unwrap();
    });

    let mut iter = rx.into_iter();
    assert_eq!(iter.next(), Some(1));
    assert_eq!(iter.next(), Some(2));
    assert_eq!(iter.next(), Some(3));
    assert_eq!(iter.next(), None);
    t.join().unwrap();
}

#[test]
fn test_sender_sync() {
    fn is_sync<T: Sync>() {}
    is_sync::<Sender<i32>>();
}

#[test]
fn test_disconnect_sender() {
    let (tx, rx) = channel();
    tx.send_sync(1).unwrap();
    drop(tx);
    assert_eq!(rx.recv_sync(), Ok(1));
    assert_eq!(rx.recv_sync(), Err(RecvError::Disconnected));
}

#[test]
fn test_disconnect_receiver() {
    let (tx, rx) = channel();
    drop(rx);
    assert_eq!(tx.send_sync(1), Err(SendError(1)));
}

#[test]
fn test_iter_disconnect() {
    let (tx, rx) = channel();
    tx.send_sync(1).unwrap();
    tx.send_sync(2).unwrap();
    tx.send_sync(3).unwrap();
    drop(tx);
    let collected: Vec<_> = rx.into_iter().collect();
    assert_eq!(collected, vec![1, 2, 3]);
}

#[test]
fn test_disconnect_async() {
    test_executors::spin_on(async {
        let (tx, rx) = channel();
        tx.send_async(1).await.unwrap();
        drop(tx);
        assert_eq!(rx.recv_async().await, Ok(1));
        assert_eq!(rx.recv_async().await, Err(RecvError::Disconnected));
    });
}
