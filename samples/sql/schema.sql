-- Schema for the sample order system: tables with constraints, indexes,
-- a view, a trigger, and the seed rows a reviewer needs to see it work.
-- Written for PostgreSQL 16.

BEGIN;

CREATE SCHEMA IF NOT EXISTS shop;

SET search_path TO shop, public;

CREATE TYPE order_status AS ENUM ('draft', 'placed', 'picked', 'shipped', 'cancelled');

CREATE TABLE customers (
    id              BIGSERIAL   PRIMARY KEY,
    email           TEXT        NOT NULL UNIQUE,
    display_name    TEXT        NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    marketing_optin BOOLEAN     NOT NULL DEFAULT FALSE,
    CONSTRAINT customers_email_lower CHECK (email = lower(email))
);

CREATE TABLE addresses (
    id          BIGSERIAL PRIMARY KEY,
    customer_id BIGINT      NOT NULL REFERENCES customers (id) ON DELETE CASCADE,
    line1       TEXT        NOT NULL,
    line2       TEXT,
    city        TEXT        NOT NULL,
    postcode    TEXT        NOT NULL,
    country     CHAR(2)     NOT NULL DEFAULT 'GB',
    is_default  BOOLEAN     NOT NULL DEFAULT FALSE
);

CREATE TABLE products (
    sku          TEXT           PRIMARY KEY,
    name         TEXT           NOT NULL,
    pennies      INTEGER        NOT NULL CHECK (pennies >= 0),
    vat_percent  NUMERIC(4, 2)  NOT NULL DEFAULT 20.00,
    stock        INTEGER        NOT NULL DEFAULT 0 CHECK (stock >= 0),
    discontinued DATE
);

CREATE TABLE orders (
    id           BIGSERIAL     PRIMARY KEY,
    customer_id  BIGINT        NOT NULL REFERENCES customers (id),
    address_id   BIGINT        REFERENCES addresses (id),
    status       order_status  NOT NULL DEFAULT 'draft',
    placed_at    TIMESTAMPTZ,
    total_pennies INTEGER      NOT NULL DEFAULT 0,
    CONSTRAINT orders_placed_when_not_draft
        CHECK (status = 'draft' OR placed_at IS NOT NULL)
);

CREATE TABLE order_lines (
    order_id  BIGINT   NOT NULL REFERENCES orders (id) ON DELETE CASCADE,
    sku       TEXT     NOT NULL REFERENCES products (sku),
    quantity  INTEGER  NOT NULL CHECK (quantity > 0),
    pennies   INTEGER  NOT NULL,
    PRIMARY KEY (order_id, sku)
);

CREATE INDEX orders_customer_status_idx ON orders (customer_id, status);
CREATE INDEX orders_placed_at_idx ON orders (placed_at DESC) WHERE placed_at IS NOT NULL;
CREATE UNIQUE INDEX addresses_one_default_per_customer
    ON addresses (customer_id) WHERE is_default;

CREATE VIEW order_totals AS
SELECT
    o.id                                   AS order_id,
    c.email                                AS customer_email,
    o.status,
    count(l.sku)                           AS line_count,
    sum(l.quantity)                        AS item_count,
    sum(l.quantity * l.pennies)            AS gross_pennies,
    round(sum(l.quantity * l.pennies) * 0.2) AS vat_pennies
FROM orders o
JOIN customers c ON c.id = o.customer_id
LEFT JOIN order_lines l ON l.order_id = o.id
GROUP BY o.id, c.email, o.status;

CREATE OR REPLACE FUNCTION refresh_order_total() RETURNS TRIGGER AS $$
BEGIN
    UPDATE orders
       SET total_pennies = COALESCE((
               SELECT sum(quantity * pennies)
                 FROM order_lines
                WHERE order_id = COALESCE(NEW.order_id, OLD.order_id)
           ), 0)
     WHERE id = COALESCE(NEW.order_id, OLD.order_id);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER order_lines_refresh_total
AFTER INSERT OR UPDATE OR DELETE ON order_lines
FOR EACH ROW EXECUTE FUNCTION refresh_order_total();

INSERT INTO customers (email, display_name, marketing_optin) VALUES
    ('ada@example.com',   'Ada Lovelace', TRUE),
    ('alan@example.com',  'Alan Turing',  FALSE),
    ('grace@example.com', 'Grace Hopper', TRUE);

INSERT INTO products (sku, name, pennies, stock) VALUES
    ('WID-1', 'Widget',   450, 12),
    ('GAD-7', 'Gadget',  1999,  2),
    ('SPR-3', 'Sprocket',  75,  0);

INSERT INTO orders (customer_id, status, placed_at) VALUES
    (1, 'placed', now()),
    (2, 'draft',  NULL);

INSERT INTO order_lines (order_id, sku, quantity, pennies) VALUES
    (1, 'WID-1', 3, 450),
    (1, 'GAD-7', 1, 1999);

COMMIT;

-- The report a human runs after seeding.
SELECT customer_email, status, item_count, gross_pennies
  FROM order_totals
 ORDER BY gross_pennies DESC NULLS LAST;
