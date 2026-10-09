import sqlite3
from pathlib import Path

from peewee import SqliteDatabase

from blankey.vault.migrations import upgrade
from blankey.vault.models import MODELS


def connect(path: Path) -> SqliteDatabase:
    """One shared connection (guarded by the vault's lock), migrated and bound to the models."""
    database = SqliteDatabase(
        path,
        pragmas={"journal_mode": "wal", "foreign_keys": 1},
        thread_safe=False,
        check_same_thread=False,
    )
    database.connect()
    database.bind(MODELS)
    upgrade(database)
    return database


def backup(database: SqliteDatabase, path: Path, keep: int = 3) -> None:
    for i in range(keep, 1, -1):
        older = path.with_name(f"{path.name}.{i - 1}")
        if older.exists():
            older.replace(path.with_name(f"{path.name}.{i}"))
    target = sqlite3.connect(path.with_name(f"{path.name}.1"))
    with target:
        database.connection().backup(target)
    target.close()
