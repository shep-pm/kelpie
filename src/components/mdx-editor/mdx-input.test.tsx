/**
 * `<MdxInput>` tests, carried from chelone's. jsdom cannot run the real editor,
 * so `MDXEditor` is a stub that reads `markdown` once like the real one, keeps
 * its own content, and has buttons that report a change and a blur.
 */

import { headingsPlugin, type RealmPlugin } from "@mdxeditor/editor";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { useState } from "react";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  type Mock,
  vi,
} from "vitest";
import { defaultInputPlugins } from "./default-plugins";
import { MdxInput } from "./mdx-input";
import { safeHtmlImagePlugin } from "./safe-html-image-plugin";

interface Hoisted {
  setMarkdown: Mock<(value: string) => void>;
  plugins: RealmPlugin[] | undefined;
  suppressHtmlProcessing: boolean | undefined;
  onError: ((payload: { error: string; source: string }) => void) | undefined;
  markdownProps: string[];
}

const hoisted = vi.hoisted(
  (): Hoisted => ({
    setMarkdown: vi.fn<(value: string) => void>(),
    plugins: undefined,
    suppressHtmlProcessing: undefined,
    onError: undefined,
    markdownProps: [],
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
      onChange?: (markdown: string, initialMarkdownNormalize: boolean) => void;
      onBlur?: (event: FocusEvent) => void;
    }
  >(function MDXEditor(props, ref) {
    // Uncontrolled like the real editor: only the first `markdown` is read.
    const current = useRef(props.markdown);
    hoisted.plugins = props.plugins;
    hoisted.suppressHtmlProcessing = props.suppressHtmlProcessing;
    hoisted.onError = props.onError;
    hoisted.markdownProps.push(props.markdown);
    useImperativeHandle(ref, () => ({
      getMarkdown: () => current.current,
      setMarkdown: (value: string) => {
        current.current = value;
        hoisted.setMarkdown(value);
      },
    }));
    const edit = (value: string, initialMarkdownNormalize = false) => {
      current.current = value;
      props.onChange?.(value, initialMarkdownNormalize);
    };
    return (
      <div
        data-testid="editor"
        className={props.className}
        data-readonly={String(props.readOnly)}
      >
        <span data-testid="content">{current.current}</span>
        <button type="button" onClick={() => edit("# Typed")}>
          type
        </button>
        <button type="button" onClick={() => edit("")}>
          clear
        </button>
        <button type="button" onClick={() => edit("- item", true)}>
          normalize
        </button>
        <button
          type="button"
          onClick={() => props.onBlur?.(new FocusEvent("blur"))}
        >
          blur
        </button>
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

/** An input held by a parent, as the agent editor holds it. */
function Held({
  start,
  seen,
  ...rest
}: {
  start: string;
  seen?: (value: string) => void;
} & Partial<React.ComponentProps<typeof MdxInput>>) {
  const [value, setValue] = useState(start);
  return (
    <>
      <button type="button" onClick={() => setValue("from outside")}>
        set from outside
      </button>
      <button type="button" onClick={() => setValue("")}>
        set empty
      </button>
      <MdxInput
        {...rest}
        value={value}
        onChange={(next) => {
          setValue(next);
          seen?.(next);
        }}
      />
    </>
  );
}

/** A parent that re-renders on demand around a held input. */
function Parent() {
  const [count, setCount] = useState(0);
  return (
    <>
      <button type="button" onClick={() => setCount(count + 1)}>
        rerender {count}
      </button>
      <Held start="# Initial" helperText={`render ${count}`} />
    </>
  );
}

beforeEach(() => {
  hoisted.setMarkdown.mockClear();
  hoisted.plugins = undefined;
  hoisted.suppressHtmlProcessing = undefined;
  hoisted.onError = undefined;
  hoisted.markdownProps = [];
  document.documentElement.classList.remove("dark");
});
afterEach(cleanup);

describe("<MdxInput />", () => {
  it("renders the initial markdown value", () => {
    render(<Held start="# Hello" />);
    expect(screen.getByTestId("content").textContent).toBe("# Hello");
  });

  it("passes the input plugin preset with the toolbar", () => {
    render(<Held start="x" />);
    expectPluginList(hoisted.plugins, defaultInputPlugins);
  });

  it("renders a label and helper text, the label for the editor", () => {
    render(<Held start="x" label="Notes" helperText="Markdown only" />);
    const label = screen.getByText("Notes").closest("label");
    expect(label).toBeTruthy();
    expect(screen.getByText("Markdown only")).toBeTruthy();
    const wrapper = screen.getByTestId("editor").parentElement;
    expect(wrapper?.id).toBeTruthy();
    expect(label?.htmlFor).toBe(wrapper?.id);
  });

  it("shows an error under the editor", () => {
    render(<Held start="x" error="Too long" />);
    expect(screen.getByRole("alert").textContent).toBe("Too long");
  });

  it("is read-only when disabled or readOnly, and editable otherwise", () => {
    const { rerender } = render(<MdxInput value="x" onChange={() => {}} />);
    expect(screen.getByTestId("editor").dataset.readonly).toBe("false");
    rerender(<MdxInput value="x" onChange={() => {}} disabled />);
    expect(screen.getByTestId("editor").dataset.readonly).toBe("true");
    rerender(<MdxInput value="x" onChange={() => {}} readOnly />);
    expect(screen.getByTestId("editor").dataset.readonly).toBe("true");
  });

  it("reports the editor markdown on change", () => {
    const seen = vi.fn();
    render(<Held start="" seen={seen} />);
    fireEvent.click(screen.getByText("type"));
    expect(seen).toHaveBeenCalledWith("# Typed");
  });

  it("keeps an empty string when the editor is cleared", () => {
    const seen = vi.fn();
    render(<Held start="# Old" seen={seen} />);
    fireEvent.click(screen.getByText("clear"));
    expect(seen).toHaveBeenCalledWith("");
  });

  it("reports nothing when the editor normalizes the initial markdown", () => {
    const seen = vi.fn();
    render(<Held start="* item" seen={seen} />);
    fireEvent.click(screen.getByText("normalize"));
    expect(seen).not.toHaveBeenCalled();
  });

  it("calls setMarkdown when the value changes from outside", () => {
    render(<Held start="one" />);
    expect(hoisted.setMarkdown).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("set from outside"));
    expect(hoisted.setMarkdown).toHaveBeenCalledTimes(1);
    expect(hoisted.setMarkdown).toHaveBeenLastCalledWith("from outside");

    fireEvent.click(screen.getByText("set empty"));
    expect(hoisted.setMarkdown).toHaveBeenLastCalledWith("");
  });

  it("does not call setMarkdown for a change the editor itself made", () => {
    render(<Held start="one" />);
    fireEvent.click(screen.getByText("type"));
    expect(hoisted.setMarkdown).not.toHaveBeenCalled();
  });

  it("does not reset the editor when the parent re-renders", () => {
    render(<Parent />);

    fireEvent.click(screen.getByText("type"));
    fireEvent.click(screen.getByText("rerender 0"));

    expect(screen.getByText("render 1")).toBeTruthy();
    expect(new Set(hoisted.markdownProps)).toEqual(new Set(["# Initial"]));
    expect(hoisted.setMarkdown).not.toHaveBeenCalled();
  });

  it("passes the editor's blur event to the onBlur prop", () => {
    const onBlur = vi.fn();
    render(<Held start="x" onBlur={onBlur} />);
    fireEvent.click(screen.getByText("blur"));
    expect(onBlur).toHaveBeenCalledTimes(1);
    expect(onBlur.mock.calls[0]?.[0]).toBeInstanceOf(FocusEvent);
  });

  it("passes suppressHtmlProcessing to the editor by default", () => {
    render(<Held start="<iframe></iframe>" />);
    expect(hoisted.suppressHtmlProcessing).toBe(true);
  });

  it("lets mdxProps turn HTML processing back on", () => {
    render(
      <Held start="<b>x</b>" mdxProps={{ suppressHtmlProcessing: false }} />,
    );
    expect(hoisted.suppressHtmlProcessing).toBe(false);
  });

  it("keeps HTML processing off when mdxProps sets the key to undefined", () => {
    render(
      <Held
        start="<b>x</b>"
        mdxProps={{ suppressHtmlProcessing: undefined }}
      />,
    );
    expect(hoisted.suppressHtmlProcessing).toBe(true);
  });

  it("swaps the editor for a textarea holding the stored markdown when the editor reports a parse error", () => {
    const body = "Intro\n\n<div>note</div>\n\nMore text";
    render(<Held start={body} />);

    act(() => hoisted.onError?.({ error: "unsupported", source: body }));

    expect(screen.queryByTestId("editor")).toBeNull();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(textarea.value).toBe(body);
  });

  it("reports an edit made in the fallback textarea", () => {
    const body = "Intro\n\n<div>note</div>";
    const seen = vi.fn();
    render(<Held start={body} seen={seen} />);
    act(() => hoisted.onError?.({ error: "unsupported", source: body }));

    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: `${body}\n\nMore` },
    });

    expect(seen).toHaveBeenCalledWith(`${body}\n\nMore`);
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe(
      `${body}\n\nMore`,
    );
  });

  it("lets mdxProps replace the plugins", () => {
    const custom = [headingsPlugin()];
    render(<Held start="x" mdxProps={{ plugins: custom }} />);
    expectPluginList(hoisted.plugins, custom);
  });

  it("adds the mdxeditor-dark class when the window is dark, and drops it when it is not", async () => {
    document.documentElement.classList.add("dark");
    render(<Held start="x" />);
    expect(
      screen.getByTestId("editor").classList.contains("mdxeditor-dark"),
    ).toBe(true);

    await act(async () => {
      document.documentElement.classList.remove("dark");
      await Promise.resolve();
    });
    expect(
      screen.getByTestId("editor").classList.contains("mdxeditor-dark"),
    ).toBe(false);
  });
});
