use std::process::ExitCode;

fn main() -> ExitCode {
    match plugroot::run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("plugroot: {error}");
            ExitCode::from(1)
        }
    }
}
