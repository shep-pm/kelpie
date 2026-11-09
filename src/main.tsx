import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles/globals.css";

const root = document.getElementById("root");

if (!root) {
  throw new Error("index.html has no #root element to mount App into");
}

// shadcn's dark variant keys off a `.dark` class; follow the OS until there is
// a theme setting.
const dark = window.matchMedia("(prefers-color-scheme: dark)");
const applyTheme = () =>
  document.documentElement.classList.toggle("dark", dark.matches);
applyTheme();
dark.addEventListener("change", applyTheme);

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
