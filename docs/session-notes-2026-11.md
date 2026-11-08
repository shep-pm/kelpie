# GUI notes from the shep-kelpie sessions, Oct 26 to Nov 8

Everything said about the app while working on shep-kelpie, pulled from one long session. Rin's words are quoted. The rest is what was proposed or decided, with where it landed.

## Why the app

1. "I think kelpie has long outgrown being a shep dog and just needs to be an app." (Nov 5)
2. Monitoring: "a lot of this will be less difficult when we have the GUI setup and I can better monitor each agent, compared to having them live in somewhere 'far away' in a process I can't recall how to get to so I have to relay everything through you, which feels inefficient." (Oct 27)
3. One PM, not two: "When we move to the GUI, my anticipation is that I do not have to communicate with two Opus 5.5 High Effort sessions. I will be communicating with the PM directly." (Nov 5)
4. Fewer settings: "Sure, in a GUI, it will be easier to navigate, but in `lookout` it's very overwhelming." (Nov 5)
5. The stuck feeling: "We've just been stuck in a weird spot between gui/shep dog for way too long and I've been stressed that we're putting all this effort into something that won't even matter in the near future. Plus the changes we've been making to shep *for* kelpie, that may not even matter for what shep is actually meant for. Or worse, actively making its primary function worse." (Nov 5)

## The two configs

Rin, Nov 5: "in my ideal world, the only kelpie configs are:

1. The agent files
2. How many decisions the PM rules on vs how many are decided by code

That's all."

- This became ADR 0020 in shep-kelpie, "the project manager makes the calls, and settings hold only what it cannot" (`docs/adr/0020-the-project-manager-makes-the-calls.md`).
- `scripts/settings-count.py` now gates new settings: each new key has to say why the PM cannot decide it.
- First rough sort of the 85 settings at the time (a quick pass on a local model, possibly counting some section headers):

| Group | Count | Fate |
|---|---|---|
| Facts | 26 | Move to onboarding, or become the small leftover |
| Safety limits | 25 | Become constants with no setting |
| Detectable | 17 | Code works them out |
| PM judgment | 17 | Deleted, the PM decides |

- Takeaway: most settings can stop being settings, but the facts have to live somewhere, so the app's setup flow matters.

## The rule for every design from Nov 4 on

Rin, Nov 4: "start thinking about things in terms of 'will this translate' or 'does it make sense to manage this way in a GUI' ya know?"

1. No setting is "restart only" without saying so. A key that still needs a restart shows as "waiting for restart" in `status` and doctor, so the app shows a badge instead of a doc note.
2. Prefer a fact the app can show over a value the user types.
3. Prefer a toggle plus a read-only "detected" panel over a list of env vars.
4. A runner answers "starting" at once, so the app never shows a false "did not answer".

## How each piece would look in the app

| Piece | Decision in shep-kelpie | In the app |
|---|---|---|
| Agent ranking | One ordered list for all projects, `[kelpie.agents] prefer` (later moved toward the PM by ADR 0020) | An Agents page with drag to reorder. No numbers to type, so no ties or gaps |
| API credit runs out | The API's refusal holds every `login: api` agent until the time it names, then work hands off with no ruling. `[kelpie.api_credit]` removed | A status badge, "API: out of credit, checking again at 04:00". No setting |
| A listed agent that was deleted | The board skips it and doctor names it | A greyed-out row |
| Light reviews | Proposed: the PM proposes a light review and the diff confirms it (later folded into ADR 0020's PM-owned review plan) | Project settings, Reviews: steppers for normal and light counts, glob chips for "light allowed for" and "always full for". Item page badge: "Light review · PM: docs only · diff confirmed", or "Full review · diff touched `src/`" |
| Package managers in the sandbox | Detection always on, `worker.package_registries` goes (#1703 to #1708) | One toggle plus a read-only "Detected: pnpm → these caches, these hosts" panel, with overrides underneath. #1708 stops at `doctor` until the app |
| Upgrade | Dog first, a canary, then every runner in parallel | One progress row per project |
| Runner boot | Socket opens before the boot forge calls, `start` reports "starting" | A runner shows "starting" right away |
| Codex and Claude usage windows | Rin asked that the PM know every agent's locks and windows ("Has it burned its 5 hour window? Its weekly window?"). The dog's agent board reads them (ADR 0018) | Not discussed for the app. A natural fit: per-agent meters with session and week used, and when each resets |
| Rulings | The PM decides what the map lets it, and only big design calls reach the maintainer | Rulings come to Rin in the app, answered in place, not through a CLI or a relay session |

## Live views

1. Each agent's turn, live: what it is doing, not just its phase. This is the "far away" problem above.
2. A terminal tab: shep has no pty today. A sketch was `pty = true` on a sheep and `shep attach <sheep>`, a live view you can type into, "probably where a GUI's terminal tab would plug in later". Not filed.
3. The PM as the one chat: free text on a ruling or a PR goes to the PM and it acts (ADR 0020). The app's chat box is that path.

## Parked for the stack talks

Rin, Nov 5: "I don't really know what the 'stack' is going to look like once we move to Tauri and where shep fits in on that exactly. We're going to have quite a few discussions about it at some point, but I just don't really want to have them now."

1. Tauri is the working assumption. Nothing in shep-kelpie assumes it, and drafts since Nov 5 are stack-neutral.
2. Where shep fits. A check on Nov 5 found no sign kelpie made shep's core job worse:
   - The dog plane is opt-in (`enabled_dogs` empty by default).
   - About 31,000 lines of dog code in the CLI, and 38% of `shep-client`.
   - About 23% of shep commits since Sep 1 were scoped to dogs, lookout or channel.
   - One risk: boot awaits dog spawns before it restores sheep, so a hung dog spawn could delay every sheep.
   - If kelpie leaves the dog plane, `metrics` and `bark` still use it.
   - Gaps: only 19 commits name kelpie, so most kelpie-driven work was not found, and "what dies if kelpie leaves" is unanswered.
3. What carries over: the work-item loop, review, the PM, the agent files, and `docs/facts/`. The retros tell the app what it must get right.
4. What may not: lookout panes, dog plumbing and many settings. PR #1699, a lookout pane, was questioned for that reason.
5. Windows: native Windows tests had 529 failures on Nov 6. The call on how much Windows matters waits for this planning.
6. Monthly API credits: kelpie can run Claude Code on an `ANTHROPIC_API_KEY` per agent. Whether the plan's credits sit in a Console org a key can draw from was not settled.

## Related in shep-kelpie

- ADR 0018: the PM decides and the dog picks the agent.
- ADR 0020: the PM makes the calls, settings hold only what it cannot.
- `CODING_STANDARDS.md`, "Fewer settings".
- `scripts/settings-count.py`.
