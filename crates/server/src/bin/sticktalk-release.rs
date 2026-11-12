//! Signing helper used by tools/release/release.ps1. Never installed on the Pi.
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair},
};
use sha2::{Digest, Sha256};
use std::{io::Read, path::PathBuf};
use sticktalk_proto::{SignedUpdateManifest, UpdateManifest};
#[derive(clap::Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(clap::Subcommand)]
enum Command {
    Keygen {
        #[arg(long)]
        key: PathBuf,
    },
    Sign {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        artifact: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long)]
        server: String,
        #[arg(long)]
        output: PathBuf,
    },
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use clap::Parser;
    match Cli::parse().command {
        Command::Keygen { key } => {
            let doc = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
                .map_err(|_| "key generation failed")?;
            // create_new prevents accidental rotation or overwriting another secret.
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut f = options.open(&key)?;
            f.write_all(doc.as_ref())?;
            f.sync_all()?;
            let pair = Ed25519KeyPair::from_pkcs8(doc.as_ref()).map_err(|_| "invalid key")?;
            println!("{}", STANDARD.encode(pair.public_key().as_ref()));
        }
        Command::Sign {
            key,
            artifact,
            version,
            server,
            output,
        } => {
            let pair = Ed25519KeyPair::from_pkcs8(&std::fs::read(key)?)
                .map_err(|_| "invalid signing key")?;
            let mut f = std::fs::File::open(artifact)?;
            let len = f.metadata()?.len();
            let mut hasher = Sha256::new();
            let mut buf = [0; 65536];
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            let sha256: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let manifest = serde_json::to_string(&UpdateManifest {
                version,
                target: "x86_64-pc-windows-msvc".into(),
                url: format!(
                    "{}/api/update/artifact?sha256={sha256}",
                    server.trim_end_matches('/')
                ),
                byte_size: len,
                sha256,
            })?;
            let signature = STANDARD.encode(pair.sign(manifest.as_bytes()).as_ref());
            let signed = SignedUpdateManifest {
                manifest,
                signature,
            };
            sticktalk_server::updates::verify(
                &STANDARD.encode(pair.public_key().as_ref()),
                &signed,
            )
            .map_err(|_| "invalid release metadata")?;
            std::fs::write(output, serde_json::to_vec(&signed)?)?;
        }
    }
    Ok(())
}
