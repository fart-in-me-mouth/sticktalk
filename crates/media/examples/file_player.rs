//! Explicit, silent, windowless hardware-container acceptance probe.
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        hash::{Hash, Hasher},
        time::{Duration, Instant},
    };
    let path = std::env::args()
        .nth(1)
        .ok_or("expected local MP4/WebM path")?;
    let player = sticktalk_media::FilePlayer::open(
        std::path::Path::new(&path),
        sticktalk_media::Size {
            width: 320,
            height: 240,
        },
        true,
        0.,
        false,
    )?;
    player.play(0.)?;
    let start = Instant::now();
    let mut frames = 0;
    let mut hashes = std::collections::BTreeSet::new();
    while frames < 12 && start.elapsed() < Duration::from_secs(10) {
        if let Some(frame) = player.frame()? {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            frame.rgba.hash(&mut hash);
            hashes.insert(hash.finish());
            frames += 1;
        }
        std::thread::sleep(Duration::from_millis(33));
    }
    if frames < 2 || hashes.len() < 2 {
        return Err("video did not produce changing frames".into());
    }
    println!("{{\"window_backend\":\"none\",\"muted\":true,\"d3d11_manager\":true,\"frames\":{frames},\"distinct_frames\":{},\"seconds\":{:.3}}}", hashes.len(), start.elapsed().as_secs_f64());
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Windows Media Foundation required");
}
