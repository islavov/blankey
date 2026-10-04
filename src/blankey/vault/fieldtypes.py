import datetime
import re
from decimal import Decimal, InvalidOperation
from enum import StrEnum

EGN_WEIGHTS = (2, 4, 8, 5, 10, 9, 7, 3, 6)
EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")
TRUE_VALUES = {"1", "true", "да", "yes", "x"}


class FieldType(StrEnum):
    TEXT = "text"
    NUMBER = "number"
    DATE = "date"
    BOOL = "bool"
    EGN = "egn"
    IBAN = "iban"
    EMAIL = "email"
    PHONE = "phone"


def valid_egn(value: str) -> bool:
    if not re.fullmatch(r"\d{10}", value):
        return False
    checksum = sum(int(d) * w for d, w in zip(value[:9], EGN_WEIGHTS, strict=True)) % 11 % 10
    return checksum == int(value[9])


def valid_iban(value: str) -> bool:
    iban = value.replace(" ", "").upper()
    if not re.fullmatch(r"[A-Z]{2}\d{2}[A-Z0-9]{10,30}", iban):
        return False
    digits = "".join(str(int(ch, 36)) for ch in iban[4:] + iban[:4])
    return int(digits) % 97 == 1


def parse_date(value: str) -> datetime.date | None:
    try:
        return datetime.datetime.strptime(value.strip(), "%d.%m.%Y").date()
    except ValueError:
        return None


def validate(field_type: FieldType, value: str) -> str | None:
    """Return an error message, or None when the value is acceptable."""
    if not value:
        return None
    match field_type:
        case FieldType.NUMBER:
            try:
                Decimal(value.replace(" ", "").replace(",", "."))
            except InvalidOperation:
                return "Invalid number"
        case FieldType.DATE:
            if parse_date(value) is None:
                return "Date must be DD.MM.YYYY"
        case FieldType.EGN:
            if not valid_egn(value):
                return "Invalid EGN"
        case FieldType.IBAN:
            if not valid_iban(value):
                return "Invalid IBAN"
        case FieldType.EMAIL:
            if not EMAIL_RE.match(value):
                return "Invalid email"
    return None


def coerce(field_type: FieldType, value: str) -> str | int | Decimal | bool:
    """Convert a stored string into the value templates see."""
    match field_type:
        case FieldType.NUMBER:
            number = Decimal(value.replace(" ", "").replace(",", "."))
            return int(number) if number == number.to_integral_value() else number
        case FieldType.BOOL:
            return value.strip().lower() in TRUE_VALUES
    return value
