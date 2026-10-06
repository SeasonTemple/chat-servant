# Triage labels

Default vocabulary, kept as-is. Each label string equals its role name.

| Label | Meaning |
| --- | --- |
| `needs-triage` | Not yet looked at by a human. |
| `needs-info` | Cannot act on it; missing context, repro steps, or a decision. |
| `ready-for-agent` | Fully specified. An agent can pick it up and finish it. |
| `ready-for-human` | Needs a human decision, physical action, or credential. |
| `wontfix` | Deliberately not doing this. Record why in the issue body. |

## Status vs labels

In markdown issues there is no server-side status field, so `status:` in the
front matter and these labels must agree:

- `status: done` ⇔ no label, or an explicit closing note
- `status: cancelled` ⇔ `wontfix`
- `status: open` ⇔ exactly one of `needs-triage`, `needs-info`,
  `ready-for-agent`, `ready-for-human`, `wontfix`

## Overrides

If the tracker later gains real labels that already express these roles, update
this table rather than creating duplicates.
