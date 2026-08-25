#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

extern crate dsh_lib;

pub fn main() {
    dsh_lib::run();
}
