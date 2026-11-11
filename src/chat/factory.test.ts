import type { Channel } from "@tauri-apps/api/core";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { createElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LogEntry, SessionEvent } from "@/bindings";

// The real Channel needs Tauri's IPC bridge, which jsdom has not got.
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: (event: unknown) => void = () => {};
  },
}));

const pmSend =
  vi.fn<(text: string, on: Channel<SessionEvent>) => Promise<null>>();
const pmHistory = vi.fn();
const pmRetry = vi.fn();
const pmCancel = vi.fn();
const pmAnswer = vi.fn();
const pmSandbox = vi.fn();
const sandboxInstall = vi.fn();

vi.mock("@/bindings", () => ({
  commands: {
    pmSend: (text: string, on: Channel<SessionEvent>) => pmSend(text, on),
    pmHistory: () => pmHistory(),
    pmRetry: (on: Channel<SessionEvent>) => pmRetry(on),
    pmCancel: () => pmCancel(),
    pmAnswer: (id: string, answer: string) => pmAnswer(id, answer),
    pmSandbox: () => pmSandbox(),
    sandboxInstall: () => sandboxInstall(),
    projectGet: async () => "/work/project",
    projectPick: async () => ({ status: "ok", data: "/work/project" }),
  },
}));

import { ChatBox } from "./ChatBox";
import { ChatProvider } from "./context";
import { createChat, useChat } from "./store";
import type { Transport } from "./transport";

/** A session of its own: every call is a spy, and none reaches a `pm_*` command. */
function workerTransport() {
  const transport = {
    history: vi.fn(async () => ({
      status: "ok" as const,
      data: [] as LogEntry[],
    })),
    send: vi.fn(async (_text: string, _on: Channel<SessionEvent>) => ({})),
    retry: vi.fn(async (_on: Channel<SessionEvent>) => ({})),
    cancel: vi.fn(async () => {}),
    answer: vi.fn(async (_id: string, _answer: string) => ({
      status: "ok" as const,
      data: null,
    })),
  } satisfies Transport;
  return transport;
}

const pmCalls = [
  pmSend,
  pmHistory,
  pmRetry,
  pmCancel,
  pmAnswer,
  pmSandbox,
  sandboxInstall,
];

beforeEach(() => {
  for (const call of [...pmCalls]) call.mockReset();
  pmSend.mockResolvedValue(null);
  pmHistory.mockResolvedValue({ status: "ok", data: [] });
  pmAnswer.mockResolvedValue({ status: "ok", data: null });
  useChat.getState().reset();
});

afterEach(cleanup);

describe("createChat", () => {
  it("makes stores that hold their own turns", async () => {
    const first = createChat(workerTransport());
    const second = createChat(workerTransport());

    await first.getState().send("for the first");
    await second.getState().send("for the second");

    expect(first.getState().turns.map((t) => t.user)).toEqual([
      "for the first",
    ]);
    expect(second.getState().turns.map((t) => t.user)).toEqual([
      "for the second",
    ]);
    expect(useChat.getState().turns).toEqual([]);
  });

  it("calls only its own transport, and a worker's never reaches a pm_ command", async () => {
    const mine = workerTransport();
    const other = workerTransport();
    const chat = createChat(mine);

    await chat.getState().send("go");
    await chat.getState().cancel();
    await chat.getState().answer("net-1", "once");

    expect(mine.history).toHaveBeenCalledTimes(1);
    expect(mine.send).toHaveBeenCalledWith("go", expect.anything());
    expect(mine.cancel).toHaveBeenCalledTimes(1);
    expect(mine.answer).toHaveBeenCalledWith("net-1", "once");
    for (const call of Object.values(other))
      expect(call).not.toHaveBeenCalled();
    for (const call of pmCalls) expect(call).not.toHaveBeenCalled();
  });

  it("retries an interrupted turn over its own transport", async () => {
    const mine = workerTransport();
    mine.history.mockResolvedValueOnce({
      status: "ok",
      data: [{ entry: "user", text: "cut off" }],
    });
    const chat = createChat(mine);

    await chat.getState().retry();

    expect(mine.retry).toHaveBeenCalledTimes(1);
    expect(pmRetry).not.toHaveBeenCalled();
  });
});

describe("a transport's watch", () => {
  it("is attached on load, and its events fold into the turns", async () => {
    const watched: Channel<SessionEvent>[] = [];
    const mine = {
      ...workerTransport(),
      watch: vi.fn((channel: Channel<SessionEvent>) => {
        watched.push(channel);
      }),
    };
    mine.history.mockResolvedValueOnce({
      status: "ok",
      data: [
        { entry: "user", text: "build it" },
        { entry: "event", kind: "text", delta: "Started" },
      ],
    });
    const chat = createChat(mine);

    await chat.getState().load();
    await chat.getState().load();

    expect(mine.watch).toHaveBeenCalledTimes(1);
    watched[0]?.onmessage({ kind: "text", delta: " working" });
    expect(chat.getState().turns.map((t) => [t.reply, t.state])).toEqual([
      ["Started working", "running"],
    ]);
    expect(pmHistory).not.toHaveBeenCalled();
  });

  it("starts a new running turn for an event that follows an ended turn", async () => {
    const watched: Channel<SessionEvent>[] = [];
    const mine = {
      ...workerTransport(),
      watch: (channel: Channel<SessionEvent>) => {
        watched.push(channel);
      },
    };
    mine.history.mockResolvedValueOnce({
      status: "ok",
      data: [
        { entry: "user", text: "build it" },
        { entry: "event", kind: "text", delta: "Done" },
        {
          entry: "event",
          kind: "turn_ended",
          stop_reason: "end_turn",
          usage: null,
        },
      ],
    });
    const chat = createChat(mine);
    await chat.getState().load();

    watched[0]?.onmessage({ kind: "text", delta: "Again" });
    watched[0]?.onmessage({
      kind: "turn_ended",
      stop_reason: "end_turn",
      usage: null,
    });
    watched[0]?.onmessage({ kind: "text", delta: "Once more" });

    expect(chat.getState().turns.map((t) => [t.reply, t.state])).toEqual([
      ["Done", "done"],
      ["Again", "done"],
      ["Once more", "running"],
    ]);
  });

  it("holds an event that arrives before the history, and folds it after", async () => {
    const mine = {
      ...workerTransport(),
      watch: vi.fn((channel: Channel<SessionEvent>) => {
        channel.onmessage({ kind: "text", delta: " early" });
      }),
    };
    mine.history.mockResolvedValueOnce({
      status: "ok",
      data: [
        { entry: "user", text: "build it" },
        { entry: "event", kind: "text", delta: "Late" },
      ],
    });
    const chat = createChat(mine);

    await chat.getState().load();

    expect(chat.getState().turns.map((t) => t.reply)).toEqual(["Late early"]);
  });

  it("stops following when the chat closes", async () => {
    const watched: Channel<SessionEvent>[] = [];
    const mine = {
      ...workerTransport(),
      watch: (channel: Channel<SessionEvent>) => {
        watched.push(channel);
      },
      unwatch: vi.fn(async (_channel: Channel<SessionEvent>) => {}),
    };
    mine.history.mockResolvedValueOnce({
      status: "ok",
      data: [{ entry: "user", text: "build it" }],
    });
    const chat = createChat(mine);
    await chat.getState().load();

    chat.getState().close();
    watched[0]?.onmessage({ kind: "text", delta: "after" });

    expect(mine.unwatch).toHaveBeenCalledWith(watched[0]);
    expect(chat.getState().turns[0]?.reply).toBe("");
  });

  it("is not attached by a transport without one", async () => {
    const mine = workerTransport();
    const chat = createChat(mine);

    await chat.getState().load();

    expect(mine.history).toHaveBeenCalledTimes(1);
    expect("watch" in mine).toBe(false);
    expect(chat.getState().turns).toEqual([]);
  });
});

describe("ChatProvider", () => {
  it("gives ChatBox its own store, and ChatBox with none uses the PM's", async () => {
    const mine = workerTransport();
    const store = createChat(mine);

    render(
      createElement(ChatProvider, {
        store,
        // biome-ignore lint/correctness/noChildrenProp: a .ts file has no JSX, and the typed props need it
        children: createElement(ChatBox, { picker: false }),
      }),
    );
    fireEvent.change(await screen.findByLabelText("Message"), {
      target: { value: "to the worker" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() =>
      expect(mine.send).toHaveBeenCalledWith(
        "to the worker",
        expect.anything(),
      ),
    );
    expect(store.getState().turns.map((t) => t.user)).toEqual([
      "to the worker",
    ]);
    expect(pmSend).not.toHaveBeenCalled();
    expect(useChat.getState().turns).toEqual([]);

    cleanup();
    render(createElement(ChatBox));
    fireEvent.change(await screen.findByLabelText("Message"), {
      target: { value: "to the PM" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() =>
      expect(pmSend).toHaveBeenCalledWith("to the PM", expect.anything()),
    );
    expect(useChat.getState().turns.map((t) => t.user)).toEqual(["to the PM"]);
    expect(mine.send).toHaveBeenCalledTimes(1);
  });
});
