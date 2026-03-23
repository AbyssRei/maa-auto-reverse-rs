use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub project_root: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub bundle_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project_root = std::env::current_dir().context("无法解析当前工作目录")?;

        Ok(Self {
            config_dir: project_root.join("config"),
            data_dir: project_root.join("data"),
            logs_dir: project_root.join("logs"),
            runtime_dir: project_root.join("runtime").join("bin"),
            bundle_dir: project_root.join("resource").join("autoreverse_bundle"),
            project_root,
        })
    }
}

pub fn app_paths() -> Result<AppPaths> {
    AppPaths::discover()
}

pub fn ensure_app_dirs() -> Result<()> {
    let paths = app_paths()?;
    std::fs::create_dir_all(&paths.config_dir)?;
    std::fs::create_dir_all(&paths.data_dir)?;
    std::fs::create_dir_all(&paths.logs_dir)?;
    Ok(())
}

pub fn file_in_data(name: &str) -> Result<PathBuf> {
    Ok(app_paths()?.data_dir.join(name))
}

pub fn file_in_config(name: &str) -> Result<PathBuf> {
    Ok(app_paths()?.config_dir.join(name))
}
