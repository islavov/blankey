import sys


def main() -> None:
    # the stdio bridge for Claude Desktop must not pay for Qt and the server
    if sys.argv[1:] == ["mcp"]:
        from blankey.mcp import bridge

        bridge.main()
        return
    from blankey import app

    app.main()


if __name__ == "__main__":
    main()
