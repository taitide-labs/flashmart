CREATE SCHEMA IF NOT EXISTS inventory;

CREATE TABLE inventory.flash_items (
    id UUID PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    item_kind TEXT NOT NULL CHECK (item_kind IN ('digital', 'physical')),
    price_cents BIGINT NOT NULL CHECK (price_cents >= 0),
    currency TEXT NOT NULL DEFAULT 'CNY' CHECK (currency = 'CNY'),
    total_stock INT NOT NULL CHECK (total_stock >= 0),
    available_stock INT NOT NULL CHECK (available_stock >= 0),
    reserved_stock INT NOT NULL DEFAULT 0 CHECK (reserved_stock >= 0),
    confirmed_stock INT NOT NULL DEFAULT 0 CHECK (confirmed_stock >= 0),
    returned_after_close_stock INT NOT NULL DEFAULT 0 CHECK (returned_after_close_stock >= 0),
    per_user_limit INT NOT NULL CHECK (per_user_limit >= 1),
    sale_starts_at TIMESTAMPTZ NOT NULL,
    sale_ends_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_flash_items_window CHECK (sale_ends_at > sale_starts_at),
    CONSTRAINT chk_flash_items_stock_conservation CHECK (
        available_stock + reserved_stock + confirmed_stock + returned_after_close_stock = total_stock
    )
);

CREATE INDEX idx_flash_items_sale_window
    ON inventory.flash_items (sale_starts_at, sale_ends_at);

CREATE TABLE inventory.stock_reservations (
    id UUID PRIMARY KEY,
    item_id UUID NOT NULL REFERENCES inventory.flash_items (id),
    buyer_id TEXT NOT NULL,
    quantity INT NOT NULL CHECK (quantity >= 1),
    status TEXT NOT NULL CHECK (status IN ('reserved', 'confirmed', 'released', 'returned')),
    idempotency_key TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_stock_reservations_item_status
    ON inventory.stock_reservations (item_id, status);

CREATE TABLE inventory.outbox_events (
    id UUID PRIMARY KEY,
    event_type TEXT NOT NULL,
    aggregate_id UUID NOT NULL,
    payload JSONB NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    published_at TIMESTAMPTZ
);

CREATE INDEX idx_inventory_outbox_unpublished
    ON inventory.outbox_events (occurred_at)
    WHERE published_at IS NULL;
