# Kelpie lifts from SK, and ships neither shep nor SK

Decided on 2026-11-08 in `docs/process-and-sk.md`, which holds the measurements. It replaces `docs/backend.md`'s plan of SK as a headless daemon behind the app.

Kelpie's engine is its own Rust code. From shep it takes one crate, `shep-tree`, as a library: it is small, has no async runtime, and depends on no other shep crate. SK is not a dependency. Its code is lifted chunk by chunk, with its tests, as each slice needs it, and SK is archived once kelpie builds itself. Until then SK keeps building kelpie, unchanged.

## Considered options

- **SK as kelpie's backend.** SK's process model (its runner a sheep, its workers lambs) is what kelpie replaces, and it depends on `shep-client`, `shep-channel` and `shep-handover` throughout. Running it brings shep's world along and breaks ADR 0001.
- **Start from zero.** SK's stream parsing, sandbox fence, review rules, leases and PM rulings were expensive to learn. Its ADRs and `docs/facts` record why each exists, and they serve as the spec.

## Consequences

- Every lift follows `.claude/skills/lift-from-sk` and gets a row in `docs/lifted.md`, so each row stands on its own once SK is archived.
- The cap on a second system: lift only what the next slice needs.
