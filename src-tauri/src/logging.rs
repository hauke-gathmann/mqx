use mqx_core::AppDirs;

/// Keep logs private, rotated daily, and free of MQTT payloads and jq expressions.
pub fn init() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let init = || -> Result<_, Box<dyn std::error::Error + Send + Sync>> {
        let directory = AppDirs::new()?.cache_dir.join("logs");
        std::fs::create_dir_all(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let appender = tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("mqx")
            .filename_suffix("log")
            .max_log_files(5)
            .build(directory)?;
        let (writer, guard) = tracing_appender::non_blocking(appender);
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .with_writer(writer)
            .try_init()?;
        tracing::info!(version = env!("CARGO_PKG_VERSION"), "mqx started");
        Ok(guard)
    };
    match init() {
        Ok(guard) => Some(guard),
        Err(error) => {
            eprintln!("Could not initialize diagnostic logging: {error}");
            None
        }
    }
}
