import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { wordmarkFor } from "../theme";

/**
 * Replaces the native Windows caption.
 *
 * The point is that it carries colour 1, the same surface as the icon rail, so
 * the two form one continuous L. A native caption bar paints its own grey there
 * and cuts the app in half.
 *
 * Glyphs are drawn rather than taken from Segoe Fluent Icons so they recolour
 * with the theme and do not depend on a font being present.
 */
export function TitleBar() {
  const [maximized, setMaximized] = useState(false);
  const win = getCurrentWindow();

  useEffect(() => {
    let alive = true;
    void win.isMaximized().then((v) => alive && setMaximized(v));
    const unlisten = win.onResized(() => {
      void win.isMaximized().then((v) => alive && setMaximized(v));
    });
    return () => {
      alive = false;
      void unlisten.then((f) => f());
    };
  }, [win]);

  return (
    <div className="titlebar">
      {/* Everything except the buttons drags the window. */}
      <div className="titlebar-drag" data-tauri-drag-region>
        <img
          className="titlebar-mark"
          src={wordmarkFor()}
          alt="BazMail"
          draggable={false}
          data-tauri-drag-region
        />
      </div>

      <div className="win-controls">
        <button
          className="win-btn"
          aria-label="Minimize"
          onClick={() => void win.minimize()}
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <path d="M0 5h10" stroke="currentColor" strokeWidth="1" />
          </svg>
        </button>

        <button
          className="win-btn"
          aria-label={maximized ? "Restore" : "Maximize"}
          onClick={() => void win.toggleMaximize()}
        >
          {maximized ? (
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
              <rect x="0.5" y="2.5" width="7" height="7" stroke="currentColor" />
              <path d="M2.5 2.5v-2h7v7h-2" stroke="currentColor" />
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
              <rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" />
            </svg>
          )}
        </button>

        <button
          className="win-btn close"
          aria-label="Close"
          onClick={() => void win.close()}
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <path d="M0 0l10 10M10 0L0 10" stroke="currentColor" strokeWidth="1" />
          </svg>
        </button>
      </div>
    </div>
  );
}
