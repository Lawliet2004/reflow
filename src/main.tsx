import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import { OverlayApp } from "./components/OverlayApp";
import "./styles/globals.css";

const params = new URLSearchParams(window.location.search);
const tauriWindowLabel = "__TAURI_INTERNALS__" in window ? getCurrentWindow().label : null;
const isOverlay = tauriWindowLabel
  ? tauriWindowLabel === "overlay"
  : params.get("window") === "overlay";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isOverlay ? <OverlayApp /> : <App />}</React.StrictMode>,
);
