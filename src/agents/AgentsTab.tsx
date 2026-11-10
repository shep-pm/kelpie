import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { AgentEditor } from "./AgentEditor";
import { useAgents } from "./store";

/** What a new agent builds on unless the maintainer names another. */
const DEFAULT_PARENT = "kelpie:pm";

function NewAgent({ onMade }: { onMade: (name: string) => void }) {
  const [name, setName] = useState("");
  const [parent, setParent] = useState(DEFAULT_PARENT);
  return (
    <form
      className="flex flex-wrap items-end gap-2"
      onSubmit={(event) => {
        event.preventDefault();
        const made = name.trim();
        void useAgents
          .getState()
          .create(made, parent.trim() === "" ? null : parent.trim())
          .then((done) => {
            if (done) onMade(made);
          });
      }}
    >
      <div className="flex flex-col gap-1">
        <label htmlFor="new-agent-name" className="text-label font-medium">
          Name
        </label>
        <Input
          id="new-agent-name"
          value={name}
          onChange={(event) => setName(event.target.value)}
          className="w-48 font-mono"
        />
      </div>
      <div className="flex flex-col gap-1">
        <label htmlFor="new-agent-parent" className="text-label font-medium">
          Extends
        </label>
        <Input
          id="new-agent-parent"
          value={parent}
          onChange={(event) => setParent(event.target.value)}
          className="w-48 font-mono"
        />
      </div>
      <Button type="submit" variant="outline">
        New agent
      </Button>
    </form>
  );
}

/** The agents in kelpie's config folder, to open or add to. */
function AgentList({ onOpen }: { onOpen: (name: string) => void }) {
  const agents = useAgents((s) => s.agents);
  const error = useAgents((s) => s.error);

  useEffect(() => {
    void useAgents.getState().loadList();
  }, []);

  return (
    <div className="flex flex-col gap-4 py-3">
      {error && (
        <p role="alert" className="text-sm text-status-failed-fg">
          {error}
        </p>
      )}
      {agents === null ? (
        <Skeleton className="h-24 w-full" />
      ) : (
        <ul className="flex flex-col divide-y rounded-md border">
          {agents.map((agent) => (
            <li key={agent.name}>
              <button
                type="button"
                className="flex w-full flex-col items-start gap-0.5 px-3 py-2 text-left hover:bg-accent"
                onClick={() => onOpen(agent.name)}
              >
                <span className="font-mono text-sm">{agent.name}</span>
                {agent.problem !== null ? (
                  <span className="text-xs text-status-failed-fg">
                    {agent.problem}
                  </span>
                ) : (
                  !agent.exists && (
                    <span className="text-xs text-muted-foreground">
                      builds on kelpie:pm until saved
                    </span>
                  )
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
      <NewAgent onMade={onOpen} />
    </div>
  );
}

/** The settings page's Agents tab: the list, and one agent open in the editor. */
export function AgentsTab() {
  const [open, setOpen] = useState<string | null>(null);
  if (open !== null) {
    return (
      <div className="py-3">
        <AgentEditor name={open} onClose={() => setOpen(null)} />
      </div>
    );
  }
  return <AgentList onOpen={setOpen} />;
}
