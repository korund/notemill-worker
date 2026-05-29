//! Model download and archive extraction.
//!
//! All network/streaming code lives here behind the `download` feature.
//! When the feature is off, the entry points return `NotImplemented` so the
//! manager still compiles without pulling in reqwest and friends.

use std::path::Path;

use crate::{Error, Result};

use super::catalog::CatalogEntry;

#[cfg(feature = "download")]
use super::hash::{verify_sha256, ShaCheck};

#[cfg(not(feature = "download"))]
pub(crate) fn add_download(
    _url: &str,
    _name: &str,
    _is_directory: bool,
    _filename: &str,
    _models_dir: &Path,
) -> Result<String> {
    Err(Error::NotImplemented(
        "download: enable feature `download` to add models by URL",
    ))
}

#[cfg(feature = "download")]
pub(crate) fn add_download(
    url: &str,
    name: &str,
    is_directory: bool,
    filename: &str,
    models_dir: &Path,
) -> Result<String> {
    let download_path = if is_directory {
        models_dir.join(format!("{filename}.tar.gz.part"))
    } else {
        models_dir.join(filename)
    };

    let tmp_entry = CatalogEntry {
        name: name.to_string(),
        family: None, // unused for download, just a placeholder
        filename: filename.to_string(),
        url: url.to_string(),
        sha256: None,
        description: None,
        is_directory: false, // download as a single file first
    };
    download_file(&tmp_entry, &download_path)?;

    let sha256 = compute_sha256(&download_path)?;

    if is_directory {
        let dest_dir = models_dir.join(filename);
        let extracting = models_dir.join(format!("{filename}.extracting"));
        if extracting.exists() {
            let _ = std::fs::remove_dir_all(&extracting);
        }
        extract_targz(&download_path, &extracting)?;
        std::fs::rename(&extracting, &dest_dir).map_err(|e| {
            Error::Model(format!(
                "rename {} -> {}: {e}",
                extracting.display(),
                dest_dir.display()
            ))
        })?;
        let _ = std::fs::remove_file(&download_path);
    }

    Ok(sha256)
}

#[cfg(feature = "download")]
fn compute_sha256(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|e| Error::Model(format!("open {} for hashing: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| Error::Model(format!("read {} for hashing: {e}", path.display())))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(not(feature = "download"))]
pub(crate) fn download_file(_entry: &CatalogEntry, _dest: &Path) -> Result<()> {
    Err(Error::NotImplemented(
        "download: enable feature `download` to fetch models",
    ))
}

#[cfg(feature = "download")]
pub(crate) fn download_file(entry: &CatalogEntry, dest: &Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Model(format!("tokio runtime: {e}")))?;
    runtime.block_on(download_to_path(&entry.name, &entry.url, dest))
}

#[cfg(not(feature = "download"))]
pub(crate) fn pull_archive(_entry: &CatalogEntry, _dir: &Path, _dest_dir: &Path) -> Result<()> {
    Err(Error::NotImplemented(
        "download: enable feature `download` to fetch models",
    ))
}

/// Download and extract a tar.gz model.
///
/// The archive is first downloaded to `<dest_dir>.tar.gz.part`, then (if sha256 is set
/// in the catalog) the archive hash is verified, then extracted to a temporary directory
/// `<dest_dir>.extracting` and atomically renamed to `dest_dir`.
#[cfg(feature = "download")]
pub(crate) fn pull_archive(entry: &CatalogEntry, models_dir: &Path, dest_dir: &Path) -> Result<()> {
    let archive_tmp = models_dir.join(format!("{}.tar.gz.part", entry.filename));
    let extracting = models_dir.join(format!("{}.extracting", entry.filename));

    if extracting.exists() {
        let _ = std::fs::remove_dir_all(&extracting);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Model(format!("tokio runtime: {e}")))?;

    runtime.block_on(download_to_path(&entry.name, &entry.url, &archive_tmp))?;

    if let ShaCheck::Mismatch { actual, expected } =
        verify_sha256(&archive_tmp, entry.sha256.as_deref())?
    {
        let _ = std::fs::remove_file(&archive_tmp);
        return Err(Error::Model(format!(
            "sha256 mismatch on archive: expected {expected}, got {actual}"
        )));
    }

    extract_targz(&archive_tmp, &extracting)?;
    std::fs::rename(&extracting, dest_dir).map_err(|e| {
        Error::Model(format!(
            "rename {} -> {}: {e}",
            extracting.display(),
            dest_dir.display()
        ))
    })?;
    let _ = std::fs::remove_file(&archive_tmp);
    Ok(())
}

#[cfg(feature = "download")]
fn extract_targz(archive: &Path, dest: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    std::fs::create_dir_all(dest)
        .map_err(|e| Error::Model(format!("mkdir {}: {e}", dest.display())))?;

    let f = std::fs::File::open(archive)
        .map_err(|e| Error::Model(format!("open archive {}: {e}", archive.display())))?;
    let dec = GzDecoder::new(f);
    let mut tar = Archive::new(dec);

    // handy tar.gz archives are packed as "root-folder/files". Extract as-is, then
    // if dest contains exactly one directory, hoist its contents up.
    tar.unpack(dest)
        .map_err(|e| Error::Model(format!("untar to {}: {e}", dest.display())))?;

    flatten_single_root(dest)?;
    Ok(())
}

/// If `dir` contains exactly one subdirectory and nothing else, hoist its
/// contents into `dir`. This keeps the models directory layout consistent
/// regardless of how each model archive is packed.
#[cfg(feature = "download")]
fn flatten_single_root(dir: &Path) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| Error::Model(format!("read_dir {}: {e}", dir.display())))?
        .filter_map(|e| e.ok())
        .collect();
    if entries.len() != 1 {
        return Ok(());
    }
    let only = entries.remove(0);
    if !only.file_type().map(|t| t.is_dir()).unwrap_or(false) {
        return Ok(());
    }
    let inner = only.path();
    for sub in std::fs::read_dir(&inner)
        .map_err(|e| Error::Model(format!("read_dir {}: {e}", inner.display())))?
    {
        let sub = sub.map_err(|e| Error::Model(format!("read_dir entry: {e}")))?;
        let target = dir.join(sub.file_name());
        std::fs::rename(sub.path(), &target).map_err(|e| {
            Error::Model(format!(
                "rename {} -> {}: {e}",
                sub.path().display(),
                target.display()
            ))
        })?;
    }
    std::fs::remove_dir(&inner)
        .map_err(|e| Error::Model(format!("rmdir {}: {e}", inner.display())))?;
    Ok(())
}

#[cfg(feature = "download")]
async fn download_to_path(label: &str, url: &str, dest: &Path) -> Result<()> {
    use futures_util::StreamExt;
    use indicatif::{ProgressBar, ProgressStyle};
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Model(format!("http client: {e}")))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Model(format!("GET {url}: {e}")))?
        .error_for_status()
        .map_err(|e| Error::Model(format!("GET {url}: {e}")))?;

    let total = response.content_length();
    let pb = match total {
        Some(len) => {
            let bar = ProgressBar::new(len);
            bar.set_style(
                ProgressStyle::with_template(
                    "{msg}\n{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, ETA {eta})",
                )
                .unwrap()
                .progress_chars("##-"),
            );
            bar
        }
        None => ProgressBar::new_spinner(),
    };
    pb.set_message(format!("Downloading {label}"));

    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Model(format!("mkdir {}: {e}", parent.display())))?;
        }
    }

    {
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| Error::Model(format!("create {}: {e}", dest.display())))?;

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| Error::Model(format!("download chunk: {e}")))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| Error::Model(format!("write {}: {e}", dest.display())))?;
            pb.inc(chunk.len() as u64);
        }
        file.flush()
            .await
            .map_err(|e| Error::Model(format!("flush {}: {e}", dest.display())))?;
    }
    pb.finish_with_message(format!("Downloaded {label}"));
    Ok(())
}
