CREATE SCHEMA IF NOT EXISTS ordering;

CREATE TABLE ordering.flash_orders (
    id UUID PRIMARY KEY,
    reservation_id UUID NOT NULL UNIQUE,
    buyer_id TEXT NOT NULL,
    item_id UUID NOT NULL,
    item_title TEXT NOT NULL,
    item_kind TEXT NOT NULL CHECK (item_kind IN ('digital', 'physical')),
    quantity INT NOT NULL CHECK (quantity >= 1),
    unit_price_cents BIGINT NOT NULL CHECK (unit_price_cents >= 0),
    total_amount_cents BIGINT NOT NULL CHECK (total_amount_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'CNY' CHECK (currency = 'CNY'),
    status TEXT NOT NULL CHECK (status IN ('pending_payment', 'paid', 'cancelled', 'expired')),
    version INT NOT NULL DEFAULT 1 CHECK (version >= 1),
    payment_deadline TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    paid_at TIMESTAMPTZ,
    cancelled_at TIMESTAMPTZ,
    expired_at TIMESTAMPTZ,
    fulfillment_hint TEXT
);

CREATE INDEX idx_flash_orders_buyer_created
    ON ordering.flash_orders (buyer_id, created_at DESC);

CREATE INDEX idx_flash_orders_active_quota
    ON ordering.flash_orders (buyer_id, item_id)
    WHERE status IN ('pending_payment', 'paid');

CREATE INDEX idx_flash_orders_pending_deadline
    ON ordering.flash_orders (payment_deadline)
    WHERE status = 'pending_payment';

CREATE TABLE ordering.idempotency_records (
    buyer_id TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    status_code SMALLINT NOT NULL,
    response_body JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (buyer_id, endpoint, idempotency_key)
);

CREATE TABLE ordering.outbox_events (
    id UUID PRIMARY KEY,
    event_type TEXT NOT NULL,
    aggregate_id UUID NOT NULL,
    payload JSONB NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    published_at TIMESTAMPTZ
);

CREATE INDEX idx_ordering_outbox_unpublished
    ON ordering.outbox_events (occurred_at)
    WHERE published_at IS NULL;
