# Intake CONTRACT v1

## What an item is

An item is a request someone sends in, its attachments, the triage and work proposed for it, and its history from
receipt through release or a terminal decision. The source is an opaque slug and the requester is an opaque key.
The crate knows nothing about a tracker, database, application or model vendor. Attachments contain metadata;
the store holds their bytes separately. All lengths below are UTF-8 bytes, not characters.

## Text and JSON rules

Every struct refuses unknown fields on deserialize and validates through the same rules as its public `validate()`
method. JSON fields are camelCase except the explicit `requesterKey` and `ref` names. Optional fields serialize
as `null`; `NewItem.snapshot` can also be absent. Enum spellings are exact and case-sensitive.

Every text field refuses U+0000–U+001F (except the block exceptions below), U+007F, U+0080–U+009F, U+2028,
U+2029, U+061C, U+200E, U+200F, U+202A–U+202E and U+2066–U+2069. A line is nonempty, has no leading or
trailing Unicode whitespace, and allows no control characters. Callers trim lines. A block permits newline and
tab, but never carriage return; callers normalize line endings. A block may be empty only where its minimum is 0.
A slug matches `[a-z][a-z0-9-]*`, is 1–32 bytes, does not end in `-` and contains no `--`.

`ItemId` is a string `IN-<n>`, where n is a u64 decimal integer at least 1, without a sign or leading zero.
`Source` is a slug. `RequesterKey` is a line of at most 320 bytes. `Reference` is a line of at most 500 bytes.
`EventKind` is a slug other than the reserved `status`. `AttachmentName` is a line of at most 255 bytes, not `.` or
`..`, with no `/`, `\` or `:`, not starting with `.`, and with a final extension of 1–16 ASCII alphanumerics.

### NewItem

| Field | JSON type | Rule |
| --- | --- | --- |
| source | string | Source slug. |
| requesterKey | string | RequesterKey line, at most 320 bytes. |
| title | string | Line, at most 200 bytes. |
| body | string | Block, 0–65,536 bytes. |
| snapshot | object or null | Snapshot below; absent also means none. |

### Snapshot

| Field | JSON type | Rule |
| --- | --- | --- |
| state | any JSON value | Compact serialization at most 1,048,576 bytes; arrays and objects nest at most 32 deep. |

A screenshot belongs in an attachment with role `screenshot`, never a special snapshot field.

### Attachment

| Field | JSON type | Rule |
| --- | --- | --- |
| index | u32 number | Store assigns 0, 1, 2 and so on in submission order. |
| name | string | AttachmentName above. |
| contentType | string | Lower-case line, at most 127 bytes, matching `[a-z0-9][a-z0-9.+-]*/[a-z0-9][a-z0-9.+-]*`. |
| role | string | `file` or `screenshot`. |
| size | u64 number | At least 1 byte. |

Deserialization checks content-type syntax; submission additionally checks the store's allowlist and matching
extension. `NewFile` holds the name, content type, role and borrowed byte slice; it is not a serialized type.

### Triage

| Field | JSON type | Rule |
| --- | --- | --- |
| kind | string | `bug`, `tweak`, `feature` or `unclear`. |
| size | string | `small` or `large`. |
| clear | boolean | Whether the request is clear. |
| summary | string | Line, at most 280 bytes. |
| details | string | Block, 0–16,384 bytes. |

### Proposal

| Field | JSON type | Rule |
| --- | --- | --- |
| kind | string | `tickets`, `no-action` or `question`. |
| tickets | array of Ticket | 1–20 for `tickets`; empty for `no-action` and `question`. |
| reason | string | Block, 1–4,096 bytes. |

### Ticket

| Field | JSON type | Rule |
| --- | --- | --- |
| title | string | Line, at most 200 bytes. |
| spec | string | Block, 1–32,768 bytes. |
| list | string | `To Do` or `Backlog`. |
| blockedBy | array of Blocker | 0–20 with no duplicates; any numeric blocker requires `Backlog`. |

### Blocker

A Blocker is an untagged u32 number or a Reference string. A number is the index of an earlier ticket in this same
proposal, strictly smaller than the index of the ticket it blocks. Blockers come first. A string names existing
external work and follows the Reference rule.

### Event and Note

Events are objects tagged by `type`, either `status` or `note`.

| Status event field | JSON type | Rule |
| --- | --- | --- |
| type | string | `status`. |
| atMs | u64 number | Unix milliseconds. |
| from | Status or null | Null only for the initial event. Later events follow the previous status. |
| to | Status | A legal move, except the initial submission. |
| ref | Reference or null | Optional external reference. |

| Note event field | JSON type | Rule |
| --- | --- | --- |
| type | string | `note`. |
| atMs | u64 number | Unix milliseconds. |
| kind | string | EventKind slug, never `status`. |
| body | string | Block, 0–16,384 bytes. |

A caller's `Note` has only `kind` and `body`, with the same rules; the store stamps its time.

### Item

| Field | JSON type | Rule |
| --- | --- | --- |
| id | string | ItemId above. |
| source | string | Source above. |
| requesterKey | string | RequesterKey above. |
| title | string | Line, at most 200 bytes. |
| body | string | Block, 0–65,536 bytes. |
| snapshot | Snapshot or null | Snapshot above. |
| attachments | array of Attachment | Consecutive indices starting at 0; at most one screenshot. |
| triage | Triage or null | Present together with proposal exactly when a status event moves to processed. |
| proposal | Proposal or null | Present together with triage exactly when a status event moves to processed. |
| status | string | One of the nine statuses below. |
| events | array of Event | Nonempty and consistent with the status history below. |
| createdAtMs | u64 number | Exactly the first event's Unix millisecond time. |
| updatedAtMs | u64 number | At least the last event's time. |

The first event is a status event from null to submitted. Each later status event's `from` equals the previous
status event's `to` and follows a legal move. The last status event's `to` equals the item's status. Event times
never decrease. Every nested field obeys its own validation rules.

### Processed

| Field | JSON type | Rule |
| --- | --- | --- |
| triage | Triage | Valid triage. |
| proposal | Proposal | Valid proposal. |

`Processed::from_json` refuses more than 131,072 bytes before parsing, with field `processed` and reason
`model output is too large`. Parsing retains the JSON parser's default nesting limit. Every parse or validation
failure from parsing becomes field `processed`, reason `not a valid processed object`, without echoing input.
Public validation errors contain a JSON field path and a fixed reason, never the offending text.

## Status machine

| Status | Meaning | Legal next statuses |
| --- | --- | --- |
| submitted | Received, not yet processed. | processed, held, cancelled |
| processed | Triaged and proposed, waiting to be routed. | queued, held, cancelled |
| queued | Accepted, its work is filed. | in_progress, cancelled |
| held | Waiting for a person. | queued, declined, cancelled |
| in_progress | Being worked. | queued, built, cancelled |
| built | The work landed. | shipped |
| shipped | Released to the sender. | none |
| declined | A person said no. | none |
| cancelled | Withdrawn. | none |

No move to the same status is legal. Only a person declines, from held. Submitted to held allows failed processing
to reach a person; in_progress to queued puts work back. Store transitions to processed use `record_processing` only.

## Attachment limits

`Limits` has `maxFiles` (u32), `maxFileBytes` (u64), `maxItemBytes` (u64), and `allowed` (array of AllowedType).
`AllowedType` has `contentType` (a lower-case content-type string) and `extensions` (lower-case ASCII alphanumeric
extension strings of 1–16 bytes). Defaults are 20 files, 104,857,600 bytes per file (100 MiB), and 262,144,000 bytes
per item (250 MiB). The screenshot counts as a file.

| Content type | Extensions |
| --- | --- |
| image/png | png |
| image/jpeg | jpg, jpeg |
| image/gif | gif |
| image/webp | webp |
| image/heic | heic |
| application/pdf | pdf |
| text/plain | txt, log |
| text/markdown | md |
| text/csv | csv |
| application/json | json |
| application/vnd.openxmlformats-officedocument.wordprocessingml.document | docx |
| application/vnd.openxmlformats-officedocument.spreadsheetml.sheet | xlsx |
| application/vnd.openxmlformats-officedocument.presentationml.presentation | pptx |

SVG and HTML are absent because they can carry script; archives and executables are deliberately excluded.
Checks refuse, in order: too many files; an empty file or one above the per-file cap; total bytes above the item cap
(with overflow-safe addition); a content type not exactly in the allowlist; an extension, lower-cased, not matching
that type; a screenshot other than PNG or JPEG; more than one screenshot. Errors name `files[i].field` or `files`.
The crate checks metadata, not file signatures; a byte reader or processor must treat all bytes as untrusted data.

## Store contract

The async `Store` trait uses sendable futures, all taking `&self`; each store provides its own locking. Its backend
error implements Error, Send, Sync and has a static lifetime. Methods return `StoreError`: `NotFound(id)`,
`NoFile { id, index }`, `IllegalMove`, `Invalid` or `Backend`. Errors use fixed phrases plus ids or inner errors;
Backend and IllegalMove expose their error source.

- `submit` validates the NewItem and checks files against its limits before storing anything. Invalid data returns
  Invalid atomically. The store assigns the next id starting at 1, stores files with indices 0 onward, sets
  submitted, records exactly one status event from null with no reference, and sets both times to now.
- `get` returns the item or NotFound.
- `list` matches every set filter field, returns ids ascending, and applies the optional limit. `Filter` has
  `statuses` (array of Status; empty means all), `source` (Source or null), `requesterKey` (RequesterKey or null), and
  `limit` (u32 or null). Its default matches everything. `Filter::pending()` selects only submitted and processed:
  these wait to be routed; the router decides by age whether processing has failed or is unfinished.
- `record_processing` validates Processed first. It accepts only submitted; any other state returns IllegalMove
  from that state to processed. It saves triage and proposal together and records the move to processed.
- `set_status` refuses processed with Invalid, field `to`, reason `only record_processing moves an item to
  processed`. Otherwise it checks the move against the current state and records from, to and the supplied
  reference. The check and write are one atomic step; a database store compares and sets the current status.
- `add_note` validates the Note and records it with the store's timestamp in every status, including terminal ones.
- `read_file` returns identical stored bytes; an absent item is NotFound and an absent index is NoFile.

Unknown ids produce NotFound for get, set_status, add_note, read_file and record_processing. Every change updates
updatedAtMs to now; time never moves backwards within an item. MemoryStore uses a poison-recovering mutex,
Infallible backend errors, and Unix milliseconds clamped to its last store stamp. It is for tests and app mocks.
`conformance::limits()` sets 3 files, 1,024 bytes per file and 2,048 per item with the default allowlist.
`conformance::run` checks an empty store configured with these limits, panicking with the broken rule on failure.

## Processor contract

A Processor receives an Item and selected borrowed File entries (attachment metadata plus bytes), and returns
Processed or its own error through a sendable future. Item text and files are data, never instructions, for any
model a processor runs. The caller chooses Files::None, Files::Images (content types beginning `image/`), or
Files::All. The crate performs no model or network call.

`process` gets the item and refuses any status other than submitted as Store(IllegalMove) without calling the
processor. It reads selected files, calls the processor, validates the result, then records processing. A processor
error returns Processor and changes nothing. Invalid output returns Invalid and changes nothing. Store errors are
wrapped as Store. Concurrent status changes are still checked atomically by record_processing.

## Versioning

`CONTRACT_VERSION = "v1"`. Adding an optional field keeps v1 only when every reader ignores it. These readers
refuse unknown fields, so any field added or changed means v2.

## Complete examples

```json item
{
  "id": "IN-1",
  "source": "web-form",
  "requesterKey": "someone@example.com",
  "title": "The total on the summary page is wrong",
  "body": "The displayed total differs from the rows.",
  "snapshot": {"state": {"page": "summary"}},
  "attachments": [
    {"index": 0, "name": "screen.png", "contentType": "image/png", "role": "screenshot", "size": 4},
    {"index": 1, "name": "details.txt", "contentType": "text/plain", "role": "file", "size": 5}
  ],
  "triage": {"kind": "bug", "size": "small", "clear": true, "summary": "The summary total is wrong", "details": "Compare it with the rows."},
  "proposal": {
    "kind": "tickets",
    "tickets": [
      {"title": "Correct the total", "spec": "Sum all visible rows.", "list": "To Do", "blockedBy": []},
      {"title": "Verify the summary", "spec": "Check the corrected total.", "list": "Backlog", "blockedBy": [0]}
    ],
    "reason": "The total should match the rows."
  },
  "status": "processed",
  "events": [
    {"type": "status", "atMs": 1000, "from": null, "to": "submitted", "ref": null},
    {"type": "status", "atMs": 1001, "from": "submitted", "to": "processed", "ref": null}
  ],
  "createdAtMs": 1000,
  "updatedAtMs": 1001
}
```

```json processed
{
  "triage": {"kind": "bug", "size": "small", "clear": true, "summary": "The summary total is wrong", "details": "Compare it with the rows."},
  "proposal": {
    "kind": "tickets",
    "tickets": [
      {"title": "Correct the total", "spec": "Sum all visible rows.", "list": "To Do", "blockedBy": []},
      {"title": "Verify the summary", "spec": "Check the corrected total.", "list": "Backlog", "blockedBy": [0]}
    ],
    "reason": "The total should match the rows."
  }
}
```
