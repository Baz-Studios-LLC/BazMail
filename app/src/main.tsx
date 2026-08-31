import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

// Marks the platform on <body> so the stylesheet can inset for macOS traffic
// lights without every component having to know where it is running.
if (navigator.userAgent.includes("Macintosh")) {
  document.body.classList.add("mac");
}

// A browser context menu in a native app is the most obvious tell that it is
// not one, so the default is suppressed everywhere the app has its own answer.
//
// Editable fields are the deliberate exception: on macOS that menu is the
// system's own text menu with cut, copy, paste and spelling, and taking it away
// would remove real function to gain consistency we do not yet offer. It goes
// once the app has a menu of its own to put there.
document.addEventListener("contextmenu", (event) => {
  const target = event.target as HTMLElement | null;
  const editable =
    target?.closest("input, textarea, [contenteditable='true']") != null;
  if (!editable) event.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
