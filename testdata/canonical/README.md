# Canonical Test Logs

Fourteen hand-built logs, each small enough to verify completely on paper. Each
targets one specific hazard.

**These expected results were derived by hand, not by running the engine.**
That is the entire point — a test whose expected output came from the code
under test proves only that the code is consistent with itself.

Where a log has no single correct answer, that is stated explicitly. Those are
**product decisions**, and the engine's job is to surface them to the user
rather than resolve them silently.

---

## 01-sequential.csv — the baseline

3 cases, 12 events, 4 activities, **1 variant**.

`Receive Order → Check Credit → Approve Order → Ship Order`

| Check | Expected |
|---|---|
| DFG edges | 3 edges, each with frequency 3 |
| Throughput | ORDER-1 30h, ORDER-2 55h, ORDER-3 26h; mean **37h** |
| Mean wait before Approve Order | (22 + 43 + 7) / 3 = **24h** — the bottleneck |
| Mean wait before Ship Order | (6 + 8 + 18) / 3 = 10h 40m |
| Mean wait before Check Credit | (2 + 4 + 1) / 3 = 2h 20m |

If this log doesn't produce exactly these numbers, nothing else matters.

---

## 02-self-loop.csv — immediate repetition

3 cases, 12 events, **3 variants**.

| Check | Expected |
|---|---|
| Self-loop occurrences | TICKET-1: 2, TICKET-3: 1 → **3 total** |
| Cases containing rework | 2 of 3 (**67%**) |
| `Investigate` occurrences | 3 + 1 + 2 = **6** |
| DFG | `Open→Investigate` 3, `Investigate→Investigate` 3, `Investigate→Resolve` 3 |

---

## 03-long-loop.csv — a cycle longer than one step

3 cases, 15 events, **3 variants**. Rework here is a *cycle*, not a self-loop.

`Review → Request Documents → Review`

| Check | Expected |
|---|---|
| Self-loops | **0** — no activity immediately repeats |
| `Review` occurrences | 3 + 1 + 2 = **6** |
| Cycle traversals | CLAIM-1: 2, CLAIM-3: 1 → **3** |
| DFG | `Submit→Review` 3, `Review→Request Documents` 3, `Request Documents→Review` 3, `Review→Approve` 3 |

A rework metric that counts only self-loops will report zero here and be
wrong.

---

## 04-parallel.csv — genuine concurrency, and the DFG's central flaw

4 cases, 16 events, **2 variants**, 2 cases each.

- `Receive → Pick Items → Print Invoice → Ship`
- `Receive → Print Invoice → Pick Items → Ship`

| Check | Expected |
|---|---|
| DFG edges | 6, including **both** `Pick→Print` (2) and `Print→Pick` (2) |
| Variant coverage | 2 variants cover 100% of cases |

**This is the log that proves the DFG lies.** Its graph contains a cycle
between Pick Items and Print Invoice, implying a case could loop
`Pick→Print→Pick→Print` indefinitely. **No such case exists.** The two
activities are concurrent, and a directly-follows graph cannot express that.

This is why variant coverage must be shown next to the map, and why the
artifact is labelled a directly-follows graph rather than "the process."

---

## 05-choice.csv — exclusive branching

5 cases, 20 events, **2 variants**: Accept 3 cases (60%), Reject 2 (40%).

| Check | Expected |
|---|---|
| DFG | `Submit→Assess` 5, `Assess→Accept` 3, `Assess→Reject` 2, `Accept→Notify` 3, `Reject→Notify` 2 |
| Edge conservation | Out-edges of `Assess` sum to 5 = its occurrence count |

That last row is a core invariant: for a non-terminal activity, outgoing edge
frequencies must sum to its occurrence count.

---

## 06-timestamp-ties.csv — identical timestamps

3 cases, 9 events. **All three cases share one variant**:
`Receive → Validate → Post`.

That result is only reachable by applying the documented tie-break, since
timestamps alone do not determine the order.

| Case | Tie |
|---|---|
| BATCH-1 | all three events at `09:00:00` |
| BATCH-2 | Receive and Validate tied at `09:00:00` |
| BATCH-3 | Validate and Post tied at `11:00:00` |

| Check | Expected |
|---|---|
| Ordering | Falls through to original file order |
| Validation report | Flags **3 cases with tied timestamps**, and notes no order field was mapped |
| Zero-duration edges | BATCH-1 both edges 0s; BATCH-2 first edge 0s; BATCH-3 second edge 0s |
| Determinism | Reprocessing must give byte-identical output |

---

## 07-missing-timestamps.csv — absent values

4 cases, 12 rows. **4 rows have no timestamp**: REQ-1 `Review`, and all three
REQ-3 rows.

| Check | Expected |
|---|---|
| Validation report | 4 rows without a parseable timestamp, with examples |
| REQ-3 | Has **no** usable timestamp — cannot be ordered at all |
| Silent dropping | **Forbidden.** Never discard rows without reporting. |

**Open decision:** whether a blank timestamp means "step did not occur"
(exclude the event) or "data is missing" (flag the case as incomplete). These
have different consequences, and the user must choose. The engine reports; it
does not guess.

---

## 08-single-event.csv — one-event cases

5 cases, 8 events. **3 of 5 cases have exactly one event.**

| Check | Expected |
|---|---|
| Variants | **3**: `[Created]` ×3, `[Created,Qualified,Converted]` ×1, `[Created,Qualified]` ×1 |
| `Created` node frequency | **5** |
| DFG edges | `Created→Qualified` 2, `Qualified→Converted` 1 |
| Single-event cases | Contribute a **node occurrence but no edge** |
| Throughput | Zero for single-event cases — must not be an error or a null |

A pipeline that assumes every case has at least one transition breaks here.
This shape is extremely common in real funnels.

---

## 09-out-of-order.csv — file order is not chronological

2 cases, 8 events, deliberately scrambled in the file.

| Check | Expected |
|---|---|
| Both cases | `Receive → Match → Approve → Pay` — **1 variant** |
| INV-1 throughput | Nov 2 09:00 → Nov 5 16:00 = **79h** |
| INV-2 throughput | Nov 3 10:00 → Nov 6 09:00 = **71h** |

If ingestion ever trusts file order over timestamps, this log produces
nonsense — which is exactly what it is here to catch.

---

## 10-duplicate-rows.csv — exact duplicates

3 cases, 12 rows, **3 of them exact duplicates of the preceding row**
(PO-1 one, PO-2 two).

| Check | Expected |
|---|---|
| Validation report | Flags 3 duplicate rows, identified by case + activity + timestamp |
| If duplicates kept | 3 variants; `Approve→Approve` and `Send→Send` self-loops appear |
| If duplicates removed | **1 variant** `Create → Approve → Send` for all cases |

**Open decision, and it matters.** A repeated approval at the *same
timestamp* is almost always an extraction artifact; a repeated approval
*minutes apart* is usually real rework. Deduplicating silently would erase
genuine rework signal; keeping duplicates silently invents it. The user must
decide, with the counts in front of them.

---

## 11-dst-boundary.csv — clock changes

3 cases, 9 events, using US daylight-saving transitions.

- **SHIFT-1** — 8 March 2026, the spring-forward day (02:00 → 03:00)
- **SHIFT-2** — 7 March 2026, an ordinary day, as the control
- **SHIFT-3** — 1 November 2026, the fall-back day (02:00 → 01:00), where
  local time `01:30` occurs **twice**

| Check | Expected |
|---|---|
| Negative durations | **Never**, under any timezone policy |
| Naive/UTC policy | Every gap is literal local arithmetic: 2h, 2h for all three cases |
| US-Eastern local policy | SHIFT-1's first gap is **1h real**, not 2h; SHIFT-3's `01:30` is ambiguous |
| Reporting | The applied timezone policy must be recorded and visible |

There is no single right answer here — the answer *depends on the declared
timezone policy*, which is precisely why the policy is an explicit, recorded
setting rather than a default nobody sees.

---

## 12-mixed-formats.csv — three timestamp formats in one column

3 cases, 9 events. Formats present:

| Format | Example |
|---|---|
| ISO with space | `2026-01-05 09:00:00` |
| Day-first slash, no seconds | `05/01/2026 11:00` |
| ISO 8601 with `Z` | `2026-01-05T14:00:00Z` |

| Check | Expected |
|---|---|
| Validation report | Flags **mixed formats within one column** |
| `05/01/2026` | **Ambiguous** — 5 January (day-first) or 1 May (month-first) |
| Resolution | Surrounding rows imply day-first, but the engine must **ask, not assume** |
| Silent coercion | **Forbidden** |
| `Z` suffix | Carries an explicit UTC offset while the other rows carry none — must be reported, not blended |

Guessing wrong here shifts every date by months and corrupts every downstream
number while looking completely plausible.

---

## 13-messy-labels.csv — label hygiene and encoding

4 cases, 11 events.

| Raw label | Issue |
|---|---|
| `Approve` / `approve` / `APPROVE` | case differences |
| `Approve ` | trailing whitespace |
| `Prüfen`, `Veröffentlichen` | non-ASCII — must round-trip exactly |
| `"Approve, Final"` | embedded comma, quoted — must parse as **one** field |

| Check | Expected |
|---|---|
| Distinct raw labels | **8** |
| After case + whitespace normalization | **5** |
| Engine behavior | Reports near-duplicates as a data-quality finding; does **not** merge silently |
| Unicode | Preserved byte-for-byte in output and evidence |
| CSV parsing | `Approve, Final` is one activity, not two fields |

The dictionary deliberately treats these as distinct
(`pm-core/src/dict.rs`). Merging is a user decision.

---

## 14-wide-format.csv — the real-world shape

5 tickets in **wide format** — one row per case, five timestamp columns, plus a
`priority` case attribute. This is the shape most ticketing and ERP systems
actually export.

Unpivoting on the four timestamp columns yields **16 events across 5 cases**:

| Ticket | Events | Trace |
|---|---|---|
| T-1001 | 4 | Created → Assigned → Resolved → Closed |
| T-1002 | 4 | Created → Assigned → Resolved → Closed |
| T-1003 | 5 | Created → Assigned → Escalated → Resolved → Closed |
| T-1004 | 2 | Created → Assigned |
| T-1005 | 1 | Created |

| Check | Expected |
|---|---|
| Events | **16** |
| Variants | **4** |
| Blank cells | Produce **no event** — the step did not occur |
| In-flight cases | T-1004 and T-1005 are open, not broken |
| `priority` | Retained as a case attribute for filtering and cohort comparison |

Note this log tests the opposite convention from `07-missing-timestamps.csv`:
here a blank unambiguously means "did not happen." That the same symbol means
different things in different layouts is why the wide-to-long builder is an
explicit, user-confirmed mapping step.

---

## Coverage summary

| Hazard | Log |
|---|---|
| Baseline correctness | 01 |
| Self-loops | 02 |
| Longer cycles | 03 |
| Concurrency / DFG limitation | 04 |
| Exclusive choice | 05 |
| Timestamp ties | 06 |
| Missing timestamps | 07 |
| Single-event cases | 08 |
| Non-chronological file order | 09 |
| Duplicate rows | 10 |
| DST transitions | 11 |
| Mixed timestamp formats | 12 |
| Label hygiene, unicode, CSV quoting | 13 |
| Wide-format input, case attributes | 14 |

## Decisions these logs surface

Four points where there is no single correct answer and the product must let
the user decide, with counts visible:

1. **07** — does a blank timestamp mean "didn't happen" or "data missing"?
2. **10** — are same-timestamp duplicates artifacts or real repetition?
3. **11** — which timezone policy applies?
4. **12/13** — how are ambiguous date formats and near-duplicate labels resolved?
