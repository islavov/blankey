from peewee import SqliteDatabase

from blankey.vault.models import FillSetRow


def up(database: SqliteDatabase) -> None:
    database.create_tables([FillSetRow])
