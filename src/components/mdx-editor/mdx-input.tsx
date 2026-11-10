import {
  MDXEditor,
  type MDXEditorMethods,
  type MDXEditorProps,
} from "@mdxeditor/editor";
import {
  memo,
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { defaultInputPlugins } from "./default-plugins";
import { safeHtmlImagePlugin } from "./safe-html-image-plugin";
import { shallowEqual } from "./shallow-equal";
import { useDark } from "./use-dark";
import "@mdxeditor/editor/style.css";
import "./mdx-editor-dark.css";

type MdxEditorPassthrough = Omit<
  MDXEditorProps,
  | "markdown"
  | "readOnly"
  | "spellCheck"
  | "onChange"
  | "onBlur"
  | "onError"
  | "ref"
>;

interface MdxInputProps {
  /** The markdown. The editor reads it on mount, and again when it differs from what the editor holds. */
  value: string;
  /** Called with the markdown after each edit the person makes. */
  onChange: (markdown: string) => void;
  /** Called when the editor loses focus, with the DOM event it sent. */
  onBlur?: (event: FocusEvent) => void;
  /** The label shown above the editor. */
  label?: ReactNode;
  /** Help under the editor. */
  helperText?: ReactNode;
  /** Why the value is refused, under the editor. */
  error?: ReactNode;
  id?: string;
  className?: string;
  disabled?: boolean;
  readOnly?: boolean;
  /**
   * Extra `MDXEditor` props. `plugins` here replaces `defaultInputPlugins`;
   * an inert `<img>` visitor is added to either list.
   * `suppressHtmlProcessing` defaults to `true`, so raw HTML stays text; set it
   * to `false` to render stored HTML live, where an
   * `<iframe src="javascript:...">` runs script as the editor mounts.
   */
  mdxProps?: MdxEditorPassthrough;
}

interface EditorProps {
  editorRef: RefObject<MDXEditorMethods | null>;
  initialMarkdown: string;
  readOnly: boolean;
  className?: string;
  mdxProps?: MdxEditorPassthrough;
  onChange: (markdown: string, initialMarkdownNormalize: boolean) => void;
  onBlur: (event: FocusEvent) => void;
  onError: () => void;
}

// The editor is uncontrolled, so a re-render with the same inputs is wasted
// work and a re-mount of its plugins is worse: skip it.
const Editor = memo(function Editor({
  editorRef,
  initialMarkdown,
  readOnly,
  className,
  mdxProps,
  onChange,
  onBlur,
  onError,
}: EditorProps) {
  // Added to the caller's list too, so `imagePlugin()` cannot run the
  // `onerror` of raw `<img>` HTML.
  const basePlugins = mdxProps?.plugins ?? defaultInputPlugins;
  const plugins = useMemo(
    () => [...basePlugins, safeHtmlImagePlugin],
    [basePlugins],
  );
  return (
    <MDXEditor
      {...mdxProps}
      plugins={plugins}
      // After the spread, so a key set to `undefined` keeps the safe default.
      suppressHtmlProcessing={mdxProps?.suppressHtmlProcessing ?? true}
      ref={editorRef}
      markdown={initialMarkdown}
      readOnly={readOnly}
      className={cn(className, mdxProps?.className)}
      onChange={onChange}
      onBlur={onBlur}
      onError={onError}
    />
  );
}, editorPropsEqual);

function editorPropsEqual(prev: EditorProps, next: EditorProps) {
  return (
    prev.readOnly === next.readOnly &&
    prev.onChange === next.onChange &&
    prev.onBlur === next.onBlur &&
    prev.onError === next.onError &&
    prev.initialMarkdown === next.initialMarkdown &&
    prev.className === next.className &&
    shallowEqual(prev.mdxProps, next.mdxProps)
  );
}

/**
 * Edits a markdown string in an MDXEditor with a formatting toolbar.
 *
 * Lifted from chelone's `MdxInput`, which read its value from a form: here the
 * value and its change come in as props. The value is always a markdown string;
 * MDX with JSX is not parsed.
 *
 * @example
 * <MdxInput label="Instructions" value={body} onChange={setBody} />
 */
function MdxInput({
  value,
  onChange,
  onBlur,
  label,
  helperText,
  error,
  id: idProp,
  className,
  disabled,
  readOnly,
  mdxProps,
}: MdxInputProps) {
  const generated = useId();
  const id = idProp ?? generated;
  const dark = useDark();
  const editorRef = useRef<MDXEditorMethods>(null);

  // The editor reads `markdown` on mount only; a parent re-render must not reset it.
  const [initialMarkdown] = useState(value);

  // The editor gets handlers that stay put, whatever the parent passes.
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  // On mount the editor re-serialises the starting markdown and reports it
  // with `initialMarkdownNormalize`; that is not an edit, so it stays out of the
  // value (no rewrite of stored markdown the person never touched).
  const handleChange = useCallback(
    (markdown: string, initialMarkdownNormalize: boolean) => {
      if (!initialMarkdownNormalize) onChangeRef.current(markdown);
    },
    [],
  );
  const onBlurRef = useRef(onBlur);
  onBlurRef.current = onBlur;
  const handleBlur = useCallback((event: FocusEvent) => {
    onBlurRef.current?.(event);
  }, []);

  // Markdown the editor cannot import (block HTML, once HTML processing is off)
  // is dropped from its content without a sign, and the next keystroke would
  // save that loss. The stored text goes in a plain textarea instead.
  const [parseFailed, setParseFailed] = useState(false);
  const handleError = useCallback(() => setParseFailed(true), []);

  // A value that changes from outside, such as a reset, is pushed into the editor.
  useEffect(() => {
    const editor = editorRef.current;
    if (editor && editor.getMarkdown() !== value) {
      editor.setMarkdown(value);
    }
  }, [value]);

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      {label !== undefined && <Label htmlFor={id}>{label}</Label>}
      <div id={id}>
        {parseFailed ? (
          <Textarea
            data-slot="mdx-input-fallback"
            value={value}
            disabled={disabled === true}
            readOnly={readOnly === true}
            aria-invalid={error ? true : undefined}
            onChange={(event) => onChange(event.target.value)}
            onBlur={(event) => onBlur?.(event.nativeEvent)}
          />
        ) : (
          <Editor
            editorRef={editorRef}
            initialMarkdown={initialMarkdown}
            readOnly={disabled === true || readOnly === true}
            className={dark ? "mdxeditor-dark" : undefined}
            mdxProps={mdxProps}
            onChange={handleChange}
            onBlur={handleBlur}
            onError={handleError}
          />
        )}
      </div>
      {helperText && (
        <p className="text-xs text-muted-foreground">{helperText}</p>
      )}
      {error && (
        <p role="alert" className="text-xs text-status-failed-fg">
          {error}
        </p>
      )}
    </div>
  );
}

export { MdxInput, type MdxInputProps };
