#[macro_export]
macro_rules! plot_with_name {
    ($plot_name:expr, $value:expr) => {{
        $crate::Client::running()
            .expect("plot_const! without a running Client")
            .plot($plot_name, $value)
    }};
}

#[cfg(feature = "tracy-profiler")]
tracing_tracy::client::register_demangler!(tracing_tracy::client::demangle::default);

#[cfg(feature = "tracy-profiler")]
#[global_allocator]
static GLOBAL: tracing_tracy::client::ProfiledAllocator<std::alloc::System> =
    tracing_tracy::client::ProfiledAllocator::new(std::alloc::System, 100);
