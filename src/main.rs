use colored::Colorize;
use jf::commands;

fn main() {
    if let Err(e) = tokio::runtime::Runtime::new()
        .expect("Failed to create tokio runtime")
        .block_on(commands::run())
    {
        eprintln!("{} {}", "Error:".red(), e);
        std::process::exit(commands::get_exit_code(&e));
    }
}
