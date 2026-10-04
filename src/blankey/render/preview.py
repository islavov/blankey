import io

import pypdfium2 as pdfium
from PIL import ImageDraw

from blankey.render.pdf_form import FormField

DEFAULT_DPI = 80


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
