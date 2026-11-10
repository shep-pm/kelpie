import type { Value } from "@/bindings";

type Json = { [key: string]: Value };

const isObject = (value: Value | undefined): value is Json =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/** The labels people read, since a key such as `pm-model` is a file's word. */
const LABELS: Record<string, string> = {
  "pm-harness": "PM harness",
  "pm-model": "PM model",
  "pm-effort": "PM effort",
  provider: "Provider",
  host: "Host",
  "reply-style": "Reply style",
  "read-anywhere": "Read anywhere by default",
  "harness-global-folder": "Read the harness's global folder",
  "network-allowlist": "Allowed domains",
};

/** Fields the form leaves out: each bridge's update row, on the harness's login row, is a slice of its own. */
const HIDDEN = new Set(["$schema", "bridge-updates"]);

/** The order a person meets the fields in; a key it does not name follows. */
const ORDER = Object.keys(LABELS);

function label(key: string): string {
  const named = LABELS[key];
  if (named) return named;
  const words = key.replaceAll("-", " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** The schema's own `$defs`, which a `$ref` names. */
function resolve(node: Json, defs: Json): Json {
  const ref = node.$ref;
  if (typeof ref !== "string") return node;
  const found = defs[ref.replace("#/$defs/", "")];
  return isObject(found) ? { ...found, ...omit(node, "$ref") } : node;
}

function omit(node: Json, key: string): Json {
  const { [key]: _dropped, ...rest } = node;
  return rest;
}

/** A property as a form field: the nullable wrapper is gone, because null is how a layer holds no value. */
function field(key: string, raw: Value, defs: Json): Json {
  let node = isObject(raw) ? raw : {};
  const description = node.description;
  // `Option<Enum>` is `anyOf [{$ref}, {type: null}]`
  if (Array.isArray(node.anyOf)) {
    const inner = node.anyOf.find((v) => isObject(v) && v.type !== "null");
    if (isObject(inner))
      node = { ...omit(node, "anyOf"), ...resolve(inner, defs) };
  }
  node = resolve(node, defs);
  // `Option<String>` is `type: [string, null]`
  if (Array.isArray(node.type)) {
    node = { ...node, type: node.type.find((t) => t !== "null") ?? "string" };
  }
  // A const per variant is what rjsf draws as a select, labelled by its title
  if (Array.isArray(node.oneOf)) {
    node = {
      ...node,
      oneOf: node.oneOf.map((variant) =>
        isObject(variant)
          ? {
              const: variant.const ?? null,
              title: String(variant.description ?? variant.const),
            }
          : variant,
      ),
    };
  }
  const { default: _default, ...rest } = node;
  return {
    ...rest,
    title: label(key),
    ...(typeof description === "string" ? { description } : {}),
  };
}

/**
 * The schema the form is drawn from: the settings schema with only `keys`, each
 * as one plain field rjsf can draw.
 *
 * `$schema` is a file's own key and never a field.
 */
export function formSchema(raw: Value, keys: string[]): Json {
  const root = isObject(raw) ? raw : {};
  const defs = isObject(root.$defs) ? root.$defs : {};
  const all = isObject(root.properties) ? root.properties : {};
  const properties: Json = {};
  for (const key of keys) {
    if (HIDDEN.has(key) || !(key in all)) continue;
    properties[key] = field(key, all[key] as Value, defs);
  }
  return { type: "object", properties };
}

/** The keys in the order a person meets them. */
export function ordered(keys: string[]): string[] {
  const rank = (key: string) => {
    const at = ORDER.indexOf(key);
    return at < 0 ? ORDER.length : at;
  };
  return [...keys].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
}
