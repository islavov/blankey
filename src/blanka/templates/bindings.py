"""Binding expressions: Jinja (sandboxed, native types) evaluated against role contexts."""

import datetime
import json
from decimal import Decimal
from functools import cache
from typing import Any

from jinja2 import ChainableUndefined, Undefined, nodes
from jinja2.nativetypes import NativeEnvironment
from jinja2.sandbox import SandboxedEnvironment

from blanka.vault.fieldtypes import FieldType, coerce


class _Environment(SandboxedEnvironment, NativeEnvironment):
    def getattr(self, obj: Any, attribute: str) -> Any:
        # data keys such as "items" or "values" must win over dict methods
        if isinstance(obj, dict) and attribute in obj:
            return obj[attribute]
        return super().getattr(obj, attribute)


def money(value: Any, decimals: int | None = None) -> str:
    if value in (None, "") or isinstance(value, Undefined):
        return ""
    number = Decimal(str(value).replace(" ", "").replace(",", "."))
    if decimals is None:
        decimals = 0 if number == number.to_integral_value() else 2
    text = f"{number:,.{decimals}f}"
    return text.replace(",", " ").replace(".", ",")


def today() -> str:
    return datetime.date.today().strftime("%d.%m.%Y")


@cache
def environment() -> _Environment:
    env = _Environment(undefined=ChainableUndefined, autoescape=False)
    env.filters["money"] = money
    env.globals["today"] = today
    return env


def evaluate(expression: str, context: dict[str, Any]) -> Any:
    result = environment().from_string(expression).render(context)
    return None if isinstance(result, Undefined) else result


def as_text(value: Any) -> str:
    match value:
        case None:
            return ""
        case bool():
            return "true" if value else "false"
        case list() | tuple():
            return ", ".join(as_text(v) for v in value)
    return str(value)


def to_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, default=str)


def referenced_paths(expression: str, roles: set[str]) -> set[str]:
    """Dotted paths rooted at a role, e.g. "applicant.address.city". Only the longest chains are kept."""
    tree = environment().parse(expression)
    paths = set()
    for node in tree.find_all((nodes.Getattr, nodes.Getitem)):
        parts = []
        current = node
        while True:
            match current:
                case nodes.Getattr(node=inner, attr=attr):
                    parts.append(attr)
                    current = inner
                case nodes.Getitem(node=inner, arg=nodes.Const(value=key)):
                    parts.append(str(key))
                    current = inner
                case nodes.Name(name=name) if name in roles:
                    parts.append(name)
                    paths.add(".".join(reversed(parts)))
                    break
                case _:
                    break
    return {p for p in paths if not any(other != p and other.startswith(p + ".") for other in paths)}


def nest(flat: dict[str, Any]) -> dict[str, Any]:
    """Turn {"a.b": 1, "list.0.x": 2} into nested dicts; all-numeric keys become lists."""
    root: dict[str, Any] = {}
    for key, value in flat.items():
        node = root
        *parents, leaf = key.split(".")
        for part in parents:
            node = node.setdefault(part, {})
        node[leaf] = value
    return _listify(root)


def _listify(node: Any) -> Any:
    if not isinstance(node, dict):
        return node
    converted = {k: _listify(v) for k, v in node.items()}
    if converted and all(k.isdigit() for k in converted):
        return [converted[k] for k in sorted(converted, key=int)]
    return converted


def profile_context(values: dict[str, tuple[FieldType, str]]) -> dict[str, Any]:
    flat = {key: coerce(ftype, value) for key, (ftype, value) in values.items() if value}
    return nest(flat)


def example_context(example: dict[str, Any]) -> dict[str, Any]:
    return {role: nest(_flatten(data)) for role, data in example.items()}


def _flatten(data: Any, prefix: str = "") -> dict[str, Any]:
    if isinstance(data, dict):
        out = {}
        for key, value in data.items():
            out |= _flatten(value, f"{prefix}{key}.")
        return out
    if isinstance(data, list):
        out = {}
        for index, value in enumerate(data):
            out |= _flatten(value, f"{prefix}{index}.")
        return out
    return {prefix[:-1]: data}
