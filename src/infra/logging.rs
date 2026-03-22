use anyhow::Result;
use chrono::Local;
use crossbeam_channel::{Receiver, Sender, unbounded};
use once_cell::sync::OnceCell;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing_subscriber::FmtSubscriber;

use crate::domain::engine::SharedLogger;
use crate::infra::paths;

static LOGGER: OnceCell<SharedLogger> = OnceCell::new();
static SUBSCRIBERS: OnceCell<Mutex<Vec<Sender<String>>>> = OnceCell::new();

pub fn init_tracing() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
}

pub fn app_logger() -> Result<SharedLogger> {
    Ok(LOGGER
        .get_or_try_init(|| {
            let file = Arc::new(Mutex::new(open_log_file()?));
            Ok::<SharedLogger, anyhow::Error>(Arc::new(move |message: String| {
                let line = format!("[{}] {}", Local::now().format("%H:%M:%S"), message);
                tracing::info!("{line}");
                if let Ok(mut file) = file.lock() {
                    let _ = writeln!(file, "{line}");
                }
                if let Some(subscribers) = SUBSCRIBERS.get() {
                    if let Ok(subscribers) = subscribers.lock() {
                        for subscriber in subscribers.iter() {
                            let _ = subscriber.send(line.clone());
                        }
                    }
                }
            }))
        })?
        .clone())
}

fn open_log_file() -> Result<std::fs::File> {
    let logs_dir = paths::app_paths()?.logs_dir;
    std::fs::create_dir_all(&logs_dir)?;
    let path: PathBuf = logs_dir.join(format!(
        "maa_auto_reverse_{}.log",
        Local::now().format("%Y%m%d")
    ));
    Ok(OpenOptions::new().create(true).append(true).open(path)?)
}

pub fn subscribe_logs() -> Receiver<String> {
    let (sender, receiver) = unbounded();
    let subscribers = SUBSCRIBERS.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut subscribers) = subscribers.lock() {
        subscribers.push(sender);
    }
    receiver
}
