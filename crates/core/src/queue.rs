//! 有界块队列（生产者-消费者）。
//!
//! 语义：满时 `push` 丢弃最旧的块（实时音频：宁可丢帧也不让延迟累积）；
//! `close` 后消费者取完剩余元素即收到 [`Recv::Closed`]。

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum Recv<T> {
    Item(T),
    /// 等待超时且队列仍为空。
    TimedOut,
    /// 队列已关闭且已取空。
    Closed,
}

struct Inner<T> {
    items: VecDeque<T>,
    closed: bool,
    dropped: u64,
}

pub struct ChunkQueue<T> {
    inner: Mutex<Inner<T>>,
    cv: Condvar,
    capacity: usize,
}

impl<T> ChunkQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                items: VecDeque::new(),
                closed: false,
                dropped: 0,
            }),
            cv: Condvar::new(),
            capacity: capacity.max(1),
        }
    }

    /// 入队；满时丢弃最旧的块。返回累计丢弃的块数。
    pub fn push(&self, item: T) -> u64 {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return g.dropped;
        }
        if g.items.len() >= self.capacity {
            g.items.pop_front();
            g.dropped += 1;
        }
        g.items.push_back(item);
        drop(g);
        self.cv.notify_one();
        self.dropped()
    }

    /// 出队，最多等待 `timeout`。
    pub fn pop_timeout(&self, timeout: Duration) -> Recv<T> {
        let deadline = std::time::Instant::now() + timeout;
        let mut g = self.inner.lock().unwrap();
        loop {
            if let Some(item) = g.items.pop_front() {
                return Recv::Item(item);
            }
            if g.closed {
                return Recv::Closed;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return Recv::TimedOut;
            }
            let (ng, _) = self.cv.wait_timeout(g, deadline - now).unwrap();
            g = ng;
        }
    }

    /// 关闭队列；唤醒所有等待者。已入队的元素仍可取出。
    pub fn close(&self) {
        self.inner.lock().unwrap().closed = true;
        self.cv.notify_all();
    }

    /// 累计丢弃的块数。
    pub fn dropped(&self) -> u64 {
        self.inner.lock().unwrap().dropped
    }

    /// 当前长度。
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn fifo_order() {
        let q = ChunkQueue::new(8);
        for i in 0..4 {
            q.push(i);
        }
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Item(0));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Item(1));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Item(2));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Item(3));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::TimedOut);
    }

    #[test]
    fn overflow_drops_oldest() {
        let q = ChunkQueue::new(3);
        for i in 0..5 {
            q.push(i);
        }
        assert_eq!(q.dropped(), 2);
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Item(2));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Item(3));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Item(4));
    }

    #[test]
    fn close_drains_then_closed() {
        let q: Arc<ChunkQueue<u32>> = Arc::new(ChunkQueue::new(4));
        q.push(1);
        q.push(2);
        q.close();
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Item(1));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Item(2));
        assert_eq!(q.pop_timeout(Duration::from_millis(1)), Recv::Closed);
        assert_eq!(q.push(3), 0); // 关闭后 push 被忽略
    }

    #[test]
    fn cross_thread_wakeup() {
        let q: Arc<ChunkQueue<u32>> = Arc::new(ChunkQueue::new(4));
        let q2 = Arc::clone(&q);
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            q2.push(42);
        });
        assert_eq!(q.pop_timeout(Duration::from_secs(2)), Recv::Item(42));
        h.join().unwrap();
    }
}
