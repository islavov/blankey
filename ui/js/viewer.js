// A case file or a generated document shown inside Blankey, so reading or printing it needs no decrypted copy in
// another app.

import { call } from "./api.js";
import { t } from "./i18n.js";
import { html, useLoad } from "./lib.js";
import { Preview } from "./preview.js";
import { openSheet, SheetHead } from "./ui/sheet.js";

/**
 * Opens the viewer. `load` resolves to the preview, or to {title, filename, preview} when the caller knows only an
 * id; `outside` opens the file in the app the computer uses for it.
 */
export const openViewer = (props) => openSheet((close) => html`<${Viewer} ...${props} close=${close} />`);

function Viewer({ title = "", name = "", load, outside, close }) {
  const loaded = useLoad(load, []);
  const preview = loaded?.preview ?? loaded;
  // a Word file shows as an approximation of its layout: it prints from Word, not from here
  const printable = preview?.type === "pages" || preview?.type === "image";
  return html`
    <div class="sheet-head viewer-head">
      <${SheetHead} icon="doc" title=${loaded?.title ?? title} reason=${loaded?.filename ?? name} />
      <div class="head-actions">
        ${printable && html`<button data-help="viewer.print" onClick=${() => call("print_window")}>${t("Print")}</button>`}
        <button data-help="viewer.outside" onClick=${outside}>${t("Open in default app")}</button>
        <button class="primary" onClick=${close}>${t("Close")}</button>
      </div>
    </div>
    <div class="sheet-body file-view"><${Preview} preview=${preview} interactive=${false} /></div>`;
}

const openOutside = (ids) => call("open_documents", { ids });

/** Generated documents: one shows inside Blankey, several open in the apps the computer uses for them. */
export function openDocuments(ids) {
  if (ids.length !== 1) return openOutside(ids);
  return openViewer({ load: () => call("document_preview", { id: ids[0] }), outside: () => openOutside(ids) });
}
