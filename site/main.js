import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "./style.css";

// Each site theme also restyles the demo, with a pane style that shows it off.
const PANE_STYLE = { amber: "minimal", catppuccin: "boxed", tokyo: "glass", nord: "boxed", gruvbox: "minimal" };
const STORAGE_KEY = "spotini.site.theme";

function applyTheme(theme, { toDemo }) {
  document.documentElement.dataset.palette = theme;
  for (const button of document.querySelectorAll("[data-theme]")) {
    button.setAttribute("aria-pressed", String(button.dataset.theme === theme));
  }
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // The theme just won't be remembered.
  }
  if (toDemo) {
    document
      .getElementById("demo-frame")
      ?.contentWindow?.postMessage({ type: "spotini-theme", palette: theme, paneStyle: PANE_STYLE[theme] }, location.origin);
  }
}

let saved = "amber";
try {
  saved = localStorage.getItem(STORAGE_KEY) ?? "amber";
} catch {
  // Private mode: start with amber.
}
if (!(saved in PANE_STYLE)) saved = "amber";
applyTheme(saved, { toDemo: false });

document.querySelector(".themes")?.addEventListener("click", (e) => {
  const theme = e.target.closest("[data-theme]")?.dataset.theme;
  if (theme) applyTheme(theme, { toDemo: true });
});

// The demo renders at a desktop window size and is scaled to fit the page.
const DEMO_WIDTH = 1100;
const DEMO_HEIGHT = 680;
const viewport = document.getElementById("demo-viewport");
const frame = document.getElementById("demo-frame");
if (viewport && frame) {
  frame.width = String(DEMO_WIDTH);
  frame.height = String(DEMO_HEIGHT);
  const fit = () => {
    const scale = Math.min(1, viewport.clientWidth / DEMO_WIDTH);
    frame.style.transform = `scale(${scale})`;
    viewport.style.height = `${DEMO_HEIGHT * scale}px`;
  };
  new ResizeObserver(fit).observe(viewport);
  // Start the demo in the visitor's chosen theme.
  frame.addEventListener("load", () => {
    if (saved !== "amber" && !frame.dataset.themed) {
      frame.dataset.themed = "1";
      applyTheme(document.documentElement.dataset.palette, { toDemo: true });
    }
  });
}
