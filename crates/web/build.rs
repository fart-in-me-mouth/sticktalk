use std::{env, fs, path::PathBuf};

fn main() {
    let spec = PathBuf::from("../../docs/UI_SPEC.md");
    println!("cargo:rerun-if-changed={}", spec.display());
    let text = fs::read_to_string(spec).expect("UI_SPEC token source");
    let mut css = String::from("/* Generated from docs/UI_SPEC.md by build.rs. */\n:root {\n");
    for line in text.lines().filter(|line| line.starts_with("| ")) {
        let cols: Vec<_> = line.split('|').map(str::trim).collect();
        if cols.len() < 4 {
            continue;
        }
        let key = cols[1];
        let value = cols[2];
        if key == "online / idle / dnd / offline" {
            for (name, color) in key.split(" / ").zip(value.split(" / ")) {
                css.push_str(&format!("  --{name}: {color};\n"));
            }
        } else if value.starts_with('#') {
            let value = if key == "mention.bg" {
                "rgb(242 184 75 / 8%)"
            } else {
                value
            };
            css.push_str(&format!("  --{}: {};\n", key.replace('.', "-"), value));
        }
    }
    for size in [4, 8, 12, 16, 24, 32] {
        css.push_str(&format!("  --space-{size}: {size}px;\n"));
    }
    css.push_str("  --radius-small: 4px; --radius: 8px; --radius-modal: 12px;\n}\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("tokens.css"),
        css,
    )
    .expect("generated tokens");
}
