import { Settings } from "lucide-react";
import { useState } from "react";
import { ChatBox } from "./chat/ChatBox";
import { Button } from "./components/ui/button";
import { SettingsPage } from "./settings/SettingsPage";

/** The PM's header, with the gear that opens the settings. */
function Header({ onSettings }: { onSettings: () => void }) {
  return (
    <header className="flex items-center justify-between border-b px-4 py-2">
      <h1 className="text-lg font-semibold">PM</h1>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label="Settings"
        onClick={onSettings}
      >
        <Settings />
      </Button>
    </header>
  );
}

export function App() {
  const [settings, setSettings] = useState(false);
  return (
    <div className="flex h-screen flex-col">
      {/* The chat stays mounted behind the page, so a message half-written is still there */}
      <div className={settings ? "hidden" : "flex min-h-0 flex-1 flex-col"}>
        <Header onSettings={() => setSettings(true)} />
        <ChatBox />
      </div>
      {settings && <SettingsPage onClose={() => setSettings(false)} />}
    </div>
  );
}
