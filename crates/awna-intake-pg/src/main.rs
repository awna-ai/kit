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
            println!("{}", cli::USAGE);
            return std::process::ExitCode::SUCCESS;
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
        Ok(output) => {
            println!("{output}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("awna-intake: {}", error.message);
            std::process::ExitCode::from(error.code)
        }
    }
}
