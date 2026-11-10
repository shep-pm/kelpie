import type { WidgetProps } from "@rjsf/utils";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import type { SettingsContext } from "./LayeredField";

type Pick = { value: string; label: string };

/** A select over `picks`, with `value` shown even where the list does not hold it. */
function Picker({
  id,
  value,
  picks,
  placeholder,
  disabled,
  onChange,
}: {
  id: string;
  value: string | undefined;
  picks: Pick[];
  placeholder?: string;
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  const shown =
    value !== undefined && !picks.some((p) => p.value === value)
      ? [{ value, label: value }, ...picks]
      : picks;
  return (
    <Select value={value ?? ""} onValueChange={onChange} disabled={disabled}>
      <SelectTrigger id={id} className="w-full">
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {shown.map((pick) => (
          <SelectItem key={pick.value} value={pick.value}>
            {pick.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/** Every field that is one of a few named values, such as the harness. */
export function EnumWidget({
  id,
  value,
  options,
  onChange,
  disabled,
  readonly,
}: WidgetProps) {
  const picks = (options.enumOptions ?? []).map((o) => ({
    value: String(o.value),
    label: o.label,
  }));
  return (
    <Picker
      id={id}
      value={typeof value === "string" ? value : undefined}
      picks={picks}
      disabled={disabled || readonly}
      onChange={onChange}
    />
  );
}

/** A yes or no. */
export function SwitchWidget({
  id,
  value,
  onChange,
  disabled,
  readonly,
  label,
}: WidgetProps) {
  return (
    <div className="flex items-center gap-2 px-0.5 py-1">
      <Switch
        id={id}
        checked={value === true}
        onCheckedChange={onChange}
        disabled={disabled || readonly}
        aria-label={label}
      />
      <span className="text-sm">{value === true ? "On" : "Off"}</span>
    </div>
  );
}

/** The field's models or efforts from the harness, or a text field where it cannot say. */
function Offering({
  kind,
  props,
}: {
  kind: "models" | "efforts";
  props: WidgetProps;
}) {
  const { id, value, onChange, onBlur, disabled, readonly, label, registry } =
    props;
  const context = registry.formContext as SettingsContext;
  const offered = context.offered;
  const current = typeof value === "string" ? value : undefined;

  if (offered?.state === "failed") {
    return (
      <div className="flex flex-col gap-1">
        <Input
          id={id}
          value={current ?? ""}
          disabled={disabled}
          readOnly={readonly}
          aria-label={label}
          onChange={(e) =>
            onChange(e.target.value === "" ? undefined : e.target.value)
          }
          onBlur={(e) => onBlur(id, e.target.value)}
        />
        {kind === "models" && (
          <p role="status" className="px-0.5 text-xs text-muted-foreground">
            {offered.error}
          </p>
        )}
      </div>
    );
  }

  const list = offered?.state === "ready" ? offered.options[kind] : [];
  return (
    <Picker
      id={id}
      value={current}
      picks={list.map((c) => ({ value: c.id, label: c.name }))}
      placeholder="Harness default"
      disabled={disabled || readonly}
      onChange={(chosen) => {
        onChange(chosen);
        // The form writes a text field when it is left, and a chosen one is done
        onBlur(id, chosen);
      }}
    />
  );
}

/** The PM's model, from what its harness lists, with a button to ask the harness again. */
export function ModelWidget(props: WidgetProps) {
  const context = props.registry.formContext as SettingsContext;
  return (
    <div className="flex items-center gap-1">
      <div className="min-w-0 flex-1">
        <Offering kind="models" props={props} />
      </div>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label="Refresh models and efforts"
        title="Ask the harness for its models and efforts again"
        onClick={context.refresh}
      >
        <RefreshCw />
      </Button>
    </div>
  );
}

/** The PM's effort, from what its harness lists. */
export function EffortWidget(props: WidgetProps) {
  return <Offering kind="efforts" props={props} />;
}
