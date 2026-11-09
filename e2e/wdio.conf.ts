import { type ChildProcess, spawn } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";

// The e2e host runs this through scripts/e2e-remote.sh, which sets
// CARGO_TARGET_DIR. The debug build is what `pnpm tauri build --debug
// --no-bundle` leaves there.
const targetDir =
  process.env.CARGO_TARGET_DIR ?? join(homedir(), "kelpie-e2e-target");
const application = join(targetDir, "debug", "kelpie-app");

// tauri-driver reads its own capability, which WebdriverIO's types lack.
declare global {
  namespace WebdriverIO {
    interface Capabilities {
      "tauri:options"?: { application: string };
    }
  }
}

let tauriDriver: ChildProcess | undefined;

// tauri-driver is a child of this run: whichever way the run ends, it goes too.
const stopDriver = () => {
  tauriDriver?.kill();
  tauriDriver = undefined;
};
process.on("exit", stopDriver);

export const config: WebdriverIO.Config = {
  runner: "local",
  hostname: "127.0.0.1",
  port: 4444,
  specs: ["./specs/**/*.e2e.ts"],
  maxInstances: 1,
  capabilities: [
    {
      "tauri:options": { application },
    },
  ],
  logLevel: "warn",
  reporters: [
    "spec",
    [
      "junit",
      {
        outputDir: join(import.meta.dirname, "..", "test-results"),
        outputFileFormat: () => "junit.xml",
      },
    ],
  ],
  framework: "mocha",
  mochaOpts: {
    ui: "bdd",
    timeout: 60_000,
  },
  beforeSession: () => {
    tauriDriver = spawn("tauri-driver", [], {
      stdio: [null, process.stdout, process.stderr],
    });
    // A driver that never starts would otherwise show up as a connection
    // error with no cause.
    tauriDriver.on("error", (error) => {
      console.error(`tauri-driver failed to start: ${error.message}`);
      process.exit(1);
    });
  },
  afterSession: stopDriver,
};
