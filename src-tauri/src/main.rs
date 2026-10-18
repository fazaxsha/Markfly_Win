// Hide the console window in release builds (the Windows "windows" subsystem).
// Without this, launching the app from Explorer spawns a cmd window behind it.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    markfly_lib::run()
}
