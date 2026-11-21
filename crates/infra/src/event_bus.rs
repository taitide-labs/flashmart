use std::sync::RwLock;

use async_trait::async_trait;
use flashmart_contracts::IntegrationEvent;

#[async_trait]
pub trait EventBus: Send + Sync {
    async fn publish(&self, event: IntegrationEvent);
}

#[derive(Debug, Default)]
pub struct InMemoryEventBus {
    events: RwLock<Vec<IntegrationEvent>>,
}

impl InMemoryEventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<IntegrationEvent> {
        self.events.read().expect("event bus lock poisoned").clone()
    }

    pub fn clear(&self) {
        self.events
            .write()
            .expect("event bus lock poisoned")
            .clear();
    }
}

#[async_trait]
impl EventBus for InMemoryEventBus {
    async fn publish(&self, event: IntegrationEvent) {
        self.events
            .write()
            .expect("event bus lock poisoned")
            .push(event);
    }
}
