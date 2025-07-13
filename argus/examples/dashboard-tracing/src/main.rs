use tracing::{Level, debug, error, info, span, warn};
use tracing_subscriber::{Registry, layer::SubscriberExt};

use argus::tracing::oculus::DashboardTcpLayer;
use tracing_subscriber::util::SubscriberInitExt;
// Test functions to generate different types of events
async fn simulate_user_login(user_id: u64, session: &str) {
    let span = tracing::info_span!("user_login", user_id = user_id, session = session);
    let _enter = span.enter();

    info!(
        user_id = user_id,
        session = session,
        "User authentication started"
    );

    // Simulate some work
    //tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    if user_id % 2 == 0 {
        info!(user_id = user_id, "User login successful");
    } else {
        error!(
            user_id = user_id,
            error_code = "INVALID_CREDENTIALS",
            "User login failed"
        );
    }
}

async fn simulate_database_operation(table: &str, operation: &str, record_count: u32) {
    let span = tracing::debug_span!("db_operation", table = table, operation = operation);
    let _enter = span.enter();

    debug!(
        table = table,
        operation = operation,
        "Starting database operation"
    );

    // Simulate DB work
    //tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    if record_count > 1000 {
        warn!(
            table = table,
            operation = operation,
            record_count = record_count,
            "Large operation detected"
        );
    }

    info!(
        table = table,
        operation = operation,
        record_count = record_count,
        duration_ms = 50,
        "Database operation completed"
    );
}

async fn simulate_api_request(endpoint: &str, method: &str, status_code: u16) {
    let span = tracing::info_span!("api_request", endpoint = endpoint, method = method);
    let _enter = span.enter();

    info!(endpoint = endpoint, method = method, "API request received");

    // Simulate request processing
    //tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    if status_code >= 400 {
        error!(
            endpoint = endpoint,
            method = method,
            status_code = status_code,
            "API request failed"
        );
    } else {
        info!(
            endpoint = endpoint,
            method = method,
            status_code = status_code,
            response_time_ms = 200,
            "API request completed"
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize tracing with dashboard layer
    let dashboard_layer = DashboardTcpLayer::new("127.0.0.1:8080".to_string()).await?;

    Registry::default()
        .with(dashboard_layer)
        .with(tracing_subscriber::fmt::layer()) // Also log to console
        .init();

    println!("Starting test program...");
    println!("Make sure to run: nc -l 8080 (or your dashboard server) to see the output");

    info!("Test program started");

    // Run various test scenarios
    let mut handles = vec![];

    // Simulate multiple user logins
    for i in 1..=5 {
        let handle = tokio::spawn(async move {
            simulate_user_login(i, &format!("session_{}", i)).await;
        });
        handles.push(handle);
    }

    // Simulate database operations
    handles.push(tokio::spawn(async {
        simulate_database_operation("users", "SELECT", 500).await;
    }));

    handles.push(tokio::spawn(async {
        simulate_database_operation("orders", "INSERT", 1500).await;
    }));

    // Simulate API requests
    handles.push(tokio::spawn(async {
        simulate_api_request("/api/users", "GET", 200).await;
    }));

    handles.push(tokio::spawn(async {
        simulate_api_request("/api/orders", "POST", 404).await;
    }));

    // Wait for all tasks to complete
    for handle in handles {
        handle.await?;
    }

    // Test nested spans
    let outer_span = tracing::info_span!("outer_operation", operation_id = "op_123");
    let _outer = outer_span.enter();

    info!("Starting complex operation");

    {
        let inner_span = tracing::debug_span!("inner_operation", step = "validation");
        let _inner = inner_span.enter();

        debug!("Validating input parameters");
        //tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        info!("Input validation completed");
    }

    {
        let inner_span = tracing::debug_span!("inner_operation", step = "processing");
        let _inner = inner_span.enter();

        debug!("Processing data");
        //tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        info!("Data processing completed");
    }

    info!("Complex operation finished");

    // Give some time for all events to be sent
    //tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    info!("Test program completed");

    Ok(())
}
