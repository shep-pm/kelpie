import Form from "@rjsf/shadcn";
import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import validator from "@rjsf/validator-ajv8";
import { ArrowLeft } from "lucide-react";
import { useEffect, useMemo } from "react";
import type { FieldView, Value } from "@/bindings";
import { MdxInput } from "@/components/mdx-editor/mdx-input";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { LayeredField, type SettingsContext } from "@/settings/LayeredField";
import { formSchema } from "@/settings/schema";
import { useSettings } from "@/settings/store";
import {
  EffortWidget,
  EnumWidget,
  ModelWidget,
  SwitchWidget,
} from "@/settings/widgets";
import { AgentPreview } from "./AgentPreview";
import { type FieldKey, useAgents } from "./store";

/** The form's fields, in the order a person meets them. */
const KEYS: FieldKey[] = ["extends", "role", "harness", "model", "effort"];

/** The settings key whose field an agent's key is drawn like. */
const LIKE: Partial<Record<FieldKey, string>> = {
  harness: "pm-harness",
  model: "pm-model",
  effort: "pm-effort",
};

const TEXT = (title: string, description: string) => ({
  type: "string",
  title,
  description,
});

const FIELDS: Record<FieldKey, { title: string; description: string }> = {
  extends: {
    title: "Extends",
    description:
      "The agent it builds on: another file's name, or kelpie:<name> for one of kelpie's own",
  },
  role: {
    title: "Role",
    description: "What the agent is for. An agent that extends none needs it",
  },
  harness: {
    title: "Harness",
    description: "The harness it runs on",
  },
  model: {
    title: "Model",
    description: "The model, named as the harness names it",
  },
  effort: {
    title: "Effort",
    description: "The effort, named as the harness names it",
  },
};

/** The form's schema: the settings' fields for what an agent shares with them, text for the rest. */
function agentSchema(raw: Value | null): RJSFSchema {
  const like =
    raw === null
      ? { properties: {} }
      : (formSchema(raw, Object.values(LIKE)) as {
          properties: Record<string, object>;
        });
  const properties: Record<string, object> = {};
  for (const key of KEYS) {
    const { title, description } = FIELDS[key];
    const drawn = like.properties[LIKE[key] ?? ""];
    properties[key] = drawn
      ? { ...drawn, title, description }
      : TEXT(title, description);
  }
  return { type: "object", properties } as RJSFSchema;
}

const widgets = {
  SelectWidget: EnumWidget,
  CheckboxWidget: SwitchWidget,
  ModelWidget,
  EffortWidget,
};
const templates = { FieldTemplate: LayeredField };
const uiSchema: UiSchema = {
  "ui:submitButtonOptions": { norender: true },
  "ui:order": [...KEYS, "*"],
  model: { "ui:widget": "ModelWidget" },
  effort: { "ui:widget": "EffortWidget" },
};

/**
 * Each form key with the layer it comes from. A key the file sets is the
 * file's own; the rest come from the parent the loader resolved them in.
 */
function viewsOf(
  name: string,
  own: Record<FieldKey, string | null>,
  resolved: FieldView[],
  parent: string | undefined,
): Record<FieldKey, FieldView> {
  const views = {} as Record<FieldKey, FieldView>;
  for (const key of KEYS) {
    const mine = own[key];
    if (mine !== null) {
      views[key] = { key, value: mine, from: name };
    } else if (key === "extends") {
      // The global `pm` builds on kelpie's own without saying so
      views[key] = { key, value: parent ?? null, from: "kelpie" };
    } else {
      views[key] = resolved.find((f) => f.key === key) ?? {
        key,
        value: null,
        from: "kelpie",
      };
    }
  }
  return views;
}

function Fields() {
  const name = useAgents((s) => s.open?.name ?? "");
  const front = useAgents((s) => s.draft?.frontmatter);
  const preview = useAgents((s) => s.preview);
  const schema = useSettings((s) => s.schema);
  const offered = useSettings((s) => s.offered);

  const views = useMemo(
    () =>
      viewsOf(
        name,
        {
          extends: front?.extends ?? null,
          role: front?.role ?? null,
          harness: front?.harness ?? null,
          model: front?.model ?? null,
          effort: front?.effort ?? null,
        },
        preview?.fields ?? [],
        preview?.chain[0],
      ),
    [name, front, preview],
  );

  const harness =
    typeof views.harness.value === "string" ? views.harness.value : null;
  useEffect(() => {
    if (harness !== null)
      void useSettings.getState().loadOffered(harness, false);
  }, [harness]);

  const context: SettingsContext = useMemo(
    () => ({
      tab: "agent",
      fields: views,
      below: {},
      harness,
      offered: harness === null ? null : (offered[harness] ?? null),
      refresh: () => {
        if (harness !== null)
          void useSettings.getState().loadOffered(harness, true);
      },
      onReset: (key) => useAgents.getState().setKey(key as FieldKey, null),
      // `kelpie`, then the parents from the farthest, then the file
      layers: ["kelpie", ...[...(preview?.chain ?? [])].reverse(), name],
    }),
    [views, harness, offered, preview, name],
  );

  const jsonSchema = useMemo(() => agentSchema(schema), [schema]);
  const data = useMemo(() => {
    const data: Record<string, Value> = {};
    for (const key of KEYS) {
      const value = views[key].value;
      if (value !== null) data[key] = value;
    }
    return data;
  }, [views]);

  return (
    <Form
      schema={jsonSchema}
      uiSchema={uiSchema}
      formData={data}
      formContext={context}
      validator={validator}
      templates={templates}
      widgets={widgets}
      noValidate
      onChange={(event) => {
        const next = event.formData as Record<string, Value | undefined>;
        for (const key of KEYS) {
          if (next[key] === data[key]) continue;
          const value = next[key];
          useAgents
            .getState()
            .setKey(key, typeof value === "string" ? value : null);
        }
      }}
    />
  );
}

function Body() {
  const body = useAgents((s) => s.draft?.body ?? "");
  const append = useAgents((s) => s.draft?.frontmatter.body === "append");
  const parented = useAgents((s) => (s.preview?.chain.length ?? 0) > 0);
  return (
    <section aria-label="Instructions" className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-label font-medium">Instructions</h2>
        <div className="flex items-center gap-2 text-sm">
          <Switch
            id="agent-append"
            checked={append}
            disabled={!parented}
            onCheckedChange={(on) => useAgents.getState().setAppend(on)}
          />
          <label htmlFor="agent-append">Add to the parent's instructions</label>
        </div>
      </div>
      <MdxInput
        label="Instructions"
        value={body}
        onChange={(next) => useAgents.getState().setBody(next)}
      />
    </section>
  );
}

function SaveBar() {
  const saved = useAgents((s) => s.saved);
  const error = useAgents((s) => s.error);
  return (
    <div className="flex flex-col gap-2">
      {error && (
        <p role="alert" className="text-sm text-status-failed-fg">
          {error}
        </p>
      )}
      <div className="flex items-center gap-2">
        <Button type="button" onClick={() => void useAgents.getState().save()}>
          Save
        </Button>
        {saved && (
          <span role="status" className="text-sm text-muted-foreground">
            Saved
          </span>
        )}
      </div>
    </div>
  );
}

/**
 * One global agent: its frontmatter as layered fields, where `extends` is one
 * more layer, its instructions in the markdown editor, and beside them the
 * agent as a session would get it.
 */
export function AgentEditor({
  name,
  onClose,
}: {
  name: string;
  onClose: () => void;
}) {
  const ready = useAgents((s) => s.open?.name === name);
  const problem = useAgents((s) => s.open?.problem ?? null);

  useEffect(() => {
    void useSettings.getState().load();
    void useAgents.getState().openAgent(name);
    return () => useAgents.getState().close();
  }, [name]);

  return (
    <section aria-label={`Agent ${name}`} className="flex flex-col gap-3">
      <header className="flex items-center gap-2">
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label="Back to the agents"
          onClick={onClose}
        >
          <ArrowLeft />
        </Button>
        <h2 className="font-mono text-lg font-semibold">{name}</h2>
      </header>
      {!ready ? (
        <Skeleton className="h-24 w-full" />
      ) : (
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
          <div className="flex min-w-0 flex-col gap-3">
            {problem !== null && (
              <p role="status" className="text-sm text-status-failed-fg">
                {problem}
              </p>
            )}
            <Fields />
            <Body />
            <SaveBar />
          </div>
          <AgentPreview />
        </div>
      )}
    </section>
  );
}
