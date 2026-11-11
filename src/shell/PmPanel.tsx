import { Settings, X } from "lucide-react";
import { useBoard } from "@/board/store";
import { ChatBox } from "@/chat/ChatBox";
import { NoSandboxBadge } from "@/chat/NoSandboxBadge";
import { NeedsYouCount, needsYouLabel } from "@/components/NeedsYouCount";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Panel, type PanelControls } from "./Panel";
import { PM_TAB, type Tab, TabBody, useTabs } from "./tabs";

/**
 * The PM panel: the PM's chat in its own tab, and a tab for each worker
 * opened beside it.
 */
export function PmPanel({
  onSettings,
  ...controls
}: PanelControls & { onSettings: () => void }) {
  const tabs = useTabs((s) => s.tabs);
  const selected = useTabs((s) => s.selected);
  const tickets = useBoard((s) => s.view?.tickets);

  // What waits on the maintainer inside one tab's worker, read from the board.
  function waits(tab: Tab) {
    const ticket = tab.ticket;
    if (ticket === undefined) return 0;
    return tickets?.find((t) => t.id === ticket)?.needs_you.length ?? 0;
  }

  return (
    <Panel
      title="PM"
      badge={<NoSandboxBadge />}
      {...controls}
      actions={
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label="Settings"
          onClick={onSettings}
        >
          <Settings />
        </Button>
      }
    >
      <Tabs
        value={selected}
        onValueChange={(id) => useTabs.getState().select(id)}
        className="min-h-0 flex-1 gap-0"
      >
        <TabsList variant="line" className="h-9 w-full justify-start border-b">
          {tabs.map((tab) => {
            const waiting = waits(tab);
            return (
              <div key={tab.id} className="relative flex items-center">
                <TabsTrigger
                  value={tab.id}
                  aria-label={
                    waiting > 0
                      ? `${tab.title}, ${needsYouLabel(waiting)}`
                      : undefined
                  }
                  className={tab.id === PM_TAB.id ? "" : "pr-7"}
                >
                  {tab.title}
                  <NeedsYouCount count={waiting} />
                </TabsTrigger>
                {tab.id !== PM_TAB.id && (
                  <button
                    type="button"
                    aria-label={`Close ${tab.title}`}
                    className="absolute right-1 rounded-sm p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                    onClick={() => useTabs.getState().close(tab.id)}
                  >
                    <X className="size-3" />
                  </button>
                )}
              </div>
            );
          })}
        </TabsList>
        {/* Mounted while another tab shows, so a half-written message stays. */}
        <TabsContent
          value={PM_TAB.id}
          forceMount
          className="flex min-h-0 flex-col data-[state=inactive]:hidden"
        >
          <ChatBox />
        </TabsContent>
        {tabs
          .filter((tab) => tab.id !== PM_TAB.id)
          .map((tab) => (
            <TabsContent
              key={tab.id}
              value={tab.id}
              className="flex min-h-0 flex-col"
            >
              <TabBody id={tab.id} />
            </TabsContent>
          ))}
      </Tabs>
    </Panel>
  );
}
