//! Command-line access to a local intake database.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod cli;

fn main() -> std::process::ExitCode {
    let args = std::env::args_os()
        .skip(1)
        .map(|argument| argument.into_string())
        .collect::<Result<Vec<_>, _>>();
    let Some(command) = args.ok().and_then(|args| cli::parse(args).ok()) else {
        eprintln!("awna-intake: invalid arguments\n{}", cli::USAGE);
        return std::process::ExitCode::from(2);
    };
    let (variable, action) = match command {
        cli::Command::Help => {
            return output(cli::USAGE);
        }
        cli::Command::Database { variable, action } => (variable, action),
    };
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        eprintln!("awna-intake: runtime could not be started");
        return std::process::ExitCode::from(1);
    };
    match runtime.block_on(cli::run(&variable, action)) {
        Ok(text) => output(&text),
        Err(error) => {
            eprintln!("awna-intake: {}", error.message);
            std::process::ExitCode::from(error.code)
        }
    }
}

fn output(text: &str) -> std::process::ExitCode {
    use std::io::Write;
    if writeln!(std::io::stdout().lock(), "{text}").is_err() {
        eprintln!("awna-intake: output could not be written");
        return std::process::ExitCode::from(1);
    }
    std::process::ExitCode::SUCCESS
}
