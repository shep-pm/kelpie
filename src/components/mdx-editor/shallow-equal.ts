/** Whether two objects hold the same values under the same keys, compared with `Object.is`. */
export function shallowEqual<T extends object>(
  a: T | undefined,
  b: T | undefined,
): boolean {
  if (a === b) return true;
  if (a === undefined || b === undefined) return false;
  const left = Object.keys(a) as (keyof T)[];
  const right = Object.keys(b);
  if (left.length !== right.length) return false;
  return left.every(
    (key) => Object.hasOwn(b, key) && Object.is(a[key], b[key]),
  );
}
