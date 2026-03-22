use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub project_root: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub legacy_root: PathBuf,
    pub legacy_config_dir: PathBuf,
    pub legacy_runtime_dir: PathBuf,
    pub legacy_bundle_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project_root = std::env::current_dir().context("无法解析当前工作目录")?;
        let repo_root = project_root
            .parent()
            .ok_or_else(|| anyhow!("无法定位仓库根目录"))?
            .to_path_buf();
        let legacy_root = repo_root.join("MaaAutoReverse");

        Ok(Self {
            data_dir: project_root.join("data"),
            logs_dir: project_root.join("logs"),
            legacy_config_dir: legacy_root.join("config"),
            legacy_runtime_dir: legacy_root.join("runtime").join("bin"),
            legacy_bundle_dir: legacy_root.join("resource").join("autoreverse_bundle"),
            project_root,
            legacy_root,
        })
    }
}

pub fn app_paths() -> Result<AppPaths> {
    AppPaths::discover()
}

pub fn ensure_app_dirs() -> Result<()> {
    let paths = app_paths()?;
    std::fs::create_dir_all(&paths.data_dir)?;
    std::fs::create_dir_all(&paths.logs_dir)?;
    Ok(())
}

pub fn file_in_data(name: &str) -> Result<PathBuf> {
    Ok(app_paths()?.data_dir.join(name))
}

pub fn legacy_exists(path: &Path) -> bool {
    path.exists()
}
