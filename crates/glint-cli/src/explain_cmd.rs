use std::io::Write;
use std::process::ExitCode;

use clap::Args as ClapArgs;
use glint_diag::{all_codes, code_info, explain};

use crate::common::{EXIT_FAILURE, EXIT_OK};

#[derive(ClapArgs)]
pub struct Args {
    /// A diagnostic code such as `E300`; without one, every code is listed
    code: Option<String>,
}

pub fn run(args: &Args) -> ExitCode {
    let mut out = std::io::stdout().lock();
    let written = match &args.code {
        None => list(&mut out),
        Some(code) => describe(&code.to_ascii_uppercase(), &mut out),
    };
    match written {
        Ok(true) => ExitCode::from(EXIT_OK),
        Ok(false) => ExitCode::from(EXIT_FAILURE),
        Err(_) => ExitCode::from(EXIT_FAILURE),
    }
}

fn list(out: &mut impl Write) -> std::io::Result<bool> {
    for info in all_codes() {
        writeln!(out, "{}  {}", info.code, info.title)?;
    }
    Ok(true)
}

fn describe(code: &str, out: &mut impl Write) -> std::io::Result<bool> {
    let (Some(info), Some(text)) = (code_info(code), explain(code)) else {
        eprintln!("glint: unknown diagnostic code `{code}`; `glint explain` lists them");
        return Ok(false);
    };
    let severity = format!("{:?}", info.severity).to_lowercase();
    writeln!(out, "{} ({severity}): {}", info.code, info.title)?;
    writeln!(out, "basis: {}\n", info.basis)?;
    writeln!(out, "{text}")?;
    Ok(true)
}
