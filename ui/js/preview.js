import { useEffect, useLayoutEffect, useRef } from "../vendor/preact-htm.js";
import { t } from "./i18n.js";
import { html } from "./lib.js";

const ZOOM_MIN = 0.5;
const ZOOM_MAX = 4;

/**
 * A template preview: html (docx with blanks as chips) or page images with a box per field.
 * `active` is highlighted; bumping `revealSeq` scrolls it into view. Clicking a field calls `onPick(name)`.
 */
export function Preview({ preview, placeholder = "", interactive = true, onPick, active, revealSeq = 0, style = "" }) {
  const area = useRef();
  usePinchZoom(area);

  useEffect(() => {
    $$var(area.current).forEach((el) => el.classList.toggle("active", el.dataset.var === active));
  });

  useEffect(() => {
    if (!revealSeq) return;
    $$var(area.current)
      .find((el) => el.dataset.var === active)
      ?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [revealSeq]);

  const pick = (e) => {
    const field = e.target.closest("[data-var]");
    if (field && onPick) onPick(field.dataset.var);
  };

  return html`<div class="preview-area" style=${style} ref=${area} onClick=${pick}>${content(preview, placeholder, interactive)}</div>`;
}

const $$var = (root) => [...root.querySelectorAll("[data-var]")];

function content(preview, placeholder, interactive) {
  if (!preview) return placeholder && html`<div class="preview-error">${placeholder}</div>`;
  if (preview.type === "error") return html`<div class="preview-error">${t("No preview: {error}", { error: preview.message })}</div>`;
  // the docx HTML comes from Blankey's own renderer, which escapes the document text
  if (preview.type === "html") return html`<div class="zoom"><div class="paper" dangerouslySetInnerHTML=${{ __html: preview.html }}></div></div>`;
  return html`<div class="zoom">${preview.pages.map((page) => html`<${PdfPage} page=${page} interactive=${interactive} />`)}</div>`;
}

function PdfPage({ page, interactive }) {
  const percent = (value, total) => `${(value / total) * 100}%`;
  const boxes = Object.entries(page.boxes).flatMap(([name, rects]) =>
    rects.map(([x0, y0, x1, y1]) => {
      const style = {
        left: percent(x0, page.width),
        top: percent(page.height - y1, page.height),
        width: percent(x1 - x0, page.width),
        height: percent(y1 - y0, page.height),
      };
      return html`<div class="box" data-var=${name} style=${style} title=${name}></div>`;
    })
  );
  return html`
    <div class=${`pdf-page ${interactive ? "" : "static"}`} style=${{ aspectRatio: `${page.width}/${page.height}` }}>
      <img src=${page.image} alt="" />${boxes}
    </div>`;
}

/** Waits until the page images are decoded, so swapping a preview in does not flash. */
export function decodePreview(preview) {
  if (preview?.type !== "pages") return Promise.resolve();
  return Promise.all(
    preview.pages.map((page) => {
      const img = new Image();
      img.src = page.image;
      return img.decode().catch(() => {});
    })
  );
}

/** Pinch (trackpad or ctrl+wheel) zooms around the pointer; double-click resets. Survives re-renders. */
function usePinchZoom(area) {
  const zoom = useRef(1);

  const apply = () => {
    const content = area.current?.querySelector(".zoom");
    if (!content) return;
    content.style.setProperty("--zoom", zoom.current);
    content.style.width = `${zoom.current * 100}%`;
  };

  // after every render: new content starts at the current zoom
  useLayoutEffect(apply);

  useEffect(() => {
    const el = area.current;
    const zoomTo = (level, x, y) => {
      const old = zoom.current;
      const next = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, level));
      if (next === old) return;
      const rect = el.getBoundingClientRect();
      const px = x - rect.left;
      const py = y - rect.top;
      const contentX = (el.scrollLeft + px) / old;
      const contentY = (el.scrollTop + py) / old;
      zoom.current = next;
      apply();
      el.scrollLeft = contentX * next - px;
      el.scrollTop = contentY * next - py;
    };
    const wheel = (e) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      zoomTo(zoom.current * Math.exp(-e.deltaY * 0.01), e.clientX, e.clientY);
    };
    // WebKit sends pinches as gesture events instead of ctrl+wheel
    let start = 1;
    const gestureStart = (e) => {
      e.preventDefault();
      start = zoom.current;
    };
    const gestureChange = (e) => {
      e.preventDefault();
      zoomTo(start * e.scale, e.clientX, e.clientY);
    };
    const reset = (e) => {
      if (!e.target.closest("[data-var]")) zoomTo(1, e.clientX, e.clientY);
    };
    el.addEventListener("wheel", wheel, { passive: false });
    el.addEventListener("gesturestart", gestureStart);
    el.addEventListener("gesturechange", gestureChange);
    el.addEventListener("dblclick", reset);
    return () => {
      el.removeEventListener("wheel", wheel);
      el.removeEventListener("gesturestart", gestureStart);
      el.removeEventListener("gesturechange", gestureChange);
      el.removeEventListener("dblclick", reset);
    };
  }, []);
}
