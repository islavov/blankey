"""Turn a filled .docx into a docxtpl template and find its blanks."""

import html
import re
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import docx
from docx.document import Document
from docx.enum.text import WD_ALIGN_PARAGRAPH
from docx.oxml.ns import qn
from docx.table import Table
from docx.text.paragraph import Paragraph

TAG_RE = re.compile(r"\{\{\s*([A-Za-z_]\w*)[^}]*\}\}")
VAR_RE = re.compile(r"^[A-Za-z_]\w*$")
BLANK = "[____]"


@dataclass(slots=True)
class ParagraphInfo:
    index: int
    location: str  # "body" or "table T row R cell C"
    text: str
    runs: list[str]
    tags: list[str] = field(default_factory=list)


def _paragraphs(document: Document) -> Iterator[tuple[str, Paragraph]]:
    for paragraph in document.paragraphs:
        yield "body", paragraph
    for t, table in enumerate(document.tables):
        for r, row in enumerate(table.rows):
            seen = set()
            for c, cell in enumerate(row.cells):
                if id(cell._tc) in seen:  # merged cells repeat
                    continue
                seen.add(id(cell._tc))
                for paragraph in cell.paragraphs:
                    yield f"table {t} row {r} cell {c}", paragraph


def inspect(path: Path) -> list[ParagraphInfo]:
    document = docx.Document(str(path))
    out = []
    for index, (location, paragraph) in enumerate(_paragraphs(document)):
        if not paragraph.text.strip():
            continue
        runs = [run.text for run in paragraph.runs]
        out.append(ParagraphInfo(index, location, paragraph.text, runs, TAG_RE.findall(paragraph.text)))
    return out


def tokenize(source: Path, target: Path, replacements: list[dict[str, Any]]) -> list[str]:
    """Replace text spans with {{ var }} tags. Each replacement: {find, var, occurrence?} where occurrence is
    1-based over the whole document (omit to replace all). A span must sit inside one run.
    Returns a report line per replacement; raises ValueError listing every replacement that failed."""
    document = docx.Document(str(source))
    paragraphs = list(_paragraphs(document))
    original = "\n".join(p.text for _, p in paragraphs)
    report, errors = [], []
    for item in replacements:
        find, var, occurrence = item.get("find", ""), item.get("var", ""), item.get("occurrence")
        if not find or not VAR_RE.match(var):
            errors.append(f"{item}: needs non-empty 'find' and an identifier 'var'")
            continue
        tag = "{{ " + var + " }}"
        seen = replaced = 0
        for _, paragraph in paragraphs:
            for run in paragraph.runs:
                text, parts, start = run.text, [], 0
                while (pos := text.find(find, start)) >= 0:
                    seen += 1
                    if occurrence is None or seen == occurrence:
                        parts.append(text[start:pos] + tag)
                        replaced += 1
                    else:
                        parts.append(text[start : pos + len(find)])
                    start = pos + len(find)
                if parts:
                    run.text = "".join(parts) + text[start:]
        if replaced:
            report.append(f"{var}: replaced {replaced}x {find!r}")
            continue
        split = [i for i, (_, p) in enumerate(paragraphs) if find in p.text]
        if occurrence is not None and seen:
            errors.append(f"{var}: {find!r} occurs {seen}x in runs, no occurrence {occurrence}")
        elif split:
            errors.append(f"{var}: {find!r} spans several runs (formatting change) in paragraph(s) {split}")
        elif find in original:
            errors.append(f"{var}: {find!r} was consumed by an earlier replacement; reorder or use occurrence")
        else:
            errors.append(f"{var}: {find!r} not found")
    if errors:
        raise ValueError("Tokenizing failed:\n" + "\n".join(errors))
    target.parent.mkdir(parents=True, exist_ok=True)
    document.save(str(target))
    return report


def variables(path: Path) -> list[str]:
    """Tag names in document order, without duplicates."""
    found: dict[str, None] = {}
    for _, paragraph in _paragraphs(docx.Document(str(path))):
        for name in TAG_RE.findall(paragraph.text):
            found.setdefault(name)
    return list(found)


def blank_contexts(path: Path, width: int = 400) -> dict[str, list[str]]:
    """For each tag: the surrounding paragraph text per occurrence, the tag shown as [____] and
    other tags as [name], trimmed to about `width` characters around the blank."""
    contexts: dict[str, list[str]] = {}
    for _, paragraph in _paragraphs(docx.Document(str(path))):
        text = paragraph.text
        for match in TAG_RE.finditer(text):
            before = TAG_RE.sub(lambda m: f"[{m.group(1)}]", text[: match.start()])
            after = TAG_RE.sub(lambda m: f"[{m.group(1)}]", text[match.end() :])
            half = max(0, (width - len(BLANK)) // 2)
            if len(before) > half:
                before = "…" + before[-half:]
            if len(after) > half:
                after = after[:half] + "…"
            contexts.setdefault(match.group(1), []).append(" ".join((before + BLANK + after).split()))
    return contexts


ALIGN = {WD_ALIGN_PARAGRAPH.CENTER: "center", WD_ALIGN_PARAGRAPH.RIGHT: "right", WD_ALIGN_PARAGRAPH.JUSTIFY: "justify"}


def _run_html(run, labels: dict[str, str]) -> str:
    text = html.escape(run.text)
    text = TAG_RE.sub(lambda m: f'<span class="blank">&nbsp;{html.escape(labels.get(m[1]) or m[1])}&nbsp;</span>', text)
    if run.bold:
        text = f"<b>{text}</b>"
    if run.italic:
        text = f"<i>{text}</i>"
    if run.underline:
        text = f"<u>{text}</u>"
    if run.font.size is not None:
        text = f'<span style="font-size:{run.font.size.pt:g}pt">{text}</span>'
    return text


def _paragraph_html(paragraph: Paragraph, labels: dict[str, str]) -> str:
    body = "".join(_run_html(run, labels) for run in paragraph.runs) or "&nbsp;"
    align = ALIGN.get(paragraph.alignment)
    style = paragraph.style.name if paragraph.style is not None else ""
    if style.startswith(("Heading", "Title")):
        body = f'<span style="font-size:15pt; font-weight:600">{body}</span>'
    return f'<p align="{align}">{body}</p>' if align else f"<p>{body}</p>"


def to_html(path: Path, labels: dict[str, str] | None = None) -> str:
    """The document body as simple HTML for a read-only preview; {{ var }} tags become
    <span class="blank"> chips showing the variable's label."""
    labels = labels or {}
    document = docx.Document(str(path))
    parts = []
    for child in document.element.body.iterchildren():
        if child.tag == qn("w:p"):
            parts.append(_paragraph_html(Paragraph(child, document), labels))
        elif child.tag == qn("w:tbl"):
            rows = []
            for row in Table(child, document).rows:
                seen, cells = set(), []
                for cell in row.cells:
                    if id(cell._tc) in seen:
                        continue
                    seen.add(id(cell._tc))
                    cells.append("<td>" + "".join(_paragraph_html(p, labels) for p in cell.paragraphs) + "</td>")
                rows.append("<tr>" + "".join(cells) + "</tr>")
            parts.append('<table width="100%" cellpadding="4">' + "".join(rows) + "</table>")
    return "\n".join(parts)
