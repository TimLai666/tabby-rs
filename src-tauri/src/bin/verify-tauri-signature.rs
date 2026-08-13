use std::{env, fs, path::Path, process};

use tabby_rs::update::service::verify_signature;

fn usage() -> ! {
    eprintln!("usage: verify-tauri-signature <artifact> <signature> <public-key>");
    process::exit(2);
}

fn read(path: &str) -> Vec<u8> {
    fs::read(Path::new(path)).unwrap_or_else(|error| {
        eprintln!("failed to read verifier input: {error}");
        process::exit(2);
    })
}

fn main() {
    let mut args = env::args().skip(1);
    let (Some(artifact_path), Some(signature_path), Some(public_key_path)) =
        (args.next(), args.next(), args.next())
    else {
        usage();
    };
    if args.next().is_some() {
        usage();
    }

    let artifact = read(&artifact_path);
    let signature = String::from_utf8(read(&signature_path)).unwrap_or_else(|_| {
        eprintln!("signature is not valid UTF-8");
        process::exit(2);
    });
    let public_key = String::from_utf8(read(&public_key_path)).unwrap_or_else(|_| {
        eprintln!("public key is not valid UTF-8");
        process::exit(2);
    });

    if let Err(error) = verify_signature(&artifact, &signature, &public_key) {
        eprintln!("updater signature verification failed: {error}");
        process::exit(1);
    }
}
