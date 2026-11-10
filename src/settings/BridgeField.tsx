import { useCallback, useEffect, useState } from "react";
import {
  type BridgeStatus,
  commands,
  type FieldView,
  type Value,
} from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { BridgeInfo } from "./harnesses";
import { LAYERS, LayerTrace } from "./LayeredField";
import { useSettings } from "./store";

/** The three ways a bridge's new versions arrive. The values are a settings file's. */
const CHOICES = [
  { value: "registry-automatic", label: "Registry, automatic" },
  { value: "registry-ask-first", label: "Registry, ask first" },
  { value: "kelpies-pin", label: "Kelpie's pin" },
];

/** What a bridge takes when no layer names it. */
const DEFAULT_CHOICE = "registry-automatic";

/** The `bridge-updates` map a layer holds, `{}` where none does. */
function held(field: FieldView | undefined): Record<string, Value> {
  const value = field?.value;
  if (typeof value !== "object" || value === null || Array.isArray(value))
    return {};
  return { ...(value as Record<string, Value>) };
}

/** This bridge's setting: its own entry, or kelpie's default with its layer. */
function policyOf(field: FieldView | undefined, id: string) {
  const mine = held(field)[id];
  if (typeof mine === "string")
    return { value: mine, from: field?.from ?? "kelpie" };
  return { value: DEFAULT_CHOICE, from: "kelpie" };
}

/** Where the version in use came from, as the row words it. */
const SOURCE = { registry: "registry", "kelpies-pin": "kelpie's pin" };

/**
 * One bridge's row: the version in use and where it came from, how its new
 * versions arrive, and every version held back or marked bad with its reason
 * (`docs/design/bridge-updates.md`, The setting).
 *
 * The setting is the machine's, so the row is the Global tab's alone, beside the
 * login of the harness the bridge reaches. Nothing here is in the rail or the
 * needs-you count: a ready version blocks nothing.
 */
export function BridgeField({
  bridge,
  harness,
  field,
}: {
  bridge: BridgeInfo;
  harness: string;
  field: FieldView | undefined;
}) {
  // `null` until the engine has answered what the bridge runs on
  const [status, setStatus] = useState<BridgeStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const read = useCallback(async () => {
    const seen = await commands.bridgeStatus(bridge.id);
    if (seen.status === "error") setError(seen.error);
    else {
      setError(null);
      setStatus(seen.data);
    }
  }, [bridge.id]);

  useEffect(() => {
    setStatus(null);
    void read();
  }, [read]);

  const chosen = policyOf(field, bridge.id);
  const askFirst = chosen.value === "registry-ask-first";
  const ready = askFirst ? (status?.ready ?? null) : null;
  const approved = ready !== null && status?.approved === ready ? ready : null;

  /** Write this bridge's entry, keeping every other bridge's. */
  async function choose(value: string) {
    await useSettings.getState().set({ kind: "global" }, "bridge-updates", {
      ...held(field),
      [bridge.id]: value,
    });
  }

  /** Drop this bridge's entry, so its value falls back to kelpie's default. */
  async function reset() {
    const map = held(field);
    delete map[bridge.id];
    await useSettings
      .getState()
      .set(
        { kind: "global" },
        "bridge-updates",
        Object.keys(map).length === 0 ? null : map,
      );
  }

  async function update() {
    const done = await commands.bridgeApprove(bridge.id);
    if (done.status === "error") {
      setError(done.error);
      return;
    }
    await read();
  }

  const id = `bridge-${bridge.id}`;
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
          {bridge.name} bridge
        </label>
        <p className="text-xs text-muted-foreground">
          Kelpie installs and updates the bridge, never {harness} itself.
        </p>
      </div>
      <div className="flex min-w-0 flex-col gap-1">
        <Select
          value={chosen.value}
          onValueChange={(next) => void choose(next)}
        >
          <SelectTrigger id={id} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {CHOICES.map((choice) => (
              <SelectItem key={choice.value} value={choice.value}>
                {choice.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <LayerTrace
          from={chosen.from}
          tab="global"
          layers={LAYERS.global}
          own={chosen.from === "global"}
          onReset={() => void reset()}
        />
        {status && (
          <div className="flex flex-wrap items-center gap-2 px-0.5">
            <span className="text-xs text-muted-foreground">In use</span>
            <span className="font-mono text-xs">{status.in_use}</span>
            <Badge variant="outline">{SOURCE[status.source]}</Badge>
          </div>
        )}
        {ready && (
          <div className="flex flex-wrap items-center gap-2 px-0.5">
            <span className="font-mono text-xs">{ready}</span>
            {approved !== null ? (
              <span className="text-xs text-muted-foreground">
                approved, used at your next message
              </span>
            ) : (
              <>
                <span className="text-xs text-muted-foreground">
                  passed its probe
                </span>
                <Button
                  type="button"
                  variant="outline"
                  size="xs"
                  onClick={() => void update()}
                >
                  Update
                </Button>
              </>
            )}
          </div>
        )}
        {status?.held_back.map((rejected) => (
          <p
            key={rejected.version}
            className="px-0.5 text-xs text-muted-foreground"
          >
            <span className="font-mono">{rejected.version}</span> held back:{" "}
            {rejected.reason}
          </p>
        ))}
        {status?.bad.map((rejected) => (
          <p
            key={rejected.version}
            className="px-0.5 text-xs text-status-failed-fg"
          >
            <span className="font-mono">{rejected.version}</span> failed:{" "}
            {rejected.reason}
          </p>
        ))}
        {error && (
          <p role="alert" className="px-0.5 text-xs text-status-failed-fg">
            {error}
          </p>
        )}
      </div>
    </fieldset>
  );
}
