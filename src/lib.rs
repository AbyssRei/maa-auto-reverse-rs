pub mod app;
pub mod domain;
pub mod infra;
pub mod orchestrator;

use anyhow::Result;

pub fn bootstrap() -> Result<()> {
    infra::logging::init_tracing();
    infra::paths::ensure_app_dirs()?;
    infra::maa::ensure_library_loaded()?;
    Ok(())
}
