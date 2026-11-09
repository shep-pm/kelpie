import type { Channel } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
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

vi.mock("@/bindings", () => ({
  commands: {
    pmSend: (text: string, on: Channel<SessionEvent>) => pmSend(text, on),
    pmCancel: () => pmCancel(),
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
});
