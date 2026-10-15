# Kelpie's app

## The idea

1. People who struggle with chat get a visual way to work with agents.
2. They see what an agent is about to build, and what it built, as interactive nodes.
3. It covers how the code connects and how the app runs.
4. People and agents use the same views: a person selects, an agent highlights.

## What kelpie already has

1. The plan: a parent issue, its slices, and what blocks what.
2. What each slice touches: `Touches:` lists, then the real diffs.
3. Where each worker is: phase, gates, review rounds.

## Views

1. The plan: slices as a chain of nodes, the one in progress lit.
2. The map: modules, with the ones a slice touches lit.
3. A decision: two or three options side by side, the recommended one marked.
4. Behaviour: the work-item loop as a state machine, real items moving through it.

## Pushback

1. A whole-codebase graph is unreadable (codebase-memory-mcp's own viewer shows a ball of 5,000 dots).
2. Each view has to be a picture drawn for one question, about 5 to 9 boxes.
3. The map viewer could serve any dog, so maybe it belongs to shep; the plan view is kelpie's.
4. Prove it with one real case as a page before building an app.

## Prior art (from memory)

1. Sourcetrail: code graph explorer, discontinued, open source.
2. CodeSee: code maps for review, closed.
3. AppMap: records what code does at runtime.
4. C4 and Structurizr: architecture at zoom levels.
5. Copilot Workspace: spec to plan to code, ended.
6. None of them joins a plan, a code map and live agent progress.

## Tech

1. React Flow with ELK layout for plan and module views.
2. Sigma.js only if a large zoomed-out view is needed.
3. Zoom levels: crate, module, file, function.
4. Data: codebase-memory-mcp for symbols, `crate::` references for module links (its call links match by name and are unreliable), kelpie's logs for behaviour.
5. Its own MCP server so agents can highlight, annotate and read the selection.

## Findings so far

1. shep-kelpie at c18cdb1b: 51 modules, 294 module links, 37 modules in one loop.
2. 43 links (108 of 1,368 references) point back up and make the loop.
3. `runner` is 40% of the code.
