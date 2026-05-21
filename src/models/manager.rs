use std::path::{Path, PathBuf};

use crate::{Error, Result};

use super::catalog::{Catalog, CatalogEntry, ModelFamily};

/// A model ready to use: path to the file or directory and engine family.
///
/// `family` is `Some` for transcription models; `None` for VAD models.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub name: String,
    pub path: PathBuf,
    pub family: Option<ModelFamily>,
    pub is_directory: bool,
}

pub struct Manager {
    dir: PathBuf,
    catalog: Catalog,
}

impl Manager {
    pub fn new(dir: PathBuf, catalog: Catalog) -> Self {
        Self { dir, catalog }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Print to stdout: catalog entries and locally present files/directories.
    pub fn print_list(&self) {
        println!("Catalog:");
        for entry in self.catalog.entries() {
            let local = self.dir.join(&entry.filename);
            let mark = if local.exists() {
                "[present]"
            } else {
                "[remote]"
            };
            let kind = if entry.is_directory { "dir" } else { "file" };
            let family_str = match entry.family {
                Some(f) => format!("{f:?}"),
                None => "vad".to_string(),
            };
            println!(
                "  {mark} {name:<20} family={family} kind={kind} fs={file}",
                mark = mark,
                name = entry.name,
                family = family_str,
                kind = kind,
                file = entry.filename,
            );
            if let Some(desc) = &entry.description {
                println!("           {desc}");
            }
        }

        println!("\nLocal models dir: {}", self.dir.display());
        match std::fs::read_dir(&self.dir) {
            Ok(read) => {
                for ent in read.flatten() {
                    let kind = match ent.file_type() {
                        Ok(t) if t.is_dir() => "dir ",
                        Ok(t) if t.is_file() => "file",
                        _ => "?   ",
                    };
                    println!("  {kind} {}", ent.file_name().to_string_lossy());
                }
            }
            Err(e) => println!("  (cannot read: {e})"),
        }
    }

    /// Download a model from the catalog into the local directory.
    ///
    /// Idempotent: if the file/directory is already present and (for single files) sha256
    /// matches, no re-download occurs. For tar.gz models re-checking sha256 would require
    /// re-downloading the archive -- directory presence is treated as sufficient.
    pub fn pull(&self, name: &str) -> Result<()> {
        let entry = self
            .catalog
            .find(name)
            .ok_or_else(|| Error::Model(format!("unknown model `{name}` (see `models list`)")))?;

        if entry.url.trim().is_empty() {
            return Err(Error::Model(format!(
                "catalog entry `{name}` has no URL -- fill in config/models.toml"
            )));
        }

        std::fs::create_dir_all(&self.dir)
            .map_err(|e| Error::Model(format!("create models dir: {e}")))?;

        let dest = self.dir.join(&entry.filename);

        if entry.is_directory {
            if dest.is_dir() {
                tracing::info!(path = %dest.display(), "model directory already present, skipping");
                return Ok(());
            }
            return pull_archive(entry, &self.dir, &dest);
        }

        // Single file.
        if dest.exists() {
            match verify_sha256(&dest, entry.sha256.as_deref())? {
                ShaCheck::Ok => {
                    tracing::info!(path = %dest.display(), "model already present and verified");
                    return Ok(());
                }
                ShaCheck::SkippedNoExpected => {
                    tracing::info!(path = %dest.display(), "model already present (no sha256 in catalog)");
                    return Ok(());
                }
                ShaCheck::Mismatch { actual, expected } => {
                    tracing::warn!(actual, expected, "existing file failed sha256 -- re-downloading");
                }
            }
        }

        download_file(entry, &dest)?;

        match verify_sha256(&dest, entry.sha256.as_deref())? {
            ShaCheck::Ok | ShaCheck::SkippedNoExpected => Ok(()),
            ShaCheck::Mismatch { actual, expected } => {
                let _ = std::fs::remove_file(&dest);
                Err(Error::Model(format!(
                    "sha256 mismatch after download: expected {expected}, got {actual}"
                )))
            }
        }
    }

    /// Download a VAD model from the catalog into the local directory.
    /// Mirror of `pull` but routes through the VAD section.
    pub fn pull_vad(&self, name: &str) -> Result<()> {
        let entry = self
            .catalog
            .find_vad(name)
            .ok_or_else(|| Error::Model(format!("unknown VAD model `{name}`")))?;

        if entry.url.trim().is_empty() {
            return Err(Error::Model(format!(
                "VAD catalog entry `{name}` has no URL"
            )));
        }

        std::fs::create_dir_all(&self.dir)
            .map_err(|e| Error::Model(format!("create models dir: {e}")))?;

        let dest = self.dir.join(&entry.filename);

        if dest.exists() {
            match verify_sha256(&dest, entry.sha256.as_deref())? {
                ShaCheck::Ok => {
                    tracing::info!(path = %dest.display(), "VAD model already present and verified");
                    return Ok(());
                }
                ShaCheck::SkippedNoExpected => {
                    tracing::info!(path = %dest.display(), "VAD model already present (no sha256 in catalog)");
                    return Ok(());
                }
                ShaCheck::Mismatch { actual, expected } => {
                    tracing::warn!(actual, expected, "existing VAD file failed sha256 -- re-downloading");
                }
            }
        }

        download_file(entry, &dest)?;

        match verify_sha256(&dest, entry.sha256.as_deref())? {
            ShaCheck::Ok | ShaCheck::SkippedNoExpected => Ok(()),
            ShaCheck::Mismatch { actual, expected } => {
                let _ = std::fs::remove_file(&dest);
                Err(Error::Model(format!(
                    "sha256 mismatch after VAD download: expected {expected}, got {actual}"
                )))
            }
        }
    }

    /// Resolve a VAD model name to a ResolvedModel (family is always None).
    pub fn resolve_vad(&self, name: &str) -> Result<ResolvedModel> {
        let entry = self
            .catalog
            .find_vad(name)
            .ok_or_else(|| Error::Model(format!("unknown VAD model `{name}`")))?;

        let local = self.dir.join(&entry.filename);
        if !local.is_file() {
            return Err(Error::Model(format!(
                "VAD model `{}` not present at {} -- run `{} models pull {}` first",
                entry.name,
                local.display(),
                env!("CARGO_PKG_NAME"),
                entry.name
            )));
        }
        if let ShaCheck::Mismatch { actual, expected } =
            verify_sha256(&local, entry.sha256.as_deref())?
        {
            return Err(Error::Model(format!(
                "VAD model `{}` failed sha256: expected {expected}, got {actual}",
                entry.name
            )));
        }
        Ok(ResolvedModel {
            name: entry.name.clone(),
            path: local,
            family: None,
            is_directory: false,
        })
    }

    /// Resolve a `--model` argument (catalog name OR file path) into a ResolvedModel.
    /// For a direct path `family_hint` is required; otherwise the family is inferred from
    /// structure (single file -> Whisper, directory -> error).
    pub fn resolve(
        &self,
        model_arg: &str,
        family_hint: Option<ModelFamily>,
    ) -> Result<ResolvedModel> {
        let direct = Path::new(model_arg);
        if direct.exists() {
            let is_dir = direct.is_dir();
            let family = match (family_hint, is_dir) {
                (Some(f), _) => f,
                (None, false) => ModelFamily::Whisper,
                (None, true) => {
                    return Err(Error::Model(format!(
                        "direct directory path `{}` requires `--family` (parakeet|giga-am|whisper)",
                        direct.display()
                    )))
                }
            };
            return Ok(ResolvedModel {
                name: direct
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| model_arg.to_string()),
                path: direct.to_path_buf(),
                family: Some(family),
                is_directory: is_dir,
            });
        }

        if let Some(entry) = self.catalog.find(model_arg) {
            let local = self.dir.join(&entry.filename);
            if entry.is_directory {
                if !local.is_dir() {
                    return Err(Error::Model(format!(
                        "model `{}` not present at {} -- run `{} models pull {}` first",
                        entry.name,
                        local.display(),
                        env!("CARGO_PKG_NAME"),
                        entry.name
                    )));
                }
            } else {
                if !local.is_file() {
                    return Err(Error::Model(format!(
                        "model `{}` not present at {} -- run `{} models pull {}` first",
                        entry.name,
                        local.display(),
                        env!("CARGO_PKG_NAME"),
                        entry.name
                    )));
                }
                if let ShaCheck::Mismatch { actual, expected } =
                    verify_sha256(&local, entry.sha256.as_deref())?
                {
                    return Err(Error::Model(format!(
                        "model `{}` failed sha256: expected {expected}, got {actual}",
                        entry.name
                    )));
                }
            }
            return Ok(ResolvedModel {
                name: entry.name.clone(),
                path: local,
                family: entry.family,  // already Option<ModelFamily>
                is_directory: entry.is_directory,
            });
        }

        Err(Error::Model(format!(
            "`{model_arg}` is neither a known catalog name nor an existing path"
        )))
    }

    /// Download a model from a URL, compute sha256/size, and register it in the
    /// external catalog file. Family must be specified; name is derived from the
    /// URL filename unless overridden.
    pub fn add(
        &self,
        url: &str,
        family: ModelFamily,
        name_override: Option<&str>,
    ) -> Result<()> {
        let url_path = url.split('?').next().unwrap_or(url);
        let url_filename = url_path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Model("cannot determine filename from URL".into()))?;

        let is_directory = url_filename.ends_with(".tar.gz") || url_filename.ends_with(".tgz");

        let filename = if is_directory {
            url_filename
                .strip_suffix(".tar.gz")
                .or_else(|| url_filename.strip_suffix(".tgz"))
                .unwrap_or(url_filename)
                .to_string()
        } else {
            url_filename.to_string()
        };

        let name = name_override
            .map(|s| s.to_string())
            .unwrap_or_else(|| derive_name(&filename));

        if self.catalog.find(&name).is_some() {
            return Err(Error::Model(format!(
                "model `{name}` already exists in catalog"
            )));
        }

        std::fs::create_dir_all(&self.dir)
            .map_err(|e| Error::Model(format!("create models dir: {e}")))?;

        let (sha256, size_bytes) = add_download(url, &name, is_directory, &filename, &self.dir)?;

        let entry = CatalogEntry {
            name: name.clone(),
            family: Some(family),
            filename,
            url: url.to_string(),
            sha256: Some(sha256),
            size_bytes: Some(size_bytes),
            description: None,
            is_directory,
        };

        Catalog::append_to_file(&entry)?;
        println!("Added `{name}` to catalog");
        Ok(())
    }
}

fn derive_name(filename: &str) -> String {
    filename
        .strip_suffix(".bin")
        .or_else(|| filename.strip_suffix(".gguf"))
        .unwrap_or(filename)
        .to_string()
}

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
fn add_download(
    _url: &str,
    _name: &str,
    _is_directory: bool,
    _filename: &str,
    _models_dir: &Path,
) -> Result<(String, u64)> {
    Err(Error::NotImplemented(
        "download: enable feature `download` to add models by URL",
    ))
}

#[cfg(feature = "download")]
fn add_download(
    url: &str,
    name: &str,
    is_directory: bool,
    filename: &str,
    models_dir: &Path,
) -> Result<(String, u64)> {
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
        size_bytes: None,
        description: None,
        is_directory: false, // download as a single file first
    };
    download_file(&tmp_entry, &download_path)?;

    let size_bytes = std::fs::metadata(&download_path)
        .map_err(|e| Error::Model(format!("stat {}: {e}", download_path.display())))?
        .len();
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

    Ok((sha256, size_bytes))
}

enum ShaCheck {
    Ok,
    SkippedNoExpected,
    Mismatch { actual: String, expected: String },
}

fn verify_sha256(path: &Path, expected: Option<&str>) -> Result<ShaCheck> {
    let expected = match expected {
        Some(s) if !s.trim().is_empty() => s.trim().to_ascii_lowercase(),
        _ => return Ok(ShaCheck::SkippedNoExpected),
    };

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
    let actual = hex::encode(hasher.finalize());

    if actual == expected {
        Ok(ShaCheck::Ok)
    } else {
        Ok(ShaCheck::Mismatch { actual, expected })
    }
}

#[cfg(not(feature = "download"))]
fn download_file(_entry: &CatalogEntry, _dest: &Path) -> Result<()> {
    Err(Error::NotImplemented(
        "download: enable feature `download` to fetch models",
    ))
}

#[cfg(not(feature = "download"))]
fn pull_archive(_entry: &CatalogEntry, _dir: &Path, _dest_dir: &Path) -> Result<()> {
    Err(Error::NotImplemented(
        "download: enable feature `download` to fetch models",
    ))
}

#[cfg(feature = "download")]
fn download_file(entry: &CatalogEntry, dest: &Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Model(format!("tokio runtime: {e}")))?;
    runtime.block_on(download_to_path(
        &entry.name,
        &entry.url,
        entry.size_bytes,
        dest,
    ))
}

/// Download and extract a tar.gz model.
///
/// The archive is first downloaded to `<dest_dir>.tar.gz.part`, then (if sha256 is set
/// in the catalog) the archive hash is verified, then extracted to a temporary directory
/// `<dest_dir>.extracting` and atomically renamed to `dest_dir`.
#[cfg(feature = "download")]
fn pull_archive(entry: &CatalogEntry, models_dir: &Path, dest_dir: &Path) -> Result<()> {
    let archive_tmp = models_dir.join(format!("{}.tar.gz.part", entry.filename));
    let extracting = models_dir.join(format!("{}.extracting", entry.filename));

    if extracting.exists() {
        let _ = std::fs::remove_dir_all(&extracting);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Model(format!("tokio runtime: {e}")))?;

    runtime.block_on(download_to_path(
        &entry.name,
        &entry.url,
        entry.size_bytes,
        &archive_tmp,
    ))?;

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
async fn download_to_path(
    label: &str,
    url: &str,
    expected_size: Option<u64>,
    dest: &Path,
) -> Result<()> {
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

    let total = response.content_length().or(expected_size);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_test_dir(suffix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("notes-capture-test-{suffix}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_vad_returns_path_when_model_present() {
        let dir = temp_test_dir("vad-present");
        let model_path = dir.join("silero_vad.onnx");
        fs::write(&model_path, b"fake-onnx").unwrap();

        // Build a manager with no sha256 so verification is skipped.
        let catalog_src = r#"
[model]
[[model.vad]]
name = "silero-vad-test"
filename = "silero_vad.onnx"
url = "https://example.com/silero_vad.onnx"
is_directory = false
"#;
        let (transcribe, vad) =
            crate::models::catalog::Catalog::parse_toml_pub(catalog_src, "test").unwrap();
        let catalog = crate::models::Catalog::from_parts(transcribe, vad);
        let mgr = Manager::new(dir.clone(), catalog);

        let resolved = mgr.resolve_vad("silero-vad-test").unwrap();
        assert_eq!(resolved.path, model_path);
        assert!(resolved.family.is_none());
    }

    #[test]
    fn resolve_vad_errors_when_model_absent() {
        let dir = temp_test_dir("vad-absent");
        let catalog_src = r#"
[model]
[[model.vad]]
name = "silero-vad-test"
filename = "silero_vad.onnx"
url = "https://example.com/silero_vad.onnx"
is_directory = false
"#;
        let (transcribe, vad) =
            crate::models::catalog::Catalog::parse_toml_pub(catalog_src, "test").unwrap();
        let catalog = crate::models::Catalog::from_parts(transcribe, vad);
        let mgr = Manager::new(dir.clone(), catalog);

        assert!(mgr.resolve_vad("silero-vad-test").is_err());
    }
}
