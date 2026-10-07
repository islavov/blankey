import contextlib
import ctypes
import ctypes.util
import json
import os
import plistlib
import sys
from pathlib import Path

if sys.platform == "win32":
    import winreg

NS_APPLICATION_ACTIVATION_POLICY_REGULAR = 0
NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY = 1
LAUNCH_AGENT_LABEL = "bg.blankey.app"
WINDOWS_RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"


def _ns_app_call(selector: bytes, arg_type, arg) -> None:
    objc = ctypes.cdll.LoadLibrary(ctypes.util.find_library("objc"))
    objc.objc_getClass.restype = ctypes.c_void_p
    objc.sel_registerName.restype = ctypes.c_void_p
    send = objc.objc_msgSend
    send.restype = ctypes.c_void_p
    send.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    ns_app = send(objc.objc_getClass(b"NSApplication"), objc.sel_registerName(b"sharedApplication"))
    send.argtypes = [ctypes.c_void_p, ctypes.c_void_p, arg_type]
    send(ns_app, objc.sel_registerName(selector), arg)


def hide_dock_icon() -> None:
    """Run as a menu bar app on macOS (the packaged app sets LSUIElement instead)."""
    if sys.platform == "darwin":
        _ns_app_call(b"setActivationPolicy:", ctypes.c_long, NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY)


def set_dock_visible(visible: bool) -> None:
    """Regular app (Dock icon + menu bar) while the main window is open, menu bar app otherwise."""
    if sys.platform == "darwin":
        policy = NS_APPLICATION_ACTIVATION_POLICY_REGULAR if visible else NS_APPLICATION_ACTIVATION_POLICY_ACCESSORY
        _ns_app_call(b"setActivationPolicy:", ctypes.c_long, policy)


def bring_to_front() -> None:
    """Accessory (menu bar) apps on macOS must activate explicitly for their windows to get focus."""
    if sys.platform == "darwin":
        _ns_app_call(b"activateIgnoringOtherApps:", ctypes.c_bool, True)


def launch_command() -> list[str]:
    if getattr(sys, "frozen", False):
        return [sys.executable]
    return [sys.executable, "-m", "blankey"]


def _launch_agent_path() -> Path:
    return Path.home() / "Library" / "LaunchAgents" / f"{LAUNCH_AGENT_LABEL}.plist"


def _xdg_autostart_path() -> Path:
    return Path.home() / ".config" / "autostart" / "blankey.desktop"


def autostart_enabled() -> bool:
    match sys.platform:
        case "darwin":
            return _launch_agent_path().exists()
        case "win32":
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, WINDOWS_RUN_KEY) as key:
                try:
                    winreg.QueryValueEx(key, "Blankey")
                    return True
                except FileNotFoundError:
                    return False
        case _:
            return _xdg_autostart_path().exists()


def set_autostart(enabled: bool) -> None:
    command = launch_command()
    match sys.platform:
        case "darwin":
            path = _launch_agent_path()
            if enabled:
                path.parent.mkdir(parents=True, exist_ok=True)
                plist = {"Label": LAUNCH_AGENT_LABEL, "ProgramArguments": command, "RunAtLoad": True}
                path.write_bytes(plistlib.dumps(plist))
            else:
                path.unlink(missing_ok=True)
        case "win32":
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, WINDOWS_RUN_KEY, 0, winreg.KEY_SET_VALUE) as key:
                if enabled:
                    winreg.SetValueEx(key, "Blankey", 0, winreg.REG_SZ, " ".join(f'"{c}"' for c in command))
                else:
                    with contextlib.suppress(FileNotFoundError):
                        winreg.DeleteValue(key, "Blankey")
        case _:
            path = _xdg_autostart_path()
            if enabled:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(
                    "[Desktop Entry]\nType=Application\nName=Blankey\n"
                    f"Exec={' '.join(command)}\nX-GNOME-Autostart-enabled=true\n",
                    encoding="utf-8",
                )
            else:
                path.unlink(missing_ok=True)


def claude_desktop_config_path() -> Path:
    match sys.platform:
        case "darwin":
            return Path.home() / "Library" / "Application Support" / "Claude" / "claude_desktop_config.json"
        case "win32":
            return Path(os.environ["APPDATA"]) / "Claude" / "claude_desktop_config.json"
        case _:
            return Path.home() / ".config" / "Claude" / "claude_desktop_config.json"


def connect_claude_desktop() -> Path:
    """Register `blankey mcp` (stdio) as an MCP server in Claude Desktop's config, keeping other entries."""
    path = claude_desktop_config_path()
    config = json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}
    command, *args = launch_command()
    config.setdefault("mcpServers", {})["blankey"] = {"command": command, "args": [*args, "mcp"]}
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(config, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return path
