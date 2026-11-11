import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles/base.css";
import "./styles/layout.css";
import "./styles/tree.css";
import "./styles/panels.css";
import "./styles/details.css";
import "./styles/overlays.css";
import "./styles/disk.css";
import "./styles/update.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
