import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/app.css";
import { migrateSavedToolIds } from "./savedToolIds";

migrateSavedToolIds(window.localStorage);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
