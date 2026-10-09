"""Schema versions, tracked in PRAGMA user_version.

A new vault gets the current schema from the models. An existing vault runs every step after its version, in order;
each step is a module with `up(database)`, numbered by the version it leads to. Version 1 is the initial schema.
"""

from peewee import SqliteDatabase

from blankey.vault.migrations import v0002_fill_sets, v0003_sealed_requests
from blankey.vault.models import MODELS

STEPS = {2: v0002_fill_sets.up, 3: v0003_sealed_requests.up}
VERSION = max(STEPS)


def upgrade(database: SqliteDatabase) -> None:
    version = database.pragma("user_version")
    if version == 0:
        with database.atomic():
            database.create_tables(MODELS)
            database.pragma("user_version", VERSION)
        return
    for target in range(version + 1, VERSION + 1):
        with database.atomic():
            STEPS[target](database)
            database.pragma("user_version", target)
