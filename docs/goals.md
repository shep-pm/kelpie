# Goals

Parked on 2026-10-14 while the maintainer gathers their thoughts. Read this first when the work resumes.

## Names

1. `SK` is shep-kelpie, the dog (shep-pm/shep-kelpie).
2. `kelpie` is this app.
3. This repo is private. Public SK text never names it.

## The two goals

1. Build the app.
2. Put SK through its paces while doing it.

## Why the app

1. Chat is a wall for people with ADHD. Long text sessions lose attention, and the maintainer ends up agreeing to everything.
2. Short statements in a numbered list do work. Decisions written that way get real attention.
3. The app shows what an agent will build and has built as interactive nodes, both how the code connects and how the app runs.
4. People and agents work in the same views.
5. The app will eventually start projects from the very beginning and see them through.

## Starting a project, A to Z

1. Kelpie (the app) is the first project started this way, so every step it needs is a test of SK.
2. Each step is done once and logged in `docs/journey.md`: what, who did it, and could SK have done it.
3. Each step done by hand that SK should own becomes a slice of SK's parent issue shep-pm/shep-kelpie#897 ("kelpie starts a project from nothing and sees it through").
4. When a slice lands, the step is redone through SK to prove it.
5. The goal is batteries included: repeatable for any new project, with the maintainer making only the decisions.
6. We already skipped a step: the repo was made by hand without the org's `init-shep` skill (see `docs/journey.md`).

## What the code graph viewer taught us

1. codebase-memory-mcp's own viewer (`--ui=true`, port 9749) is striking but too much at once. Being visual is not enough; it overwhelms.
2. So each view in the app must answer one question with a few boxes (about 5 to 9). The full map stays in the background.
3. shep and SK are nearly the same size: shep 127,570 lines of code in 8 crates, SK 103,374 in one crate (tracked `.rs`, tests not counted).
   On 2026-10-23, counting tests: shep 302,668 lines, SK 304,352 in a workspace of 11 crates.
4. shep looks orderly in the viewer. SK is a tangle: 37 of its 51 modules form one loop (`map/`).

## Parked until the maintainer's write-up

1. The app's design and views (`docs/gui-idea.md`). The backend, repos and stack are decided in `docs/backend.md`, and partly replaced by `docs/process-and-sk.md`.
2. The slices of SK#897.
3. ~~Redoing step 0 properly with `init-shep`, through a pull request.~~ Done on 2026-11-08 (journey 0.11).

## Running meanwhile in SK

1. Untangling SK into a workspace of crates, done by SK's own workers, not by hand in a session. Done: 11 crates by 2026-10-23.
2. Two possible agent roles, discussed in `docs/roles.md`.
