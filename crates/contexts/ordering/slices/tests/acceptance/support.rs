use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, TimeZone, Utc};
use flashmart_contracts::{
    Clock, ConfirmStockDeductionRequest, ConfirmStockOutcome, FlashItemSnapshot, InventoryPort,
    InventoryPortError, ReleaseReservedStockRequest, ReleaseStockOutcome, ReserveStockOutcome,
    ReserveStockRequest, ReturnDeductedStockRequest, ReturnStockOutcome, WindowStatus,
};
use flashmart_infra::{
    AppState, FixedClock, InMemoryEventBus, SequenceIdGenerator, collect_active_routes,
};
use flashmart_primitives::{Currency, ItemId, ItemKind, MoneyCents, ReservationId};
use sqlx::PgPool;

pub fn base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
}

#[derive(Debug, Clone)]
struct FakeReservation {
    item_id: ItemId,
    quantity: i32,
    status: &'static str,
}

#[derive(Debug, Default)]
struct FakeInventoryInner {
    items: HashMap<ItemId, FlashItemSnapshot>,
    reservations: HashMap<ReservationId, FakeReservation>,
    release_calls: usize,
    return_calls: usize,
}

#[derive(Clone)]
pub struct FakeInventoryPort {
    clock: Arc<dyn Clock>,
    inner: Arc<Mutex<FakeInventoryInner>>,
}

pub struct SeedItem<'a> {
    pub item_id: ItemId,
    pub title: &'a str,
    pub kind: ItemKind,
    pub price_cents: i64,
    pub total_stock: i32,
    pub per_user_limit: i32,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

impl FakeInventoryPort {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            inner: Arc::new(Mutex::new(FakeInventoryInner::default())),
        }
    }

    pub fn seed_item(&self, seed: SeedItem<'_>) {
        let now = self.clock.now();
        let window_status = compute_window(now, seed.starts_at, seed.ends_at);
        self.inner.lock().unwrap().items.insert(
            seed.item_id,
            FlashItemSnapshot {
                id: seed.item_id,
                title: seed.title.to_owned(),
                description: "测试商品".to_owned(),
                item_kind: seed.kind,
                price_cents: MoneyCents::try_new(seed.price_cents).unwrap(),
                currency: Currency::Cny,
                total_stock: seed.total_stock,
                available_stock: seed.total_stock,
                per_user_limit: seed.per_user_limit,
                sale_starts_at: seed.starts_at,
                sale_ends_at: seed.ends_at,
                window_status,
            },
        );
    }

    pub fn release_call_count(&self) -> usize {
        self.inner.lock().unwrap().release_calls
    }

    pub fn return_call_count(&self) -> usize {
        self.inner.lock().unwrap().return_calls
    }
}

fn compute_window(
    now: DateTime<Utc>,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
) -> WindowStatus {
    if now < starts_at {
        WindowStatus::Upcoming
    } else if now > ends_at {
        WindowStatus::Ended
    } else {
        WindowStatus::Active
    }
}

#[async_trait]
impl InventoryPort for FakeInventoryPort {
    async fn list_flash_items(&self) -> Result<Vec<FlashItemSnapshot>, InventoryPortError> {
        let now = self.clock.now();
        let guard = self.inner.lock().unwrap();
        Ok(guard
            .items
            .values()
            .cloned()
            .map(|mut item| {
                item.window_status = compute_window(now, item.sale_starts_at, item.sale_ends_at);
                item
            })
            .collect())
    }

    async fn get_flash_item(
        &self,
        item_id: ItemId,
    ) -> Result<Option<FlashItemSnapshot>, InventoryPortError> {
        let now = self.clock.now();
        let guard = self.inner.lock().unwrap();
        Ok(guard.items.get(&item_id).cloned().map(|mut item| {
            item.window_status = compute_window(now, item.sale_starts_at, item.sale_ends_at);
            item
        }))
    }

    async fn reserve_stock(
        &self,
        req: ReserveStockRequest,
    ) -> Result<ReserveStockOutcome, InventoryPortError> {
        let now = self.clock.now();
        let mut guard = self.inner.lock().unwrap();
        let item = guard
            .items
            .get_mut(&req.item_id)
            .ok_or(InventoryPortError::ItemNotFound(req.item_id))?;
        let status = compute_window(now, item.sale_starts_at, item.sale_ends_at);
        if status != WindowStatus::Active {
            return Err(InventoryPortError::SaleWindowNotActive { status });
        }
        if item.available_stock < req.quantity.get() {
            return Err(InventoryPortError::InsufficientStock);
        }
        item.available_stock -= req.quantity.get();
        item.window_status = status;
        let snapshot = item.clone();
        guard.reservations.insert(
            req.reservation_id,
            FakeReservation {
                item_id: req.item_id,
                quantity: req.quantity.get(),
                status: "reserved",
            },
        );
        Ok(ReserveStockOutcome {
            reservation_id: req.reservation_id,
            item_snapshot: snapshot,
            idempotent_replay: false,
        })
    }

    async fn confirm_stock_deduction(
        &self,
        req: ConfirmStockDeductionRequest,
    ) -> Result<ConfirmStockOutcome, InventoryPortError> {
        let mut guard = self.inner.lock().unwrap();
        let res = guard
            .reservations
            .get_mut(&req.reservation_id)
            .ok_or(InventoryPortError::ReservationNotFound(req.reservation_id))?;
        let replay = res.status == "confirmed";
        res.status = "confirmed";
        Ok(ConfirmStockOutcome {
            reservation_id: req.reservation_id,
            idempotent_replay: replay,
        })
    }

    async fn release_reserved_stock(
        &self,
        req: ReleaseReservedStockRequest,
    ) -> Result<ReleaseStockOutcome, InventoryPortError> {
        let now = self.clock.now();
        let mut guard = self.inner.lock().unwrap();
        guard.release_calls += 1;
        let res = guard
            .reservations
            .get_mut(&req.reservation_id)
            .ok_or(InventoryPortError::ReservationNotFound(req.reservation_id))?;
        if res.status == "released" {
            return Ok(ReleaseStockOutcome {
                reservation_id: req.reservation_id,
                restocked_to_available: false,
                idempotent_replay: true,
            });
        }
        res.status = "released";
        let item_id = res.item_id;
        let qty = res.quantity;
        let item = guard.items.get_mut(&item_id).unwrap();
        let active =
            compute_window(now, item.sale_starts_at, item.sale_ends_at) == WindowStatus::Active;
        if active {
            item.available_stock += qty;
        }
        Ok(ReleaseStockOutcome {
            reservation_id: req.reservation_id,
            restocked_to_available: active,
            idempotent_replay: false,
        })
    }

    async fn return_deducted_stock(
        &self,
        req: ReturnDeductedStockRequest,
    ) -> Result<ReturnStockOutcome, InventoryPortError> {
        let now = self.clock.now();
        let mut guard = self.inner.lock().unwrap();
        guard.return_calls += 1;
        let res = guard
            .reservations
            .get_mut(&req.reservation_id)
            .ok_or(InventoryPortError::ReservationNotFound(req.reservation_id))?;
        if res.status == "returned" {
            return Ok(ReturnStockOutcome {
                reservation_id: req.reservation_id,
                restocked_to_available: false,
                idempotent_replay: true,
            });
        }
        res.status = "returned";
        let item_id = res.item_id;
        let qty = res.quantity;
        let item = guard.items.get_mut(&item_id).unwrap();
        let active =
            compute_window(now, item.sale_starts_at, item.sale_ends_at) == WindowStatus::Active;
        if active {
            item.available_stock += qty;
        }
        Ok(ReturnStockOutcome {
            reservation_id: req.reservation_id,
            restocked_to_available: active,
            idempotent_replay: false,
        })
    }
}

pub fn test_state(pool: PgPool, clock: Arc<FixedClock>) -> (AppState, Arc<FakeInventoryPort>) {
    flashmart_ordering_slices::link();
    let id_gen = Arc::new(SequenceIdGenerator::new(1));
    let event_bus = Arc::new(InMemoryEventBus::new());
    let fake_port = Arc::new(FakeInventoryPort::new(clock.clone()));
    let state = AppState::new(
        pool,
        clock,
        id_gen,
        fake_port.clone() as Arc<dyn InventoryPort>,
        event_bus,
        Duration::minutes(15),
    );
    (state, fake_port)
}

pub fn test_router(state: AppState) -> axum::Router {
    collect_active_routes(state)
}
