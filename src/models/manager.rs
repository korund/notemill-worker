use std::path::{Path, PathBuf};

use crate::engine::EngineKind;
use crate::{Error, Result};

use super::catalog::{Catalog, CatalogEntry};

/// A model ready to use: path to the file and engine family.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub name: String,
    pub path: PathBuf,
    pub engine_kind: EngineKind,
}

pub struct Manager {
    dir: PathBuf,
    catalog: Catalog,
}

impl Manager {
    pub fn new(dir: PathBuf, catalog: Catalog) -> Self {
        Self { dir, catalog }
    }

    /// Print to stdout: catalog entries and locally present files.
    pub fn print_list(&self) {
        println!("Catalog:");
        for entry in self.catalog.entries() {
            let local = self.dir.join(&entry.filename);
            let mark = if local.exists() { "[present]" } else { "[remote]" };
            println!(
                "  {mark} {name:<32} family={family:?} file={file}",
                mark = mark,
                name = entry.name,
                family = entry.family,
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
                    if ent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                        println!("  - {}", ent.file_name().to_string_lossy());
                    }
                }
            }
            Err(e) => println!("  (cannot read: {e})"),
        }
    }

    /// Download a model from the catalog into the local directory.
    pub fn pull(&self, name: &str) -> Result<()> {
        let entry = self
            .catalog
            .find(name)
            .ok_or_else(|| Error::Model(format!("unknown model `{name}` (see `models list`)")))?;

        std::fs::create_dir_all(&self.dir)
            .map_err(|e| Error::Model(format!("create models dir: {e}")))?;

        let dest = self.dir.join(&entry.filename);
        download(entry, &dest)?;
        Ok(())
    }

    /// Resolve a `--model` argument (catalog name OR file path) into a ResolvedModel.
    pub fn resolve(&self, model_arg: &str) -> Result<ResolvedModel> {
        let direct = Path::new(model_arg);
        if direct.is_file() {
            // Direct path -- engine family is not auto-detected yet.
            // TODO: auto-detect engine family from magic bytes / extension.
            return Err(Error::Model(format!(
                "direct model paths require explicit family detection (not implemented yet); \
                 prefer a name from the catalog: {model_arg}"
            )));
        }

        if let Some(entry) = self.catalog.find(model_arg) {
            let local = self.dir.join(&entry.filename);
            if !local.exists() {
                return Err(Error::Model(format!(
                    "model `{}` not present at {} -- run `notes-capture models pull {}` first",
                    entry.name,
                    local.display(),
                    entry.name
                )));
            }
            // TODO: verify sha256 when present.
            return Ok(ResolvedModel {
                name: entry.name.clone(),
                path: local,
                engine_kind: entry.family.engine_kind(),
            });
        }

        Err(Error::Model(format!(
            "`{model_arg}` is neither a known catalog name nor an existing file"
        )))
    }
}

#[cfg(not(feature = "download"))]
fn download(_entry: &CatalogEntry, _dest: &Path) -> Result<()> {
    Err(Error::NotImplemented(
        "download: enable feature `download` (reqwest+tokio) to fetch models",
    ))
}

#[cfg(feature = "download")]
fn download(_entry: &CatalogEntry, _dest: &Path) -> Result<()> {
    // TODO: streaming reqwest GET -> file, progress bar, sha256 verification.
    Err(Error::Model("download not implemented yet".into()))
}
