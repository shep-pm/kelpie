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
  FieldView,
  Options,
  ResolvedView,
  Target,
  Value,
} from "@/bindings";

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

const settingsGet = vi.fn<(project: string | null) => Promise<unknown>>();
const settingsSet =
  vi.fn<(layer: Target, path: string, value: Value) => Promise<unknown>>();
const settingsReset =
  vi.fn<(layer: Target, path: string) => Promise<unknown>>();
const settingsSchema = vi.fn<() => Promise<Value>>();
const harnessOptions =
  vi.fn<(harness: string, refresh: boolean) => Promise<unknown>>();
const loginStatus = vi.fn<(harness: string) => Promise<unknown>>();
const loginSet = vi.fn<(harness: string, value: string) => Promise<unknown>>();
const loginClear = vi.fn<(harness: string) => Promise<unknown>>();

vi.mock("@/bindings", () => ({
  commands: {
    settingsGet: (project: string | null) => settingsGet(project),
    settingsSet: (layer: Target, path: string, value: Value) =>
      settingsSet(layer, path, value),
    settingsReset: (layer: Target, path: string) => settingsReset(layer, path),
    settingsSchema: () => settingsSchema(),
    harnessOptions: (harness: string, refresh: boolean) =>
      harnessOptions(harness, refresh),
    loginStatus: (harness: string) => loginStatus(harness),
    loginSet: (harness: string, value: string) => loginSet(harness, value),
    loginClear: (harness: string) => loginClear(harness),
  },
}));

import { SettingsPage } from "./SettingsPage";
import { useSettings } from "./store";

/** The schema as the Rust types write it: nullable types, a `$ref`, `oneOf` of consts. */
const schema = {
  $defs: {
    Harness: {
      description: "A harness kelpie can run a session on",
      oneOf: [
        { const: "claude-code", description: "Claude Code", type: "string" },
        { const: "goose", description: "goose", type: "string" },
      ],
    },
  },
  $schema: "https://json-schema.org/draft/2020-12/schema",
  additionalProperties: false,
  properties: {
    $schema: { default: null, type: ["string", "null"] },
    host: {
      default: null,
      description: "The host a provider runs on",
      type: ["string", "null"],
    },
    "pm-effort": {
      default: null,
      description: "How much effort the PM puts in",
      type: ["string", "null"],
    },
    "pm-harness": {
      anyOf: [{ $ref: "#/$defs/Harness" }, { type: "null" }],
      default: null,
      description: "Which harness the PM's next message runs on",
    },
    "pm-model": {
      default: null,
      description: "Which model the PM runs on",
      type: ["string", "null"],
    },
    "read-anywhere": {
      default: null,
      description: "Whether an agent may read anywhere by default",
      type: ["boolean", "null"],
    },
  },
  title: "Settings",
  type: "object",
} as unknown as Value;

const field = (key: string, value: Value, from: string): FieldView => ({
  key,
  value,
  from,
});

const globalFields = [
  field("pm-harness", "claude-code", "kelpie"),
  field("pm-model", "opus", "kelpie"),
  field("pm-effort", null, "kelpie"),
  field("host", "http://one", "global"),
  field("read-anywhere", false, "kelpie"),
];

function resolved(project: FieldView[] = globalFields): ResolvedView {
  return {
    global: globalFields,
    project: { id: "p1", path: "/work/project", fields: project },
  };
}

const offered: Options = {
  models: [
    { id: "opus", name: "Opus", description: null },
    { id: "sonnet", name: "Sonnet", description: "Faster" },
  ],
  efforts: [{ id: "high", name: "High", description: null }],
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
  useSettings.getState().reset();
  settingsSchema.mockResolvedValue(schema);
  settingsGet.mockResolvedValue(ok(resolved()));
  settingsSet.mockResolvedValue(ok(null));
  settingsReset.mockResolvedValue(ok(null));
  harnessOptions.mockResolvedValue(ok(offered));
  loginStatus.mockResolvedValue(ok(false));
  loginSet.mockResolvedValue(ok(null));
  loginClear.mockResolvedValue(ok(null));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function openPage() {
  render(<SettingsPage onClose={() => {}} />);
  await screen.findByRole("group", { name: "PM model" });
}

function openProjectTab() {
  fireEvent.mouseDown(screen.getByRole("tab", { name: "Project" }), {
    button: 0,
  });
}

const row = (name: string) => screen.getByRole("group", { name });

describe("the settings page", () => {
  it("a field says which layer it comes from", async () => {
    settingsGet.mockResolvedValue(
      ok(
        resolved([
          field("pm-harness", "claude-code", "kelpie"),
          field("pm-model", "haiku", "project"),
          field("pm-effort", null, "kelpie"),
          field("host", "http://one", "global"),
          field("read-anywhere", false, "kelpie"),
        ]),
      ),
    );
    await openPage();

    expect(within(row("PM model")).getByText("from kelpie")).toBeTruthy();
    expect(within(row("Host")).getByText("from global")).toBeTruthy();

    openProjectTab();

    const model = await screen.findByRole("group", { name: "PM model" });
    expect(within(model).getByText("set in this project")).toBeTruthy();
    expect(within(row("Host")).getByText("from global")).toBeTruthy();
    expect(within(row("PM effort")).getByText("from kelpie")).toBeTruthy();
  });

  it("reset falls back to the layer below", async () => {
    settingsGet.mockResolvedValue(
      ok(
        resolved([
          field("pm-harness", "claude-code", "kelpie"),
          field("pm-model", "haiku", "project"),
          field("pm-effort", null, "kelpie"),
          field("host", "http://one", "global"),
          field("read-anywhere", false, "kelpie"),
        ]),
      ),
    );
    await openPage();
    openProjectTab();
    const model = await screen.findByRole("group", { name: "PM model" });
    settingsGet.mockResolvedValue(
      ok(
        resolved([
          field("pm-harness", "claude-code", "kelpie"),
          field("pm-model", "sonnet", "global"),
          field("pm-effort", null, "kelpie"),
          field("host", "http://one", "global"),
          field("read-anywhere", false, "kelpie"),
        ]),
      ),
    );

    fireEvent.click(
      within(model).getByRole("button", { name: /reset to global/i }),
    );

    await waitFor(() =>
      expect(settingsReset).toHaveBeenCalledWith(
        { kind: "project", id: "p1" },
        "pm-model",
      ),
    );
    const after = await screen.findByRole("group", { name: "PM model" });
    expect(await within(after).findByText("from global")).toBeTruthy();
    expect(within(after).queryByRole("button", { name: /reset/i })).toBeNull();
  });

  it("the model select lists the harness's options", async () => {
    await openPage();

    const select = await screen.findByRole("combobox", { name: "PM model" });
    await waitFor(() =>
      expect(harnessOptions).toHaveBeenCalledWith("claude", false),
    );
    fireEvent.keyDown(select, { key: "Enter" });

    expect(await screen.findByRole("option", { name: "Opus" })).toBeTruthy();
    expect(screen.getByRole("option", { name: /Sonnet/ })).toBeTruthy();
  });

  it("a model chosen on the Global tab is written to the global layer", async () => {
    await openPage();
    const select = await screen.findByRole("combobox", { name: "PM model" });
    await waitFor(() => expect(harnessOptions).toHaveBeenCalled());
    fireEvent.keyDown(select, { key: "Enter" });

    fireEvent.click(await screen.findByRole("option", { name: /Sonnet/ }));

    await waitFor(() =>
      expect(settingsSet).toHaveBeenCalledWith(
        { kind: "global" },
        "pm-model",
        "sonnet",
      ),
    );
  });

  it("the refresh button asks the harness again", async () => {
    await openPage();
    await waitFor(() => expect(harnessOptions).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: /refresh/i }));

    await waitFor(() =>
      expect(harnessOptions).toHaveBeenLastCalledWith("claude", true),
    );
  });

  it("a harness that cannot be asked leaves the model a text field and says why", async () => {
    harnessOptions.mockResolvedValue({
      status: "error",
      error: "kelpie has no goose harness to ask",
    });
    await openPage();

    expect(await screen.findByText(/no goose harness to ask/)).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "PM model" })).toBeTruthy();
  });

  it("a login shows set or not set and never its value", async () => {
    loginStatus.mockResolvedValue(ok(true));
    await openPage();

    const login = await screen.findByRole("group", { name: "Login" });
    expect(await within(login).findByText("set")).toBeTruthy();
    expect(within(login).getByText("claude setup-token")).toBeTruthy();

    const paste = within(login).getByLabelText("Paste a token");
    fireEvent.change(paste, { target: { value: "sk-secret-token" } });
    loginStatus.mockResolvedValue(ok(true));
    fireEvent.click(within(login).getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(loginSet).toHaveBeenCalledWith("claude", "sk-secret-token"),
    );
    await waitFor(() => expect((paste as HTMLInputElement).value).toBe(""));
    expect(document.body.textContent).not.toContain("sk-secret-token");

    loginStatus.mockResolvedValue(ok(false));
    fireEvent.click(within(login).getByRole("button", { name: "Clear" }));

    await waitFor(() => expect(loginClear).toHaveBeenCalledWith("claude"));
    expect(await within(login).findByText("not set")).toBeTruthy();
  });
});
