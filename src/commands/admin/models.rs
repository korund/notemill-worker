use std::path::PathBuf;

use crate::cli::ModelsCommand;
use crate::{models, Result};

pub fn run(dir: Option<PathBuf>, cmd: ModelsCommand) -> Result<()> {
    let models_dir = dir.unwrap_or_else(|| PathBuf::from("models"));
    let catalog = models::Catalog::load()?;
    let manager = models::Manager::new(models_dir, catalog);
    match cmd {
        ModelsCommand::List => {
            manager.print_list();
            Ok(())
        }
        ModelsCommand::Pull { name } => manager.pull(&name),
        ModelsCommand::Add { url, family, name } => {
            manager.add(&url, family.into(), name.as_deref())
        }
    }
}
