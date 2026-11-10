import { generateTemplates } from "@rjsf/shadcn";
import type { FieldTemplateProps } from "@rjsf/utils";
import { RotateCcw } from "lucide-react";
import type { ComponentType } from "react";
import type { FieldView } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { Offered } from "./store";

/** The settings tabs, and an agent file, whose layers are its `extends` chain. */
export type Tab = "global" | "project" | "agent";

/** What the fields of one tab share, handed to rjsf as its `formContext`. */
export type SettingsContext = {
  tab: Tab;
  /** The tab's fields by key, each with the layer it came from. */
  fields: Record<string, FieldView>;
  /** The Global tab's fields, which a Project field falls back to. Empty on the Global tab. */
  below: Record<string, FieldView>;
  /** The PM's harness on this tab, `null` where none is known. */
  harness: string | null;
  /** What the harness offers, `null` before anything asked. */
  offered: Offered | null;
  /** Ask the harness for its models and efforts again. */
  refresh: () => void;
  /** Clear one field of this tab's layer. */
  onReset: (key: string) => void;
  /**
   * An agent's layers, lowest first: `kelpie`, the parents it builds on, then
   * the file itself. Only the agent tab has them; the settings tabs' are fixed.
   */
  layers?: string[];
};

const LAYERS: Record<Tab, string[]> = {
  global: ["kelpie", "global"],
  project: ["kelpie", "global", "project"],
  agent: [],
};

/** The layer a tab's reset falls back to; an agent falls back to its nearest parent. */
function below(tab: Tab, layers: string[]): string {
  if (tab === "agent") return layers[layers.length - 2] ?? "kelpie";
  return tab === "project" ? "global" : "kelpie";
}

/** How far up `layers` a `from` is. A layer not in them is the bottom one. */
function depth(from: string, layers: string[]): number {
  return Math.max(0, layers.indexOf(from));
}

function chip(from: string, tab: Tab, own: boolean): string {
  if (own && tab === "project") return "set in this project";
  if (own && tab === "agent") return "set in this file";
  return `from ${from}`;
}

/** A value short enough to sit in a button, `null` where it would not be. */
function inline(value: FieldView["value"] | undefined): string | null {
  if (typeof value === "string" && value !== "") return value;
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  return null;
}

const Plain = generateTemplates()
  .FieldTemplate as ComponentType<FieldTemplateProps>;

/** The Global tab's own layer is `global`, and a Project's is `project`. */
const OWN: Record<Exclude<Tab, "agent">, string> = {
  global: "global",
  project: "project",
};

/**
 * A setting's row: its label and help, the control, and under it the layer the
 * value comes from with a reset to the layer below.
 *
 * It draws the fields of the form's top level; what is inside one (an array's
 * items) is rjsf's own.
 */
export function LayeredField(props: FieldTemplateProps) {
  const { id, label, rawDescription, children, errors, registry } = props;
  const context = registry.formContext as SettingsContext;
  const key = id.startsWith("root_") ? id.slice("root_".length) : null;
  const view = key === null ? undefined : context.fields[key];
  if (key === null || view === undefined) return <Plain {...props} />;

  const layers = context.layers ?? LAYERS[context.tab];
  const reached = depth(view.from, layers);
  // An agent file's own layer is the last one: its name
  const own =
    context.tab === "agent"
      ? view.from === layers[layers.length - 1]
      : view.from === OWN[context.tab];
  const inherited =
    context.tab === "project" ? inline(context.below[key]?.value) : null;

  return (
    <fieldset
      aria-labelledby={`${id}-label`}
      className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)] gap-x-6 gap-y-1 border-b py-3"
    >
      <div className="flex flex-col gap-0.5">
        <label
          id={`${id}-label`}
          htmlFor={id}
          className="text-label font-medium"
        >
          {label}
        </label>
        {rawDescription && (
          <p className="text-xs text-muted-foreground">{rawDescription}</p>
        )}
      </div>
      <div className="flex min-w-0 flex-col gap-1">
        {children}
        <div className="flex flex-wrap items-center gap-2 px-0.5">
          <span
            className="inline-flex gap-0.5"
            role="img"
            aria-label={`layer ${reached + 1} of ${layers.length}`}
            title={layers.join(" · ")}
          >
            {layers.map((layer, step) => (
              <span
                key={layer}
                className={cn(
                  "h-1 w-3.5 rounded-sm",
                  step <= reached ? "bg-foreground" : "bg-border-strong/40",
                )}
              />
            ))}
          </span>
          <Badge variant={own ? "secondary" : "outline"}>
            {chip(view.from, context.tab, own)}
          </Badge>
          {own && (
            <Button
              type="button"
              variant="link"
              size="xs"
              onClick={() => context.onReset(key)}
            >
              <RotateCcw />
              Reset to {below(context.tab, layers)}
              {inherited !== null && (
                <span className="font-mono text-xs">({inherited})</span>
              )}
            </Button>
          )}
        </div>
        {errors}
      </div>
    </fieldset>
  );
}
