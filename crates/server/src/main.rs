use clap::Parser;
use sticktalk_server::{App, Config};
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
