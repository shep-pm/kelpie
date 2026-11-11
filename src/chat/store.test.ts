import type { Channel } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  LogEntry,
  SandboxAnswer,
  SandboxView,
  SessionEvent,
} from "@/bindings";

// The real Channel needs Tauri's IPC bridge, which jsdom has not got.
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (event: unknown) => void = () => {};
  },
}));

const pmSend =
  vi.fn<
    (
      text: string,
      on: Channel<SessionEvent>,
    ) => Promise<string | null | undefined>
  >();
const pmCancel = vi.fn(async () => {});
const pmRetry = vi.fn<(on: Channel<SessionEvent>) => Promise<void>>();
const pmHistory =
  vi.fn<
    () => Promise<
      { status: "ok"; data: LogEntry[] } | { status: "error"; error: string }
    >
  >();
const pmAnswer =
  vi.fn<
    (
      id: string,
      answer: SandboxAnswer,
    ) => Promise<
      { status: "ok"; data: null } | { status: "error"; error: string }
    >
  >();

const pmSandbox = vi.fn<() => Promise<SandboxView>>();

vi.mock("@/bindings", () => ({
  commands: {
    pmSend: (text: string, on: Channel<SessionEvent>) => pmSend(text, on),
    pmCancel: () => pmCancel(),
    pmRetry: (on: Channel<SessionEvent>) => pmRetry(on),
    pmHistory: () => pmHistory(),
    pmAnswer: (id: string, answer: SandboxAnswer) => pmAnswer(id, answer),
    pmSandbox: () => pmSandbox(),
    projectGet: async () => "/work/project",
    projectPick: async () => ({ status: "ok", data: "/work/project" }),
  },
}));

import { useChat } from "./store";

/** The channel the last `send` handed to `pmSend`: feed it events like the engine does. */
function emit(event: SessionEvent) {
  const channel = pmSend.mock.calls.at(-1)?.[1];
  if (!channel?.onmessage) throw new Error("no channel is listening");
  channel.onmessage(event);
}

/** One `request_network` call, and the event it puts on its turn. */
const asked = {
  id: "net-1",
  host: "docs.rs",
  why: "to read the tokio::process docs",
};
const askedEvent: SessionEvent = { kind: "sandbox_request", ...asked };

const usage = {
  input: 1200,
  output: 340,
  cache_read: 900,
  cache_write: 0,
  cache_lifetime: null,
};

beforeEach(() => {
  pmSend.mockReset();
  pmSend.mockResolvedValue(undefined);
  pmCancel.mockClear();
  pmRetry.mockReset();
  pmRetry.mockResolvedValue(undefined);
  pmHistory.mockReset();
  pmHistory.mockResolvedValue({ status: "ok", data: [] });
  pmAnswer.mockReset();
  pmAnswer.mockResolvedValue({ status: "ok", data: null });
  pmSandbox.mockReset();
  pmSandbox.mockResolvedValue({ refusal: null });
  useChat.getState().reset();
});

describe("the chat store", () => {
  it("text deltas append to the running turn", async () => {
    await useChat.getState().send("hello");
    emit({ kind: "text", delta: "Hel" });
    emit({ kind: "text", delta: "lo there" });

    const [turn] = useChat.getState().turns;
    expect(turn).toMatchObject({
      user: "hello",
      reply: "Hello there",
      state: "running",
    });
    expect(pmSend).toHaveBeenCalledWith("hello", expect.anything());
  });

  it("a tool step updates in place by id", async () => {
    await useChat.getState().send("go");
    emit({ kind: "tool_step", id: "a", title: "Read a.ts", status: "running" });
    emit({ kind: "tool_step", id: "b", title: "Read b.ts", status: "running" });
    emit({ kind: "tool_step", id: "a", title: "Read a.ts", status: "done" });

    expect(useChat.getState().turns[0]?.steps).toEqual([
      { id: "a", title: "Read a.ts", status: "done" },
      { id: "b", title: "Read b.ts", status: "running" },
    ]);
  });

  it("a sandbox request stays on its turn until it is answered", async () => {
    await useChat.getState().send("go");
    emit(askedEvent);
    emit({ kind: "text", delta: "Waiting on you" });

    expect(useChat.getState().turns[0]?.sandboxRequests).toEqual([
      { ...asked, answer: null },
    ]);
  });

  it("answering a sandbox request sends the answer and keeps it on the turn", async () => {
    await useChat.getState().send("go");
    emit(askedEvent);

    await useChat.getState().answer("net-1", "project");

    expect(pmAnswer).toHaveBeenCalledWith("net-1", "project");
    expect(useChat.getState().turns[0]?.sandboxRequests).toEqual([
      { ...asked, answer: "project" },
    ]);
  });

  it("an answer the app could not carry out says so on the request", async () => {
    pmAnswer.mockResolvedValue({
      status: "error",
      error: "docs.rs is allowed for this session, but not saved",
    });
    await useChat.getState().send("go");
    emit(askedEvent);

    await useChat.getState().answer("net-1", "project");

    expect(useChat.getState().turns[0]?.sandboxRequests).toEqual([
      {
        ...asked,
        answer: "project",
        error: "docs.rs is allowed for this session, but not saved",
      },
    ]);
  });

  it("a request still waiting when the turn ends is denied", async () => {
    await useChat.getState().send("go");
    emit(askedEvent);
    emit({
      kind: "sandbox_request",
      id: "net-2",
      host: "crates.io",
      why: "to check the crate versions",
    });
    await useChat.getState().answer("net-1", "once");
    emit({ kind: "turn_ended", stop_reason: "end_turn", usage: null });

    expect(useChat.getState().turns[0]?.sandboxRequests).toEqual([
      { ...asked, answer: "once" },
      {
        id: "net-2",
        host: "crates.io",
        why: "to check the crate versions",
        answer: "deny",
      },
    ]);
  });

  it("turn ended records its usage", async () => {
    await useChat.getState().send("go");
    emit({ kind: "text", delta: "done" });
    emit({ kind: "turn_ended", stop_reason: "end_turn", usage });

    expect(useChat.getState().turns[0]).toMatchObject({ state: "done", usage });
  });

  it("a cancelled turn ends as cancelled", async () => {
    await useChat.getState().send("go");
    emit({ kind: "turn_ended", stop_reason: "cancelled", usage: null });

    expect(useChat.getState().turns[0]).toMatchObject({
      state: "cancelled",
      usage: null,
    });
  });

  it("a failed event ends the turn with its message", async () => {
    await useChat.getState().send("go");
    emit({ kind: "failed", message: "the harness is not installed" });

    expect(useChat.getState().turns[0]).toMatchObject({
      state: "failed",
      error: "the harness is not installed",
    });
  });

  it("a failed turn asks what the sandbox found and keeps the view", async () => {
    pmSandbox.mockResolvedValue({ refusal: "no_capabilities" });
    await useChat.getState().send("go");
    emit({ kind: "failed", message: "the sandbox cannot start" });

    await vi.waitFor(() =>
      expect(useChat.getState().sandbox).toEqual({
        refusal: "no_capabilities",
      }),
    );
    expect(pmSandbox).toHaveBeenCalledTimes(1);
  });

  it("a turn that works clears the view a failed one kept", async () => {
    pmSandbox.mockResolvedValue({ refusal: "no_namespaces" });
    await useChat.getState().send("go");
    emit({ kind: "failed", message: "the sandbox cannot start" });
    await vi.waitFor(() =>
      expect(useChat.getState().sandbox?.refusal).toBe("no_namespaces"),
    );

    await useChat.getState().send("again");
    emit({ kind: "text", delta: "Hello" });
    emit({ kind: "turn_ended", stop_reason: "end_turn", usage: null });

    expect(useChat.getState().sandbox).toBeNull();
  });

  it("a turn that goes on after a failed event is running again", async () => {
    await useChat.getState().send("go");
    emit({ kind: "failed", message: "kept the default model" });
    emit({ kind: "text", delta: "still here" });

    expect(useChat.getState().turns[0]).toMatchObject({
      state: "running",
      reply: "still here",
      error: "kept the default model",
    });

    await useChat.getState().send("second");
    expect(pmSend).toHaveBeenCalledTimes(1);

    emit({ kind: "turn_ended", stop_reason: "end_turn", usage: null });
    expect(useChat.getState().turns[0]).toMatchObject({
      state: "done",
      error: "kept the default model",
    });
  });

  it("send is refused while a turn runs", async () => {
    await useChat.getState().send("first");
    await useChat.getState().send("second");

    expect(pmSend).toHaveBeenCalledTimes(1);
    expect(useChat.getState().turns).toHaveLength(1);
  });

  it("send works again once the turn has ended", async () => {
    await useChat.getState().send("first");
    emit({ kind: "turn_ended", stop_reason: "end_turn", usage: null });
    await useChat.getState().send("second");

    expect(useChat.getState().turns.map((t) => t.user)).toEqual([
      "first",
      "second",
    ]);
  });

  it("a send the backend rejects fails the turn", async () => {
    pmSend.mockRejectedValue("no session");
    await useChat.getState().send("go");

    expect(useChat.getState().turns[0]).toMatchObject({
      state: "failed",
      error: "no session",
    });
  });

  it("cancel asks the PM to stop", async () => {
    await useChat.getState().send("go");
    await useChat.getState().cancel();

    expect(pmCancel).toHaveBeenCalledTimes(1);
  });

  it("history loads before the first send", async () => {
    let release: (history: { status: "ok"; data: LogEntry[] }) => void =
      () => {};
    pmHistory.mockReturnValue(new Promise((resolve) => (release = resolve)));

    const sending = useChat.getState().send("new question");
    await Promise.resolve();
    expect(pmSend).not.toHaveBeenCalled();

    release({
      status: "ok",
      data: [
        { entry: "user", text: "old question" },
        { entry: "event", kind: "text", delta: "old " },
        { entry: "event", kind: "text", delta: "answer" },
        { entry: "event", kind: "turn_ended", stop_reason: "end_turn", usage },
      ],
    });
    await sending;
    await useChat.getState().load();

    expect(pmHistory).toHaveBeenCalledTimes(1);
    expect(useChat.getState().turns).toMatchObject([
      { user: "old question", reply: "old answer", state: "done", usage },
      { user: "new question", state: "running" },
    ]);
  });

  it("a divider entry shows in the history", async () => {
    const switched = "Switched from Claude Code (opus) to goose (qwen)";
    const ended = {
      entry: "event",
      kind: "turn_ended",
      stop_reason: "end_turn",
      usage: null,
    } as const;
    pmHistory.mockResolvedValue({
      status: "ok",
      data: [
        { entry: "user", text: "first" },
        ended,
        // A switch is logged before the message that starts the new session
        { entry: "divider", text: switched },
        { entry: "user", text: "second" },
        ended,
        // A refused resume is logged while its message's turn is running
        { entry: "user", text: "third" },
        { entry: "event", kind: "failed", message: "could not resume" },
        { entry: "divider", text: "Couldn't resume, started a new session" },
        { entry: "event", kind: "started", session_id: "s2" },
        ended,
      ],
    });

    await useChat.getState().load();

    expect(useChat.getState().turns).toMatchObject([
      { user: "first", dividers: [] },
      { user: "second", dividers: [switched] },
      {
        user: "third",
        dividers: ["Couldn't resume, started a new session"],
        state: "done",
      },
    ]);
  });

  it("a send that switched the harness shows the divider it was given", async () => {
    pmSend.mockResolvedValue("Switched from Claude Code to goose");

    await useChat.getState().send("hello");

    expect(useChat.getState().turns).toMatchObject([
      { user: "hello", dividers: ["Switched from Claude Code to goose"] },
    ]);
  });

  it("a turn with no end shows as interrupted", async () => {
    pmHistory.mockResolvedValue({
      status: "ok",
      data: [
        { entry: "user", text: "first" },
        {
          entry: "event",
          kind: "turn_ended",
          stop_reason: "end_turn",
          usage: null,
        },
        { entry: "user", text: "cut off" },
        { entry: "event", kind: "text", delta: "Par" },
      ],
    });

    await useChat.getState().load();

    expect(useChat.getState().turns).toMatchObject([
      { user: "first", state: "done" },
      { user: "cut off", reply: "Par", state: "interrupted" },
    ]);
  });

  it("a turn the log ends with a failure stays failed", async () => {
    pmHistory.mockResolvedValue({
      status: "ok",
      data: [
        { entry: "user", text: "go" },
        { entry: "event", kind: "failed", message: "the harness exited" },
      ],
    });

    await useChat.getState().load();

    expect(useChat.getState().turns[0]).toMatchObject({
      state: "failed",
      error: "the harness exited",
    });
  });

  it("an interrupted turn the maintainer retried shows once", async () => {
    pmHistory.mockResolvedValue({
      status: "ok",
      data: [
        { entry: "user", text: "cut off" },
        { entry: "event", kind: "text", delta: "Par" },
        { entry: "user", text: "cut off" },
        { entry: "event", kind: "text", delta: "Whole answer" },
        {
          entry: "event",
          kind: "turn_ended",
          stop_reason: "end_turn",
          usage: null,
        },
      ],
    });

    await useChat.getState().load();

    expect(useChat.getState().turns).toMatchObject([
      { user: "cut off", reply: "Whole answer", state: "done" },
    ]);
  });

  it("a history that cannot be read is kept as an error and the chat still works", async () => {
    pmHistory.mockResolvedValue({ status: "error", error: "line 3 is torn" });

    await useChat.getState().send("go");

    expect(useChat.getState().historyError).toBe("line 3 is torn");
    expect(pmSend).toHaveBeenCalledTimes(1);
  });

  it("retry replaces the interrupted turn with the same message sent again", async () => {
    pmHistory.mockResolvedValue({
      status: "ok",
      data: [
        { entry: "user", text: "cut off" },
        { entry: "event", kind: "text", delta: "Par" },
      ],
    });
    await useChat.getState().load();

    await useChat.getState().retry();

    expect(pmRetry).toHaveBeenCalledTimes(1);
    expect(pmSend).not.toHaveBeenCalled();
    expect(useChat.getState().turns).toMatchObject([
      { user: "cut off", reply: "", state: "running" },
    ]);
  });
});
