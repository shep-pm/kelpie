import Form from "@rjsf/shadcn";
import type { RJSFSchema, UiSchema } from "@rjsf/utils";
import validator from "@rjsf/validator-ajv8";
import { ArrowLeft } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AgentsTab } from "@/agents/AgentsTab";
import type { FieldView, Target, Value } from "@/bindings";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BridgeField } from "./BridgeField";
import { GithubField } from "./GithubField";
import { HARNESSES } from "./harnesses";
import { LayeredField, type SettingsContext, type Tab } from "./LayeredField";
import { LoginField } from "./LoginField";
import { formSchema, ordered } from "./schema";
import { useSettings } from "./store";
import { EffortWidget, EnumWidget, ModelWidget, SwitchWidget } from "./widgets";

type Data = Record<string, Value | undefined>;

const widgets = {
  SelectWidget: EnumWidget,
  CheckboxWidget: SwitchWidget,
  ModelWidget,
  EffortWidget,
};

const templates = { FieldTemplate: LayeredField };

/** What the form holds: a field no layer sets is absent, not `null`. */
function toData(fields: FieldView[]): Data {
  const data: Data = {};
  for (const field of fields) {
    if (field.value !== null) data[field.key] = field.value;
  }
  return data;
}

/** What a file holds for an empty form field: `null`, which clears it. */
function toValue(raw: Value | undefined): Value {
  if (raw === undefined || raw === null) return null;
  if (typeof raw === "string") return raw.trim() === "" ? null : raw;
  if (Array.isArray(raw)) {
    const kept = raw.filter((v) => typeof v !== "string" || v.trim() !== "");
    return kept.length === 0 ? null : kept;
  }
  return raw;
}

const same = (a: Value, b: Value) => JSON.stringify(a) === JSON.stringify(b);

type Kind = "text" | "list" | "choice";

/**
 * How a field is written. A `text` field changes with every key, so it is
 * written when the person leaves it; a `list` is written when an item is added
 * or removed, and when one is left; a `choice` is written as it is chosen.
 */
function kindOf(schema: RJSFSchema, key: string): Kind {
  const field = schema.properties?.[key];
  if (typeof field !== "object") return "choice";
  if (field.type === "array") return "list";
  return field.type === "string" && field.oneOf === undefined
    ? "text"
    : "choice";
}

/** The key of the top-level field a rjsf id such as `root_pm-model_0` is in. */
const keyOf = (id: string) => id.replace(/^root_/, "").split("_")[0] ?? "";

function TabForm({
  tab,
  id,
  fields,
  below,
}: {
  tab: Tab;
  /** The project's id, on the Project tab. */
  id: string | null;
  fields: FieldView[];
  below: FieldView[];
}) {
  const schema = useSettings((s) => s.schema);
  const offered = useSettings((s) => s.offered);
  const [data, setData] = useState<Data>(() => toData(fields));
  const latest = useRef(data);
  latest.current = data;
  useEffect(() => setData(toData(fields)), [fields]);

  const target: Target = useMemo(
    () => (id === null ? { kind: "global" } : { kind: "project", id }),
    [id],
  );
  const keys = useMemo(() => ordered(fields.map((f) => f.key)), [fields]);
  const jsonSchema = useMemo(
    () => (schema === null ? null : (formSchema(schema, keys) as RJSFSchema)),
    [schema, keys],
  );

  const byKey = useMemo(
    () => Object.fromEntries(fields.map((f) => [f.key, f])),
    [fields],
  );
  const harness = useMemo(() => {
    const value = byKey["pm-harness"]?.value;
    return typeof value === "string" ? value : null;
  }, [byKey]);

  useEffect(() => {
    if (harness !== null)
      void useSettings.getState().loadOffered(harness, false);
  }, [harness]);

  const commit = useCallback(
    (key: string, raw: Value | undefined) => {
      const value = toValue(raw);
      if (same(value, byKey[key]?.value ?? null)) return;
      void useSettings.getState().set(target, key, value);
    },
    [byKey, target],
  );

  const context: SettingsContext = useMemo(
    () => ({
      tab,
      fields: byKey,
      below: Object.fromEntries(below.map((f) => [f.key, f])),
      harness,
      offered: harness === null ? null : (offered[harness] ?? null),
      refresh: () => {
        if (harness !== null)
          void useSettings.getState().loadOffered(harness, true);
      },
      onReset: (key) => void useSettings.getState().unset(target, key),
    }),
    [tab, byKey, below, harness, offered, target],
  );

  const uiSchema: UiSchema = useMemo(
    () => ({
      "ui:submitButtonOptions": { norender: true },
      "ui:order": [...keys, "*"],
      "pm-model": { "ui:widget": "ModelWidget" },
      "pm-effort": { "ui:widget": "EffortWidget" },
    }),
    [keys],
  );

  if (jsonSchema === null) return <Skeleton className="h-24 w-full" />;

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
      onChange={(event, changed) => {
        const next = event.formData as Data;
        const before = latest.current;
        setData(next);
        const key = keyOf(changed ?? "");
        if (key === "") return;
        const kind = kindOf(jsonSchema, key);
        const items = (v: Value | undefined) =>
          Array.isArray(v) ? v.length : 0;
        if (
          kind === "choice" ||
          (kind === "list" && items(next[key]) !== items(before[key]))
        ) {
          commit(key, next[key]);
        }
      }}
      onBlur={(changed, value) => {
        const key = keyOf(changed);
        const kind = kindOf(jsonSchema, key);
        if (kind === "text" && changed === `root_${key}`) commit(key, value);
        else if (kind === "list") commit(key, latest.current[key]);
      }}
    />
  );
}

function Fields({ tab }: { tab: Tab }) {
  const view = useSettings((s) => s.view);
  if (view === null) return <Skeleton className="h-24 w-full" />;
  if (tab === "global") {
    return (
      <>
        <TabForm tab="global" id={null} fields={view.global} below={[]} />
        <LoginFor fields={view.global} />
        <GithubField />
        <BridgeFor fields={view.global} />
      </>
    );
  }
  if (view.project === null) {
    return (
      <p className="py-6 text-sm text-muted-foreground">
        Pick a project to set what is its own.
      </p>
    );
  }
  return (
    <TabForm
      key={view.project.id}
      tab="project"
      id={view.project.id}
      fields={view.project.fields}
      below={view.global}
    />
  );
}

/** The login of the PM's harness, on the Global tab: a login is not a project's. */
function LoginFor({ fields }: { fields: FieldView[] }) {
  const harness = fields.find((f) => f.key === "pm-harness")?.value;
  if (typeof harness !== "string") return null;
  return <LoginField harness={harness} />;
}

/** The bridge of the PM's harness, beside its login: the version in use and how new ones arrive. */
function BridgeFor({ fields }: { fields: FieldView[] }) {
  const harness = fields.find((f) => f.key === "pm-harness")?.value;
  if (typeof harness !== "string") return null;
  const info = HARNESSES[harness];
  if (info?.bridge === undefined) return null;
  return (
    <BridgeField
      bridge={info.bridge}
      harness={info.name}
      field={fields.find((f) => f.key === "bridge-updates")}
    />
  );
}

/** The settings of kelpie and of the project, one form for each, drawn from the settings schema. */
export function SettingsPage({ onClose }: { onClose: () => void }) {
  const error = useSettings((s) => s.error);

  useEffect(() => {
    void useSettings.getState().load();
  }, []);

  return (
    <section aria-label="Settings" className="flex min-h-0 flex-1 flex-col">
      <header className="flex items-center gap-2 border-b px-4 py-2">
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label="Back to the chat"
          onClick={onClose}
        >
          <ArrowLeft />
        </Button>
        <h1 className="text-xl font-semibold">Settings</h1>
      </header>
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex max-w-3xl flex-col gap-3 px-4 py-4">
          {error && (
            <p role="alert" className="text-sm text-status-failed-fg">
              {error}
            </p>
          )}
          <Tabs defaultValue="global">
            <TabsList>
              <TabsTrigger value="global">Global</TabsTrigger>
              <TabsTrigger value="project">Project</TabsTrigger>
              <TabsTrigger value="agents">Agents</TabsTrigger>
            </TabsList>
            <TabsContent value="global">
              <Fields tab="global" />
            </TabsContent>
            <TabsContent value="project">
              <Fields tab="project" />
            </TabsContent>
            <TabsContent value="agents">
              <AgentsTab />
            </TabsContent>
          </Tabs>
        </div>
      </ScrollArea>
    </section>
  );
}
