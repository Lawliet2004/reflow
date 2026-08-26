import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { OverlayApp } from "./components/OverlayApp";
import "./styles/globals.css";

const params = new URLSearchParams(window.location.search);
const isOverlay = params.get("window") === "overlay";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isOverlay ? <OverlayApp /> : <App />}</React.StrictMode>
);
