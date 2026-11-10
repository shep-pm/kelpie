import {
  MDXEditor,
  type MDXEditorMethods,
  type MDXEditorProps,
  type RealmPlugin,
} from "@mdxeditor/editor";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { defaultFieldPlugins } from "./default-plugins";
import { safeHtmlImagePlugin } from "./safe-html-image-plugin";
import { useDark } from "./use-dark";
import "@mdxeditor/editor/style.css";
import "./mdx-editor-dark.css";

interface MdxFieldProps
  extends Omit<MDXEditorProps, "markdown" | "readOnly" | "ref"> {
  /** The markdown to show, `null` or `undefined` where there is none. */
  value: string | null | undefined;
  /** What to show where there is no value: text is read as markdown, anything else stands in for the editor. */
  empty?: ReactNode;
  /**
   * The editor plugins. Defaults to `defaultFieldPlugins`. The field adds an
   * inert `<img>` visitor to whatever list it gets, so `imagePlugin()` cannot
   * run the `onerror` of raw `<img>` HTML.
   */
  plugins?: RealmPlugin[];
  /**
   * Keeps raw HTML in the markdown as text. Defaults to `true`, since a field
   * shows text other people wrote. Pass `false` to render stored HTML live:
   * an `<iframe src="javascript:...">` then runs script as the field mounts, and
   * a `javascript:` link runs on click.
   */
  suppressHtmlProcessing?: boolean;
}

/**
 * Renders a markdown value in a read-only MDXEditor.
 *
 * Lifted from chelone's `MdxField`, which read its value from a record: here it
 * comes in as `value`, and `empty` is not translated.
 *
 * @example
 * <MdxField value={body} empty="No instructions" />
 */
function MdxField({
  value,
  empty,
  className,
  plugins = defaultFieldPlugins,
  suppressHtmlProcessing = true,
  onError,
  ...rest
}: MdxFieldProps) {
  const dark = useDark();
  const editorRef = useRef<MDXEditorMethods>(null);
  const safePlugins = useMemo(
    () => [...plugins, safeHtmlImagePlugin],
    [plugins],
  );
  // The markdown the editor failed to parse. A different value gets a new try.
  const [failedMarkdown, setFailedMarkdown] = useState<string | null>(null);

  const emptyIsText = typeof empty === "string";
  const markdown = value ?? (emptyIsText ? empty : "");

  // The editor takes `markdown` only on mount, so later values are pushed in.
  useEffect(() => {
    const editor = editorRef.current;
    if (editor && editor.getMarkdown() !== markdown) {
      editor.setMarkdown(markdown);
    }
  }, [markdown]);

  if (value == null && empty && !emptyIsText) return <>{empty}</>;

  // On a parse error MDXEditor renders nothing, so show the value as text.
  if (failedMarkdown === markdown) {
    return (
      <pre
        data-slot="mdx-field-fallback"
        className={cn("whitespace-pre-wrap break-words text-sm", className)}
      >
        {markdown}
      </pre>
    );
  }

  return (
    <MDXEditor
      {...rest}
      ref={editorRef}
      markdown={markdown}
      readOnly
      plugins={safePlugins}
      suppressHtmlProcessing={suppressHtmlProcessing}
      onError={(payload) => {
        setFailedMarkdown(markdown);
        onError?.(payload);
      }}
      className={cn(dark && "mdxeditor-dark", className)}
    />
  );
}

export { MdxField, type MdxFieldProps };
