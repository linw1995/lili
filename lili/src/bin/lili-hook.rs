#[tokio::main]
async fn main() {
    let result = lili_lib::hook_forwarder::run_from_environment().await;
    if let Some(diagnostic) = result.diagnostic {
        eprintln!("{diagnostic}");
    }
    std::process::exit(i32::from(result.exit_code.value()));
}
