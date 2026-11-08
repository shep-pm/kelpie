---

name: zustand-subscription-patterns
description: Use when wiring React components to a Zustand store and deciding how to subscribe — passing store fields to child components, prop-drilling state, reaching for useShallow to "optimize re-renders", placing selectors in a parent/container, or chasing extra re-renders in a list/dashboard/form fed by a Zustand store.

---

# Zustand Subscription Patterns

## Overview

**Render isolation comes from component boundaries, not selector granularity.** A selector only scopes a re-render if it lives in the component that should re-render. Move the same selectors into one parent and you get one fat render unit: every change re-renders everything — and you pay for every selector.

**The rule:** subscribe to the **narrowest value** you need, in the **component that uses it**. Don't drill store state through props. Don't reach for `useShallow` to "fix" a prop-drill.

This skill is backed by a measured benchmark (5 strategies × N ∈ {3,50,500}, React.Profiler + selector-call instrumentation). Render/selector **counts are deterministic**; ms are relative (dev build, machine-dependent) but the ordering is stable.

## The five strategies (and the verdict)

| #      | Strategy                              | Renders on 1-field change | Selector evals | Verdict                                          |
| ------ | ------------------------------------- | :-----------------------: | :------------: | ------------------------------------------------ |
| **S3** | Per-field selector **in each child**  |           **1**           |   ~N (cheap)   | ✅ **Default. Use whenever viable.**             |
| S5     | Whole object + `React.memo` children  |             2             |       ~4       | ✅ Fallback when you must prop-drill             |
| S1     | Whole object → props (no memo)        |            N+1            |       ~4       | ⚠️ Naive baseline; wastes N renders              |
| **S2** | **Wide `useShallow` → props**         |          **N+1**          | ~4 (O(N) each) | ❌ **Avoid. Costs more than S1, fixes nothing.** |
| **S4** | Per-field selectors **all in parent** |          **N+1**          |    **~4N**     | ❌ **Worst of all. The "efficient" trap.**       |

N = number of children/fields.

## Core rule: subscribe in the consuming component (S3)

```tsx
// ✅ GOOD — each child subscribes to its own field. Parent subscribes to nothing.
function ProfileCard() {
  return <><FirstName /><LastName /><Email /><Age /><Bio /></>
}
function FirstName() {
  const firstName = useStore((s) => s.profile.firstName) // primitive → Object.is
  return <span>{firstName}</span>
}
// Change email → only <Email> re-renders. Parent never re-renders. 1 render total.
```

Why it works: the selector returns a **primitive**, compared with `Object.is` (zustand's default). When `email` changes, `firstName` is unchanged → `<FirstName>` skips. The parent holds no subscription, so it never re-renders on store changes.

## ❌ Anti-pattern S2: `useShallow` over a wide slice, drilled as props

```tsx
// ❌ BAD — useShallow does NOT stop the children re-rendering on a real change.
function ProfileCard() {
  const { firstName, lastName, email, age, bio } = useStore(
    useShallow((s) => ({ ...s.profile })) // wide slice
  )
  return <><FirstName v={firstName} /><LastName v={lastName} />/* ... */</>
}
```

**Why it's wrong (measured):** on a real field change the picked object differs → parent re-renders → **all N children re-render anyway** — identical render count to S1 (N+1). But S2 _also_ pays a wide shallow-compare on **every** store write: rebuild an N-key object + compare N keys. At N=500 that selector phase is the **most expensive of all five strategies**. So S2 is strictly worse than the naive S1 on a real change, and only "wins" on a no-op ref churn (new object reference, identical values) — a rare case. **Reaching for `useShallow` to optimize a prop-drill makes it slower, not faster.**

`useShallow` is **not the villain** — _wide_ `useShallow` + prop-drilling is. See the narrow-`useShallow` exception below.

## ❌ Anti-pattern S4: per-field selectors, all in the parent (the "efficient" trap)

```tsx
// ❌ BAD — looks fine-grained, is the worst pattern measured.
function Dashboard() {
  const a = useStore((s) => s.a)
  const b = useStore((s) => s.b)
  // ... 20 individual selectors at the top of the container
  return <><WidgetA a={a} /><WidgetB b={b} />/* ... */</>
}
```

This **feels** efficient ("I used granular selectors!") and is the single most common mistake — but the granularity buys **zero** isolation:

- One value changes → that subscription fires → the **parent** re-renders → **all N children re-render** (N+1, ties the worst).
- Measured selector evaluations on a one-field change: **~4N** (N to detect the change on the store notification, ~3N more when the parent re-renders and re-runs every hook, twice each via `useSyncExternalStore`). At N=500 that is **~2000 selector calls to change one field** vs S3's ~500 and S1's ~4.
- Net: highest total cost of all five strategies.

**The selector only isolates a render when it sits in the component that should re-render.** Push each selector down into its child (→ S3).

## ✅ Fallback S5: if you MUST prop-drill, `React.memo` the children

Can't restructure into leaf components (drilling through a fixed presentational tree)? Keep the whole-object subscription and memoize the children:

```tsx
const Widget = React.memo(function Widget({ v }: { v: number }) { return <span>{v}</span> })
function Parent() {
  const obj = useStore((s) => s.obj)
  return <>{keys.map((k) => <Widget key={k} v={obj[k]} />)}</>
}
```

Measured: renders drop to **2** (parent + the one changed child), selector cost stays cheap (~4, one whole-object selector). **Catch:** the parent still re-renders and reconciles N child elements + runs N memo compares on **every** change — so commit time is mid-pack (≈8× S3 at N=500), and it still does work on no-op churn. Good rescue for prop-drilling; **S3 is still better** because it skips the parent entirely.

## When narrow `useShallow` IS correct

`useShallow` cost is **O(slice width)**. A small fixed slice into **one** component is idiomatic and nearly free — the problem is only width + drilling.

```tsx
// ✅ GOOD — one component genuinely needs two fields together.
function FullName() {
  const { firstName, lastName } = useStore(
    useShallow((s) => ({ firstName: s.profile.firstName, lastName: s.profile.lastName }))
  )
  return <span>{firstName} {lastName}</span>
}
```

Measured selector phase by width: 3 keys ≈ 0.0001 ms (free), 500 keys ≈ 0.053 ms (~500×). Use `useShallow` when a single component reads a **small, fixed** set of fields and you'd otherwise return a fresh object/array. Don't use it to paper over prop-drilling.

## The bound on S3: expensive selectors

S3's "cheap ~N selector evals" assumes the **selector is cheap** (a field read). zustand runs **every** subscriber's selector on **every** store write. If the selector _derives_ (e.g. `Object.values(s.items).reduce(...)`), N subscribers = N × the derive:

| N   | 1 derive | N derives | penalty  |
| --- | -------- | --------- | -------- |
| 50  | 0.024 ms | 0.74 ms   | **31×**  |
| 500 | 0.015 ms | 7.2 ms    | **488×** |

For a **derived/expensive** value needed in many places: compute it **once** (in the store as state, or once in a parent + memo/drill, or via `reselect`/`proxy-memoize`), not in N subscribers. Atomic selectors are for **reading** state cheaply, not for replicating expensive derivations.

## Our actual findings (5-strategy sweep, N=500, change one field)

| Strategy                   | Renders | Selector calls | Commit ms | Total ms |
| -------------------------- | :-----: | :------------: | :-------: | :------: |
| **S3** per-field in child  |  **1**  |      503       |   0.12    | **0.14** |
| S5 whole obj + memo        |    2    |       4        |   1.07    |   1.07   |
| S1 whole obj → props       |   501   |       4        |   2.74    |   2.74   |
| S2 wide useShallow → props |   501   |       4        |   2.95    |   3.01   |
| S4 selectors in parent     |   501   |    **2000**    |   3.14    | **3.16** |

No-op ref churn (new object ref, same values): S1 wastes **N+1** renders; S2/S3/S4 → **0**; S5 → 1 (parent only). ms are dev-build/machine-relative; counts are exact.

**Takeaways:** S3 ≈ 20× cheaper than the prop-drill family. S2 is _worse_ than naive S1 on real changes. S4 is dead last. S5 is the best prop-drill rescue but still 8× S3.

## Refactoring an existing tree into S3

When the task is "convert this non-S3 code to S3" (not "pick a pattern"), follow this procedure. For **each store value currently passed as a prop:**

1. **Find the real consumer** — the component that actually reads the value for display/logic. The subscription belongs there.
2. **Store-specific or generic?**
   - _Store-specific_ child (only ever shows this store's data) → move the selector into it, delete the prop.
   - _Generic/reusable_ child (used elsewhere with non-store props, e.g. `<StatTile label value>`) → **do NOT couple it to the store.** Leave it prop-driven; add a thin **store-bound wrapper** that selects the value and renders the generic child. The wrapper is the render unit, so isolation still holds.
3. **Select primitives, not objects** — never select an object field (`s.user`, `s.rows[id]`); it returns a reference that churns on unrelated writes and re-renders anyway. Select the primitives you render, or a small fixed slice via narrow `useShallow`.
4. **Parent ends up subscribing to nothing** — or only to _structural_ data (an id list), never to the row data itself.
5. **Lists/collections** — parent maps the id list → `<Row id={id} />`; each `Row` selects `s.byId[id].field` itself. Pass the **id** (structure), not the data object.
6. **Derived props** (totals, filtered counts) — don't re-derive in each child (the expensive-selector trap, N× the work). Lift the derivation to store state or one shared memoized selector (`reselect`/`proxy-memoize`); children read the result as a primitive.
7. **API-change flag** — converting a prop-driven child to subscribe changes its signature. If it's reused, prefer the wrapper (step 2) over editing the shared component.

| Source pattern               | Convert by                                                                                                               |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| S1 (whole obj → props)       | push each field's selector into its store-specific child; **wrap** generic children                                      |
| S4 (selectors in parent)     | move each selector down into the child that uses it; extract a child if none exists                                      |
| S2 (wide useShallow → props) | drop the useShallow+drill; per-field selectors in children (narrow useShallow only if one child needs a small fixed set) |
| object field drilled         | select the primitives, or narrow `useShallow` — never the object reference                                               |
| derived value drilled        | derive once (store state / memoized selector); child reads the primitive                                                 |

```tsx
// Generic <StatTile> is reused with non-store props elsewhere → DON'T couple it.
// Add a thin store-bound wrapper; the generic component stays pure.
function VisitsTile() {
  const value = useStore((s) => s.stats.visits) // wrapper carries the subscription
  return <StatTile label="Visits" value={value} /> // generic child untouched
}
```

## Common mistakes

| Mistake                                                                                 | Reality                                                                                                                                         |
| --------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| "I'll use `useShallow` to stop the children re-rendering."                              | On a real value change S2 re-renders all N children anyway (same as S1) and adds an O(N) compare every write. It only helps on no-op ref churn. |
| "20 individual selectors at the top of my container is fine-grained and efficient."     | It's the **worst** pattern — N+1 renders + ~4N selector evals. Granularity in one component isolates nothing. Push selectors into the children. |
| "Selecting in 500 components runs 500 selectors — that must be too many subscriptions." | ~N cheap field-reads is fine and far cheaper than N re-renders. Only worry when the _selector itself_ is expensive (then derive once).          |
| "Atomic selectors fix everything."                                                      | Only if cheap. An expensive derive in N subscribers is N× the work — up to ~488× at N=500.                                                      |
| "Prop-drilling the store is fine, it's simpler."                                        | Without `React.memo` it re-renders every child on every change (S1). If you must drill, memo the children (S5).                                 |

## Red flags — stop and reconsider

- You're adding `useShallow` to a selector that feeds **props passed down** → you're building S2. Subscribe in the child instead.
- All your `useStore` calls sit at the top of one container component → you're building S4. Push them down.
- A child receives a store value as a **prop** when it could `useStore` it directly → prefer the direct subscription (S3) — **unless the child is a generic/reusable presentational component**, in which case wrap it, don't couple it (see Refactoring step 2).
- A selector does `.map`/`.filter`/`.reduce`/`Object.values` and lives in many components → expensive-selector trap; derive once.
- "It's granular so it must be efficient" → granularity ≠ isolation. Isolation = the selector lives in the component that re-renders.
