import { html, render, useLayoutEffect, useRef, useState } from "../../vendor/preact-htm.js";

const root = () => document.getElementById("menu-root");
let open = false;

export const isMenuOpen = () => open;

/** A pop-up menu at (x, y). items: [{label, checked, action}] | "-" | {head} */
export function popup(x, y, items, minWidth = 0) {
  open = true;
  render(html`<${Menu} x=${x} y=${y} items=${items} minWidth=${minWidth} />`, root());
}

export function hidePopup() {
  if (!open) return;
  open = false;
  render(null, root());
}

document.addEventListener("mousedown", (e) => {
  if (open && !root().contains(e.target)) hidePopup();
});

function Menu({ x, y, items, minWidth }) {
  const menu = useRef();
  const [position, setPosition] = useState(null);
  // measured before paint: flips above the anchor when there is no room below
  useLayoutEffect(() => {
    const rect = menu.current.getBoundingClientRect();
    setPosition({
      left: Math.min(x, window.innerWidth - rect.width - 8),
      top: y + rect.height > window.innerHeight - 8 ? Math.max(8, y - rect.height - 36) : y,
    });
  }, [x, y, items]);
  const style = {
    minWidth: `${Math.max(260, minWidth)}px`,
    left: `${position?.left ?? x}px`,
    top: `${position?.top ?? y}px`,
    visibility: position ? "visible" : "hidden",
  };
  return html`
    <div class="menu" ref=${menu} style=${style}>
      ${items.map((item) => {
        if (item === "-") return html`<div class="menu-sep"></div>`;
        if (item.head) return html`<div class="menu-head">${item.head}</div>`;
        const choose = () => {
          hidePopup();
          item.action();
        };
        return html`<div class=${`menu-item${item.checked ? " checked" : ""}`} title=${item.label} onClick=${choose}>${item.label}</div>`;
      })}
    </div>`;
}
