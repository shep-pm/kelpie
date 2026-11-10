import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { browser, expect } from "@wdio/globals";

const resultsDir = join(import.meta.dirname, "..", "..", "test-results");

describe("the app window", () => {
  it("opens one window", async () => {
    const handles = await browser.getWindowHandles();
    expect(handles).toHaveLength(1);
  });

  it("shows the PM's chat box", async () => {
    // A fresh data folder has no project yet, so the box offers the picker;
    // one that has a project offers the composer.
    await browser.waitUntil(
      async () =>
        (await $("button=Pick a folder").isExisting()) ||
        (await $("textarea").isExisting()),
    );
  });

  it("saves a screenshot of the page", async () => {
    // Text stays invisible until the web fonts load, and the capture is a
    // frame, so wait for both before taking it.
    await browser.execute(async () => {
      await document.fonts.ready;
      await new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      );
    });
    mkdirSync(resultsDir, { recursive: true });
    await browser.saveScreenshot(join(resultsDir, "window.png"));
  });
});
