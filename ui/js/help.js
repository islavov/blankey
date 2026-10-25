// The help box: what the element under the pointer is for. Nav items explain the idea behind their part of Blankey;
// everything else gets a short note on what it does. One listener for the whole window finds the nearest element with a
// data-help key (or a button with a title of its own). The sidebar box shows it; while a sheet covers the sidebar, the
// sheet's footer note does. On or off is remembered on this machine.

import { useEffect, useState } from "../vendor/preact-htm.js";
import { t } from "./i18n.js";
import { html, update } from "./lib.js";
import { toast } from "./ui/feedback.js";

// [title, text], read at render time in the current language
const HELP = {
  "help.box": () => [t("Help"), t("Explains what is under the pointer. Hide it with ✕ and turn it on again in Settings.")],
  "help.close": () => [t("Hide help"), t("Hides this box. Turn it on again in Settings, under General.")],

  "nav.activity": () => [t("Activity"), t("When your AI assistant needs something from you, such as values for a profile, a file or a filled document, it sends a request. Nothing happens until you answer it.")],
  "nav.fillsets": () => [t("Fill sets"), t("A fill set remembers how documents were filled: the templates, which profile plays each role and where each value comes from. Generate it again later with the current data.")],
  "nav.documents": () => [t("Documents"), t("The documents Blankey made from your templates and profiles, kept encrypted in the vault. Export one to save a copy outside it.")],
  "nav.templates": () => [t("Templates"), t("A document with blanks: a Word file, a PDF form or a Typst source. Blanks for personal data are bound to profile fields; your AI assistant can fill the rest.")],
  "nav.profiles": () => [t("Profiles"), t("Where the sensitive data lives: names, ID numbers, addresses, bank details and files. Your AI assistant fills other blanks itself, but here it sees only field names, never values.")],
  "nav.links": () => [t("Collection links"), t("A request you send to someone for their data and files. They are encrypted in the sender's browser and can only be opened on this computer, not by Blankey Online or anyone in between. Useful for an ID card photo, for example.")],
  "sidebar.relay": () => [t("Blankey Online"), t("Lets web agents such as claude.ai reach Blankey. The dot is green when connected. Click to sign in or see its settings.")],
  "sidebar.lock": () => [t("Lock"), t("Everything Blankey keeps is encrypted with your password, on this computer. Locking makes it unreadable until you unlock it again.")],
  "sidebar.theme": () => [t("Light or dark"), t("Switches between the light and the dark look. Remembered on this computer.")],
  "sidebar.settings": () => [t("Settings"), t("Language, updates, this help, the MCP server and Blankey Online.")],
  search: () => [t("Search"), t("Narrows the list on this page as you type. Cleared when you go to another page.")],

  "page.activity": () => [t("Activity"), t("Requests from your AI assistant. Double-click one to open it: a waiting one to answer it, a done one to see what it made.")],
  "page.fillsets": () => [t("Fill sets"), t("Click to select, Cmd/Ctrl-click or Shift-click for several. Double-click to open the fill form.")],
  "page.documents": () => [t("Documents"), t("Double-click to open; Cmd/Ctrl-click or Shift-click selects several.")],
  "page.templates": () => [t("Templates"), t("Ready-made documents with blanks, and the folders new ones come from.")],
  "page.profiles": () => [t("Profiles"), t("Pick a profile on the left and edit its fields on the right. Each value is encrypted on its own.")],
  "page.links": () => [t("Collection links"), t("The links you made to collect data. Click one to see what came in; double-click to copy it.")],
  "page.settings": () => [t("Settings"), t("How Blankey works on this computer, and Blankey Online for web agents.")],

  "activity.open": () => [t("Open"), t("Opens the selected request: a waiting one to answer it, a fill in the fill form, or the documents it made.")],
  "fillsets.open": () => [t("Open"), t("Opens the selected fill set in the fill form, to change its profiles or sources.")],
  "fillsets.generate": () => [t("Generate"), t("Makes the documents of the selected fill sets again, with the profiles' current values.")],
  "fillsets.delete": () => [t("Delete"), t("Deletes the selected fill sets. The documents made from them are kept.")],
  "documents.open": () => [t("Open"), t("Opens the selected documents in the app your computer uses for them.")],
  "documents.export": () => [t("Export"), t("Saves a copy of the selected document where you choose, outside the vault.")],
  "documents.delete": () => [t("Delete"), t("Removes the selected documents from the vault. Copies you exported stay.")],

  "templates.view": () => [t("Templates or folders"), t("Templates are ready to fill in. Folders are where you keep forms; Blankey reads them in place.")],
  "templates.fill": () => [t("Fill"), t("Opens the fill form for the selected templates, to fill them in together.")],
  "templates.copyid": () => [t("Copy ID"), t("Copies the template's id, to name it to your AI assistant.")],
  "templates.delete": () => [t("Delete"), t("Deletes the selected templates. Generated documents are kept; fill sets using them no longer open.")],
  "templates.list": () => [t("Templates"), t("Click one to see it, Cmd/Ctrl-click to select several, double-click to fill it in.")],
  "templates.preview": () => [t("Preview"), t("The template with its blanks marked. Pinch or Ctrl-scroll to zoom, double-click to reset.")],
  "templates.bindings": () => [t("Bindings"), t("Each blank and the profile field or expression that fills it. Blanks without one are asked for when you fill.")],
  "templates.folder": () => [t("Show in Finder"), t("Shows the template's folder: its source file and how its blanks are bound.")],
  "folders.add": () => [t("Add folder…"), t("Adds a folder of forms and model contracts. Blankey reads the files where they are and never changes them.")],
  "folders.rescan": () => [t("Rescan"), t("Reads the folders again for new, changed or removed files.")],
  "folders.root": () => [t("Folder"), t("A folder Blankey reads. Remove takes it off the list; its files stay where they are.")],
  "folders.files": () => [t("Files"), t("Files in your folders that are not templates yet. Click one to preview it; ask your AI assistant to make a template from it.")],
  "folders.pii": () => [t("Personal data…"), t("Shows what is hidden from your AI assistant when it reads this file, and lets you hide more or less.")],
  "folders.open": () => [t("Open"), t("Opens the file in the app your computer uses for it.")],
  "folders.reveal": () => [t("Show in Finder"), t("Shows the file in its folder.")],

  "profiles.new": () => [t("New profile"), t("Makes an empty profile for a person, a company or anything else whose data you fill in often.")],
  "profiles.rename": () => [t("Rename"), t("Changes the name or the kind of the selected profile.")],
  "profiles.delete": () => [t("Delete profile"), t("Deletes the selected profiles with all their fields and files.")],
  "profiles.list": () => [t("Profiles"), t("Click one to edit it; Cmd/Ctrl-click or Shift-click selects several to delete.")],
  "profiles.fields": () => [t("Fields"), t("The key names the value for templates (address.city), the label is for people, the type checks the value. Edit, then Save.")],
  "profiles.add": () => [t("Add field"), t("Adds an empty row for a new field.")],
  "profiles.reveal": () => [t("Show values"), t("Shows the values instead of dots.")],
  "profiles.save": () => [t("Save"), t("Saves the edited fields to the vault. Problems are listed under the table.")],
  "profiles.remove": () => [t("Remove field"), t("Removes this field when you save.")],
  "profiles.files": () => [t("Files"), t("Documents of this person or company, such as an ID card scan: the ones you add and the ones sent through a collection link. Encrypted in the vault.")],

  "links.new": () => [t("New link"), t("Makes a web form that asks someone for profile fields and files. The link is copied for you to send.")],
  "links.copy": () => [t("Copy link"), t("Copies the selected link, to send it again.")],
  "links.revoke": () => [t("Revoke"), t("Stops the selected links taking answers. What came in stays here.")],
  "links.delete": () => [t("Delete"), t("Removes the selected links and what they received that you have not saved.")],
  "links.filters": () => [t("Filters"), t("Shows all links, the ones with submissions to review, or the open or closed ones.")],
  "links.side": () => [t("Submissions"), t("What came in through this link. Review one to save its values into a profile.")],
  "links.signin": () => [t("Sign in"), t("Collection links go through Blankey Online. Sign in to make them.")],
  "link.profile": () => [t("Save into"), t("The profile the answers go into when you accept a submission, or a new one.")],
  "link.fields": () => [t("Fields"), t("Tick the profile fields to ask for and the ones that are required. Add fields the profile does not have yet.")],
  "link.files": () => [t("Files"), t("How many files people may attach, and of which kinds. 0 asks for none.")],
  "link.expires": () => [t("Expires after"), t("The link stops taking answers after this.")],
  "link.submissions": () => [t("Submissions"), t("How many times the form can be sent.")],
  "link.create": () => [t("Create link"), t("Makes the link and copies it, ready to send.")],

  "settings.language": () => [t("Language"), t("Blankey's language on this computer. Default uses your Blankey Online account's language.")],
  "settings.version": () => [t("Version"), t("The version you have. Check now looks for a newer one.")],
  "settings.updates": () => [t("Updates"), t("Looks for new versions every few hours. Nothing is installed until you choose to.")],
  "settings.statistics": () => [t("Statistics"), t("Anonymous counts that help improve Blankey. Never names or contents.")],
  "settings.help": () => [t("Help"), t("Shows the help box in the sidebar.")],
  "settings.mcp": () => [t("MCP server"), t("Where AI assistants on this computer, such as Claude Desktop or Claude Code, connect to Blankey. Copy the URL into their settings.")],
  "settings.relay": () => [t("Online server"), t("The Blankey Online server web agents connect through. Another server is only for one you run yourself.")],
  "settings.connector": () => [t("Connector URL"), t("Add this as a custom connector in claude.ai to reach Blankey from the web.")],
  "settings.account": () => [t("Account"), t("The Blankey Online account this computer is signed in with.")],
  "settings.status": () => [t("Status"), t("Whether Blankey is connected to Blankey Online right now.")],
  "settings.agents": () => [t("Web agents"), t("The web agents you allowed to reach Blankey. Revoke one to cut it off.")],

  "fill.name": () => [t("Fill set"), t("The name this fill is saved under, to open or generate it again later.")],
  "fill.role": () => [t("Roles"), t("A template names the parties it needs, such as applicant or landlord. Choose which profile plays each role.")],
  "fill.field": () => [t("Blank"), t("A blank of the documents. Click to find it in the preview; Up and Down move between blanks.")],
  "fill.source": () => [t("Source"), t("Where the value comes from: a profile field, the template default, the assistant's suggestion or a value you type. The assistant learns only which, never the value.")],
  "fill.preview": () => [t("Preview"), t("The documents as they will come out. Click a blank to find its row; pinch or Ctrl-scroll to zoom.")],
  "fill.save": () => [t("Save"), t("Saves the fill set without making documents.")],
  "fill.generate": () => [t("Save and Generate"), t("Saves the fill set and makes the documents, kept encrypted under Documents.")],
  "sheet.cancel": () => [t("Cancel"), t("Closes without saving. When your AI assistant asked for this, it learns that you cancelled.")],

  "request.fields": () => [t("Fields"), t("The fields your AI assistant asked for. What you type goes into the profile; the assistant never sees it.")],
  "request.suggestion": () => [t("Suggested"), t("Puts the value your AI assistant suggested into the field.")],
  "request.save": () => [t("Save"), t("Saves the values into the profile and tells the assistant they are there.")],
  "request.choose": () => [t("Choose file…"), t("Picks the file to share. Blankey keeps a copy; the assistant can read only that copy, with personal data hidden.")],
  "request.share": () => [t("Share"), t("Gives the assistant the file, with the marked values hidden.")],
  "request.allow": () => [t("Allow"), t("Lets the assistant read this file, with the marked values hidden.")],
  "submission.profile": () => [t("Save into"), t("The profile the sent values go into, or a new one. Check and correct them before you save.")],
  "submission.files": () => [t("Files"), t("The files that came with it. Open or export a copy; Keep stores it encrypted with the profile.")],
  "submission.discard": () => [t("Discard"), t("Deletes this submission from this computer. Values and files you saved stay.")],
  "submission.later": () => [t("Later"), t("Closes it for now. It waits on the link until you save or discard it.")],
  "submission.save": () => [t("Save"), t("Saves the values into the profile and keeps the ticked files.")],

  "pii.list": () => [t("Hidden values"), t("What your AI assistant does not see in this file. Click a value to step through its places in the preview.")],
  "pii.toggle": () => [t("Show or hide"), t("Crossed out: hidden from the assistant. Open: the assistant sees it. Click to switch.")],
  "pii.remove": () => [t("Remove"), t("Takes the value off the list. A value Blankey found counts as not personal, and the assistant sees it.")],
  "pii.places": () => [t("Places"), t("Steps to the previous or next place of this value in the file.")],
  "pii.typed": () => [t("Text to hide…"), t("Type text to hide everywhere in this file, then Hide.")],
  "pii.preview": () => [t("Preview"), t("Select text to hide it from the assistant. Click a marked value to show or hide it.")],
  "pii.done": () => [t("Done"), t("Closes the review. Your changes are already kept for this file.")],
};

const KEY = "blankey.help";

function stored() {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

let enabled = stored() !== "off";
let hovered = null;
let pending = null;
const listeners = new Set();

export const helpEnabled = () => enabled;

/** Shows or hides the help box; the choice is remembered on this machine. */
export function setHelpEnabled(on) {
  enabled = on;
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    // private storage unavailable: the choice lasts until the window reloads
  }
  show(null);
  update({});
}

/** [title, text] of a help key, or undefined for an unknown one. */
export const helpText = (key) => HELP[key]?.();

/** The nearest element that has help: a known data-help key, or a button with a title of its own. */
function helpAt(target) {
  for (let el = target instanceof Element ? target : null; el; el = el.parentElement) {
    if (HELP[el.getAttribute("data-help")]) return el;
    if (el.tagName === "BUTTON" && (el.title || el.getAttribute("aria-label"))) return el;
  }
  return null;
}

const entryOf = (el) => helpText(el.getAttribute("data-help")) || [el.title || el.getAttribute("aria-label"), ""];

function show(el) {
  clearTimeout(pending);
  if (el === hovered) return;
  hovered = el;
  listeners.forEach((listener) => listener());
}

/** From a control out to the pane around it is often the gap before the next control: that waits a moment. */
function point(el) {
  clearTimeout(pending);
  if (el === hovered) return;
  if (hovered?.isConnected && (!el || el.contains(hovered))) pending = setTimeout(() => show(el), 120);
  else show(el);
}

document.addEventListener("mouseover", (e) => enabled && point(helpAt(e.target)));
// the pointer left the window
document.addEventListener("mouseout", (e) => enabled && !e.relatedTarget && point(null));

function useHovered() {
  const [, force] = useState(0);
  useEffect(() => {
    const listener = () => force((n) => n + 1);
    listeners.add(listener);
    return () => listeners.delete(listener);
  }, []);
  return hovered?.isConnected ? hovered : null;
}

/** The sidebar box: the help of what is under the pointer, else what the current page is for. */
export function HelpBox({ page }) {
  const el = useHovered();
  const [title, text] = el && !el.closest(".sheet") ? entryOf(el) : helpText(`page.${page}`);
  const close = () => {
    setHelpEnabled(false);
    toast(t("Help is off. Turn it on again in Settings."));
  };
  return html`
    <div class="help-box" data-help="help.box">
      <div class="help-head">
        <span class="help-title">${title}</span>
        <button class="ghost help-close" data-help="help.close" aria-label=${t("Hide help")} onClick=${close}>✕</button>
      </div>
      <div class="help-text">${text}</div>
    </div>`;
}

/** A sheet's footer note: the help of what is under the pointer in the sheet, else the note itself. */
export function HelpNote({ children }) {
  const el = useHovered();
  if (!el?.closest(".sheet")) return html`<span class="note">${children}</span>`;
  const [title, text] = entryOf(el);
  return html`<span class="note"><b>${title}</b>${text && `: ${text}`}</span>`;
}
