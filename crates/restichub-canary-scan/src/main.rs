use std::path::PathBuf;
use std::process::{Command, Stdio};

use restichub_canary_scan::{Canary, Encoding, Finding, Scanner};

/// The process result: 0 no finding, 1 a finding, 2 a usage or an input/output error.
#[repr(i32)]
enum ExitCode {
    Clean = 0,
    Found = 1,
    Usage = 2,
}

/// The command text. It holds no canary value.
const USAGE: &str = "\
Canary secret scan.

Usage:
  restichub-canary-scan --canary <VALUE> --file <PATH> [options]

Canary input:
  --canary <VALUE>        Add one canary value. Repeat the option for more values.
  --canary-file <PATH>    Read the canary values from a file, one in each line.

Targets (one is necessary):
  --file <PATH>           Scan a file, for example a log or a container log.
  --dir <PATH>            Scan a directory and its subdirectories.
  --database <URL>        Scan a PostgreSQL dump. The tool runs pg_dump with the URL.
  --cmdline               Scan the process command lines of /proc on Linux.
  --cmdline-file <PATH>   Scan a file with the process command lines.

Other:
  -h, --help              Print this text.

The tool prints one line for each finding. The exit code gives the result:
  0  no finding
  1  a finding
  2  a usage or an input/output error
";

#[derive(Default)]
struct Options {
    canaries: Vec<Canary>,
    canary_files: Vec<PathBuf>,
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    databases: Vec<String>,
    cmdline: bool,
    cmdline_files: Vec<PathBuf>,
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("canary-scan: {message}");
            ExitCode::Usage
        }
    };
    std::process::exit(code as i32);
}

fn run() -> Result<ExitCode, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return Ok(ExitCode::Clean);
    }
    let options = parse(&args)?;
    let scanner = Scanner::new(&options.canaries);
    let mut findings = Vec::new();
    run_targets(&scanner, &options, &mut findings)?;
    print_findings(&findings);
    Ok(if findings.is_empty() {
        ExitCode::Clean
    } else {
        ExitCode::Found
    })
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        let option = args[index].clone();
        index += 1;
        match option.as_str() {
            "--canary" => options
                .canaries
                .push(Canary::parse(&value(args, &mut index, &option)?)),
            "--canary-file" => options
                .canary_files
                .push(PathBuf::from(value(args, &mut index, &option)?)),
            "--file" => options
                .files
                .push(PathBuf::from(value(args, &mut index, &option)?)),
            "--dir" => options
                .directories
                .push(PathBuf::from(value(args, &mut index, &option)?)),
            "--database" => options.databases.push(value(args, &mut index, &option)?),
            "--cmdline" => options.cmdline = true,
            "--cmdline-file" => options
                .cmdline_files
                .push(PathBuf::from(value(args, &mut index, &option)?)),
            other => return Err(format!("unknown option {other}")),
        }
    }
    for path in &options.canary_files {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            options.canaries.push(Canary::parse(line));
        }
    }
    if options.canaries.is_empty() {
        return Err("give one canary with --canary or --canary-file".to_owned());
    }
    let has_target = !options.files.is_empty()
        || !options.directories.is_empty()
        || !options.databases.is_empty()
        || options.cmdline
        || !options.cmdline_files.is_empty();
    if !has_target {
        return Err("give one target".to_owned());
    }
    Ok(options)
}

fn value(args: &[String], index: &mut usize, option: &str) -> Result<String, String> {
    let value = args
        .get(*index)
        .ok_or_else(|| format!("{option} needs a value"))?
        .clone();
    *index += 1;
    Ok(value)
}

fn run_targets(
    scanner: &Scanner,
    options: &Options,
    findings: &mut Vec<Finding>,
) -> Result<(), String> {
    for path in &options.files {
        findings.extend(scanner.scan_file(path).map_err(|error| error.to_string())?);
    }
    for path in &options.directories {
        findings.extend(
            scanner
                .scan_directory(path)
                .map_err(|error| error.to_string())?,
        );
    }
    for path in &options.cmdline_files {
        findings.extend(scanner.scan_file(path).map_err(|error| error.to_string())?);
    }
    for url in &options.databases {
        findings.extend(scan_database(scanner, url)?);
    }
    if options.cmdline {
        findings.extend(scan_cmdline(scanner)?);
    }
    Ok(())
}

fn scan_database(scanner: &Scanner, url: &str) -> Result<Vec<Finding>, String> {
    // The connection URL holds the database password. The child reads it from the environment, so
    // the password never reaches the process arguments of pg_dump (FR-AG-06).
    let mut child = Command::new("pg_dump")
        .args(["--data-only", "--no-owner", "--no-privileges"])
        .env("PGDATABASE", url)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("cannot start pg_dump: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "pg_dump gives no output".to_owned())?;
    let findings = scanner
        .scan_reader("postgresql", stdout)
        .map_err(|error| error.to_string())?;
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("pg_dump failed".to_owned());
    }
    Ok(findings)
}

#[cfg(target_os = "linux")]
fn scan_cmdline(scanner: &Scanner) -> Result<Vec<Finding>, String> {
    let paths = restichub_canary_scan::cmdline_paths(std::path::Path::new("/proc"))
        .map_err(|error| error.to_string())?;
    let mut findings = Vec::new();
    for path in paths {
        // A process can end between the directory read and the file read. Skip the file then.
        if let Ok(found) = scanner.scan_file(&path) {
            findings.extend(found);
        }
    }
    Ok(findings)
}

#[cfg(not(target_os = "linux"))]
fn scan_cmdline(_scanner: &Scanner) -> Result<Vec<Finding>, String> {
    Err(
        "--cmdline needs Linux; collect the command lines into a file and use --cmdline-file"
            .to_owned(),
    )
}

fn print_findings(findings: &[Finding]) {
    use std::io::Write as _;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for finding in findings {
        let Finding {
            target,
            offset,
            line,
            snippet,
            ..
        } = finding;
        let encoding = encoding_text(finding);
        let _ = writeln!(
            out,
            "found target={target} offset={offset} line={line} encoding={encoding} snippet={snippet}"
        );
    }
}

fn encoding_text(finding: &Finding) -> String {
    match finding.encoding {
        Encoding::Base64Standard | Encoding::Base64Url => {
            let label = finding.encoding.label();
            let alignment = finding.alignment;
            format!("{label}+{alignment}")
        }
        _ => finding.encoding.label().to_owned(),
    }
}
