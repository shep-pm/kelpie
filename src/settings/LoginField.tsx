import { useCallback, useEffect, useState } from "react";
import { commands } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { HARNESSES } from "./harnesses";

/**
 * A harness's login: whether one is kept, a box to paste a new one into, and
 * Clear. The value goes to the keychain and never comes back, so the page can
 * only say *set* or *not set*.
 */
export function LoginField({ harness }: { harness: string }) {
  const info = HARNESSES[harness];
  const login = info?.login;
  // `null` until the keychain has answered
  const [kept, setKept] = useState<boolean | null>(null);
  const [pasted, setPasted] = useState("");
  const [error, setError] = useState<string | null>(null);
  const key = login?.key;

  const read = useCallback(async () => {
    if (key === undefined) return;
    const status = await commands.loginStatus(key);
    if (status.status === "error") setError(status.error);
    else setKept(status.data);
  }, [key]);

  useEffect(() => {
    setKept(null);
    setError(null);
    void read();
  }, [read]);

  if (key === undefined || login === undefined) return null;

  async function save() {
    if (key === undefined) return;
    const done = await commands.loginSet(key, pasted);
    // The box is emptied either way: a pasted token is not kept on the page
    setPasted("");
    setError(done.status === "error" ? done.error : null);
    await read();
  }

  async function clear() {
    if (key === undefined) return;
    const done = await commands.loginClear(key);
    setError(done.status === "error" ? done.error : null);
    await read();
  }

  return (
    <fieldset
      aria-labelledby="login-label"
      className="grid grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)] gap-x-6 gap-y-1 border-b py-3"
    >
      <div className="flex flex-col gap-0.5">
        <span id="login-label" className="text-label font-medium">
          Login
        </span>
        <p className="text-xs text-muted-foreground">
          {info?.name}'s token, kept in the OS keychain. Run{" "}
          <code className="font-mono">{login.command}</code> to make one.
        </p>
      </div>
      <div className="flex min-w-0 flex-col gap-2">
        <div className="flex items-center gap-2 px-0.5">
          {kept !== null && (
            <Badge variant={kept ? "secondary" : "outline"}>
              {kept ? "set" : "not set"}
            </Badge>
          )}
          {kept === true && (
            <Button
              type="button"
              variant="link"
              size="xs"
              onClick={() => void clear()}
            >
              Clear
            </Button>
          )}
        </div>
        <div className="flex items-center gap-2">
          <Input
            type="password"
            autoComplete="off"
            aria-label="Paste a token"
            placeholder="Paste a token"
            value={pasted}
            onChange={(e) => setPasted(e.target.value)}
          />
          <Button
            type="button"
            variant="outline"
            disabled={pasted.trim() === ""}
            onClick={() => void save()}
          >
            Save
          </Button>
        </div>
        {error && (
          <p role="alert" className="px-0.5 text-xs text-status-failed-fg">
            {error}
          </p>
        )}
      </div>
    </fieldset>
  );
}
