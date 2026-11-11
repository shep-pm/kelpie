import { describe, expect, it } from "vitest";

import type { Value } from "@/bindings";

import { formSchema, ordered } from "./schema";

/**
 * A few fields as the Rust types write them, with the `Theme` def a `$ref`
 * names.
 */
const raw: Value = {
  $defs: {
    PickUp: {
      description: "Who a project's tickets come from",
      oneOf: [
        {
          const: "me",
          description: "Me: the issues the signed-in login opened",
          type: "string",
        },
        {
          const: "everyone",
          description: "Everyone: every open issue",
          type: "string",
        },
      ],
    },
    Theme: {
      description: "How kelpie's window looks",
      oneOf: [
        { const: "system", description: "System", type: "string" },
        { const: "light", description: "Light", type: "string" },
        { const: "dark", description: "Dark", type: "string" },
      ],
    },
  },
  properties: {
    "lease-capacity": {
      additionalProperties: { maximum: 64, minimum: 1, type: "integer" },
      default: null,
      description: "How many commands hold each lease at once.",
      type: ["object", "null"],
    },
    "pm-harness": {
      default: null,
      description: "Which harness the PM's next message runs on",
      type: ["string", "null"],
    },
    "run-without-sandbox": {
      default: null,
      description: "Whether the project's PM runs without a sandbox.",
      type: ["boolean", "null"],
    },
    "pick-up": {
      anyOf: [{ $ref: "#/$defs/PickUp" }, { type: "null" }],
      default: null,
      description: "Whose of a project's issues kelpie picks up as tickets.",
    },
    "ready-label": {
      default: null,
      description: "The label a ticket carries to be Ready.",
      type: ["string", "null"],
    },
    "done-window": {
      default: null,
      description:
        "How many days a closed ticket stays in the board's Done column.",
      maximum: 365,
      minimum: 1,
      type: ["integer", "null"],
    },
    "worker-slots": {
      default: null,
      description: "How many of this project's workers run at once.",
      maximum: 64,
      minimum: 0,
      type: ["integer", "null"],
    },
    theme: {
      anyOf: [{ $ref: "#/$defs/Theme" }, { type: "null" }],
      default: null,
      description: "How kelpie's window looks",
    },
  },
  type: "object",
};

const keys = ["pm-harness", "worker-slots", "lease-capacity"];

const drawn = formSchema(raw, keys);
const themed = formSchema(raw, ["theme"]);
const field = (key: string, from = drawn) =>
  (from.properties as Record<string, Record<string, unknown>>)[key];

describe("the settings form's fields", () => {
  it("draws the worker cap as one plain field, named", () => {
    const slots = field("worker-slots") ?? {};

    expect(slots.title).toBe("Worker slots");
    expect(slots.type).toBe("integer");
    expect(slots.minimum).toBe(0);
    expect(slots.maximum).toBe(64);
    expect(slots.description).toContain("run at once");
  });

  it("leaves the lease capacities out until they have a row of their own", () => {
    expect(field("lease-capacity")).toBeUndefined();
  });

  it("shows the worker cap beside the worker's other fields", () => {
    expect(ordered(keys)).toEqual([
      "pm-harness",
      "worker-slots",
      "lease-capacity",
    ]);
  });

  it("draws a choice as a string field, each option titled", () => {
    const theme = field("theme", themed) ?? {};

    expect(theme.title).toBe("Theme");
    expect(theme.type).toBe("string");
    expect(theme.oneOf).toEqual([
      { const: "system", title: "System" },
      { const: "light", title: "Light" },
      { const: "dark", title: "Dark" },
    ]);
  });

  it("shows the theme beside how kelpie phrases its replies", () => {
    expect(ordered(["read-anywhere", "theme", "reply-style"])).toEqual([
      "reply-style",
      "theme",
      "read-anywhere",
    ]);
  });

  it("draws Run without a sandbox as a plain boolean field", () => {
    const run =
      field("run-without-sandbox", formSchema(raw, ["run-without-sandbox"])) ??
      {};

    expect(run.title).toBe("Run without a sandbox");
    expect(run.type).toBe("boolean");
    expect(run.oneOf).toBeUndefined();
  });

  it("draws whose issues a project picks up as a choice of Me and Everyone", () => {
    const board = formSchema(raw, ["pick-up", "ready-label", "done-window"]);

    const pick = field("pick-up", board) ?? {};
    expect(pick.title).toBe("Pick up issues from");
    expect(pick.type).toBe("string");
    expect(pick.oneOf).toEqual([
      { const: "me", title: "Me: the issues the signed-in login opened" },
      { const: "everyone", title: "Everyone: every open issue" },
    ]);
  });

  it("draws the ready label as text and the Done window as a whole number of days", () => {
    const board = formSchema(raw, ["pick-up", "ready-label", "done-window"]);

    const label = field("ready-label", board) ?? {};
    expect(label.title).toBe("Ready label");
    expect(label.type).toBe("string");
    expect(label.oneOf).toBeUndefined();

    const days = field("done-window", board) ?? {};
    expect(days.title).toBe("Done window (days)");
    expect(days.type).toBe("integer");
    expect(days.minimum).toBe(1);
    expect(days.maximum).toBe(365);
  });

  it("shows the board's fields after the sandbox ones", () => {
    expect(
      ordered(["done-window", "ready-label", "pick-up", "read-anywhere"]),
    ).toEqual(["read-anywhere", "pick-up", "ready-label", "done-window"]);
  });
});
