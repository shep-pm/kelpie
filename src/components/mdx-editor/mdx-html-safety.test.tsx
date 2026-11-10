/**
 * Raw `<img>` HTML in stored markdown, through the real editor. jsdom loads no
 * images, so an `onerror` never fires here; instead the test watches for the
 * attacker's markup being parsed by a live element's `innerHTML` setter, which
 * is how MDXEditor's own `<img>` visitor lets the browser run the handler.
 */

import { imagePlugin } from "@mdxeditor/editor";
import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { MdxField } from "./mdx-field";
import { MdxInput } from "./mdx-input";
import { readHtmlImage } from "./safe-html-image-plugin";

const attack = '<img src=x alt="pic" onerror="alert(document.cookie)">';

let parsed: string[] = [];
let restore: () => void = () => {};

beforeEach(() => {
  parsed = [];
  const original = Object.getOwnPropertyDescriptor(
    Element.prototype,
    "innerHTML",
  );
  if (!original?.set) throw new Error("innerHTML setter missing");
  const { set } = original;
  Object.defineProperty(Element.prototype, "innerHTML", {
    ...original,
    set(value: string) {
      parsed.push(String(value));
      set.call(this, value);
    },
  });
  restore = () =>
    Object.defineProperty(Element.prototype, "innerHTML", original);
});

afterEach(() => {
  restore();
  cleanup();
});

async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 300));
}

function parsedAttack() {
  return parsed.filter((html) => html.includes("onerror"));
}

describe("raw <img> HTML in stored markdown", () => {
  it("is not parsed by a live element in MdxField with the default plugins", async () => {
    render(<MdxField value={attack} />);
    await settle();
    expect(parsedAttack()).toEqual([]);
    // The image itself still goes through, with its src and alt.
    expect(parsed).toContain('<img src="x" alt="pic">');
  });

  it("is not parsed by a live element in MdxField with plugins={[imagePlugin()]}", async () => {
    render(<MdxField value={attack} plugins={[imagePlugin()]} />);
    await settle();
    expect(parsedAttack()).toEqual([]);
  });

  it("is not parsed by a live element in MdxInput with the default plugins", async () => {
    render(<MdxInput value={attack} onChange={() => {}} />);
    await settle();
    expect(parsedAttack()).toEqual([]);
  });

  it("is not parsed by a live element in MdxInput with plugins={[imagePlugin()]}", async () => {
    render(
      <MdxInput
        value={attack}
        onChange={() => {}}
        mdxProps={{ plugins: [imagePlugin()] }}
      />,
    );
    await settle();
    expect(parsedAttack()).toEqual([]);
  });
});

describe("readHtmlImage", () => {
  it("keeps src, alt, title and size and drops every on* attribute", () => {
    expect(
      readHtmlImage(
        '<img src="/a.png" alt="pic" title="t" width="20" height="10" onerror="x()" onload="y()">',
      ),
    ).toEqual({
      src: "/a.png",
      altText: "pic",
      title: "t",
      width: 20,
      height: 10,
    });
  });

  it("reads an unquoted attribute", () => {
    expect(readHtmlImage("<img src=x>")?.src).toBe("x");
  });

  it.each([
    "javascript:alert(1)",
    " JavaScript:alert(1)",
    "java\tscript:alert(1)",
    "vbscript:msgbox(1)",
    "data:text/html,<script>alert(1)</script>",
    "data:image/svg+xml;base64,AAAA",
  ])("empties a src of %j", (src) => {
    const image = readHtmlImage(`<img src="${src}">`);
    expect(image?.src).toBe(src.startsWith("data:image/") ? src : "");
  });

  it("keeps http, https, relative and data:image sources", () => {
    for (const src of [
      "https://example.com/a.png",
      "http://example.com/a.png",
      "../a.png",
      "data:image/png;base64,AAAA",
    ]) {
      expect(readHtmlImage(`<img src="${src}">`)?.src).toBe(src);
    }
  });

  it("returns null when the HTML holds no image", () => {
    expect(readHtmlImage("<imgfoo>")).toBeNull();
  });
});
