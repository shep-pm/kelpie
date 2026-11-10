import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import type {
  AgentView,
  FieldView,
  Frontmatter,
  ResolvedAgent,
  Value,
} from "@/bindings";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });
const refused = (error: string) => ({ status: "error" as const, error });

const agentsList = vi.fn<() => Promise<unknown>>();
const agentGet = vi.fn<(name: string) => Promise<unknown>>();
const agentPreview =
  vi.fn<
    (name: string, frontmatter: Frontmatter, body: string) => Promise<unknown>
  >();
const agentSave =
  vi.fn<
    (name: string, frontmatter: Frontmatter, body: string) => Promise<unknown>
  >();
const settingsSchema = vi.fn<() => Promise<Value>>();
const settingsGet = vi.fn<(project: string | null) => Promise<unknown>>();
const harnessOptions =
  vi.fn<(harness: string, refresh: boolean) => Promise<unknown>>();

vi.mock("@/bindings", () => ({
  commands: {
    agentsList: () => agentsList(),
    agentGet: (name: string) => agentGet(name),
    agentPreview: (name: string, frontmatter: Frontmatter, body: string) =>
      agentPreview(name, frontmatter, body),
    agentSave: (name: string, frontmatter: Frontmatter, body: string) =>
      agentSave(name, frontmatter, body),
    settingsSchema: () => settingsSchema(),
    settingsGet: (project: string | null) => settingsGet(project),
    harnessOptions: (harness: string, refresh: boolean) =>
      harnessOptions(harness, refresh),
  },
}));

// jsdom cannot run the real editor: a textarea and a div stand in for it
vi.mock("@/components/mdx-editor/mdx-input", () => ({
  MdxInput: ({
    value,
    onChange,
    label,
  }: {
    value: string;
    onChange: (value: string) => void;
    label?: string;
  }) => (
    <textarea
      aria-label={label}
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));
vi.mock("@/components/mdx-editor/mdx-field", () => ({
  MdxField: ({ value }: { value: string | null }) => (
    <div data-testid="preview-body">{value}</div>
  ),
}));

import { AgentEditor } from "./AgentEditor";
import { useAgents } from "./store";

const settingsJsonSchema = {
  $defs: {
    Harness: {
      oneOf: [{ const: "claude-code", description: "Claude Code" }],
    },
  },
  properties: {
    "pm-harness": {
      anyOf: [{ $ref: "#/$defs/Harness" }, { type: "null" }],
      description: "Which harness the PM's next message runs on",
    },
    "pm-model": {
      description: "Which model the PM runs on",
      type: ["string", "null"],
    },
    "pm-effort": {
      description: "How much effort the PM puts in",
      type: ["string", "null"],
    },
  },
  type: "object",
} as unknown as Value;

const field = (key: string, value: Value, from: string): FieldView => ({
  key,
  value,
  from,
});

const noKeys: Frontmatter = {
  extends: null,
  role: null,
  harness: null,
  model: null,
  effort: null,
  body: null,
};

/** The PM's own instructions, which `child` builds on. */
const parentBody = "You are the PM.";

/**
 * What the loader would answer for a draft. The merge itself is the engine's
 * and is tested there; this is enough of it to see the editor ask and show.
 */
function resolve(front: Frontmatter, body: string): ResolvedAgent {
  const own = (key: keyof Frontmatter, value: string | null) =>
    front[key] ? field(key, front[key], "child") : field(key, value, "kelpie");
  return {
    chain: ["kelpie:pm"],
    fields: [
      field("role", "pm", "kelpie:pm"),
      own("harness", null),
      own("model", null),
      own("effort", null),
      front.body === "append"
        ? field("body", `${parentBody}\n\n${body}`, "child")
        : field(
            "body",
            body === "" ? parentBody : body,
            body === "" ? "kelpie:pm" : "child",
          ),
    ],
  };
}

const child: AgentView = {
  name: "child",
  exists: true,
  frontmatter: { ...noKeys, extends: "kelpie:pm", model: "opus" },
  body: "My line.",
  problem: null,
  resolved: resolve(
    { ...noKeys, extends: "kelpie:pm", model: "opus" },
    "My line.",
  ),
};

beforeAll(() => {
  // Radix's select and switch read what jsdom does not have
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  Element.prototype.hasPointerCapture = () => false;
  Element.prototype.setPointerCapture = () => {};
  Element.prototype.releasePointerCapture = () => {};
  Element.prototype.scrollIntoView = () => {};
});

beforeEach(() => {
  useAgents.getState().reset();
  agentsList.mockResolvedValue(ok([]));
  agentGet.mockResolvedValue(ok(child));
  agentPreview.mockImplementation(async (_name, front, body) =>
    ok(resolve(front, body)),
  );
  agentSave.mockResolvedValue(ok(null));
  settingsSchema.mockResolvedValue(settingsJsonSchema);
  settingsGet.mockResolvedValue(ok({ global: [], project: null }));
  harnessOptions.mockResolvedValue(ok({ models: [], efforts: [] }));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function openChild() {
  render(<AgentEditor name="child" onClose={() => {}} />);
  await screen.findByRole("group", { name: "Model" });
}

const row = (name: string) => screen.getByRole("group", { name });

describe("the agent editor", () => {
  it("a key from the parent says so", async () => {
    await openChild();

    expect(within(row("Role")).getByText("from kelpie:pm")).toBeTruthy();
    expect(within(row("Model")).getByText("set in this file")).toBeTruthy();
    expect(within(row("Effort")).getByText("from kelpie")).toBeTruthy();
  });

  it("body append shows the parent's body first in the preview", async () => {
    await openChild();
    const preview = screen.getByRole("complementary", { name: "Preview" });
    // Without it the file's own body replaces the parent's
    await waitFor(() =>
      expect(within(preview).getByTestId("preview-body").textContent).toBe(
        "My line.",
      ),
    );

    fireEvent.click(
      screen.getByRole("switch", { name: "Add to the parent's instructions" }),
    );

    await waitFor(() =>
      expect(within(preview).getByTestId("preview-body").textContent).toBe(
        `${parentBody}\n\nMy line.`,
      ),
    );
    expect(agentPreview).toHaveBeenLastCalledWith(
      "child",
      expect.objectContaining({ body: "append" }),
      "My line.",
    );
  });

  it("a refused save shows why and keeps the form", async () => {
    const why =
      "agent child: `extends` names kelpie:nothing, which kelpie has no agent for";
    agentSave.mockResolvedValue(refused(why));
    await openChild();

    fireEvent.change(within(row("Extends")).getByRole("textbox"), {
      target: { value: "kelpie:nothing" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Instructions" }), {
      target: { value: "A new line." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect((await screen.findByRole("alert")).textContent).toBe(why);
    expect(agentSave).toHaveBeenCalledTimes(1);
    expect(agentSave).toHaveBeenCalledWith(
      "child",
      expect.objectContaining({ extends: "kelpie:nothing", model: "opus" }),
      "A new line.",
    );
    // What was typed is still in the form, to fix
    expect(
      (within(row("Extends")).getByRole("textbox") as HTMLInputElement).value,
    ).toBe("kelpie:nothing");
    expect(
      (
        screen.getByRole("textbox", {
          name: "Instructions",
        }) as HTMLTextAreaElement
      ).value,
    ).toBe("A new line.");
    expect(screen.queryByText("Saved")).toBeNull();
  });
});
