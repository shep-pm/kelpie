import type { FieldView } from "@/bindings";
import { MdxField } from "@/components/mdx-editor/mdx-field";
import { useAgents } from "./store";

const LABELS: Record<string, string> = {
  role: "Role",
  harness: "Harness",
  model: "Model",
  effort: "Effort",
};

function shown(value: FieldView["value"]): string {
  return typeof value === "string" && value !== "" ? value : "default";
}

/**
 * The agent as a session would get it, its `extends` chain resolved: each key
 * with the layer it comes from, and the instructions the parents and the file
 * make together.
 */
export function AgentPreview() {
  const name = useAgents((s) => s.open?.name);
  const preview = useAgents((s) => s.preview);
  const problem = useAgents((s) => s.previewError);
  const body = preview?.fields.find((f) => f.key === "body");

  return (
    <aside
      aria-label="Preview"
      className="flex min-w-0 flex-col gap-3 rounded-md border bg-card p-3"
    >
      <h2 className="text-label font-medium">As a session gets it</h2>
      {problem !== null && (
        <p role="status" className="text-sm text-status-failed-fg">
          {problem}
        </p>
      )}
      {preview !== null && (
        <>
          <p className="font-mono text-xs text-muted-foreground">
            {[...preview.chain]
              .reverse()
              .concat(name ?? [])
              .join(" > ")}
          </p>
          <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 text-sm">
            {preview.fields
              .filter((f) => f.key in LABELS)
              .map((f) => (
                <div key={f.key} className="contents">
                  <dt className="text-muted-foreground">{LABELS[f.key]}</dt>
                  <dd>
                    {shown(f.value)}{" "}
                    <span className="text-xs text-muted-foreground">
                      from {f.from}
                    </span>
                  </dd>
                </div>
              ))}
          </dl>
          <div className="min-w-0 border-t pt-3">
            <MdxField
              value={typeof body?.value === "string" ? body.value : ""}
              empty={
                <p className="text-sm text-muted-foreground">
                  No instructions.
                </p>
              }
            />
          </div>
        </>
      )}
    </aside>
  );
}
