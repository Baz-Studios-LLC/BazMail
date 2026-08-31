import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

// Marks the platform on <body> so the stylesheet can inset for macOS traffic
// lights without every component having to know where it is running.
if (navigator.userAgent.includes("Macintosh")) {
  document.body.classList.add("mac");
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
