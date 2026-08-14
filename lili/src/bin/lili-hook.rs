#[tokio::main]
async fn main() {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if arguments.as_slice() == ["--version"] {
        println!("lili-hook {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let result = lili_lib::hook_forwarder::run_from_environment().await;
    if let Some(diagnostic) = result.diagnostic {
        eprintln!("{diagnostic}");
    }
    std::process::exit(i32::from(result.exit_code.value()));
}
