import {
  addImportVisitor$,
  type MdastImportVisitor,
  realmPlugin,
  UnrecognizedMarkdownConstructError,
} from "@mdxeditor/editor";

interface HtmlImage {
  src: string;
  altText: string;
  title: string;
  width?: number;
  height?: number;
}

// Browsers drop tabs and newlines inside a URL and ignore leading control
// characters, so `java\tscript:` is a `javascript:` URL.
const BLOCKED_SRC = /^(javascript|vbscript|data(?!:image\/)):/i;

function safeSrc(src: string) {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: that is the point
  const normalized = src.replace(/[\u0000- ]/g, "");
  return BLOCKED_SRC.test(normalized) ? "" : src;
}

function size(value: string | null) {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : undefined;
}

/**
 * Reads the image out of an HTML string without a browser loading it:
 * `DOMParser` makes a document with no browsing context, so nothing fetches and
 * no handler runs. Only `src`, `alt`, `title`, `width` and `height` survive, so
 * every `on*` attribute is dropped, and a `javascript:`, `vbscript:` or
 * non-image `data:` source is emptied.
 */
function readHtmlImage(html: string): HtmlImage | null {
  const image = new DOMParser()
    .parseFromString(html, "text/html")
    .querySelector("img");
  if (!image) return null;
  return {
    src: safeSrc(image.getAttribute("src") ?? ""),
    altText: image.getAttribute("alt") ?? "",
    title: image.getAttribute("title") ?? "",
    width: size(image.getAttribute("width")),
    height: size(image.getAttribute("height")),
  };
}

function escapeAttribute(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function toSafeHtml({ src, altText, title, width, height }: HtmlImage) {
  const entries: [string, string][] = [
    ["src", src],
    ["alt", altText],
    ["title", title],
    ["width", width === undefined ? "" : String(width)],
    ["height", height === undefined ? "" : String(height)],
  ];
  const attributes = entries
    .filter(([, value]) => value !== "")
    .map(([name, value]) => `${name}="${escapeAttribute(value)}"`);
  return `<img ${attributes.join(" ")}>`;
}

// MDXEditor's own `<img>` visitor sets `innerHTML` on a live element, so the
// browser loads the image and runs `onerror`. This one goes first, rewrites the
// node to the attributes `readHtmlImage` keeps, and hands it on, so that
// visitor only ever sees markup with nothing to run.
const safeHtmlImageVisitor: MdastImportVisitor<{
  type: "html";
  value: string;
}> = {
  testNode: (node) =>
    node.type === "html" && node.value.trim().startsWith("<img"),
  visitNode({ mdastNode, actions }) {
    const image = readHtmlImage(mdastNode.value);
    if (!image) {
      throw new UnrecognizedMarkdownConstructError("Invalid HTML image");
    }
    mdastNode.value = toSafeHtml(image);
    actions.nextVisitor();
  },
  priority: 1000,
};

/**
 * Makes raw `<img>` HTML in markdown inert. `MdxField` and `MdxInput` add it to
 * whatever `plugins` they get, since `imagePlugin()` would otherwise run the
 * `onerror` of `<img src=x onerror="...">`.
 */
const safeHtmlImagePlugin = realmPlugin({
  init(realm) {
    realm.pubIn({ [addImportVisitor$]: safeHtmlImageVisitor });
  },
})();

export { readHtmlImage, safeHtmlImagePlugin };
