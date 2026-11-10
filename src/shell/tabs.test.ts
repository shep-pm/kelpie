import { beforeEach, describe, expect, it } from "vitest";
import { useTabs } from "./tabs";

const ids = () => useTabs.getState().tabs.map((t) => t.id);

beforeEach(() => useTabs.getState().reset());

describe("the PM panel's tabs", () => {
  it("starts with the PM tab, selected", () => {
    expect(ids()).toEqual(["pm"]);
    expect(useTabs.getState().selected).toBe("pm");
  });

  it("opening a tab adds it and selects it, and opening it again adds no second one", () => {
    useTabs.getState().open({ id: "w1", title: "#120" });
    expect(ids()).toEqual(["pm", "w1"]);
    expect(useTabs.getState().selected).toBe("w1");

    useTabs.getState().open({ id: "w2", title: "#121" });
    useTabs.getState().open({ id: "w1", title: "#120" });
    expect(ids()).toEqual(["pm", "w1", "w2"]);
    expect(useTabs.getState().selected).toBe("w1");
  });

  it("closing the selected tab selects the one before it", () => {
    useTabs.getState().open({ id: "w1", title: "#120" });
    useTabs.getState().open({ id: "w2", title: "#121" });

    useTabs.getState().close("w2");
    expect(ids()).toEqual(["pm", "w1"]);
    expect(useTabs.getState().selected).toBe("w1");

    useTabs.getState().close("w1");
    expect(useTabs.getState().selected).toBe("pm");
  });

  it("closing a tab that is not selected keeps the selection", () => {
    useTabs.getState().open({ id: "w1", title: "#120" });
    useTabs.getState().open({ id: "w2", title: "#121" });

    useTabs.getState().close("w1");
    expect(ids()).toEqual(["pm", "w2"]);
    expect(useTabs.getState().selected).toBe("w2");
  });

  it("the PM tab cannot close", () => {
    useTabs.getState().close("pm");
    expect(ids()).toEqual(["pm"]);
    expect(useTabs.getState().selected).toBe("pm");
  });

  it("selecting a tab that is not open changes nothing", () => {
    useTabs.getState().select("w9");
    expect(useTabs.getState().selected).toBe("pm");
  });
});
