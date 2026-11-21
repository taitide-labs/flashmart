use chrono::{DateTime, Utc};
use uuid::Uuid;

/// 非确定性时钟抽象，生产注入系统时钟，测试注入可控时钟
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// 非确定性标识生成抽象
pub trait IdGenerator: Send + Sync {
    fn next_uuid(&self) -> Uuid;
}
