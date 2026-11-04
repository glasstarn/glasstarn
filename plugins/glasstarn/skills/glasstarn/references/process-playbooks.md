# Process playbooks: what the business wants

Recognise the process from its step names, item types and fields, then lead
with what "good" means for it. State the read in one line instead of asking:
"This looks like order-to-cash, so I've focused on getting paid sooner. Tell me
if your goal is different." Change course at once if the user corrects it.

Targets (SLAs, payment terms, promised turnaround) come from the user or their
documents. Without them, report the measured figures and the spread; never
invent a benchmark or an industry average.

| Process (typical signs) | What the business wants | Lead with |
|---|---|---|
| Order-to-cash, invoices out (order, delivery, invoice issued, payment received, dunning, credit note) | Cash sooner, fewer disputes | days from order or invoice to cash; unpaid and late value; disputes, credit notes and reminders; orders not yet invoiced |
| Purchase-to-pay, invoices in (requisition, purchase order, goods receipt, invoice received, approval, payment) | Pay on time, no surprises, controlled spend | approval delays; invoices without a matching order or receipt; price or quantity changes after ordering; paid late or very early; repeated approvals |
| Sales pipeline (lead, opportunity, stage changes, closed won/lost) | Win more, faster | time per stage; where deals stall or are lost, with value; deals that move backwards; won deals that wait for an order |
| IT or service tickets (created, assigned, reassigned, resolved, reopened, closed) | Fast first resolution | time to resolve against the SLA if supplied; reassignments between teams; reopened tickets; backlog age |
| Claims or cases (submitted, documents requested, assessed, approved, paid, rejected) | Fast, fair decisions without leakage | cycle time; loops waiting for documents; rework; outcome differences between groups (as associations only) |
| Onboarding or HR (offer, contract, equipment, access, start) | Ready on day one | time to ready; which hand-off delays it; steps done after the start date |
| Permits or public services (application, completeness check, review, decision, issued) | Predictable turnaround | time against the promised period if supplied; missing-information loops; backlog; variation between offices |
| Manufacturing or maintenance (order released, set-up, run, inspection, rework, repair) | Throughput and first-time quality | waiting before operations; rework and scrap loops; repairs repeating on the same asset |
| Logistics (booking, pick, pack, load, transport, delivery, return) | On-time, complete delivery | late or partial deliveries; failed delivery attempts; returns; dwell time at hand-offs |

When several fit, choose the one the step names support best and say so. When
none fits, use the general goal "less time, less rework, fewer items stuck".

## Health check when no question is asked

Run these with engine queries, never with arithmetic in the model:

1. **Speed:** median and slow-end time end to end; which hand-off carries the
   most waiting (`findings`, `bottlenecks`, `follow`).
2. **Leakage:** where items stop or drop out, and the value involved
   (`findings`, `journey`, `values` by last step).
3. **Rework:** which steps repeat, in what share of items, and the time added.
4. **Variation:** how many routes; how much the main routes cover; whether
   items off the main routes take longer.
5. **Money:** total, typical and open value when a number field holds amounts
   (`values`). If none does, say so and use counts and times.
6. **People and automation:** hand-offs between teams and repetitive steps,
   only when resources are recorded (`automation` targeting).
7. **Groups:** which region, type, amount band or person goes with slow items,
   or with one slow hand-off (the engine's driver findings).
8. **Skipped steps and controls:** a usual step, such as an approval, missing
   before a later one, and who or what those items share (the engine's
   skipped-step findings). In an improvement investigation report it as a
   finding; do not turn the work into an audit.

Deliver, in this order and in the playbook's language:

- **One headline answer** in a sentence: the biggest supported fact framed by
  the business goal. Example: "A won deal takes 41 days to become cash; almost
  half of that is customers paying invoices."
- **Every engine finding**, grouped as speed, repeated work, groups and
  skipped steps, largest first, each with: what happens (with the
  engine's number), why it matters for the goal (time or money involved, as
  computed by the engine), the evidence reference, how confident the evidence
  is (coverage, denominators), and the next check that would confirm a cause.
- **What is working well**, in one line, when the engine shows a fast, steady
  hand-off.
- **Questions worth asking next**, two or three, specific to what was found:
  "Shall I check whether the late payers are the larger discounted deals?"

Save the headline, observations (kind `observed`) and possible causes (kind
`hypothesis`) with `workspace record` so the browser Summary shows them.
Label every possible cause as a hypothesis until a query supports it.
