import { Maximize2, Minimize2, PanelRightClose } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";

/** What a panel's header asks of the shell. */
export type PanelControls = {
  maximised: boolean;
  onCollapse: () => void;
  /** Maximise the panel, or return it when it is maximised. */
  onMaximise: () => void;
};

type Props = PanelControls & {
  title: string;
  /** The panel's NeedsYouCount, once it has one. */
  count?: ReactNode;
  /** Buttons to the left of collapse and maximise. */
  actions?: ReactNode;
  children: ReactNode;
};

/** A docked panel: a header with its controls, and a body under it. */
export function Panel({
  title,
  count,
  actions,
  maximised,
  onCollapse,
  onMaximise,
  children,
}: Props) {
  return (
    <section
      aria-label={title}
      className="flex h-full min-h-0 min-w-0 flex-col bg-background"
    >
      <header className="flex h-10 shrink-0 items-center gap-2 border-b px-3">
        <h2 className="text-sm font-semibold">{title}</h2>
        {count}
        <span className="ml-auto flex items-center gap-0.5">
          {actions}
          {!maximised && (
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={`Collapse ${title}`}
              onClick={onCollapse}
            >
              <PanelRightClose />
            </Button>
          )}
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={`${maximised ? "Restore" : "Maximise"} ${title}`}
            onClick={onMaximise}
          >
            {maximised ? <Minimize2 /> : <Maximize2 />}
          </Button>
        </span>
      </header>
      <div className="flex min-h-0 flex-1 flex-col">{children}</div>
    </section>
  );
}

/** The icon strip a folded panel leaves: one button per panel, which reopens it. */
export function PanelStrip({ children }: { children: ReactNode }) {
  return (
    <nav
      aria-label="Panels"
      className="flex w-12 shrink-0 flex-col items-center gap-1 border-l bg-background py-2"
    >
      {children}
    </nav>
  );
}

export function PanelStripButton({
  title,
  icon,
  onOpen,
}: {
  title: string;
  icon: ReactNode;
  onOpen: () => void;
}) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      aria-label={`Open ${title}`}
      onClick={onOpen}
    >
      {icon}
    </Button>
  );
}
