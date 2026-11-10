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

  // Text stays invisible until the web fonts load, and the capture is a frame,
  // so wait for both before each shot.
  const settle = () =>
    browser.execute(async () => {
      await document.fonts.ready;
      await new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      );
    });

  // `src/main.tsx` toggles this class from the OS's colour scheme; the test
  // sets it itself so both shots are the same whatever the host's scheme is.
  const setDark = (dark: boolean) =>
    browser.execute((on) => {
      document.documentElement.classList.toggle("dark", on);
    }, dark);

  it("saves a light screenshot of the page", async () => {
    await setDark(false);
    await settle();
    mkdirSync(resultsDir, { recursive: true });
    await browser.saveScreenshot(join(resultsDir, "window-light.png"));
  });

  it("saves a dark screenshot of the page", async () => {
    await setDark(true);
    await settle();
    mkdirSync(resultsDir, { recursive: true });
    await browser.saveScreenshot(join(resultsDir, "window-dark.png"));
  });
});
