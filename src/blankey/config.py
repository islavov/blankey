import tomllib
from dataclasses import asdict, dataclass
from pathlib import Path

from platformdirs import user_data_path

APP_NAME = "Blankey"
ASSETS_DIR = Path(__file__).parent / "assets"
FONTS_DIR = ASSETS_DIR / "fonts"
HEALTH_PATH = "/health"
HEALTH_TEXT = "blankey"


@dataclass(slots=True)
class Config:
    data_dir: Path
    port: int = 8765
    auto_lock_minutes: int = 15
    pkcs11_lib: str = ""

    @property
    def db_path(self) -> Path:
        return self.data_dir / "vault.db"

    @property
    def templates_dir(self) -> Path:
        return self.data_dir / "templates"

    @property
    def previews_dir(self) -> Path:
        return self.data_dir / "previews"

    @property
    def config_path(self) -> Path:
        return self.data_dir / "config.toml"

    @property
    def mcp_url(self) -> str:
        return f"http://127.0.0.1:{self.port}/mcp"

    def save(self) -> None:
        values = {k: v for k, v in asdict(self).items() if k != "data_dir"}
        lines = [f"{k} = {v!r}" if isinstance(v, int) else f'{k} = "{v}"' for k, v in values.items()]
        self.config_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def load_config(data_dir: Path | None = None) -> Config:
    data_dir = data_dir or user_data_path(APP_NAME, appauthor=False)
    data_dir.mkdir(parents=True, exist_ok=True)
    config = Config(data_dir=data_dir)
    if config.config_path.exists():
        stored = tomllib.loads(config.config_path.read_text(encoding="utf-8"))
        for key, value in stored.items():
            if hasattr(config, key) and key != "data_dir":
                setattr(config, key, value)
    else:
        config.save()
    config.templates_dir.mkdir(exist_ok=True)
    config.previews_dir.mkdir(exist_ok=True)
    return config
