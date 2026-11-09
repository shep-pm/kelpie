# Harness-neutral skills, instructions and MCP dependency management: state of play, 2026-11-09

Method: official docs, spec repos and source repos only. Read from shallow clones (vercel-skills d3701c1, openskills 57d933a, agentskills 83021e2, apm 6b13c37, aps 44a0ff8, add-mcp, ghcli 736c68f, pi bd0d815, goose 0edf2bd). Star counts, licences and release dates come from `gh api repos/<owner>/<repo>` on 2026-11-09. "Source" means read from code, not docs, and may change without notice. "Unconfirmed" means no primary source found or sources conflict.

## 1. Comparison table: tools that install skills per project

| Tool | Licence | Installs to | Lockfile | Pinning | Skills | Instructions | MCP | Harnesses | Maturity |
|---|---|---|---|---|---|---|---|---|---|
| vercel-labs/skills (`npx skills`, skills.sh) | MIT | Project: `.agents/skills/` canonical copy plus symlinks (or `--copy`) into agent dirs; `-g` to user dirs | Yes, `skills-lock.json` (project), content SHA-256 | `owner/repo@v1.2.3` tag or commit; `check` exits 1 on drift; `--frozen-lockfile` | Yes | No | No | 70+ rows in docs/agents.md incl. Claude Code, Codex, pi, goose | 38.7k stars, v1.7.6 on 2026-11-03 |
| `gh skill` (GitHub CLI) | MIT (cli/cli) | Project (default) or user; `.agents/skills/` shared by many hosts, `.claude/skills/` for Claude | Partly: lock is user-level `~/.agents/.skill-lock.json`, not per project. Per-skill provenance written into SKILL.md frontmatter | `skill@tag`, `--pin <tag or sha>`; unpinned uses latest tag, else HEAD | Yes | No | No | About 50 `--agent` values incl. claude-code, codex, goose, pi (source) | gh v2.103.3 on 2026-10-26; `gh skill` labelled preview in docs |
| openskills (numman-ali) | Apache-2.0 (LICENSE, package.json; GitHub API says NOASSERTION) | Project `./.claude/skills` default, `--universal` `./.agent/skills`, `--global` `~/.claude/skills` | No lockfile found | None found; `update` re-pulls from git | Yes | Writes an `<available_skills>` block into AGENTS.md via `sync` | No | Any agent that reads AGENTS.md | 10.8k stars, v1.5.0 on 2026-01-17, last push 2026-01-18 (stale) |
| Claude Code plugin marketplaces | Claude Code is proprietary (unconfirmed, no LICENSE read); marketplace.json format is documented, not separately licensed | Plugin cache `~/.claude/plugins/cache/...` (user folder, not repo) | No project lockfile; `installed_plugins.json` is per user | `ref`, `sha` (40 char), npm `version`, archive `sha256`; manifest or entry `version` | Yes (plugin `skills/`) | No (plugins carry commands, agents, hooks, skills) | Yes (`.mcp.json` inside plugin) | Claude Code only | Official; docs current to v2.1.296 |
| Codex skills and plugins | Apache-2.0 (openai/codex) | Repo `.agents/skills`, user `~/.agents/skills`, admin `/etc/codex/skills` | None documented | None documented | Yes (`$skill-installer`, plugins) | n/a | Plugins can bundle MCP | Codex only | 134k stars |
| pi packages (`pi install`) | MIT (earendil-works/pi) | Personal `~/.pi/agent/settings.json`; `-l` writes `.pi/settings.json`; packages fetched, not vendored | None documented | `npm:pkg@1.0.0`, `git:...@v1` | Yes | Extensions, prompts, themes | Native `pi mcp`, `.pi/mcp.json` | pi only | 121k stars |
| Microsoft APM (`apm`) | MIT | Renders into each harness dir in the repo; fetches into gitignored `apm_modules/` | Yes, `apm.lock.yaml` (commit SHA, content hashes, per-file hashes) | Git ref, tag, commit, semver range; `apm install --frozen` | Yes | Yes (`.claude/rules/`, `.cursor/rules/`, compiled AGENTS.md, CLAUDE.md, GEMINI.md) | Yes (`dependencies.mcp`) | 11 stable targets incl. claude, codex, copilot, cursor, gemini, opencode; no goose, no pi | 4.3k stars, v0.35.0 on 2026-10-22, pre-1.0 |
| Tessl (`tessl`) | Unconfirmed (CLI source not found) | Project `.tessl/plugins/<name>/` or `~/.tessl` | `tessl.json` manifest; separate lockfile not documented | `workspace/plugin@version` or `@commit`; `outdated`, `update`, `--force` for majors | Yes | Yes (plugins) | `init --agent` configures MCP | claude-code, cursor, gemini, antigravity, codex, openhands, openclaw, copilot, agents | Commercial registry; maturity unconfirmed |
| rulesync (dyoshikawa) | MIT | Generates tool files from `.rulesync/`; `rulesync fetch` pulls skills | No lockfile seen (unconfirmed) | Not documented | Yes | Yes | Yes | Lists goose and pi | 1.6k stars, v43.8.0 on 2026-11-08 |
| ruler (intellectronica) | MIT | Generates from `.ruler/`; adds outputs to .gitignore | No | No | Yes | Yes | Yes (not for goose, pi) | Goose: `.goosehints`; pi: `AGENTS.md`, `.pi/skills` | 3.0k stars, v0.3.46 on 2026-10-22, "Beta Research Preview" |
| add-mcp (Neon) | Apache-2.0 | Writes each agent's MCP file, project or `-g` | No | No (remote URL or package name) | No | No | Yes | 24 agents incl. claude-code, codex, goose, pi | 321 stars, v2.6.1 on 2026-11-01 |

## 2. Format layer

### 2.1 Agent Skills open format
- Standardises only what is inside a skill directory: required `SKILL.md` with YAML frontmatter `name` (max 64, lowercase, hyphens, must match parent dir), `description` (max 1024); optional `license`, `compatibility` (max 500), `metadata` (string to string map), `allowed-tools` (experimental). Optional dirs `scripts/`, `references/`, `assets/`. Source: https://agentskills.io/specification
- Does NOT standardise where skills live, how they are installed, versioned, locked or fetched. The only version hook is the free-form `metadata.version` shown in the example. Source: same page; and "the Agent Skills specification does not mandate where skill directories live" in agentskills/agentskills `docs/client-implementation/adding-skills-support.mdx` @83021e2.
- Recommended discovery convention: client-specific dir plus `<project>/.agents/skills/` and `~/.agents/skills/`; some clients also scan `.claude/skills/`. Same file. Collision rule: deterministic, warn on shadowing.
- Origin: Anthropic, released as open standard; governance adopted 2026-11-04 under "a Series of LF Projects, LLC"; code and spec Apache-2.0, docs CC-BY-4.0. Sources: https://agentskills.io ; agentskills/agentskills `GOVERNANCE.md` @83021e2. Repo 27.1k stars.
- Client showcase lists 46 clients, including Claude Code, ChatGPT and Codex, goose and pi. Source: agentskills/agentskills `docs/snippets/clients.jsx` @83021e2.
- Claude Code accepts extra frontmatter (`when_to_use`, `disable-model-invocation`, `context`, `hooks`, `paths`, `model`, and more) and warns that spec-only packaging rejects unknown keys. Source: https://code.claude.com/docs/en/skills

### 2.2 Where each target harness looks

| Harness | Project | User | Source |
|---|---|---|---|
| Claude Code | `.claude/skills/<name>/SKILL.md`, nested `<subdir>/.claude/skills/`, plugin skills | `~/.claude/skills/`, enterprise managed dir | https://code.claude.com/docs/en/skills . `.agents/skills` is not mentioned in the docs I read (unconfirmed that it is ignored). |
| Codex | `.agents/skills` in cwd and each parent up to repo root | `~/.agents/skills`; admin `/etc/codex/skills`; bundled system skills | https://learn.chatgpt.com/docs/build-skills (redirect from developers.openai.com/codex/skills) |
| pi | `.pi/skills/`, `.agents/skills/` (cwd up to repo root); gated by project trust | `~/.pi/agent/skills/`, `~/.agents/skills/` | pi `packages/coding-agent/docs/configuration.md`, `skills.md`, `security.md` @bd0d815 |
| goose | `.agents/skills/` (recommended), also `.goose/skills/`, `.claude/skills/` | `~/.agents/skills/`, also `~/.claude/skills/`, `~/.config/goose/skills/` | https://goose-docs.ai/docs/guides/context-engineering/using-skills ; source `crates/goose/src/sources.rs` @0edf2bd |

Practical result: `.agents/skills/` covers Codex, pi, goose. Claude Code needs `.claude/skills/` (symlink or copy). goose and Claude Code both also read `.claude/skills/`, so a single `.claude/skills/` plus an `.agents/skills/` mirror covers all four. vercel-labs/skills automates exactly this via canonical `.agents/skills/` plus symlinks (`docs/agents.md`, `src/installer.ts` @d3701c1).

### 2.3 Agent Plugins 1.x (separate, newer packaging spec)
- Directory with required root `plugin.json` (closed schema: `$schema`, `name`, `displayName`, `version`, `description`, `author`, `homepage`, `repository`, `license`, `keywords`, `extensions`), `skills/`, optional `mcp.json`. Covers skills and MCP servers only. Spec text CC-BY-4.0, schemas Apache-2.0. Latest in repo is 1.1.0 "Working Draft". Source: agentplugins/agent-plugins-spec `spec/1.1.0.md`, `LICENSE.md` @44a0ff8; https://agent-plugins.org
- Explicitly leaves distribution, install, permissions to clients. `FUTURE_CONSIDERATIONS.md` lists signing, permissions, secrets handling as not done. No client list on the site; adoption by Claude Code, Codex, pi or goose: not found.
- 1.4k stars; no GitHub releases.

## 3. Tool sections

### vercel-labs/skills (skills.sh)
- MIT, 38.7k stars, v1.7.6 published 2026-11-03, Node 22.20+. https://github.com/vercel-labs/skills ; `package.json` @d3701c1
- Project scope is the default and is documented as "committed with your project"; `-g` installs to user dirs. `docs/commands.md`.
- Symlink default (canonical copy in `.agents/skills/`, links per agent) or `--copy`. A committed symlink farm works, but a copy-mode install is the safer vendoring choice for sandboxes. `docs/commands.md`.
- Lockfile `skills-lock.json` v1, deliberately timestamp-free and alphabetically sorted to merge cleanly. Entry fields: `source`, `sourceUrl`, `ref`, `sourceType`, `skillPath`, `version` (npm only), `via`, `computedHash` (SHA-256 over skill files), `installMode`. Source: `src/local-lock.ts` @d3701c1. Note: it records branch or tag `ref` and a content hash, not a resolved commit SHA (source).
- `experimental_install` restores from lock; `--frozen-lockfile` fails on hash mismatch and is the default when `CI` is set. `experimental_sync` reads `skills` in `package.json` and npm packages that ship skills. Both marked experimental. `docs/commands.md`.
- `skills check` exits 1 when updates exist. Pinning: `owner/repo@v1.2.3`. `docs/commands.md`.
- Sources: GitHub, GitLab, Azure Repos, any git URL, local path, zip/tar URLs (10 MiB cap), Notion. `docs/commands.md`.
- Telemetry exists (`src/telemetry.ts`); not read in detail. Unconfirmed what it sends.
- skills.sh is a Vercel-run leaderboard and directory; page mentions audits but shows no results; no versioning content. https://skills.sh

### gh skill (GitHub CLI)
- cli/cli is MIT, 46.9k stars, latest v2.103.3 (2026-10-26). `gh skill` was announced 2026-04-16 as public preview (version number v2.90.0 is from a secondary source, unconfirmed). https://github.blog/changelog/2026-04-16-manage-agent-skills-with-github-cli/ ; https://cli.github.com/manual/gh_skill_install
- `gh skill install <repo> [skill[@version]] --agent --scope project|user --pin <tag|sha> --dir --from-local`. Unpinned resolves to latest tagged release, else default branch HEAD. Pinned skills are skipped by `gh skill update`. Manual page above.
- Provenance is written into the installed SKILL.md frontmatter metadata: `github-repo`, `github-ref`, `github-tree-sha`, `github-path`, `github-pinned`. Source: `internal/skills/frontmatter/frontmatter.go` @736c68f. This means a vendored skill carries its own pin and drift info with no sidecar file.
- Lockfile exists in source but is user-level: `~/.agents/.skill-lock.json`, version 3 "must match Vercel's CURRENT_LOCK_VERSION for interop". Source: `internal/skills/lockfile/lockfile.go` @736c68f. Not a per-project lock.
- Agent registry in source includes goose (`.goose/skills`, `~/.config/goose/skills`) and pi (`.pi/skills`, `~/.pi/agent/skills`), which differ from what those harnesses call preferred (see 2.2). `internal/skills/registry/registry.go` @736c68f. Source, not docs.

### openskills
- Apache-2.0, 10.8k stars, v1.5.0 on 2026-01-17, last push 2026-01-18, Node 20.6+. https://github.com/numman-ali/openskills ; `LICENSE`, `package.json` @57d933a
- `openskills install owner/repo`, `sync` (writes AGENTS.md block), `update`, `manage`, `remove`. Default `./.claude/skills`, `--universal` `./.agent/skills` (singular, differs from the `.agents` convention), `--global` `~/.claude/skills`. README.
- No lockfile or pinning found in README or repo root. Ten months since last commit: treat as unmaintained.

### Claude Code plugin marketplaces
- `.claude-plugin/marketplace.json`: required `name`, `owner`, `plugins[]`; each entry `name` plus `source`. Source types: relative path, `github`, `url`, `git-subdir` (each with `ref` and 40-char `sha`), `npm` (`package`, `version` range or tag, `registry`), `archive` (`url`, `sha256`), `command`. https://code.claude.com/docs/en/plugins/marketplace-reference.md
- Version resolution: manifest `version`, else entry `version`, else 12-char commit SHA (git sources) or 12-char sha256 (archive); `npm` and non-git local give `unknown`. https://code.claude.com/docs/en/plugins/loading.md
- Scopes: `claude plugin install <p>@<m> --scope user|project|local` writes `enabledPlugins` to `~/.claude/settings.json`, `.claude/settings.json` (committed) or `.claude/settings.local.json`. https://code.claude.com/docs/en/plugins/install.md
- Committing `enabledPlugins` does not download anything: each collaborator must still run `claude plugin install ... --scope project`, and project-enabled external plugins are not fetched automatically. Same page and loading.md ("Enabled in project settings but not installed"). A cloud session ignores them.
- Files go to `~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/` with state in `installed_plugins.json` and `known_marketplaces.json` (per user). Not in the repo, no lockfile. loading.md.
- Marketplace pinning in project settings: `extraKnownMarketplaces` with a `git` source and `ref`; `/plugin marketplace add owner/repo#v1.2.0` pins a tag. install.md, marketplace-reference.md.
- Plugin dependencies and `allowCrossMarketplaceDependenciesOn` exist. marketplace-reference.md.
- Managed policy: `strictKnownMarketplaces`, `blockedMarketplaces`, `strictPluginOnlyCustomization`. marketplace-reference.md, loading.md.
- Alternative that vendors: put skills directly in `.claude/skills/` (committed). https://code.claude.com/docs/en/skills ("Share skills").
- Harness lock-in: only Claude Code reads this.

### Codex
- Skill dirs in 2.2. `$skill-installer <name>` installs curated skills "and can also fetch skills from other repositories". Docs say installer is for local setup and recommend plugins for distribution. Plugins share a plugin directory between ChatGPT and Codex and can bundle skills plus MCP. No versioning or lockfile mentioned. https://learn.chatgpt.com/docs/build-skills
- Per-skill disable via `[[skills.config]]` in `~/.codex/config.toml`. Same page.
- Plugin pin or version semantics: not found on that page.

### pi packages
- Install forms `pi install npm:@x/y@1.0.0`, `git:github.com/x/y@v1`, `./local`; `-l` writes `.pi/settings.json` in the project; packages can ship skills (`skills/` dir or `pi.skills` manifest key). Pinned npm versions, tags and commits do not float. Declarations are in the `packages` array. `packages/coding-agent/docs/packages.md` @bd0d815; https://pi.dev/docs/latest/packages
- Project packages load only after project trust is granted. Same page.
- Packages are fetched at runtime, not vendored in the repo. No lockfile documented. Pi-only.

### Microsoft APM
- MIT, 4.3k stars, v0.35.0 on 2026-10-22. https://github.com/microsoft/apm ; OpenAPM v0.1 spec at https://microsoft.github.io/apm/specs/openapm-v01/
- `apm.yml` declares skills, agents, plugins, full packages (instructions, prompts, hooks) and `dependencies.mcp` (registry id or self-defined stdio/http). `docs/src/content/docs/reference/manifest-schema.md` @6b13c37
- `ref:` accepts a branch, tag, SHA or semver range; lock records `resolved_commit`, `resolved_ref`, `content_hash`, `deployed_files`, `deployed_file_hashes`. `apm install --frozen` replays exact commits. Lockfile v1 (git) and v2 (registry or semver). `reference/lockfile-spec.md` @6b13c37.
- Commit `apm.yml`, `apm.lock.yaml` and rendered target files; `apm_modules/` stays gitignored and some integrations load live from it, so it must be present at runtime. `consumer/install-packages.md`, `reference/lockfile-spec.md` @6b13c37. This is the closest existing match to the stated goal.
- Renders per target (targets-matrix.md): claude `.claude/{rules,agents,commands,skills}`, hooks merged into `.claude/settings.json`; codex `.codex/` plus `.agents/skills`, compile to AGENTS.md, no per-file instructions; copilot `.github/`; gemini, cursor, opencode, windsurf, kiro, antigravity. MCP supported for all those.
- goose and pi: no mention anywhere in the docs tree (grep for "goose" returned nothing) so they are not targets. Adding them means contributing a target or an `agent-skills` fallback (`.agents/skills` only; skills only).
- Python CLI (pyproject.toml); requires registries feature to be switched on with `apm experimental enable registries` for registry deps. Pre-1.0 and fast-moving: lockfile spec is a "Working Draft".

### Tessl
- CLI: `tessl init` creates `tessl.json`; `tessl install <ws>/<plugin>[@version|@commit]`, `--global` to `~/.tessl`, `--agent`, `--skill`, `--strict`; `tessl outdated`, `tessl update` (majors need `--force`). Installed to `.tessl/plugins/<name>/`. https://docs.tessl.io/reference/cli-commands ; https://docs.tessl.io/tutorials/using-tessl-as-a-package-manager
- Lockfile: not documented on those pages (unconfirmed). Licence of CLI and whether `.tessl/` is meant to be committed: unconfirmed. Registry is a hosted commercial service. goose and pi are not in its agent list.

### rulesync and ruler (config generators)
- rulesync: MIT, 1.6k stars, v43.8.0 (2026-11-08). Generates rules, commands, MCP, ignore files, subagents, skills, hooks from a single source; lists goose and pi; `rulesync fetch owner/repo` pulls remote skills. https://github.com/dyoshikawa/rulesync . Lockfile and pin support: not found (unconfirmed). The README table is flattened so per-tool feature support is unconfirmed.
- ruler: MIT, 3.0k stars, v0.3.46 (2026-10-22), "Beta Research Preview". Source folder `.ruler/` with `AGENTS.md`, `ruler.toml`, `skills/`, `agents/`; concatenates rules, propagates MCP, adds generated files to `.gitignore`. Goose: `.goosehints`, skills `.agents/skills/`, no MCP. Pi: `AGENTS.md`, skills `.pi/skills/`, no MCP. https://github.com/intellectronica/ruler . pi MCP row is stale versus pi's own docs (section 5).
- Neither resolves remote versions to pins; they are sync tools, not dependency managers.

## 4. Instruction files

| Harness | File it reads | Notes | Source |
|---|---|---|---|
| Claude Code | `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md`, `~/.claude/CLAUDE.md`, `.claude/rules/` | Reads `AGENTS.md` natively only when no CLAUDE.md or CLAUDE.local.md exists above cwd (v2.1.277+). Setting `claude-md-and-agents-md` loads both. A CLAUDE.md containing `@AGENTS.md` imports it; symlink is also deduped. | https://code.claude.com/docs/en/memory |
| Codex | `AGENTS.override.md`, then `AGENTS.md`, then names in `project_doc_fallback_filenames` (config.toml), root to cwd, one per dir; global `~/.codex/AGENTS.md`; 32 KiB cap (`project_doc_max_bytes`) | CLAUDE.md is not a default; must be added as a fallback name. | https://learn.chatgpt.com/docs/agent-configuration/agents-md |
| pi | `AGENTS.override.md`, `AGENTS.md`, `AGENTS.MD`, `CLAUDE.md`, `CLAUDE.MD` in agent dir, cwd and parents | Reads CLAUDE.md natively. SYSTEM.md and APPEND_SYSTEM.md are separate. | pi `packages/coding-agent/docs/configuration.md` @bd0d815 |
| goose | `AGENTS.md` then `.goosehints`, cwd up to git root plus nested; global `~/.config/goose/.goosehints` | `CONTEXT_FILE_NAMES` env var (JSON array) replaces the list, e.g. `["CLAUDE.md", ".goosehints"]`. Hints need Developer extension on. | https://goose-docs.ai/docs/guides/context-engineering/using-goosehints ; goose docs file @0edf2bd |

- AGENTS.md: plain Markdown, no required fields, nearest file wins, stewarded by the Agentic AI Foundation (Linux Foundation). https://agents.md . The site does not mention CLAUDE.md.
- Cheapest harmony: write AGENTS.md as the single source, add a one-line `CLAUDE.md` containing `@AGENTS.md` (Claude Code docs describe this). Only Claude Code needs it; pi, goose and Codex read AGENTS.md directly.
- Sync tools: ruler and rulesync (generate per-tool files from one source), openskills `sync` (injects skills list into AGENTS.md), APM `compile` (emits AGENTS.md, CLAUDE.md, GEMINI.md; instructions as `.claude/rules` and `.cursor/rules` natively). Sections 3 and sources above. None versions instruction files as dependencies except APM.

## 5. MCP server config per project

| Harness | Project file | User file | Format | Source |
|---|---|---|---|---|
| Claude Code | `.mcp.json` (committed) | `~/.claude.json` (user and local scope) | JSON `mcpServers`; `${VAR}` and `${VAR:-default}` expansion | https://code.claude.com/docs/en/mcp |
| Codex | `.codex/config.toml` (trusted projects only) | `~/.codex/config.toml` | TOML `[mcp_servers.<name>]`; does not read `.mcp.json` for general config | https://learn.chatgpt.com/docs/extend/mcp?surface=cli |
| pi | `.pi/mcp.json` (after project trust) | `~/.pi/agent/mcp.json` | JSON `mcpServers`, stdio and streamable HTTP; `pi mcp add [-l]` | pi `packages/coding-agent/docs/mcp.md` @bd0d815 |
| goose | Not found in goose docs or Rust source (unconfirmed). add-mcp claims `.goose/config.yaml` | `~/.config/goose/config.yaml` under `extensions:` | YAML (`cmd`, `args`, `envs`, `type: stdio`) | https://goose-docs.ai/docs/getting-started/using-extensions ; add-mcp README |

- Claude Code details: project `.mcp.json` servers prompt for approval in interactive sessions; in `-p`, Agent SDK and cloud sessions they load without a prompt; an untrusted folder ignores `enableAllProjectMcpServers`. Precedence local, project, user, plugin, claude.ai. Fields do not merge across scopes. Credential env vars such as `ANTHROPIC_API_KEY` expand to empty in remote `url` and `headers`. https://code.claude.com/docs/en/mcp
- Conflict note: add-mcp's README says "Pi has no built-in MCP" and routes through `pi-mcp-adapter`. pi's own docs at bd0d815 document native MCP. Trust the pi repo; add-mcp's pi entry is out of date (path `.pi/mcp.json` matches anyway).
- add-mcp (Neon): writes the native file per agent, project by default, `-g` global; commands `find`, `list`, `remove`, `sync` (aka `unify`), `list-agents`; programmatic `upsertServer()` API; registry at add-mcp.com. Has no lockfile and no version pins; it is an installer into native config, not a dependency resolver. https://github.com/neondatabase/add-mcp ; README @ clone
- APM `dependencies.mcp` writes MCP for claude, codex, copilot, cursor, gemini, opencode, windsurf, kiro and records ownership in `apm.lock.yaml`. Source: `reference/manifest-schema.md`, `reference/targets-matrix.md` @6b13c37.
- Agent Plugins `mcp.json` is the only neutral file format: `$schema` plus `mcpServers`, stdio and streamable-http, ties to plugin version. No evidence that harnesses read it yet. `spec/1.1.0.md` @44a0ff8.
- No MCP spec-level project config file exists in anything I read; `.mcp.json` is a Claude Code convention that Copilot CLI and Mastra Code also read (add-mcp README).

## 6. Gaps: what none of them does

1. Lockfile that pins a resolved commit for every source in one project file, across skills, MCP and instructions, readable by all four harnesses. APM does (commit SHA plus hashes) but lacks goose and pi. vercel-labs/skills locks `ref` and content hash but not a resolved commit (source), skills only. gh skill locks only at user level.
2. Vendoring with verification. Everything vendors differently: vercel-labs/skills and gh skill write real files into the repo, APM keeps `apm_modules/` gitignored and some harness integrations load from it live, Claude Code marketplaces and pi packages stay in user folders. Nobody offers "vendor everything, verify hashes on startup, fail closed".
3. Multi-harness rendering that includes goose and pi plus Claude Code and Codex. Skills only: vercel-labs/skills and gh skill cover all four. Instructions or MCP: rulesync lists goose and pi, ruler partly, add-mcp covers MCP for all four, APM none for goose or pi.
4. One declaration for skills, MCP servers and instruction files with version ranges. APM comes closest. rulesync and ruler have no pins. Tessl has a manifest but pin and lock behaviour is unconfirmed.
5. Goose project-scoped MCP: not documented. A tool cannot safely write a per-repo goose MCP config. Unconfirmed.
6. Secrets. Only Claude Code documents `${VAR}` expansion and blocks credential vars; other formats vary (Codex `env_vars`, `bearer_token_env_var`). No tool maps one secret reference scheme to every harness. Agent Plugins lists secrets handling as future work.
7. Sandbox-friendliness. Most tools assume network plus a home directory plus `npx`/Python/`gh` on the host: vercel-labs/skills (Node 22.20+, git or GitHub API at install), gh skill (needs `gh`, auth), APM (Python, git), Claude Code plugins (state under `~/.claude/plugins`, `${CLAUDE_PLUGIN_ROOT}` changes per version, project plugins need a trust dialog and are skipped in cloud sessions). Project MCP files are trust-gated in Claude Code (prompt), Codex (trusted projects), pi (project trust). None offers an offline "restore from lockfile and vendored files only" mode for a sandbox that has no network or no user home. APM has offline bundles and a frozen install, partially closing this (consumer/deploy-a-bundle.md, unconfirmed in detail).
8. Trust and provenance. The Agent Plugins spec defers signing and permissions; gh skill has no signature check that I found; vercel-labs/skills sends telemetry (content unconfirmed). No tool checks that a skill's `allowed-tools` or bundled `scripts/` are compatible with a sandbox policy.
9. Stability. gh skill is preview, vercel `experimental_install` and `experimental_sync` are experimental, APM is pre-1.0 with lockfile spec "Working Draft", Agent Plugins is a "Working Draft", openskills is stale since January 2026.
