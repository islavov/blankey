from peewee import BlobField, SqliteDatabase
from playhouse.migrate import SqliteMigrator, migrate


def up(database: SqliteDatabase) -> None:
    migrator = SqliteMigrator(database)
    migrate(
        migrator.add_column("requests", "sealed_payload", BlobField(null=True)),
        migrator.add_column("requests", "sealed_result", BlobField(null=True)),
    )
