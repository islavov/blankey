"""AcroForm inspection and Unicode-safe filling.

Text values are drawn onto the page with an embedded TTF and the filled widget is
removed (flattened), so output never depends on the form's /DA fonts. Checkboxes
and radio buttons keep their native widgets and are switched via /AS.
"""

import io
import unicodedata
from dataclasses import dataclass, field
from functools import cache

from pypdf import PdfReader, PdfWriter
from pypdf.generic import ArrayObject, DictionaryObject, NameObject
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas

from blanka.config import FONTS_DIR

FONT_NAME = "BlankaSans"
FONT_FILE = FONTS_DIR / "NotoSans-Regular.ttf"
DEFAULT_SIZE = 9.0
MIN_SIZE = 5.0
PADDING = 2.0
COMB_FLAG = 1 << 24
OFF_VALUES = {"", "false", "none", "0", "off", "/off"}


@dataclass(slots=True)
class FormField:
    name: str
    type: str  # text | checkbox | radio | choice | signature
    page: int  # 1-based
    rects: list[list[float]]
    max_len: int | None = None
    comb: bool = False
    states: list[str] = field(default_factory=list)


@dataclass(slots=True)
class Fit:
    fits: bool
    size: float
    ratio: float  # needed width / available width at the minimum font size


@cache
def font_name() -> str:
    pdfmetrics.registerFont(TTFont(FONT_NAME, str(FONT_FILE)))
    return FONT_NAME


@cache
def _covered_chars() -> frozenset[int]:
    font_name()
    return frozenset(pdfmetrics.getFont(FONT_NAME).face.charToGlyph)


def missing_glyphs(value: str) -> list[str]:
    covered = _covered_chars()
    return sorted({ch for ch in value if not ch.isspace() and ord(ch) not in covered})


def _inherited(annot: DictionaryObject, key: str):
    node = annot
    while node is not None:
        if key in node:
            return node[key]
        parent = node.get("/Parent")
        node = parent.get_object() if parent is not None else None
    return None


def _field_name(annot: DictionaryObject) -> str | None:
    parts = []
    node = annot
    while node is not None:
        if "/T" in node:
            parts.append(str(node["/T"]))
        parent = node.get("/Parent")
        node = parent.get_object() if parent is not None else None
    return ".".join(reversed(parts)) or None


def _widgets(page):
    for ref in page.get("/Annots") or []:
        annot = ref.get_object()
        if annot.get("/Subtype") == "/Widget":
            yield ref, annot


def _field_type(annot: DictionaryObject) -> str:
    ft = _inherited(annot, "/FT")
    flags = int(_inherited(annot, "/Ff") or 0)
    match ft:
        case "/Btn":
            return "radio" if flags & (1 << 15) else "checkbox"
        case "/Ch":
            return "choice"
        case "/Sig":
            return "signature"
    return "text"


def inspect_form(pdf: bytes) -> list[FormField]:
    reader = PdfReader(io.BytesIO(pdf))
    fields: dict[str, FormField] = {}
    for page_no, page in enumerate(reader.pages, start=1):
        for _, annot in _widgets(page):
            name = _field_name(annot)
            if not name:
                continue
            rect = [round(float(x), 1) for x in annot["/Rect"]]
            if name not in fields:
                ftype = _field_type(annot)
                flags = int(_inherited(annot, "/Ff") or 0)
                max_len = _inherited(annot, "/MaxLen")
                fields[name] = FormField(
                    name=name,
                    type=ftype,
                    page=page_no,
                    rects=[],
                    max_len=int(max_len) if max_len is not None else None,
                    comb=bool(flags & COMB_FLAG) and ftype == "text",
                )
            fields[name].rects.append(rect)
            if fields[name].type in {"checkbox", "radio"}:
                for state in annot.get("/AP", {}).get("/N", {}):
                    if state != "/Off" and state not in fields[name].states:
                        fields[name].states.append(state)
    return list(fields.values())


def measure(rect: list[float], value: str, comb_len: int = 0) -> Fit:
    """Pick a font size for a single-line value and report whether it fits."""
    x0, y0, x1, y1 = rect
    width, height = x1 - x0, y1 - y0
    name = font_name()
    if comb_len:
        cell = width / comb_len
        size = min(DEFAULT_SIZE, height - 1)
        widest = max((pdfmetrics.stringWidth(ch, name, size) for ch in value), default=0)
        ratio = max(len(value) / comb_len, widest / cell if cell else 0)
        return Fit(fits=ratio <= 1, size=size, ratio=round(ratio, 2))
    available = width - 2 * PADDING
    size = min(DEFAULT_SIZE, height - 3)
    needed = pdfmetrics.stringWidth(value, name, size)
    if needed > available:
        size = max(MIN_SIZE, size * available / needed)
        needed = pdfmetrics.stringWidth(value, name, size)
    ratio = needed / available if available > 0 else float("inf")
    return Fit(fits=ratio <= 1.0001, size=size, ratio=round(ratio, 2))


def _draw(c: canvas.Canvas, rect: list[float], value: str, comb_len: int) -> None:
    x0, y0, _, y1 = rect
    height = y1 - y0
    fit = measure(rect, value, comb_len)
    c.setFont(font_name(), fit.size)
    baseline = y0 + (height - fit.size * 0.7) / 2
    if comb_len:
        cell = (rect[2] - x0) / comb_len
        for i, ch in enumerate(value.rjust(comb_len)[-comb_len:]):
            c.drawCentredString(x0 + cell * (i + 0.5), baseline, ch)
    else:
        c.drawString(x0 + PADDING, baseline, value)


def is_on(value: str) -> bool:
    return value.strip().lower() not in OFF_VALUES


def fill_form(pdf: bytes, values: dict[str, str]) -> bytes:
    """Fill fields by name. Text is flattened; buttons get their on-state when the value is truthy.

    For radio groups and multi-state checkboxes the value may name the state, e.g. "/Choice2".
    """
    writer = PdfWriter(clone_from=PdfReader(io.BytesIO(pdf)))
    values = {k: unicodedata.normalize("NFC", v) for k, v in values.items() if v is not None}
    flattened: set[str] = set()

    for page in writer.pages:
        buf = io.BytesIO()
        c = canvas.Canvas(buf, pagesize=(float(page.mediabox.width), float(page.mediabox.height)))
        kept = ArrayObject()
        for ref, annot in _widgets(page):
            name = _field_name(annot)
            value = values.get(name) if name else None
            if value is None:
                kept.append(ref)
                continue
            if _inherited(annot, "/FT") == "/Btn":
                kept.append(ref)
                _set_button(annot, value)
                continue
            if value:
                flags = int(_inherited(annot, "/Ff") or 0)
                comb_len = int(_inherited(annot, "/MaxLen") or 0) if flags & COMB_FLAG else 0
                _draw(c, [float(x) for x in annot["/Rect"]], value, comb_len)
            flattened.add(name)
        c.save()
        page.merge_page(PdfReader(buf).pages[0])
        page[NameObject("/Annots")] = kept

    acroform = writer._root_object.get("/AcroForm")
    if acroform is not None:
        acroform = acroform.get_object()
        acroform[NameObject("/Fields")] = ArrayObject(
            f for f in acroform.get("/Fields", []) if _field_name(f.get_object()) not in flattened
        )
        if "/NeedAppearances" in acroform:
            del acroform["/NeedAppearances"]
    out = io.BytesIO()
    writer.write(out)
    return out.getvalue()


def _set_button(annot: DictionaryObject, value: str) -> None:
    states = [s for s in annot.get("/AP", {}).get("/N", {}) if s != "/Off"]
    if value.startswith("/"):
        on = value if value in states else None
    else:
        on = states[0] if is_on(value) and len(states) == 1 else None
    owner = annot if "/T" in annot else annot["/Parent"].get_object()
    if on:
        annot[NameObject("/AS")] = NameObject(on)
        owner[NameObject("/V")] = NameObject(on)
    elif "/Off" in annot.get("/AP", {}).get("/N", {}):
        annot[NameObject("/AS")] = NameObject("/Off")
