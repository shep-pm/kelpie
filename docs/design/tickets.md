# Tickets and the GitHub slice

Phase 2's ticket surface: the Board panel, kelpie reading a project's tickets from GitHub, and kelpie opening a worker's draft pull request. It brings in what `docs/design/workers.md` left waiting: Start, the worker's chat, Note and Stop and tell, `start_worker`, usage drafts and the draft pull request. It places itself in `docs/design/ux.md` (Layout, Tickets and links, Workers, Attention) and lifts from SK's ADR 0014, `core/ticket.rs`, `ports/tickets.rs` and `adapters/gh/`, row by row from `docs/lift-plan.md` (GitHub). Terms are in `GLOSSARY.md`.

**Status:** confirmed by the maintainer on 2026-11-10: tickets in and a draft pull request out, GitHub only; the maintainer's own `gh` login; SK's `gh` adapter behind the forge adapter; chelone's board with drag and drop and one live move; Pick up issues from Me by default; three waits mark a ticket as needing the maintainer. Confirmed on 2026-11-11: a ticket with a live worker opens its tab, and any other links out.

## Use case

Today SK picks and starts the work, and the maintainer reads tickets on GitHub and sees a pull request at the end.

1. The maintainer opens a project. The Board panel shows its tickets in five columns, read from GitHub and refreshed about once a minute.
2. They click Start on a Ready ticket, or drag it to Building. A worker starts on it (`docs/design/workers.md`).
3. The ticket sits in Building. Clicking it opens the worker's tab in the PM panel, where they watch it and send a Note.
4. When the worker's turn ends with its branch pushed, kelpie opens the draft pull request as the maintainer, and the ticket moves to Review.
5. The maintainer reviews it on GitHub, marks it ready and merges it. The ticket moves to Done when the issue closes.

Why it matters: kelpie starts and follows its own work, the last step before it builds itself.

## Design

### Scope

| In this slice | Later |
|---|---|
| Reading GitHub tickets, the Board panel, Start, the worker's tab, Note, Stop and tell, `start_worker`, usage drafts, opening the draft pull request | Phase 3, lifted as SK built them: the gate, review, merge, dispatch, rulings, triage and the board's own writes |
| GitHub | Local tickets (ADR 0014's second source), in their own issue, since nothing waits on them. Other forges through the forge adapter |

### Where tickets come from

Lifted as SK settled it (SK ADR 0014):

1. A project has one ticket source, and the source is the ticket's truth. Kelpie keeps no copy of a ticket beyond what its board remembers.
2. `TicketId` everywhere. A GitHub ticket's id is its issue number, and a local one's is `k` and a counter.
3. The `Tickets` port without SK's `&ForgeSlug`: the GitHub source holds its repo. `open_issues` and `fetch_image` move to a GitHub-only trait (`docs/lift-plan.md`, The seams).
4. What kelpie adds to a ticket (notes to its worker, later its threads and links) is kept in kelpie's data folder under the ticket's id.

An issue is a ticket when the per-project setting Pick up issues from says so: Me (the default) takes the issues the signed-in login opened, and Everyone takes every open issue. An issue carrying the ready label is a ticket either way. The ready label's name is a project setting, `ready-for-agent` by default (`docs/lift-plan.md`, Labels).

### The board

| Column | A ticket is here when |
|---|---|
| Draft | Its issue is open without the ready label |
| Ready | Its issue carries the ready label |
| Building | A worker's session on it is live, Paused included, or its worker waits on the maintainer with no pull request of its own |
| Review | Its pull request is open, its worker waiting on the maintainer included |
| Done | Its issue is closed |

1. A column is a fact read from GitHub and the engine, not a place the maintainer puts it.
2. A ticket on the board shows its number, the title, the priority label and the needs-you marker. Tickets sort by priority, then oldest first.
3. Ready tickets carry Start.
4. Clicking a ticket whose worker is live opens its worker's tab: a Building ticket, or a Review ticket whose worker waits on the maintainer. A Review ticket whose gate line says a reviewer is reading its head opens the read's tab instead. Clicking any other opens its issue, or its pull request in Review, on GitHub. The gate's ready-to-merge line is a wait on the merge yes and not on the worker, so that ticket links out (`gate-and-merge.md`, The Review column).
5. The marker shows while the ticket's worker waits on the maintainer: a sandbox request, a fence refusal holding the ticket (#83), or Paused after a quit or because its sandbox cannot start, which says why. Phase 3 adds rulings. It counts toward the project's badge in the rail (`docs/design/ux.md`, Attention).
6. A ticket whose worker waits on the maintainer stays in Review while its pull request is open, since a gate ruling is that ticket's needs-you line and its card must not jump to Building (`docs/design/gate-and-merge.md`, The Review column). A waiting ticket with no pull request yet, such as one whose draft could not open, stays in Building.

The board is chelone's `KanbanBoard` (`packages/react/src/components/kanban/`, MIT, the maintainer's own), adapted: chelone's list, resource, router and i18n hooks come out, and tickets go in. It brings dnd-kit, a hold before a touch drag so a swipe still scrolls, the keyboard's Move to menu, and the optimistic move that puts a ticket back when the move fails.

Drag and drop runs a move table:

| Move | Phase 2 | Phase 3 |
|---|---|---|
| Ready to Building | Start, the same function as the button | |
| Draft to Ready | Snaps back | Writes the ready label |
| Review to Done | Snaps back | Merges |
| Any other | Snaps back, with one line saying why | |

### The worker's tab

A worker's chat opens as a tab in the PM panel (PM, #120, #121), in the same chat view the PM uses. Closing a tab hides it, and the worker keeps running. A worker's modal stays in `docs/design/ux.md`, Later.

Clicking a Review card whose gate line says a reviewer is reading its head (`Reviewing, read 1 of 2`, `docs/design/gate-and-merge.md`, The Review column) opens the read as a tab of its own beside the worker's: the same chat view, read-only, titled with the reviewer, with each look at the pull request behind the line naming it (`docs/design/review.md`, What the maintainer sees). The read takes no message from the maintainer, and what it found reaches the worker as the worker's next message, so the worker's tab is where the findings are read.

### GitHub

1. Who kelpie acts as. The maintainer's own `gh` login, for every read and for opening the draft pull request, as SK does, so the pull request shows as theirs. Workers push with the App's token (#84), and `Grant::Worker` stays without pull requests or comments. The login never reaches a worker: SK's `login_token` stays behind.
2. How kelpie calls GitHub. SK's `gh` adapter, lifted behind the forge adapter's traits, one `gh` child per call through the process tree. Workers already need `gh` for the push helper, so this adds no dependency. A REST adapter can replace it later behind the same traits.
3. Signing in. Kelpie holds no GitHub token. The settings page shows "GitHub: signed in as" the login `gh api user` names, or says to run `gh auth login` when there is none.
4. Staying current. Kelpie polls while it runs, about once a minute, with ETags on REST lists, and nothing polls once it quits (ADR 0001). SK's hold on a used-up rate limit comes with it, as one local pace, since one process needs no second budget through the App. A held limit shows on the board as the time it lifts.
5. The draft pull request. Opened from the worker's hand-over file through `POST repos/{repo}/pulls` with `draft=true` and the text as `-f` fields (SK's `gh/pulls.rs`), its body ending `Resolves #N`. In Phase 2 the maintainer marks it ready on GitHub. Phase 3's gate takes that over.
6. GitHub.com only. Enterprise comes through the forge adapter when someone needs it.

## Synergy

| Reuses or connects to | How |
|---|---|
| Workers (`docs/design/workers.md`) | Start, a drop on Building and the PM's `start_worker` call the one start function (#80). The Building column follows the worker's slot and session. The draft pull request completes the hand-over |
| The PM chat (`docs/design/pm-chat.md`) | Worker tabs share its chat view and panel |
| Kelpie tools (`docs/design/kelpie-tools.md`) | `start_worker` |
| Settings (`docs/design/settings.md`) | Pick up issues from and the ready label, per project |
| Attention (`docs/design/ux.md`) | The needs-you marker feeds the rail's badge and the tray |
| The process tree (ADR 0001) | Every `gh` call is a child in the tree |
| The graph (later) | Filtering the board by area, and a ticket's links, wait on it |
| chelone | Its board is lifted, not rebuilt |

It would duplicate GitHub's issue and pull request pages, so the board links out to them instead.
