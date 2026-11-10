---
name: lift-from-sk
description: Use when writing or building a kelpie issue in an area SK already covers, or when bringing code from shep-kelpie (SK) into kelpie, whether a module, an adapter, a crate or its tests. Covers finding the SK source in docs/lift-plan.md, what comes with the code, what stays behind, and the row each lift leaves in docs/lifted.md.
---

# Lifting code from SK

SK is shep-pm/shep-kelpie. Kelpie treats it as a treasure chest: lift what the current slice needs, with its tests, and leave the rest. SK is archived once kelpie builds itself, so whatever SK knew and kelpie needs has to arrive with the lift. `docs/lift-plan.md` maps every SK module to a verdict, a home in kelpie and a phase. Lifting is the default there: code a row marks lift or adapt is not written fresh.

## Steps

0. **Find the row.** Look up the issue's area in `docs/lift-plan.md`. The row names the SK paths, the cut and the tests to bring. An issue being written gets them in its first `What to build` bullet. A row marked new or leave means there is nothing to lift, and the rest of these steps are skipped.
1. **Pin the source.** Clone SK at the commit the issue pins into `$TMPDIR`: `git clone --filter=blob:none https://github.com/shep-pm/shep-kelpie "$TMPDIR/sk" && git -C "$TMPDIR/sk" checkout <commit>`. A worker's sandbox reads no checkout outside its own and reaches github.com, and SK is public. With no commit pinned, use `origin/main` and note its commit. Every later step refers to that commit.
2. **Read the why.** Read the ADRs in SK's `docs/adr/` and the files in `docs/facts/` that touch the chunk. Done when you can name each edge case the chunk handles and the incident or ADR behind it.
3. **Map the edges.** List every import the chunk makes from outside itself: `shep_client`, `shep_channel`, `shep_handover`, SK's `core` ports, settings, the runner. Each one is either replaced by a kelpie equivalent, narrowed to a small trait kelpie implements, or left behind together with the code that needed it.
4. **Lift the tests first.** Copy every test that covers the lifted code and run it before changing anything around it. Tests stay in kelpie's own test names and style once they pass.
5. **Lift the code, scoped to the slice.** Bring the paths the slice's issue needs. A branch only SK used stays in SK. Processes are started through kelpie's engine tree (CLAUDE.md, Rules).
6. **Translate the words.** SK talks in sheep, lambs, flocks, dogs and the shepherd. In kelpie those are processes, sessions and the engine. `GLOSSARY.md` holds the terms.
7. **Record the lift.** Add a row to `docs/lifted.md`, and mark the row in `docs/lift-plan.md` as `lifted (#n)`.

Done when every SK test covering the lifted code passes in kelpie or appears in the row as dropped with its reason, and `git grep -n -i -E 'shep_client|shep_channel|shep_handover|SHEP_HOME|sheep|lamb' -- <lifted files>` prints nothing.
