use std::{collections::BTreeMap, env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-changed=data/emoji-test.txt");
    println!("cargo:rerun-if-changed=data/aliases.json");
    let aliases: serde_json::Value =
        serde_json::from_str(&fs::read_to_string("data/aliases.json").unwrap()).unwrap();
    let mut alias_map = BTreeMap::new();
    for a in aliases.as_array().unwrap() {
        let glyph: String = a["unified"]
            .as_str()
            .unwrap()
            .split('-')
            .map(|s| char::from_u32(u32::from_str_radix(s, 16).unwrap()).unwrap())
            .filter(|&c| c != '\u{fe0f}')
            .collect();
        alias_map.insert(
            glyph,
            a["short_names"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
        );
    }
    let input = fs::read_to_string("data/emoji-test.txt").unwrap();
    let mut rows = Vec::new();
    let mut variants: BTreeMap<String, [String; 5]> = BTreeMap::new();
    let mut cat = 0;
    for line in input.lines() {
        if let Some(g) = line.strip_prefix("# group: ") {
            cat = match g {
                "Smileys & Emotion" | "People & Body" | "Component" => 0,
                "Animals & Nature" => 1,
                "Food & Drink" => 2,
                "Activities" => 3,
                "Travel & Places" => 4,
                "Objects" => 5,
                "Symbols" => 6,
                "Flags" => 7,
                _ => panic!("unknown group"),
            };
        }
        let Some((codes, rest)) = line.split_once(';') else {
            continue;
        };
        if !rest.trim_start().starts_with("fully-qualified") {
            continue;
        }
        let glyph: String = codes
            .split_whitespace()
            .map(|s| char::from_u32(u32::from_str_radix(s, 16).unwrap()).unwrap())
            .collect();
        let comment = rest.split_once('#').unwrap().1.trim();
        let (_, tail) = comment.split_once(" E").unwrap();
        let (version, name) = tail.split_once(' ').unwrap();
        let mut v = version.split('.').map(|n| n.parse::<u8>().unwrap());
        let version = [v.next().unwrap(), v.next().unwrap()];
        let normalize = |s: &str| s.chars().filter(|&c| c != '\u{fe0f}').collect::<String>();
        let tones: Vec<_> = glyph
            .chars()
            .filter(|c| ('\u{1f3fb}'..='\u{1f3ff}').contains(c))
            .collect();
        if !tones.is_empty() {
            if tones.iter().all(|c| *c == tones[0]) {
                let base: String = glyph
                    .chars()
                    .filter(|c| !('\u{1f3fb}'..='\u{1f3ff}').contains(c))
                    .collect();
                variants.entry(normalize(&base)).or_default()
                    [(tones[0] as u32 - 0x1f3fb) as usize] = glyph;
            }
            continue;
        }
        let key = normalize(&glyph);
        let names = alias_map.get(&key).cloned().unwrap_or_else(|| {
            vec![name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
                .split('_')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("_")]
        });
        rows.push((glyph, name.to_owned(), names.join("|"), cat, version, key));
    }
    let mut pool = Vec::new();
    let mut interned = BTreeMap::new();
    let mut intern = |s: &str| -> u32 {
        if let Some(&offset) = interned.get(s) {
            return offset;
        }
        let offset = pool.len() as u32;
        pool.extend_from_slice(s.as_bytes());
        pool.push(0);
        interned.insert(s.to_owned(), offset);
        offset
    };
    let mut records = Vec::new();
    let mut tone_table = Vec::new();
    for (glyph, name, aliases, cat, version, key) in rows {
        for s in [&glyph, &name, &aliases] {
            records.extend_from_slice(&intern(s).to_le_bytes());
        }
        let tone_index = if let Some(tones) = variants.get(&key) {
            let index = tone_table.len() / 20 + 1;
            for tone in tones {
                tone_table.extend_from_slice(&intern(tone).to_le_bytes());
            }
            index as u16
        } else {
            0
        };
        records.extend_from_slice(&tone_index.to_le_bytes());
        records.extend_from_slice(&[cat, version[0], version[1]]);
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("pool.bin"), &pool).unwrap();
    fs::write(out.join("records.bin"), &records).unwrap();
    fs::write(out.join("tones.bin"), &tone_table).unwrap();
    println!(
        "cargo:warning=emoji tables: {} bytes, {} base entries",
        pool.len() + records.len() + tone_table.len(),
        records.len() / 17
    );
}
