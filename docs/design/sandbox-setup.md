# Sandbox setup

Some machines need a one-time change before any sandbox can start on them. On Ubuntu 24.04 and later, AppArmor strips the capabilities from bwrap's user namespace, and every session fails with `bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted` (crabpot's README, Linux setup). This note covers how kelpie finds that out, how it makes the change, and what runs when the person declines. It places itself in `docs/design/ux.md` (Sandbox, Attention) and amends ADR 0005. Terms are in `GLOSSARY.md`.

**Status:** confirmed by the maintainer on 2026-11-10: running without a sandbox (opt in per project, the PM only), the profile scoped to kelpie, Install through `pkexec`, the AppImage offering only Run without a sandbox, and Windows later.

## Use case

1. The maintainer, or anyone trying kelpie on Ubuntu 24.04, opens a project. The PM's sandbox cannot start.
2. Instead of a failed session, Attention shows why and offers **Install** and **Run without a sandbox**.
3. Install asks for their password once and makes the change. The PM starts sandboxed, and every later session does too.
4. Someone who cannot or will not install can still use the PM on that project, unsandboxed and marked as such. Workers wait until the sandbox can start.

Why it matters: the most common Linux desktop cannot run kelpie's sandbox as it ships, and CI no longer covers it (PR #50 pins the sandbox tests to Ubuntu 22.04). Without this, kelpie on Ubuntu is a failed session with an error from bwrap.

## Design

### Flow

```text
session start (PM, thread or worker)
  Sandbox::start, then `true` in it
    ran                         the session starts sandboxed
    refused: no capabilities    Attention: "Ubuntu restricts the sandbox"   Install · Run without a sandbox
    refused: no namespaces      Attention: "This kernel has no sandbox"     Run without a sandbox
  Install
    pkexec, through the process tree        one password prompt
    the profile loaded                      the session starts sandboxed
  Run without a sandbox (this project)
    PM and its threads start unsandboxed    a red "No sandbox" badge on the chat
    workers stay refused
```

### Telling the failures apart

Kelpie checks when a session's sandbox starts, as crabpot reports it, and does not read sysctls ahead of time. The two refusals are told apart by bwrap's own words, as PR #50's `crates/kelpie-engine/tests/sandbox.rs` already does: `RTM_NEWADDR` means the namespace was made with no capabilities, which the profile fixes. `No permissions to create new namespace` means the kernel gives none at all, which no profile fixes, so Install is not offered.

### Install

1. The package ships the profile root-owned, in its own data folder. The button runs one `pkexec` command, spawned through the process tree (ADR 0001), that copies it into `/etc/apparmor.d/` and loads it with `apparmor_parser -r`. Nothing is copied from a folder the person can write, so the prompt cannot be turned into root for another file.
2. One password prompt. With no polkit agent running, which a minimal window manager may lack, the item shows the two commands to run in a terminal instead.
3. The profile names kelpie's own binary, and lets the system's bwrap make namespaces only when kelpie starts it (below). `kernel.apparmor_restrict_unprivileged_userns` stays at 1, and kelpie never turns it off: that opens user namespaces to every process on the machine.
4. Kelpie retries the session once the profile is loaded, and the retry starts sandboxed without a relaunch. AppArmor attaches a profile when a program is exec'd, so the kelpie that ran Install stays `unconfined`, and only the `kelpie` profile moves bwrap into `kelpie_bwrap`. While kelpie reads `unconfined` in `/proc/self/attr/current` and `/sys/kernel/security/apparmor/policy/profiles/` holds a `kelpie.<n>` entry, `Sandbox::command` runs crabpot's command as `<kelpie's binary> --sandbox-hop -- <program> <args>`, with its directory and environment. That fresh exec gets the profile, and `sandbox::run_from_args` execs the rest in place, so the pid, process group and `KELPIE_RUN` stay as the process tree set them (ADR 0001). It needs no `aa-exec`, `change_onexec` or `unsafe`. A confined kelpie, a host with no `kelpie` profile loaded and macOS run the command as crabpot wraps it. An AppImage, or a build run from a path the profile does not name, hops too, but the fresh exec gets no profile, so its sandbox is refused as before, with one extra `--sandbox-hop` process in front. An install that fails names `pkexec`'s or `apparmor_parser`'s error in the item.

### The profile

crabpot's README grants `userns` to `/usr/bin/bwrap` and disables Ubuntu's `bwrap-userns-restrict`. That lifts the restriction for every program that calls the system bwrap, which is what Ubuntu's profile is there to stop. Kelpie's profile scopes it to kelpie instead, and ships nothing of bwrap's: it uses the system's `bubblewrap` package.

It is three profiles in one file:

1. Kelpie's binary, which starts `/usr/bin/bwrap` into the second and runs everything else as itself.
2. bwrap as kelpie started it, with `userns`. crabpot's helper, kelpie's binary run again inside, stays in it. Anything else it runs is stacked under the third: bwrap sets `no_new_privs`, and AppArmor then allows a stack and no other change.
3. The sandboxed command, with no `userns`, `capability` or `mount`.

```text
abi <abi/4.0>,
include <tunables/global>

profile kelpie <kelpie> {
  file, network, capability, signal, ptrace, unix, dbus, mount, umount, pivot_root,
  /** ix,
  /usr/bin/bwrap Px -> kelpie_bwrap,
}

profile kelpie_bwrap flags=(attach_disconnected,mediate_deleted) {
  /{,**} rwlkm, network, capability, signal, ptrace, unix, dbus, mount, umount, pivot_root, userns,
  <kelpie> ix,
  /** Pix -> &kelpie_sandboxed,
}

profile kelpie_sandboxed flags=(attach_disconnected,mediate_deleted) {
  file, network, signal, ptrace, unix, dbus,
  /** ix,
}
```

`<kelpie>` is the binary's installed path. `kelpie_bwrap` spells out its file rules, since `file,` grants `ix` on every path and AppArmor refuses it beside `/** Pix`.

Tested on Ubuntu 24.04.3 (kernel 6.8, restriction at 1) with crabpot 0.10.8 on 2026-11-10, as an unprivileged user:

| | No profile | This profile |
|---|---|---|
| A session's sandbox | `RTM_NEWADDR` | Starts: writes its folder, not the home folder, `~/.ssh` hidden, allowed domain 200, another refused |
| `unshare` and a nested bwrap inside | | Refused |
| bwrap started by anything else, or a copy of bwrap | Refused | Still refused |

An agent inside cannot reach the second profile: by its rules, kelpie's binary run from inside the sandbox stays in the third. The README's profile, by contrast, grants `userns` to the helper's path wherever it is run from.

Open: a host where Ubuntu's own `bwrap-userns-restrict` is loaded, which stock 24.04 does not load. Kelpie's switch names its profile, which should win over that one's path, but no such host has tried it.

### The AppImage

An AppImage runs from a new mount path each launch, so no profile can name it. On an affected host it offers Run without a sandbox, and its item says the `.deb` or `.rpm` can install the profile.

### Running without a sandbox

1. **Run without a sandbox** is a project setting, off by default. It applies only while the sandbox cannot start: once the profile is in, the project's sessions are sandboxed again whichever way it is set.
2. It covers the PM and its threads, which run while the maintainer watches. Workers never run without a sandbox: they run unattended and hold a token that pushes. A worker that cannot start its sandbox is refused, as SK refuses one (`a_missing_sandbox_runs_nothing`, `crates/adapters/src/adapters/claude/sandbox.rs` in SK).
3. An unsandboxed session shows a red **No sandbox** badge in its chat header for as long as it runs, and Attention keeps its item open with Install on it.
4. What still holds without the sandbox: the PM changes kelpie's state through kelpie's tools, and its permissions are its tool list. What does not: its reads, writes and network are the person's own, logins and secrets included.
5. A home-folder project is still refused (`ux.md`, Sandbox, item 5). The setting is for a machine that cannot sandbox, not a folder kelpie will not.

### Windows

Windows has a one-time install of the same shape (`crabpot windows-install`). Once Windows is proven (`ux.md`, Sandbox, item 6), it uses this note's item and buttons, with its own install behind Install. Not built in this note's slices.

## Synergy

| Reuses or connects to | How |
|---|---|
| The sandbox (ADR 0005) | The same `Sandbox::start`. The amendment adds running without one, opt in and PM only |
| PR #50's sandbox tests | Their split between the two bwrap refusals becomes the engine's |
| The process tree (ADR 0001) | `pkexec` is spawned through it like any command |
| Attention (`ux.md`) | One item per affected host, with Install and Run without a sandbox |
| Settings (`settings.md`) | One project field, Run without a sandbox |
| Packaging | The profile ships as a package resource naming the binary's installed path, and the `.deb` and `.rpm` depend on `bubblewrap` |
| Workers (`workers.md`) | A worker refused for no sandbox waits, holding no slot, like a Paused one |

What it would duplicate: nothing. The home-folder refusal stays its own check.
