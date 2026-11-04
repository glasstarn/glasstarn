PRAGMA foreign_keys = ON;
BEGIN;

CREATE TABLE event_map_type (
  ocel_type TEXT PRIMARY KEY,
  ocel_type_map TEXT NOT NULL UNIQUE
);
INSERT INTO event_map_type VALUES
  ('place order', 'PlaceOrder'),
  ('create package', 'CreatePackage'),
  ('deliver package', 'DeliverPackage');

CREATE TABLE object_map_type (
  ocel_type TEXT PRIMARY KEY,
  ocel_type_map TEXT NOT NULL UNIQUE
);
INSERT INTO object_map_type VALUES
  ('order', 'Order'),
  ('item', 'Item'),
  ('package', 'Package');

CREATE TABLE event (
  ocel_id TEXT PRIMARY KEY,
  ocel_type TEXT NOT NULL REFERENCES event_map_type (ocel_type)
);
INSERT INTO event VALUES
  ('e-place', 'place order'),
  ('e-pack', 'create package'),
  ('e-deliver', 'deliver package');

CREATE TABLE object (
  ocel_id TEXT PRIMARY KEY,
  ocel_type TEXT NOT NULL REFERENCES object_map_type (ocel_type)
);
INSERT INTO object VALUES
  ('order-1', 'order'),
  ('item-1', 'item'),
  ('item-2', 'item'),
  ('package-1', 'package');

CREATE TABLE event_PlaceOrder (
  ocel_id TEXT PRIMARY KEY REFERENCES event (ocel_id),
  ocel_time TIMESTAMP NOT NULL,
  "item count" INTEGER
);
INSERT INTO event_PlaceOrder VALUES ('e-place', '1970-01-01 00:00:00', 2);

CREATE TABLE event_CreatePackage (
  ocel_id TEXT PRIMARY KEY REFERENCES event (ocel_id),
  ocel_time TIMESTAMP NOT NULL
);
INSERT INTO event_CreatePackage VALUES ('e-pack', '1970-01-02 00:00:00');

CREATE TABLE event_DeliverPackage (
  ocel_id TEXT PRIMARY KEY REFERENCES event (ocel_id),
  ocel_time TIMESTAMP NOT NULL,
  successful BOOLEAN
);
INSERT INTO event_DeliverPackage VALUES ('e-deliver', '1970-01-03 00:00:00', 1);

CREATE TABLE object_Order (
  ocel_id TEXT NOT NULL REFERENCES object (ocel_id),
  ocel_time TIMESTAMP NOT NULL,
  ocel_changed_field TEXT,
  status TEXT
);
INSERT INTO object_Order VALUES
  ('order-1', '1970-01-01 00:00:00', NULL, 'open'),
  ('order-1', '1970-01-02 00:00:00', 'status', 'shipped');

CREATE TABLE object_Item (
  ocel_id TEXT NOT NULL REFERENCES object (ocel_id),
  ocel_time TIMESTAMP NOT NULL,
  ocel_changed_field TEXT,
  price REAL
);
INSERT INTO object_Item VALUES
  ('item-1', '1970-01-01 00:00:00', NULL, 10.5),
  ('item-2', '1970-01-01 00:00:00', NULL, 20.0);

CREATE TABLE object_Package (
  ocel_id TEXT NOT NULL REFERENCES object (ocel_id),
  ocel_time TIMESTAMP NOT NULL,
  ocel_changed_field TEXT
);
INSERT INTO object_Package VALUES ('package-1', '1970-01-01 00:00:00', NULL);

CREATE TABLE event_object (
  ocel_event_id TEXT NOT NULL REFERENCES event (ocel_id),
  ocel_object_id TEXT NOT NULL REFERENCES object (ocel_id),
  ocel_qualifier TEXT NOT NULL,
  PRIMARY KEY (ocel_event_id, ocel_object_id, ocel_qualifier)
);
INSERT INTO event_object VALUES
  ('e-place', 'order-1', 'order'),
  ('e-place', 'item-1', 'item'),
  ('e-place', 'item-2', 'item'),
  ('e-pack', 'package-1', 'result'),
  ('e-pack', 'item-1', 'packed item'),
  ('e-pack', 'item-2', 'packed item'),
  ('e-deliver', 'package-1', 'package');

CREATE TABLE object_object (
  ocel_source_id TEXT NOT NULL REFERENCES object (ocel_id),
  ocel_target_id TEXT NOT NULL REFERENCES object (ocel_id),
  ocel_qualifier TEXT NOT NULL,
  PRIMARY KEY (ocel_source_id, ocel_target_id, ocel_qualifier)
);
INSERT INTO object_object VALUES
  ('item-1', 'order-1', 'part of'),
  ('item-1', 'package-1', 'packed in'),
  ('item-2', 'order-1', 'part of');

COMMIT;
