use clap::Parser;
use rLan::{Client, Error, ParseOptions};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "lanzou", about = "Lanzou share link resolver / downloader")]
struct Args {
    /// Share URL
    url: String,

    /// Share password
    #[arg(short = 'p', long = "pwd")]
    password: Option<String>,

    /// Download after resolve
    #[arg(long = "down")]
    down: bool,

    /// Output directory for --down
    #[arg(short = 'o', long = "output-dir", default_value = ".")]
    output_dir: String,

    /// Force save filename
    #[arg(short = 'f', long = "filename")]
    filename: Option<String>,

    /// Do not resolve CDN direct URL
    #[arg(long = "no-resolve")]
    no_resolve: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let mut client = match Client::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[error] {e}");
            return ExitCode::from(1);
        }
    };

    let opts = ParseOptions {
        password: args.password.clone(),
        resolve_direct: !args.no_resolve,
    };

    let result = match client.parse(&args.url, opts) {
        Ok(r) => r,
        Err(Error::PasswordRequired) => {
            eprintln!("[error] password required; pass --pwd / -p");
            eprintln!("example: lanzou --pwd 5grc https://wwbss.lanzouu.com/ioHpR10k7d4b");
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("[error] {e}");
            return ExitCode::from(1);
        }
    };

    println!("============================================================");
    println!("  fid:      {}", result.fid);
    println!("  filename: {}", result.filename.as_deref().unwrap_or("?"));
    println!(
        "  password: {}",
        if result.password_protected { "yes" } else { "no" }
    );
    println!("  cdn:      {}", result.cdn_domain);
    println!("  telecom:  {}", result.telecom);
    println!("  normal:   {}", result.normal);
    if let Some(d) = &result.direct {
        println!("  direct:   {d}");
    }
    println!("============================================================");

    if args.down {
        let url = result
            .direct
            .as_deref()
            .unwrap_or(result.telecom.as_str());
        let name = args
            .filename
            .as_deref()
            .or(result.filename.as_deref());
        match client.download(url, &args.output_dir, name, None) {
            Ok(p) => {
                println!("[done] saved: {}", p.display());
            }
            Err(e) => {
                eprintln!("[error] download failed: {e}");
                return ExitCode::from(1);
            }
        }
    }

    ExitCode::SUCCESS
}
