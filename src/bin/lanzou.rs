use clap::{Parser, Subcommand};
use lanzou_share::{Account, Client, EntryKind, Error, ParseOptions};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(name = "lanzou", about = "Lanzou share resolve + account CLI")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Legacy: share URL as first positional when no subcommand
    #[arg(global = false)]
    url: Option<String>,

    #[arg(short = 'p', long = "pwd", global = true)]
    password: Option<String>,

    #[arg(long = "down", global = true)]
    down: bool,

    #[arg(short = 'o', long = "output-dir", default_value = ".", global = true)]
    output_dir: String,

    #[arg(short = 'f', long = "filename", global = true)]
    filename: Option<String>,

    #[arg(long = "no-resolve", global = true)]
    no_resolve: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Parse a share URL (default when URL is given)
    Parse {
        url: String,
        #[arg(short = 'p', long = "pwd")]
        password: Option<String>,
        #[arg(long = "down")]
        down: bool,
        #[arg(short = 'o', long = "output-dir", default_value = ".")]
        output_dir: String,
        #[arg(short = 'f', long = "filename")]
        filename: Option<String>,
        #[arg(long = "no-resolve")]
        no_resolve: bool,
    },
    /// Login and save cookie
    Login {
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(long = "cookie-str")]
        cookie_str: Option<String>,
        #[arg(short = 'c', long = "cookie", default_value = "./lanzou.cookie", env = "LANZOU_COOKIE")]
        cookie: PathBuf,
    },
    /// Remove cookie file
    Logout {
        #[arg(short = 'c', long = "cookie", default_value = "./lanzou.cookie", env = "LANZOU_COOKIE")]
        cookie: PathBuf,
    },
    /// List folder entries
    List {
        #[arg(long = "folder", default_value = "-1")]
        folder: String,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", default_value = "./lanzou.cookie", env = "LANZOU_COOKIE")]
        cookie: PathBuf,
    },
    /// Upload a local file
    Upload {
        file: PathBuf,
        #[arg(long = "folder", default_value = "-1")]
        folder: String,
        #[arg(long = "set-pwd")]
        set_pwd: Option<String>,
        #[arg(long = "set-desc")]
        set_desc: Option<String>,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", default_value = "./lanzou.cookie", env = "LANZOU_COOKIE")]
        cookie: PathBuf,
    },
    /// Create folder
    Mkdir {
        name: String,
        #[arg(long = "folder", default_value = "-1")]
        folder: String,
        #[arg(long = "desc", default_value = "")]
        desc: String,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", default_value = "./lanzou.cookie", env = "LANZOU_COOKIE")]
        cookie: PathBuf,
    },
    /// Delete file or folder
    Rm {
        #[arg(long = "file")]
        file: Option<String>,
        #[arg(long = "folder")]
        folder: Option<String>,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", default_value = "./lanzou.cookie", env = "LANZOU_COOKIE")]
        cookie: PathBuf,
    },
    /// Show file/folder info
    Info {
        #[arg(long = "file")]
        file: Option<String>,
        #[arg(long = "folder")]
        folder: Option<String>,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", default_value = "./lanzou.cookie", env = "LANZOU_COOKIE")]
        cookie: PathBuf,
    },
    /// Set share password
    Passwd {
        #[arg(long = "file")]
        file: Option<String>,
        #[arg(long = "folder")]
        folder: Option<String>,
        #[arg(short = 'p', long = "pwd")]
        pwd: String,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", default_value = "./lanzou.cookie", env = "LANZOU_COOKIE")]
        cookie: PathBuf,
    },
}

fn main() -> ExitCode {
    // Support legacy: `lanzou <url> ...` without subcommand.
    let mut argv: Vec<String> = std::env::args().collect();
    if argv.len() >= 2 {
        let a1 = &argv[1];
        if a1.starts_with("http://") || a1.starts_with("https://") {
            argv.insert(1, "parse".into());
        }
    }
    let cli = Cli::parse_from(argv);

    match cli.command {
        None => {
            eprintln!("usage: lanzou <parse|login|list|upload|...> ...");
            eprintln!("       lanzou <share-url> [flags]");
            ExitCode::from(1)
        }
        Some(Commands::Parse {
            url,
            password,
            down,
            output_dir,
            filename,
            no_resolve,
        }) => cmd_parse(url, password, down, output_dir, filename, no_resolve),
        Some(Commands::Login { user, pass, cookie_str, cookie }) => cmd_login(user, pass, cookie_str, cookie),
        Some(Commands::Logout { cookie }) => {
            let _ = std::fs::remove_file(&cookie);
            println!("[ok] cookie removed: {}", cookie.display());
            ExitCode::SUCCESS
        }
        Some(Commands::List {
            folder,
            user,
            pass,
            cookie,
        }) => cmd_list(folder, user, pass, cookie),
        Some(Commands::Upload {
            file,
            folder,
            set_pwd,
            set_desc,
            user,
            pass,
            cookie,
        }) => cmd_upload(file, folder, set_pwd, set_desc, user, pass, cookie),
        Some(Commands::Mkdir {
            name,
            folder,
            desc,
            user,
            pass,
            cookie,
        }) => cmd_mkdir(name, folder, desc, user, pass, cookie),
        Some(Commands::Rm {
            file,
            folder,
            user,
            pass,
            cookie,
        }) => cmd_rm(file, folder, user, pass, cookie),
        Some(Commands::Info {
            file,
            folder,
            user,
            pass,
            cookie,
        }) => cmd_info(file, folder, user, pass, cookie),
        Some(Commands::Passwd {
            file,
            folder,
            pwd,
            user,
            pass,
            cookie,
        }) => cmd_passwd(file, folder, pwd, user, pass, cookie),
    }
}

fn open_account(
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> Result<Account, ExitCode> {
    let u = user.unwrap_or_default();
    let p = pass.unwrap_or_default();
    let mut acc = match Account::new(&u, &p) {
        Ok(a) => a.with_cookie_file(&cookie),
        Err(e) => {
            eprintln!("[error] {e}");
            return Err(ExitCode::from(1));
        }
    };
    if !acc.verification() {
        if u.is_empty() || p.is_empty() {
            eprintln!("[error] not logged in; run: lanzou login --user U --pass P");
            return Err(ExitCode::from(2));
        }
        if let Err(e) = acc.ensure_login() {
            eprintln!("[error] login: {e}");
            return Err(ExitCode::from(1));
        }
    }
    Ok(acc)
}

fn cmd_parse(
    url: String,
    password: Option<String>,
    down: bool,
    output_dir: String,
    filename: Option<String>,
    no_resolve: bool,
) -> ExitCode {
    let mut client = match Client::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[error] {e}");
            return ExitCode::from(1);
        }
    };
    let opts = ParseOptions {
        password,
        resolve_direct: !no_resolve,
    };
    let result = match client.parse(&url, opts) {
        Ok(r) => r,
        Err(Error::PasswordRequired) => {
            eprintln!("[error] password required; pass --pwd / -p");
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
        if result.password_protected {
            "yes"
        } else {
            "no"
        }
    );
    println!("  cdn:      {}", result.cdn_domain);
    println!("  telecom:  {}", result.telecom);
    println!("  normal:   {}", result.normal);
    if let Some(d) = &result.direct {
        println!("  direct:   {d}");
    }
    println!("============================================================");
    if down {
        let u = result.direct.as_deref().unwrap_or(result.telecom.as_str());
        let name = filename.as_deref().or(result.filename.as_deref());
        match client.download(u, &output_dir, name, None) {
            Ok(p) => println!("[done] saved: {}", p.display()),
            Err(e) => {
                eprintln!("[error] download failed: {e}");
                return ExitCode::from(1);
            }
        }
    }
    ExitCode::SUCCESS
}

fn cmd_login(
    user: Option<String>,
    pass: Option<String>,
    cookie_str: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    if let Some(cs) = cookie_str {
        let mut acc = match Account::new("", "") {
            Ok(a) => a.with_cookie_file(&cookie),
            Err(e) => {
                eprintln!("[error] {e}");
                return ExitCode::from(1);
            }
        };
        if let Err(e) = acc.set_cookie(cs) {
            eprintln!("[error] {e}");
            return ExitCode::from(1);
        }
        if !acc.verification() {
            eprintln!("[error] cookie invalid or expired (verification failed)");
            return ExitCode::from(1);
        }
        println!("[ok] cookie imported and verified, saved to {}", cookie.display());
        return ExitCode::SUCCESS;
    }
    let user = user.unwrap_or_default();
    let pass = pass.unwrap_or_default();
    if user.is_empty() || pass.is_empty() {
        eprintln!("usage: lanzou login --user U --pass P");
        eprintln!("   or: lanzou login --cookie-str 'PHPSESSID=...; ylogin=...'");
        return ExitCode::from(1);
    }
    let mut acc = match Account::new(user, pass) {
        Ok(a) => a.with_cookie_file(&cookie),
        Err(e) => {
            eprintln!("[error] {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = acc.login() {
        eprintln!("[error] {e}");
        eprintln!("tip: if captcha is required, login in browser then:");
        eprintln!("  lanzou login --cookie-str 'PHPSESSID=...; ylogin=...'");
        return ExitCode::from(1);
    }
    println!("[ok] logged in, cookie saved to {}", cookie.display());
    ExitCode::SUCCESS
}

fn cmd_list(
    folder: String,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    match acc.list(&folder) {
        Ok(list) => {
            println!("folder={folder}  entries={}", list.len());
            for e in list {
                let kind = match e.kind {
                    EntryKind::Folder => "DIR ",
                    EntryKind::File => "FILE",
                };
                let extra = e
                    .size
                    .or(e.url)
                    .unwrap_or_default();
                println!("  [{kind}] id={:<12}  {}  {}", e.id, e.name, extra);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[error] {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_upload(
    file: PathBuf,
    folder: String,
    set_pwd: Option<String>,
    set_desc: Option<String>,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    println!("[upload] {} -> folder {folder}", file.display());
    match acc.upload(&file, &folder) {
        Ok(res) => {
            println!("[ok] uploaded");
            println!("  file_id: {}", res.file_id);
            println!("  name:    {}", res.name);
            if let Some(pwd) = set_pwd {
                if !res.file_id.is_empty() {
                    if let Err(e) = acc.set_file_password(&res.file_id, &pwd) {
                        eprintln!("[warn] set password: {e}");
                    } else {
                        println!("  password set");
                    }
                }
            }
            if let Some(desc) = set_desc {
                if !res.file_id.is_empty() {
                    if let Err(e) = acc.set_file_describe(&res.file_id, &desc) {
                        eprintln!("[warn] set desc: {e}");
                    } else {
                        println!("  description set");
                    }
                }
            }
            if !res.file_id.is_empty() {
                if let Ok((share, pwd)) = acc.get_file_download_info(&res.file_id) {
                    println!("  share:   {share}");
                    if !pwd.is_empty() {
                        println!("  share_pwd: {pwd}");
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[error] {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_mkdir(
    name: String,
    folder: String,
    desc: String,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    match acc.create_folder(&name, &folder, &desc) {
        Ok(raw) => {
            println!("[ok] mkdir {name}");
            println!("{raw}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[error] {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_rm(
    file: Option<String>,
    folder: Option<String>,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    let res = if let Some(id) = file {
        acc.delete_file(&id)
    } else if let Some(id) = folder {
        acc.delete_folder(&id)
    } else {
        eprintln!("usage: lanzou rm --file ID | --folder ID");
        return ExitCode::from(1);
    };
    match res {
        Ok(raw) => {
            println!("[ok] deleted");
            println!("{raw}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[error] {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_info(
    file: Option<String>,
    folder: Option<String>,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    if let Some(id) = file {
        match acc.get_file_info(&id) {
            Ok(fi) => {
                println!("file_id:   {}", fi.id);
                println!("share:     {}", fi.share_url);
                println!("password:  {}", fi.password);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("[error] {e}");
                ExitCode::from(1)
            }
        }
    } else if let Some(id) = folder {
        match acc.get_folder_info(&id) {
            Ok(info) => {
                println!("name:      {}", info.name);
                println!("desc:      {}", info.description);
                println!("url:       {}", info.url);
                println!("password:  {}", info.password);
                println!("files:     {}", info.file_count);
                println!("size:      {}", info.file_size);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("[error] {e}");
                ExitCode::from(1)
            }
        }
    } else {
        eprintln!("usage: lanzou info --file ID | --folder ID");
        ExitCode::from(1)
    }
}

fn cmd_passwd(
    file: Option<String>,
    folder: Option<String>,
    pwd: String,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    let res = if let Some(id) = file {
        acc.set_file_password(&id, &pwd)
    } else if let Some(id) = folder {
        acc.set_folder_password(&id, &pwd)
    } else {
        eprintln!("usage: lanzou passwd --file ID --pwd XXX | --folder ID --pwd XXX");
        return ExitCode::from(1);
    };
    match res {
        Ok(raw) => {
            println!("[ok] password updated");
            println!("{raw}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[error] {e}");
            ExitCode::from(1)
        }
    }
}
