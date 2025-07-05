#[macro_export]
macro_rules! plot_with_name {
    ($plot_name:expr, $value:expr) => {{
        tracing::client::Client::running()
            .expect("plot_const! without a running Client")
            .plot($plot_name, $value)
    }};
}

tracing_tracy::client::register_demangler!(tracing_tracy::client::demangle::default);

#[global_allocator]
static GLOBAL: tracing_tracy::client::ProfiledAllocator<std::alloc::System> =
    tracing_tracy::client::ProfiledAllocator::new(std::alloc::System, 100);

#[derive(Debug, thiserror::Error)]
pub enum TracyError {
    #[error("Tracy profiler is already running, not launching a new instance")]
    AlreadyRunning,
    #[error("Failed to launch Tracy profiler: {0}")]
    LaunchFailed(std::io::Error),
    #[error("Io error: {0}")]
    IoError(std::io::Error),
}

pub fn launch_tracy() -> Result<(), TracyError> {
    #[cfg(unix)]
    return unix::launch_tracy();

    #[cfg(not(unix))]
    panic!("launch_tracy is only supported on Unix systems");
}

#[cfg(unix)]
mod unix {
    use super::TracyError;

    pub(super) fn launch_tracy() -> Result<(), TracyError> {
        use std::{os::unix::process::CommandExt, process::Stdio};

        #[cfg(not(feature = "tracy-profiler"))]
        panic!(
            "Argument --launch-tracy is not supported because the feature tracy-profiler is not enabled"
        );

        if is_tracy_running() {
            return Err(TracyError::AlreadyRunning);
        }

        std::process::Command::new("tracy-profiler")
            .args(["-a", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(TracyError::LaunchFailed)?;

        Ok(())
    }
    fn is_tracy_running() -> bool {
        use std::process::Command;

        let output = Command::new("pgrep")
            .arg("-x")
            .arg("tracy-profiler")
            .output()
            .unwrap();

        output.status.success() && !output.stdout.is_empty()
    }
}
