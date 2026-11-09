import type { Channel } from "@tauri-apps/api/core";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
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
import type { SessionEvent } from "@/bindings";

// The real Channel needs Tauri's IPC bridge, which jsdom has not got.
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (event: unknown) => void = () => {};
  },
}));

const pmSend =
  vi.fn<(text: string, on: Channel<SessionEvent>) => Promise<void>>();
const pmCancel = vi.fn(async () => {});
const projectGet = vi.fn<() => Promise<string | null>>();
const projectPick = vi.fn();

vi.mock("@/bindings", () => ({
  commands: {
    pmSend: (text: string, on: Channel<SessionEvent>) => pmSend(text, on),
    pmCancel: () => pmCancel(),
    projectGet: () => projectGet(),
    projectPick: () => projectPick(),
  },
}));

import { ChatBox } from "./ChatBox";
import { useChat } from "./store";

function emit(event: SessionEvent) {
  const channel = pmSend.mock.calls.at(-1)?.[1];
  if (!channel?.onmessage) throw new Error("no channel is listening");
  act(() => channel.onmessage(event));
}

async function openBox() {
  render(<ChatBox />);
  return screen.findByPlaceholderText("Message the PM");
}

async function sendMessage(text: string) {
  const box = await openBox();
  fireEvent.change(box, { target: { value: text } });
  fireEvent.keyDown(box, { key: "Enter" });
  await waitFor(() => expect(pmSend).toHaveBeenCalled());
  return box;
}

const baseUsage = {
  input: 1200,
  output: 340,
  cache_read: 0,
  cache_write: 0,
  cache_lifetime: null,
};

beforeAll(() => {
  // jsdom has no `Element.scrollTo`, which Streamdown calls from an animation
  // frame to follow a streaming code block.
  Element.prototype.scrollTo = () => {};
});

beforeEach(() => {
  pmSend.mockReset();
  pmSend.mockResolvedValue(undefined);
  pmCancel.mockClear();
  projectGet.mockReset();
  projectGet.mockResolvedValue("/work/project");
  projectPick.mockReset();
  useChat.getState().reset();
});

afterEach(cleanup);

describe("the chat box", () => {
  it("Enter sends and Shift+Enter adds a line", async () => {
    const box = await openBox();
    fireEvent.change(box, { target: { value: "plan the slice" } });

    // Shift+Enter is left to the textarea, which adds the line.
    const shiftEnter = fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
    expect(shiftEnter).toBe(true);
    expect(pmSend).not.toHaveBeenCalled();

    const enter = fireEvent.keyDown(box, { key: "Enter" });
    expect(enter).toBe(false);
    await waitFor(() =>
      expect(pmSend).toHaveBeenCalledWith("plan the slice", expect.anything()),
    );
    expect((box as HTMLTextAreaElement).value).toBe("");
  });

  it("Enter on an empty box sends nothing", async () => {
    const box = await openBox();
    fireEvent.keyDown(box, { key: "Enter" });
    expect(pmSend).not.toHaveBeenCalled();
  });

  it("Stop replaces Send while a turn runs", async () => {
    await sendMessage("hello");

    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    fireEvent.click(await screen.findByRole("button", { name: "Stop" }));
    expect(pmCancel).toHaveBeenCalledTimes(1);

    emit({ kind: "turn_ended", stop_reason: "cancelled", usage: null });
    expect(await screen.findByRole("button", { name: "Send" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("streams the reply into the box", async () => {
    await sendMessage("hello");
    emit({ kind: "text", delta: "Hi " });
    emit({ kind: "text", delta: "there" });

    expect(await screen.findByText("Hi there")).toBeTruthy();
    expect(screen.getByText("hello")).toBeTruthy();
  });

  it("consecutive steps of one kind merge into one line", async () => {
    await sendMessage("look around");
    for (const [id, title] of [
      ["1", "Read a.ts"],
      ["2", "Read b.ts"],
      ["3", "Read c.ts"],
    ] as const) {
      emit({ kind: "tool_step", id, title, status: "done" });
    }
    emit({
      kind: "tool_step",
      id: "4",
      title: "Grep ProcessTree",
      status: "done",
    });

    expect(await screen.findByText("Read 3 files")).toBeTruthy();
    expect(screen.getByText("Grep ProcessTree")).toBeTruthy();
    expect(screen.queryByText("Read a.ts")).toBeNull();
  });

  it("the footer shows cache read and written", async () => {
    await sendMessage("hello");
    emit({
      kind: "turn_ended",
      stop_reason: "end_turn",
      usage: {
        ...baseUsage,
        cache_read: 900,
        cache_write: 50,
        cache_lifetime: "five_minutes",
      },
    });

    const footer = await screen.findByLabelText("Usage");
    expect(footer.textContent).toBe(
      "1.2k in · 340 out · cache read 900 · cache written 50 · cache 5m",
    );
  });

  it("the footer leaves out a field the harness did not report", async () => {
    await sendMessage("hello");
    emit({ kind: "turn_ended", stop_reason: "end_turn", usage: baseUsage });

    const footer = await screen.findByLabelText("Usage");
    expect(footer.textContent).toBe("1.2k in · 340 out");
  });

  it("the footer rolls a count just under a million up to M", async () => {
    await sendMessage("hello");
    emit({
      kind: "turn_ended",
      stop_reason: "end_turn",
      usage: { ...baseUsage, input: 999_960 },
    });

    const footer = await screen.findByLabelText("Usage");
    expect(footer.textContent).toBe("1.0M in · 340 out");
  });

  it("a long reply folds behind the rest, and a code block that crosses the fold stays one block", async () => {
    await sendMessage("explain");
    const code = Array.from({ length: 40 }, (_, i) => `line ${i}`).join("\n");
    emit({
      kind: "text",
      delta: `Intro\n\n\`\`\`\n${code}\n\`\`\`\n\nAfter the code`,
    });
    emit({ kind: "turn_ended", stop_reason: "end_turn", usage: null });

    const trigger = await screen.findByRole("button", {
      name: /^Show the rest · \d+ lines$/,
    });
    expect(trigger.getAttribute("aria-expanded")).toBe("false");

    const after = screen.getByText("After the code");
    expect(after.closest('[data-streamdown="code-block"]')).toBeNull();

    fireEvent.click(trigger);
    expect(
      screen
        .getByText("After the code")
        .closest('[data-streamdown="code-block"]'),
    ).toBeNull();
    expect(screen.getByRole("button", { name: "Show less" })).toBeTruthy();
  });

  it("shows a failed turn's message under the turn", async () => {
    await sendMessage("hello");
    emit({ kind: "failed", message: "claude is not installed" });

    expect(await screen.findByText("claude is not installed")).toBeTruthy();
  });

  it("with no project the box offers a folder picker", async () => {
    projectGet.mockResolvedValue(null);
    projectPick.mockResolvedValue({ status: "ok", data: "/work/project" });
    render(<ChatBox />);

    expect(screen.queryByPlaceholderText("Message the PM")).toBeNull();
    fireEvent.click(
      await screen.findByRole("button", { name: "Pick a folder" }),
    );

    expect(projectPick).toHaveBeenCalledTimes(1);
    expect(await screen.findByPlaceholderText("Message the PM")).toBeTruthy();
  });
});
