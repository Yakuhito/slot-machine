mod cli;

#[tokio::main]
async fn main() {
    cli::run_cli().await;
}
