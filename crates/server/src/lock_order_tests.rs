//! The rule that keeps the server from stopping: the global lock is never
//! waited for while a pooled database connection is held (`http::budgeted`).
//!
//! Commands, new connections and the housekeeping pass take the lock first
//! and a connection second. One request doing it the other way round is
//! enough for both to wait on each other once the pool's connections are all
//! held that way, and the pool has four.
//!
//! This reads the server's own sources. For every `let` that binds a pooled
//! connection (`pool.acquire()`, `pool.begin()`, or one of the route helpers
//! that return one) it follows the binding to the end of its block and
//! refuses a `core.lock()` on the way, unless the connection was dropped
//! first. A connection used only inside one statement
//! (`auth(&mut *pool.acquire().await?, ..)`) is not a binding.
use std::path::{Path, PathBuf};

/// Route helpers that return a pooled connection with the person.
const HELPERS: [&str; 5] = ["budgeted(", "card_rest(", "person(", "rest(", "who("];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            sources(&path, out);
        } else if name.ends_with(".rs") && !name.contains("tests") {
            out.push(path);
        }
    }
}

/// The text with all whitespace removed, and for each character kept the
/// line it came from.
fn squeezed(text: &str) -> (Vec<char>, Vec<usize>) {
    let (mut chars, mut lines, mut line) = (vec![], vec![], 1);
    for ch in text.chars() {
        if ch == '\n' {
            line += 1;
        }
        if !ch.is_whitespace() {
            chars.push(ch);
            lines.push(line);
        }
    }
    (chars, lines)
}
fn at(code: &[char], i: usize, word: &str) -> bool {
    word.chars()
        .enumerate()
        .all(|(n, ch)| code.get(i + n) == Some(&ch))
}
fn word_start(code: &[char], i: usize) -> bool {
    i == 0 || !(code[i - 1].is_alphanumeric() || code[i - 1] == '_')
}

/// `(the binding's line, the lock's line)` for every violation in one file.
fn violations(text: &str) -> Vec<(usize, usize)> {
    // Inline test modules are not the server.
    let text = text
        .find("#[cfg(test)]\nmod tests {")
        .map_or(text, |cut| &text[..cut]);
    let (code, lines) = squeezed(text);
    let mut found = vec![];
    let mut i = 0;
    while i < code.len() {
        // A statement that starts with `let` ...
        let starts = at(&code, i, "let")
            && (i == 0 || matches!(code[i - 1], ';' | '{' | '}'))
            && code
                .get(i + 3)
                .is_some_and(|c| *c == '(' || *c == '_' || c.is_alphabetic());
        if !starts {
            i += 1;
            continue;
        }
        let end = (i..code.len())
            .find(|n| code[*n] == ';')
            .unwrap_or(code.len());
        let statement = &code[i..end];
        let written: String = statement.iter().collect();
        // ... and whose value is a pooled connection: the statement ends in
        // the acquire itself, or it calls a helper that returns one.
        let acquired = ["pool.acquire().await", "pool.begin().await"]
            .iter()
            .any(|a| {
                written.rfind(a).is_some_and(|n| {
                    let rest = &written[n + a.len()..];
                    rest.is_empty() || rest == "?" || rest == ".unwrap()"
                })
            });
        let helped = (0..statement.len())
            .any(|n| HELPERS.iter().any(|h| at(statement, n, h)) && word_start(statement, n));
        if !(acquired || helped) {
            i = end + 1;
            continue;
        }
        // Its name: the first identifier of the pattern, after `mut`.
        let mut n = 3;
        while statement.get(n) == Some(&'(') {
            n += 1;
        }
        if at(statement, n, "mut") {
            n += 3;
        }
        let name: String = statement[n..]
            .iter()
            .take_while(|c| c.is_alphanumeric() || **c == '_')
            .collect();
        let dropped = format!("drop({name})");
        // To the end of its block, or to where it is given back.
        let (mut depth, mut j) = (0i32, end + 1);
        while j < code.len() {
            match code[j] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        break;
                    }
                }
                _ => {}
            }
            if at(&code, j, &dropped) {
                break;
            }
            if at(&code, j, "core.lock()") {
                found.push((lines[i], lines[j]));
            }
            j += 1;
        }
        i = end + 1;
    }
    found
}

#[test]
fn the_global_lock_is_never_waited_for_with_a_pooled_connection_in_hand() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = vec![];
    sources(&root, &mut files);
    assert!(files.len() > 50, "the server's sources were found");
    let mut found = vec![];
    for path in files {
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\r\n", "\n");
        for (bound, lock) in violations(&text) {
            let name = path.strip_prefix(&root).unwrap().display().to_string();
            found.push(format!(
                "{name}:{lock} waits for the lock with the connection bound at line {bound}"
            ));
        }
    }
    assert!(
        found.is_empty(),
        "take the lock first, or give the connection back before it (see http::budgeted):\n{}",
        found.join("\n")
    );
}

#[test]
fn the_check_sees_what_it_is_for() {
    let bad = "async fn f(app: App) {\n    let mut c = app.0.pool.acquire().await?;\n    app.0\n        .core\n        .lock()\n        .await;\n}\n";
    assert_eq!(violations(bad), vec![(2, 4)]);
    let helper = "async fn f() {\n    let (mut c, u) = who(&app, &h).await?;\n    let core = app.0.core.lock().await;\n}\n";
    assert_eq!(violations(helper), vec![(2, 3)]);
    // Given back first, in a block of its own, used for one statement only,
    // or taken after the lock.
    for good in [
        "async fn f() {\n    let c = app.0.pool.acquire().await?;\n    drop(c);\n    app.0.core.lock().await;\n}\n",
        "async fn f() {\n    let u = {\n        let mut c = app.0.pool.acquire().await?;\n        auth(&mut c).await?\n    };\n    app.0.core.lock().await;\n}\n",
        "async fn f() {\n    let (_, u) = auth(&mut *app.0.pool.acquire().await?, t).await?;\n    app.0.core.lock().await;\n}\n",
        "async fn f() {\n    let mut core = app.0.core.lock().await;\n    let mut c = app.0.pool.acquire().await?;\n}\n",
    ] {
        assert_eq!(violations(good), vec![], "{good}");
    }
}
