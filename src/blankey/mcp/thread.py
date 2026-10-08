import threading

from blankey.core import Blankey


class McpThread(threading.Thread):
    """Runs the MCP server. The SDK takes about a second to import, so that happens here, off the UI thread."""

    def __init__(self, app: Blankey):
        super().__init__(name="blankey-mcp", daemon=True)
        self.app = app
        self.server = None
        self.stopped = False

    @property
    def running(self) -> bool:
        return self.is_alive() and self.server is not None and self.server.started

    def run(self) -> None:
        from blankey.mcp.server import create_server

        self.server = create_server(self.app)
        if not self.stopped:
            self.server.run()

    def stop(self) -> None:
        self.stopped = True
        if self.server is not None:
            self.server.should_exit = True
