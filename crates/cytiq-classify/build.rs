//! Stage a pinned, verified CPU runtime alongside Cargo's binaries.
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

const VERSION: &str = "1.25.1";

fn distribution(target: &str) -> Result<(&'static str, &'static str), Box<dyn Error>> {
    match target {
        "aarch64-apple-darwin" => Ok((
            "osx-arm64",
            "18987ec3187b5f29ba798109750f6135060560ad4e0a52678fcc753ee8fb3091",
        )),
        "x86_64-unknown-linux-gnu" => Ok((
            "linux-x64",
            "eb566a49cfc49ef0642f809b69340b5bb656c7c4905ba873526d226f2c005816",
        )),
        "aarch64-unknown-linux-gnu" => Ok((
            "linux-aarch64",
            "daa71b56b00c4ab34798a3d96ca41a32ece4d3e302dc2386d3cca83fd4491214",
        )),
        _ => Err(format!("No bundled ONNX Runtime {VERSION} CPU distribution for {target}").into()),
    }
}

fn digest(path: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn download(cache: &Path, platform: &str, checksum: &str) -> Result<(), Box<dyn Error>> {
    if cache.exists() && digest(cache)? == checksum {
        return Ok(());
    }
    let url = format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{VERSION}/onnxruntime-{platform}-{VERSION}.tgz"
    );
    println!("cargo:warning=Downloading ONNX Runtime {VERSION} CPU for {platform}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(180)))
        .https_only(true)
        .build()
        .into();
    let mut response = agent.get(&url).call()?;
    let mut temporary = tempfile::NamedTempFile::new_in(cache.parent().unwrap())?;
    io::copy(
        &mut response.body_mut().as_reader().take(128 * 1024 * 1024),
        temporary.as_file_mut(),
    )?;
    if digest(temporary.path())? != checksum {
        return Err(format!("ONNX Runtime archive SHA-256 mismatch: {url}").into());
    }
    temporary.persist(cache)?;
    Ok(())
}

fn stage(archive: &Path, directory: &Path, platform: &str) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(directory)?;
    let root = format!("onnxruntime-{platform}-{VERSION}");
    let library = if platform.starts_with("osx") {
        format!("libonnxruntime.{VERSION}.dylib")
    } else {
        format!("libonnxruntime.so.{VERSION}")
    };
    let alias = if platform.starts_with("osx") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    };
    let mut found = Vec::new();
    let mut tar = tar::Archive::new(GzDecoder::new(File::open(archive)?));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?;
        let relative = path.strip_prefix("./").unwrap_or(&path);
        let relative = relative.strip_prefix(&root)?;
        let name = relative.to_string_lossy();
        let destination = if name == format!("lib/{library}") {
            alias
        } else if name == "lib/libonnxruntime_providers_shared.so" {
            "libonnxruntime_providers_shared.so"
        } else if name == "LICENSE" {
            "ONNX-RUNTIME-LICENSE"
        } else if name == "ThirdPartyNotices.txt" {
            "ThirdPartyNotices.txt"
        } else {
            continue;
        };
        // Only named regular files are extracted; archive paths cannot escape lib/.
        let output = directory.join(destination);
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        io::copy(&mut entry, temporary.as_file_mut())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o644))?;
        }
        temporary.persist(&output)?;
        println!("cargo:rerun-if-changed={}", output.display());
        found.push(destination);
    }
    for required in [alias, "ONNX-RUNTIME-LICENSE", "ThirdPartyNotices.txt"] {
        if !found.contains(&required) {
            return Err(format!("ONNX Runtime archive is missing {required}").into());
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    let target = env::var("TARGET")?;
    let (platform, checksum) = distribution(&target)?;
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is missing")?);
    let cache = out.join(format!("onnxruntime-{VERSION}-{platform}.tgz"));
    download(&cache, platform, checksum)?;
    // OUT_DIR is <target>/<profile>/build/<package>/out, including --target builds.
    let profile = out
        .ancestors()
        .nth(3)
        .ok_or("Unexpected Cargo OUT_DIR layout")?;
    stage(&cache, &profile.join("lib"), platform)?;
    Ok(())
}
