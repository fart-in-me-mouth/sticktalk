//! Offline release utility. Private keys are never installed with the application.
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair},
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use sticktalk_desktop_platform::updater::{Updater, TARGET};
use sticktalk_proto::{SignedUpdateManifest, UpdateManifest};
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") if args.len() == 2 => {
            let path = Path::new(&args[1]);
            outside_repository(path)?;
            let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| "key generation failed")?;
            let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(path)?;
            file.write_all(doc.as_ref())?;
            file.sync_all()?;
            let key = Ed25519KeyPair::from_pkcs8(doc.as_ref()).map_err(|_| "invalid key")?;
            println!("{}", STANDARD.encode(key.public_key().as_ref()));
        }
        Some("sign") if args.len() == 6 => {
            // sign <private-key> <installer> <version> <origin> <envelope-output>
            outside_repository(Path::new(&args[1]))?;
            let key = Ed25519KeyPair::from_pkcs8(&std::fs::read(&args[1])?).map_err(|_| "invalid key")?;
            let public = option_env!("STICKTALK_UPDATE_PUBLIC_KEY").map(|p| STANDARD.decode(p)).transpose()?;
            if public.is_some_and(|bytes| bytes.as_slice() != key.public_key().as_ref()) {
                return Err("Signing key does not match the public key compiled into this release tool/client bundle".into());
            }
            let mut file = std::fs::File::open(&args[2])?;
            let byte_size = file.metadata()?.len();
            let mut hash = Sha256::new();
            let mut buffer = [0; 65536];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 { break; }
                hash.update(&buffer[..n]);
            }
            let sha256 = format!("{:x}", hash.finalize());
            let manifest = serde_json::to_string(&UpdateManifest {
                version: args[3].clone(), target: TARGET.into(), byte_size,
                url: format!("{}/api/update/artifact?sha256={sha256}",args[4].trim_end_matches('/')), sha256,
            })?;
            let envelope = SignedUpdateManifest { signature: STANDARD.encode(key.sign(manifest.as_bytes()).as_ref()), manifest };
            // Reuse the exact client policy before publishing anything.
            let verifier = Updater::new(&args[4], "0.0.0", std::env::temp_dir(), key.public_key().as_ref().try_into()?)?;
            verifier.verify(envelope.clone())?;
            std::fs::write(&args[5], serde_json::to_vec(&envelope)?)?;
        }
        Some("stage") if args.len() == 4 => {
            let updater = Updater::embedded(&args[1], &args[2], PathBuf::from(&args[3]))?;
            let update = updater.check().await?.ok_or("no newer update")?;
            let stage = updater.download(&update).await?;
            println!("Staged {}", stage.version());
        }
        _ => return Err("keygen <key> | sign <key> <exe> <version> <origin> <json> | stage <origin> <installed-version> <staging>".into()),
    }
    Ok(())
}
fn outside_repository(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path
        .parent()
        .ok_or("key needs an absolute external directory")?
        .canonicalize()?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .canonicalize()?;
    if !path.is_absolute() || parent.starts_with(repo) {
        return Err("Private key must be outside the repository".into());
    }
    Ok(())
}
