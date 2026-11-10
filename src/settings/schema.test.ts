import { describe, expect, it } from "vitest";

import { formSchema, ordered } from "./schema";

/** The two fields the worker slice adds, as the Rust types write them. */
const raw = {
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
  },
  type: "object",
};

const keys = ["pm-harness", "worker-slots", "lease-capacity"];

const drawn = formSchema(raw, keys);
const field = (key: string) =>
  (drawn.properties as Record<string, Record<string, unknown>>)[key];

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
});
