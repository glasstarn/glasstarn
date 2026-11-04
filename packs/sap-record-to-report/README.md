# SAP journal entries and month-end close (workflow not yet tested)

Accounting documents on SAP ECC and S/4HANA on-premise or private cloud, from
entry to reversal or clearing, followed by journal entry. The pack consumes
local exports. Its base journal path has scoped S/4HANA trial validation;
optional workflow still has only synthetic coverage.

| Table | Contents |
|---|---|
| BKPF | Accounting headers, including both original and reversal documents |
| VBKPF | Optional parked documents |
| SWWWIHEAD, SWW_CONTOB | Optional completed workflow tasks linked to FIPP objects |
| BSAS | Optional cleared items on open-item G/L accounts |
| CDHDR, CDPOS | Optional accounting changes |

Steps include Park journal entry, Post journal entry, Change journal entry,
Reverse journal entry and Clear item. Reversals remain objects in their own
right, retaining their own changes and clearing history as well as their link
to the original entry. Posting date and period remain separate from entry time.

The optional workflow step is **Complete journal workflow task**. Completion
does not establish that a task was an approval. The workflow join uses
SWW_CONTOB `OBJKEY`, restricted to FIPP, and SWWWIHEAD `WI_AED_TIMESTAMP`.
That compact timestamp is UTC, including its fractional precision. Export
FIPP references and their linked headers together; unrelated object types do
not participate in the join.

Clearing retains date precision. Other system times use the confirmed fixed
offset; daylight-saving changes are not inferred. Period opening/closing,
reconciliation and consolidation are not covered by these exports.

## Live validation

`pack check` and `pack build` passed on scoped SAP S/4HANA 2025 FPS01 trial
exports. Four journal timelines matched the source records: common and slow
cases, a reversal with clearing, and a change on a reversal document. Required
reference checks remained enabled, and reversal objects resolved the earlier
clearing/change reconciliation failures.

The trial's workflow reference query returned no matching FIPP rows, so its
linked header set was empty. This is not a test of live workflow
behavior or proof that no other workflows exist in the system. The pack's
overall status remains untested until that optional path is exercised on
matching source records. Synthetic tests cover the actual column names, UTC
timestamp and unrelated-object filtering.

Product names are trademarks of their owners and are used only to say which
system this pack reads. Glasstarn is not affiliated with or endorsed by them; see
[trademarks and compatibility](../../docs/trademarks.md).
