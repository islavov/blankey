import sqlite3
from pathlib import Path

SCHEMA = [
    """
    CREATE TABLE meta (
        key TEXT PRIMARY KEY,
        value BLOB NOT NULL
    );
    CREATE TABLE profiles (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        kind TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL
    );
    CREATE TABLE fields (
        profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
        key TEXT NOT NULL,
        label TEXT NOT NULL DEFAULT '',
        type TEXT NOT NULL DEFAULT 'text',
        value_length INTEGER NOT NULL DEFAULT 0,
        nonce BLOB,
        ciphertext BLOB,
        updated_at TEXT NOT NULL,
        PRIMARY KEY (profile_id, key)
    );
    CREATE TABLE documents (
        id INTEGER PRIMARY KEY,
        template_id TEXT NOT NULL,
        title TEXT NOT NULL,
        profiles TEXT NOT NULL,
        pages INTEGER NOT NULL,
        signed TEXT NOT NULL DEFAULT '',
        encrypted INTEGER NOT NULL DEFAULT 0,
        filename TEXT NOT NULL,
        created_at TEXT NOT NULL,
        nonce BLOB NOT NULL,
        ciphertext BLOB NOT NULL
    );
    CREATE TABLE requests (
        id INTEGER PRIMARY KEY,
        kind TEXT NOT NULL,
        payload TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'pending',
        result TEXT,
        created_at TEXT NOT NULL,
        resolved_at TEXT
    );
    CREATE TABLE audit (
        id INTEGER PRIMARY KEY,
        ts TEXT NOT NULL,
        actor TEXT NOT NULL,
        action TEXT NOT NULL,
        target TEXT NOT NULL DEFAULT ''
    );
    """,
    """
    CREATE TABLE fill_sets (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL UNIQUE,
        templates TEXT NOT NULL,
        profiles TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        nonce BLOB NOT NULL,
        ciphertext BLOB NOT NULL
    );
    """,
    """
    ALTER TABLE requests ADD COLUMN sealed_payload BLOB;
    ALTER TABLE requests ADD COLUMN sealed_result BLOB;
    """,
]


def connect(path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(path, check_same_thread=False, isolation_level=None)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    migrate(conn)
    return conn


def migrate(conn: sqlite3.Connection) -> None:
    version = conn.execute("PRAGMA user_version").fetchone()[0]
    for index, script in enumerate(SCHEMA[version:], start=version + 1):
        conn.executescript(f"BEGIN; {script}; PRAGMA user_version={index}; COMMIT;")


def backup(conn: sqlite3.Connection, path: Path, keep: int = 3) -> None:
    for i in range(keep, 1, -1):
        older = path.with_name(f"{path.name}.{i - 1}")
        if older.exists():
            older.replace(path.with_name(f"{path.name}.{i}"))
    target = sqlite3.connect(path.with_name(f"{path.name}.1"))
    with target:
        conn.backup(target)
    target.close()
