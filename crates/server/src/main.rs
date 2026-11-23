use clap::Parser;
use sticktalk_server::{App, Config};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        // Worker and blocking threads. The deepest command needs well under
        // 1 MiB optimized and 2 MiB in a debug build (`stack_tests`); this is
        // twice that. The reserve is address space: pages are committed as used.
        .thread_stack_size(4 * 1024 * 1024)
        .enable_all()
        .build()?;
    // Startup and the accept loop are polled on a thread whose stack is
    // chosen here, like the workers', not on the main thread (whose stack is
    // the platform's: 1 MiB on Windows).
    std::thread::Builder::new()
        .name("sticktalk-main".into())
        .stack_size(4 * 1024 * 1024)
        .spawn(move || runtime.block_on(Box::pin(run())).map_err(|e| e.to_string()))?
        .join()
        .map_err(|_| "server thread panicked")??;
    Ok(())
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sticktalk_server=info".into()),
        )
        .init();
    let config = Config::parse();
    if let Some(sticktalk_server::admin::Operation::Admin { ref command }) =
        config.operations.command
    {
        return sticktalk_server::admin::run(&config, command)
            .await
            .map_err(|e| format!("admin command failed: {:?}", e.0.code).into());
    }
    if let Some(sticktalk_server::admin::Operation::Storage(ref command)) =
        config.operations.command
    {
        return sticktalk_server::storage_node::run(
            command,
            config
                .operations
                .storage
                .storage_secret
                .as_deref()
                .unwrap_or_default(),
        )
        .await
        .map_err(|e| format!("storage node failed: {:?}", e.0.code).into());
    }
    let bind = config.bind;
    let app = App::open(config)
        .await
        .map_err(|_| "server initialization failed")?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(address=%listener.local_addr()?, "StickTalk listening");
    let stop = app.clone();
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal handler");
            tokio::select! { _=tokio::signal::ctrl_c()=>{}, _=term.recv()=>{} }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        stop.shutdown();
    });
    app.serve(listener).await?;
    Ok(())
}
