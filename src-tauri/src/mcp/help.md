# How Blankey works, for answering the user's "how do I…" questions

Answer from this guide in the user's language, in a few short steps. Name the page and the button. When you can do
the thing yourself with a tool, offer to do it instead of only explaining. Never ask for personal values in chat.

## What Blankey is

Blankey fills documents (contracts, declarations, forms) from data kept encrypted on the user's computer. The AI
assistant never sees the values in profiles, only field names, labels, types and how long each value is. Values go
into documents inside Blankey, after the user confirms a fill.

The pages are in the sidebar: Cases, Activity, Fill sets, Documents, Templates, Profiles, Collection links. Settings
and Lock are at the bottom.

## Cases

A case is one matter: a sale, a lease, a power of attorney. It keeps the people in it with their roles, the files the
client brought, and the templates, fill sets and documents made for it.

- New case: Cases → New case, then give it a title.
- Tabs of a case: AI assistant (on accounts that have it), Files, Profiles, Templates, Fill sets, Documents, Activity.
- Files are the originals the case starts from: what the client brought and what the user imports. Documents are the
  results: what Blankey filled in from a template.
- Add files: drop them on the open case, or Files → Import…. Files held over another case in the list for two
  seconds open it. Give each a description (what it is and whose); the assistant finds files by that description and
  never sees the file name.
- A file's ⋮ menu: Open (shown inside Blankey), Export… (a copy outside the vault), Rename…, Description…, Personal
  data… (what is hidden from the assistant in a PDF or Word file), Delete.
- Copy link, at the top of a case, copies blankey://case/<id>. Pasted to an assistant outside Blankey, it tells that
  assistant which case to work in.
- Notes on the case are read by the assistant.

## Profiles

A profile holds the data of one person, company or other party: names, ID numbers, addresses, bank details, files.

- New profile: Profiles → New profile; in a case, the Profiles tab adds one to the case or links an existing one with
  a role such as buyer or seller.
- Each field has a key (address.city) that templates use, a label for people, a type that checks the value (text,
  number, date, bool, egn, iban, email, phone) and the value.
- Values show as dots. The eye beside a value shows that one value; Show values shows all of them.
- Save stores the edited fields; problems are listed under the table.
- The assistant can ask for a profile's values: a form opens in Blankey, the user types them there, and the assistant
  only learns that it was saved.

## Templates

A template is a document with blanks: a Word file, a PDF form or a Typst source. Blanks for personal data are bound to
profile fields; other blanks get values at fill time.

- The user does not build templates by hand. They ask the assistant: "make a template from this document". The
  document is a file in the case, or a file in one of the folders added under Templates.
- Word (.docx) and PDF files can become templates. An old .doc file is converted to .docx when it is imported, if
  the computer has LibreOffice or is a Mac; the layout can shift, so it is worth a look. Where it could not be
  converted it shows as "Word (.doc)" and has to be saved as .docx in Word and imported again. A scan or photo cannot
  become a Word template.
- The Templates page also lists the folders the user added and the files in them; Blankey rebuilds a template when
  its file changes.

## Filling a document

1. The user asks the assistant to fill a template for the people in the case, or opens a fill set.
2. A fill form opens in Blankey. It lists every blank with where its value comes from: a profile field, a value the
   assistant suggested, or one the user types.
3. The user checks it and presses Save and Generate. Blankey makes the documents and keeps them encrypted under
   Documents.
4. Documents → Open shows one; Export saves a copy outside the vault.

A fill set remembers how documents were filled, so the same documents can be generated again later with current data
or for another party.

## Activity

When the assistant needs something from the user (values for a profile, a file, a fill to confirm, a collection link to
approve), it sends a request. Requests wait under Activity, and Blankey shows the waiting one. Nothing happens until
the user answers it. Cancelling a request is always allowed.

## Collection links

A collection link asks someone else for their data and files, such as a client's ID card details. It needs a Blankey
Online account.

- Collection links → New link: choose the profile it fills, the fields and how many files to ask for, how long it
  stays open. The link is copied when it is created.
- The user sends the link themselves (email, chat). The other person fills a web form; what they send is encrypted in
  their browser and can only be opened on the user's computer.
- What came in shows on the Collection links page, marked "to review". The user reviews it and saves it into the
  profile; then documents can be filled from that profile.
- Revoke stops a link; Delete removes it with what was not saved.

## Blankey Online and assistants

- The case's AI assistant tab is for accounts that have the assistant. It works inside the open case and keeps working
  while the user looks at another page. Esc or Stop stops it.
- Other assistants connect through MCP. On the same computer (Claude Desktop, Claude Code): Settings → MCP server,
  copy the URL into the assistant's settings. Web assistants such as claude.ai: sign in to Blankey Online in Settings
  and add the Connector URL shown there as a custom connector; Blankey asks before it serves an assistant for the
  first time.
- In files the assistant reads, personal data is masked (like Xxxx Xxxxxx). Personal data… on a file shows what is
  hidden and lets the user hide more or less.

## Lock, password and safety

- Everything is encrypted with the user's password on this computer. Lock (bottom of the sidebar) makes it unreadable
  until it is unlocked; the assistant cannot work on a locked vault.
- A forgotten password cannot be reset by anyone. The recovery key shown when the vault was created is the only other
  way in.
- Exported files and files opened in another app are copies outside the vault; Blankey does not protect those.

## Settings

Language (English or Bulgarian), light or dark look, updates, the help box, the MCP server and Blankey Online.

## When something does not work

- "The assistant is not available": sign in to Blankey Online in Settings with an account that has it.
- The assistant cannot read a file: it reads PDF and Word (.docx) files only. Save other formats as one of those and
  import the file again.
- A value is rejected on save: the field's type checks it, for example the check digit of an EGN or an IBAN.
- Nothing happens after asking the assistant for a fill: look under Activity for a request that is waiting.
