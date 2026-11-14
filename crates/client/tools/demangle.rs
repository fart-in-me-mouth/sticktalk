//! Tiny offline helper for the heap-profile MSVC map; no application profile overrides.
use std::io::{self, BufRead};
fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.expect("symbol input");
        println!("{:#}", rustc_demangle::demangle(&line));
    }
}
