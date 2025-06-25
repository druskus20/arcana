#[macro_export]
macro_rules! plot_with_name {
    ($plot_name:expr, $value:expr) => {{
        $crate::Client::running()
            .expect("plot_const! without a running Client")
            .plot($plot_name, $value)
    }};
}

tracing_tracy::client::register_demangler!(tracing_tracy::client::demangle::default);

#[global_allocator]
static GLOBAL: tracing_tracy::client::ProfiledAllocator<std::alloc::System> =
    tracing_tracy::client::ProfiledAllocator::new(std::alloc::System, 100);

#[cfg(unix)]
pub fn launch_tracy() -> std::io::Result<()> {
    use std::{os::unix::process::CommandExt, process::Stdio};
    #[cfg(not(feature = "tracy-profiler"))]
    panic!(
        "Argument --launch-tracy is not supported because the feature tracy-profiler is not enabled"
    );

    std::process::Command::new("tracy-profiler")
        .args(["-a", "127.0.0.1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;

    Ok(())
}
