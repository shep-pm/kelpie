/**
 * `<MdxField>` tests, carried from chelone's. jsdom cannot run the real editor,
 * so `MDXEditor` is a stub that renders its `markdown` and keeps the real
 * module's exports.
 */

import { headingsPlugin, type RealmPlugin } from "@mdxeditor/editor";
import { act, cleanup, render, screen } from "@testing-library/react";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  type Mock,
  vi,
} from "vitest";
import { defaultFieldPlugins } from "./default-plugins";
import { MdxField } from "./mdx-field";
import { safeHtmlImagePlugin } from "./safe-html-image-plugin";

interface Hoisted {
  setMarkdown: Mock<(value: string) => void>;
  plugins: RealmPlugin[] | undefined;
  suppressHtmlProcessing: boolean | undefined;
  onError: ((payload: { error: string; source: string }) => void) | undefined;
}

const hoisted = vi.hoisted(
  (): Hoisted => ({
    setMarkdown: vi.fn<(value: string) => void>(),
    plugins: undefined,
    suppressHtmlProcessing: undefined,
    onError: undefined,
  }),
);

vi.mock("@mdxeditor/editor", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@mdxeditor/editor")>();
  const { forwardRef, useImperativeHandle, useRef } = await import("react");
  const MDXEditor = forwardRef<
    unknown,
    {
      markdown: string;
      className?: string;
      readOnly?: boolean;
      plugins?: RealmPlugin[];
      suppressHtmlProcessing?: boolean;
      onError?: (payload: { error: string; source: string }) => void;
    }
  >(function MDXEditor(props, ref) {
    // Uncontrolled like the real editor: only the first `markdown` is read.
    const current = useRef(props.markdown);
    hoisted.plugins = props.plugins;
    hoisted.suppressHtmlProcessing = props.suppressHtmlProcessing;
    hoisted.onError = props.onError;
    useImperativeHandle(ref, () => ({
      getMarkdown: () => current.current,
      setMarkdown: (value: string) => {
        current.current = value;
        hoisted.setMarkdown(value);
      },
    }));
    return (
      <div
        data-testid="editor"
        className={props.className}
        data-readonly={String(props.readOnly)}
      >
        {props.markdown}
      </div>
    );
  });
  return { ...actual, MDXEditor };
});

/** The editor gets the given list as is, with the inert `<img>` plugin last. */
function expectPluginList(
  actual: RealmPlugin[] | undefined,
  given: RealmPlugin[],
) {
  expect(actual).toBeDefined();
  const list = actual as RealmPlugin[];
  expect(list).toHaveLength(given.length + 1);
  given.forEach((plugin, index) => {
    expect(list[index]).toBe(plugin);
  });
  expect(list[given.length]).toBe(safeHtmlImagePlugin);
}

beforeEach(() => {
  hoisted.setMarkdown.mockClear();
  hoisted.plugins = undefined;
  hoisted.suppressHtmlProcessing = undefined;
  hoisted.onError = undefined;
  document.documentElement.classList.remove("dark");
});
afterEach(cleanup);

describe("<MdxField />", () => {
  it("renders the markdown, read-only", () => {
    render(<MdxField value="# Hello" />);
    const editor = screen.getByTestId("editor");
    expect(editor.textContent).toBe("# Hello");
    expect(editor.dataset.readonly).toBe("true");
  });

  it("passes the field plugin preset by default and honors a plugins prop", () => {
    const { unmount } = render(<MdxField value="x" />);
    expectPluginList(hoisted.plugins, defaultFieldPlugins);
    unmount();

    const custom = [headingsPlugin()];
    render(<MdxField value="x" plugins={custom} />);
    expectPluginList(hoisted.plugins, custom);
  });

  it("renders an empty string as the markdown when the value is missing", () => {
    render(<MdxField value={null} empty="*no notes*" />);
    expect(screen.getByTestId("editor").textContent).toBe("*no notes*");
  });

  it("returns an empty element in place of the editor when the value is missing", () => {
    render(<MdxField value={undefined} empty={<em>Nothing yet</em>} />);
    expect(screen.getByText("Nothing yet")).toBeTruthy();
    expect(screen.queryByTestId("editor")).toBeNull();
  });

  it("calls setMarkdown when the value changes", () => {
    const { rerender } = render(<MdxField value="one" />);
    expect(hoisted.setMarkdown).not.toHaveBeenCalled();

    rerender(<MdxField value="two" />);
    expect(hoisted.setMarkdown).toHaveBeenCalledTimes(1);
    expect(hoisted.setMarkdown).toHaveBeenCalledWith("two");

    rerender(<MdxField value="two" />);
    expect(hoisted.setMarkdown).toHaveBeenCalledTimes(1);
  });

  it("adds the mdxeditor-dark class when the window is dark", () => {
    document.documentElement.classList.add("dark");
    render(<MdxField value="x" />);
    expect(
      screen.getByTestId("editor").classList.contains("mdxeditor-dark"),
    ).toBe(true);
  });

  it("omits the mdxeditor-dark class when the window is light", () => {
    render(<MdxField value="x" />);
    expect(
      screen.getByTestId("editor").classList.contains("mdxeditor-dark"),
    ).toBe(false);
  });

  it("passes suppressHtmlProcessing to the editor by default", () => {
    render(<MdxField value="<iframe></iframe>" />);
    expect(hoisted.suppressHtmlProcessing).toBe(true);
  });

  it("lets a caller turn HTML processing back on", () => {
    render(<MdxField value="<b>x</b>" suppressHtmlProcessing={false} />);
    expect(hoisted.suppressHtmlProcessing).toBe(false);
  });

  it("shows the raw markdown as text when the editor reports a parse error", () => {
    const body = "<img src=x>";
    render(<MdxField value={body} />);
    expect(screen.queryByTestId("editor")).not.toBeNull();

    act(() => hoisted.onError?.({ error: "bad attribute", source: body }));

    const fallback = document.querySelector('[data-slot="mdx-field-fallback"]');
    expect(fallback?.tagName).toBe("PRE");
    expect(fallback?.textContent).toBe(body);
    expect(screen.queryByTestId("editor")).toBeNull();
  });

  it("calls a caller's onError as well as showing the fallback", () => {
    const onError = vi.fn();
    render(<MdxField value="<img src=x>" onError={onError} />);
    const payload = { error: "bad", source: "<img src=x>" };
    act(() => hoisted.onError?.(payload));
    expect(onError).toHaveBeenCalledWith(payload);
  });

  it("tries the editor again when the value changes after a parse error", () => {
    const { rerender } = render(<MdxField value="<img src=x>" />);
    act(() => hoisted.onError?.({ error: "bad", source: "<img src=x>" }));
    expect(screen.queryByTestId("editor")).toBeNull();

    rerender(<MdxField value="# Fixed" />);
    expect(screen.getByTestId("editor").textContent).toBe("# Fixed");
  });
});
