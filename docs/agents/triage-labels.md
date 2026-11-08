# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in this repo's issue tracker.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Asks SK to triage this issue (not a hold) |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.

## How SK reads them

SK's `kelpie-app` runner checks every open issue once a minute. An issue the maintainer opened that passes `shep kelpie issue check` gets `ready-for-agent` and goes on the board, with no call to a model. `triage.pick` doesn't change this: it decides only whose failing issues get researched.

- `needs-triage` does not hold an issue back. To SK it means "triage this".
- To hold an issue that passes the check, file it with `needs-info`. SK leaves alone any issue carrying `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix` or `possible-spam`.
- Once `ready-for-agent` is taken off an issue, SK never puts it back. Add it again by hand to release the issue.

Source: shep-pm/shep-kelpie `docs/guide/triage.md`.
