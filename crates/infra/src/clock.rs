use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration, Utc};
use flashmart_contracts::{Clock, IdGenerator};
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// 可控确定性时钟，供黑盒验收与领域单测推进时间窗/支付保留期
#[derive(Debug)]
pub struct FixedClock {
    current: RwLock<DateTime<Utc>>,
}

impl FixedClock {
    pub fn new(initial: DateTime<Utc>) -> Self {
        Self {
            current: RwLock::new(initial),
        }
    }

    pub fn set(&self, next: DateTime<Utc>) {
        *self.current.write().expect("clock lock poisoned") = next;
    }

    pub fn advance(&self, delta: Duration) {
        let mut guard = self.current.write().expect("clock lock poisoned");
        *guard += delta;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.current.read().expect("clock lock poisoned")
    }
}

#[derive(Debug, Default)]
pub struct UuidV4Generator;

impl IdGenerator for UuidV4Generator {
    fn next_uuid(&self) -> Uuid {
        Uuid::new_v4()
    }
}

/// 可控确定性 UUID 生成器
#[derive(Debug)]
pub struct SequenceIdGenerator {
    counter: AtomicU64,
}

impl SequenceIdGenerator {
    pub fn new(start: u64) -> Self {
        Self {
            counter: AtomicU64::new(start),
        }
    }
}

impl Default for SequenceIdGenerator {
    fn default() -> Self {
        Self::new(1)
    }
}

impl IdGenerator for SequenceIdGenerator {
    fn next_uuid(&self) -> Uuid {
        let seq = self.counter.fetch_add(1, Ordering::Relaxed);
        Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0000 | u128::from(seq))
    }
}
