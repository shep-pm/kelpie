import { useState } from "react";
import { SettingsPage } from "./settings/SettingsPage";
import { Shell } from "./shell/Shell";

export function App() {
  const [settings, setSettings] = useState(false);
  return (
    <div className="flex h-screen flex-col">
      {/* The shell stays mounted behind the page, so a message half-written is still there */}
      <div className={settings ? "hidden" : "flex min-h-0 flex-1 flex-col"}>
        <Shell onSettings={() => setSettings(true)} />
      </div>
      {settings && <SettingsPage onClose={() => setSettings(false)} />}
    </div>
  );
}
