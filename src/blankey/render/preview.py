import io

import pypdfium2 as pdfium
from PIL import Image, ImageDraw

from blankey.render.pdf_form import FormField

DEFAULT_DPI = 80


def page_sizes(pdf: bytes) -> list[tuple[float, float]]:
    return [(page.get_width(), page.get_height()) for page in pdfium.PdfDocument(pdf)]


def page_count(pdf: bytes) -> int:
    return len(pdfium.PdfDocument(pdf))


def render_pages(pdf: bytes, pages: list[int] | None = None, dpi: int = DEFAULT_DPI) -> list[bytes]:
    """Render 1-based pages to PNG bytes. Form widgets are drawn as well."""
    doc = pdfium.PdfDocument(pdf)
    doc.init_forms()
    selected = pages or list(range(1, len(doc) + 1))
    images = []
    for number in selected:
        bitmap = doc[number - 1].render(scale=dpi / 72, may_draw_forms=True)
        buf = io.BytesIO()
        bitmap.to_pil().save(buf, format="PNG", optimize=True)
        images.append(buf.getvalue())
    return images


def annotate_fields(pdf: bytes, fields: list[FormField], page: int, dpi: int = 110) -> bytes:
    """Render one page with each field's rectangle outlined and labelled with its name."""
    doc = pdfium.PdfDocument(pdf)
    pdf_page = doc[page - 1]
    height = pdf_page.get_height()
    scale = dpi / 72
    image = pdf_page.render(scale=scale).to_pil().convert("RGB")
    draw = ImageDraw.Draw(image)
    for f in fields:
        for x0, y0, x1, y1 in f.rects if f.page == page else []:
            box = (x0 * scale, (height - y1) * scale, x1 * scale, (height - y0) * scale)
            draw.rectangle(box, outline=(220, 30, 30), width=1)
            draw.text((box[0] + 1, box[1] - 9), f.name, fill=(220, 30, 30))
    buf = io.BytesIO()
    image.save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def field_labels(pdf: bytes, fields: list[FormField], limit: int = 60) -> dict[str, str]:
    """Nearby printed text as a label: right of small boxes (checkbox style), else above, else left of the field.
    Empty for scans without a text layer."""
    doc = pdfium.PdfDocument(pdf)
    text_pages = {}
    labels = {}
    for f in fields:
        if not f.rects:
            continue
        if f.page not in text_pages:
            text_pages[f.page] = doc[f.page - 1].get_textpage()
        text = text_pages[f.page]
        x0, y0, x1, y1 = f.rects[0]
        right = (x1 + 2, y0 - 1, x1 + 60, y1 + 1)
        above = (x0 - 10, y1, x1 + 10, y1 + 14)
        left = (max(0, x0 - 200), y0 - 1, x0, y1 + 1)
        order = [right, above, left] if x1 - x0 < 20 else [above, left]
        for box_left, bottom, box_right, top in order:
            found = " ".join(text.get_text_bounded(left=box_left, bottom=bottom, right=box_right, top=top).split())
            if found:
                labels[f.name] = found if len(found) <= limit else found[: limit - 1].rstrip() + "…"
                break
    return labels


def find_text(pdf: bytes, needles: dict[str, str]) -> dict[str, list[tuple[int, tuple]]]:
    """Where each needle is printed: name -> [(1-based page, (x0, y0, x1, y1) in points, origin bottom-left)].
    A needle broken across lines yields one rectangle per line."""
    found: dict[str, list[tuple[int, tuple]]] = {}
    for number, page in enumerate(pdfium.PdfDocument(pdf), start=1):
        text = page.get_textpage()
        for name, needle in needles.items():
            searcher = text.search(needle, match_case=True)
            while (match := searcher.get_next()) is not None:
                index, count = match
                for i in range(text.count_rects(index, count)):
                    found.setdefault(name, []).append((number, text.get_rect(i)))
    return found


GRID_STEP, GRID_LABEL_STEP = 50, 100  # points


def layout_lines(pdf: bytes, page: int, limit: int = 300) -> list[dict]:
    """Printed text on a 1-based page as lines with [x0, y0, x1, y1] in PDF points (origin bottom-left)."""
    text = pdfium.PdfDocument(pdf)[page - 1].get_textpage()
    lines = []
    for i in range(min(text.count_rects(), limit)):
        left, bottom, right, top = text.get_rect(i)
        found = " ".join(text.get_text_bounded(left=left, bottom=bottom, right=right, top=top).split())
        if found:
            lines.append({"text": found, "rect": [round(left, 1), round(bottom, 1), round(right, 1), round(top, 1)]})
    return lines


def grid_page(pdf: bytes, page: int, dpi: int = 80) -> bytes:
    """The page with a coordinate grid in PDF points (origin bottom-left) for placing boxes, also on scans."""
    doc = pdfium.PdfDocument(pdf)
    doc.init_forms()
    pdf_page = doc[page - 1]
    width, height = pdf_page.get_width(), pdf_page.get_height()
    scale = dpi / 72
    image = pdf_page.render(scale=scale, may_draw_forms=True).to_pil().convert("RGBA")
    overlay = Image.new("RGBA", image.size, (0, 0, 0, 0))
    draw = ImageDraw.Draw(overlay)
    for x in range(0, int(width) + 1, GRID_STEP):
        strong = x % GRID_LABEL_STEP == 0
        draw.line([(x * scale, 0), (x * scale, image.height)], fill=(220, 30, 30, 110 if strong else 50))
        if strong:
            draw.text((x * scale + 2, 2), str(x), fill=(220, 30, 30, 255))
    for y in range(0, int(height) + 1, GRID_STEP):
        strong = y % GRID_LABEL_STEP == 0
        top = (height - y) * scale
        draw.line([(0, top), (image.width, top)], fill=(220, 30, 30, 110 if strong else 50))
        if strong:
            draw.text((2, top - 12), str(y), fill=(220, 30, 30, 255))
    buf = io.BytesIO()
    Image.alpha_composite(image, overlay).convert("RGB").save(buf, format="PNG", optimize=True)
    return buf.getvalue()
