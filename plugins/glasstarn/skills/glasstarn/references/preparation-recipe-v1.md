# Preparation recipe version 1

Use SHA-256 values returned by `workspace inventory`. The engine accepts this
bounded JSON structure:

```json
{
  "version": 1,
  "inputs": [
    { "id": "orders", "sha256": "<inventory hash>" },
    { "id": "suppliers", "sha256": "<inventory hash>" }
  ],
  "events": [
    {
      "input": "orders",
      "case_id": "order_id",
      "activity": { "value": "Order created" },
      "timestamp": "created_at",
      "optional_timestamp": false,
      "resource": "created_by",
      "case_attributes": [
        { "column": "company_code", "as": "company_code" }
      ]
    }
  ],
  "lookups": [
    {
      "event_input": "orders",
      "event_key": "supplier_id",
      "input": "suppliers",
      "lookup_key": "supplier_id",
      "fields": [
        { "column": "region", "as": "supplier_region" }
      ],
      "allow_unmatched": false
    }
  ]
}
```

`activity` must contain exactly one of `column` or `value`. Event rules require
`input`, `case_id`, `activity`, and `timestamp`. They may also contain
`optional_timestamp`, `end_timestamp`, `event_order`, `resource`, `lifecycle`,
`cost`, and `case_attributes`. Omit unused optional fields.

Each `case_attributes` or lookup `fields` item names an input `column` and an
output name in `as`. Output names cannot overwrite standard fields or another
field used for the same event input.

An optional blank timestamp skips that event and is counted. Required blank
case, activity, or timestamp values block the preparation. Lookups are strictly
many-to-one: blank or duplicate lookup keys block. Unmatched event keys block
unless `allow_unmatched` was explicitly confirmed. Source input and spreadsheet
row are added automatically to every prepared event.
