import { describe, expect, it } from "vitest";

import type { Value } from "@/bindings";

import { formSchema, ordered } from "./schema";

/**
 * A few fields as the Rust types write them, with the `Theme` def a `$ref`
 * names.
 */
const raw: Value = {
  $defs: {
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
});
