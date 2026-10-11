// no console window on Windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // the stdio bridge for Claude Desktop starts without the UI
    if std::env::args().skip(1).eq(["mcp"]) {
        blankey_lib::mcp::bridge::main();
        return;
    }
    blankey_lib::run()
}
