import io

import pytest
from pypdf import PdfReader

from blankey.render import pdf_form
from blankey.templates import TemplateKind, engine
from blankey.templates.bindings import evaluate, example_context, money, nest, referenced_paths
from blankey.vault import FieldInput, FieldType

FIELDS = {
    "name": "{{ applicant.name }}",
    "egn": "{{ applicant.egn }}",
    "email": "{{ applicant.email }}",
    "citizen_bg": "{{ applicant.citizenship == 'българско' }}",
    "status": "{{ '/married' if applicant.married else '/single' }}",
}


@pytest.fixture
def template(app, form_pdf, tmp_path):
    source = tmp_path / "form.pdf"
    source.write_bytes(form_pdf)
    return app.templates.save(
        "test-form", "Test", TemplateKind.PDF_FORM, {"applicant": "person"}, FIELDS, source_file=source
    )


def text_of(pdf: bytes) -> str:
    return "".join(page.extract_text() for page in PdfReader(io.BytesIO(pdf)).pages)


def test_referenced_paths():
    roles = {"applicant", "loan"}
    assert referenced_paths("{{ applicant.address.city }}", roles) == {"applicant.address.city"}
    assert referenced_paths("{{ loan.obligations[0].creditor }} {{ x.y }}", roles) == {"loan.obligations.0.creditor"}
    assert referenced_paths("{{ 'X' if applicant.a == 'b' else loan.c | money }}", roles) == {
        "applicant.a",
        "loan.c",
    }


def test_nest_builds_lists():
    assert nest({"a.b": 1, "l.1.x": "b", "l.0.x": "a"}) == {"a": {"b": 1}, "l": [{"x": "a"}, {"x": "b"}]}


def test_evaluate_native_and_filters():
    ctx = example_context({"loan": {"amount": 150000, "items": [1, 2]}})
    assert evaluate("{{ loan.amount | money }}", ctx) == "150 000"
    assert evaluate("{{ loan.missing.deep }}", ctx) is None
    assert evaluate("{{ loan.items | sum }}", ctx) == 3
    assert money("1234.5") == "1 234,50"


def test_render_example_fills_cyrillic(app, template):
    ctx = example_context(
        {"applicant": {"name": "Жана Щерева", "egn": "7501020018", "citizenship": "българско", "married": True}}
    )
    rendered = engine.render(template, ctx)
    assert rendered.warnings == []
    text = text_of(rendered.content)
    assert "Жана Щерева" in text
    assert "7501020018" in "".join(text.split())
    fields = PdfReader(io.BytesIO(rendered.content)).get_fields()
    assert "name" not in fields  # flattened
    assert fields["citizen_bg"]["/V"] == "/Yes"
    assert fields["status"]["/V"] == "/married"


def test_unknown_form_field_is_reported(app, template):
    template.fields["nope"] = "{{ applicant.name }}"
    rendered = engine.render(template, example_context({"applicant": {"name": "x"}}))
    assert any("nope" in w for w in rendered.warnings)


def test_check_bindings_locked_and_unlocked(app, template):
    pid = app.vault.create_profile("Жана", "person")
    app.vault.set_values(
        pid,
        [
            FieldInput("name", "Жана Щерева"),
            FieldInput("egn", "7501020018", type=FieldType.EGN),
            FieldInput("email", "a.very.long.email.address@example.com", type=FieldType.EMAIL),
            FieldInput("citizenship", ""),
        ],
    )
    app.vault.lock()
    reports = {r.field: r for r in app.check_bindings("test-form", {"applicant": pid})}
    assert reports["name"].status == "ok" and reports["name"].fit_ratio is None
    assert reports["citizen_bg"].status == "empty"
    assert reports["status"].missing == ["applicant.married"]

    app.vault.unlock("correct horse battery staple")
    reports = {r.field: r for r in app.check_bindings("test-form", {"applicant": pid})}
    assert reports["email"].status == "overflow" and reports["email"].fit_ratio > 1
    assert reports["name"].fit_ratio < 1


def test_missing_glyphs():
    assert pdf_form.missing_glyphs("Иван 😀") == ["😀"]


def test_typst_template(app):
    source = "#let data = json(bytes(sys.inputs.data))\n= Декларация\nДолуподписаният #data.name, ЕГН #data.egn\n"
    template = app.templates.save(
        "decl",
        "Декларация",
        TemplateKind.TYPST,
        {"applicant": "person"},
        {"name": "{{ applicant.name }}", "egn": "{{ applicant.egn }}"},
        source_text=source,
    )
    rendered = engine.render(template, example_context({"applicant": {"name": "Иван Иванов", "egn": "123"}}))
    assert "Иван Иванов" in text_of(rendered.content)


def test_field_crops_and_printed_labels(form_pdf):
    from PIL import Image

    from blankey.render import preview

    fields = pdf_form.inspect_form(form_pdf)
    crops = preview.field_crops(form_pdf, fields, dpi=72)
    assert set(crops) == {f.name for f in fields}
    name = next(f for f in fields if f.name == "name")
    x0, y0, x1, y1 = name.rects[0]
    image = Image.open(io.BytesIO(crops["name"]))
    expected_width = min(595, x1 + preview.CROP_RIGHT) - max(0, x0 - preview.CROP_LEFT)
    assert abs(image.width - expected_width) <= 1
    assert abs(image.height - (y1 - y0 + preview.CROP_ABOVE + preview.CROP_BELOW)) <= 1
    labels = preview.field_labels(form_pdf, fields)
    assert labels["name"] == "Name" and labels["egn"] == "EGN"


BOXES = [
    {"name": "name", "page": 1, "rect": [90, 785, 400, 803]},
    {"name": "egn", "page": 1, "rect": [90, 745, 210, 761], "max_len": 10, "comb": True},
    {"name": "married", "page": 1, "rect": [100, 706, 112, 718], "type": "check"},
]


def test_add_fields_turns_a_flat_pdf_into_a_form(flat_pdf):
    form = pdf_form.add_fields(flat_pdf, BOXES)
    fields = {f.name: f for f in pdf_form.inspect_form(form)}
    assert fields["name"].rects == [[90.0, 785.0, 400.0, 803.0]] and not fields["name"].comb
    assert (fields["egn"].max_len, fields["egn"].comb) == (10, True)
    assert (fields["married"].max_len, fields["married"].comb) == (1, True)
    filled = pdf_form.fill_form(form, {"name": "Иван Петров", "egn": "7501020018", "married": "X"})
    text = text_of(filled)
    assert "Иван Петров" in text and "X" in text
    assert pdf_form.inspect_form(filled) == []  # flattened


def test_add_fields_rejects_bad_boxes(flat_pdf):
    bad = [
        {"name": "1x", "page": 1, "rect": [0, 0, 10, 10]},
        {"name": "a", "page": 2, "rect": [0, 0, 10, 10]},
        {"name": "b", "page": 1, "rect": [10, 10, 5, 5]},
        {"name": "c", "page": 1, "rect": [500, 800, 700, 900]},
        {"name": "c", "page": 1, "rect": [0, 0, 10, 10], "type": "radio"},
    ]
    with pytest.raises(ValueError) as error:
        pdf_form.add_fields(flat_pdf, bad)
    message = str(error.value)
    for expected in ("name must be", "page must be 1..1", "x1 > x0", "outside the page", "duplicate", "text or check"):
        assert expected in message


def test_layout_lines_and_grid(flat_pdf):
    from blankey.render import preview

    lines = preview.layout_lines(flat_pdf, 1)
    assert [line["text"] for line in lines] == ["Name:", "EGN:", "Married"]
    x0, y0, x1, y1 = lines[0]["rect"]
    assert x1 > x0
    assert 39 < x0 < 42 and 788 < y0 < 791 and y1 > y0
    assert preview.grid_page(flat_pdf, 1).startswith(b"\x89PNG")
