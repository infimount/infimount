import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import "./index.css";
import { useThemeStore } from "./store/themeStore";

// Initialize theme on app load
const { theme } = useThemeStore.getState();
useThemeStore.getState().setTheme(theme);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);

