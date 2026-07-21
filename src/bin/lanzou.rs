use clap::{Parser, Subcommand};
use lanzou_share::{Account, Client, EntryKind, Error, ListEntry, ParseOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

fn default_cookie_path() -> PathBuf {
    if let Ok(v) = std::env::var("LANZOU_COOKIE") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home).join(".lanzou").join("cookie");
    }
    PathBuf::from("./lanzou.cookie")
}

fn ensure_cookie_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
}

#[derive(Parser, Debug)]
#[command(name = "lanzou", about = "Lanzou share resolve + account CLI")]
struct Cli {
    /// Interactive shell (cd / ls / download)
    #[arg(short = 'i', long = "interactive", global = true)]
    interactive: bool,

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
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// Remove cookie file
    Logout {
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// List folder entries
    List {
        #[arg(long = "folder", default_value = "-1")]
        folder: String,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
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
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
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
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
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
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
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
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
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
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// Download file (via share info) or folder (recursive, concurrent)
    Download {
        /// File/folder id or name in current folder
        target: String,
        #[arg(long = "folder", default_value = "-1")]
        folder: String,
        #[arg(short = 'o', long = "output-dir", default_value = ".")]
        output_dir: String,
        #[arg(short = 'j', long = "jobs", default_value_t = 3)]
        jobs: usize,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// Interactive shell
    Interactive {
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
        #[arg(short = 'o', long = "output-dir", default_value = ".")]
        output_dir: String,
        #[arg(short = 'j', long = "jobs", default_value_t = 3)]
        jobs: usize,
    },
}

fn cookie_or_default(c: Option<PathBuf>) -> PathBuf {
    c.unwrap_or_else(default_cookie_path)
}

fn main() -> ExitCode {
    // Support legacy: `lanzou <url> ...` without subcommand.
    // Also map bare `-i` to interactive subcommand.
    let mut argv: Vec<String> = std::env::args().collect();
    if argv.len() >= 2 {
        let a1 = &argv[1];
        if a1.starts_with("http://") || a1.starts_with("https://") {
            argv.insert(1, "parse".into());
        } else if a1 == "-i" || a1 == "--interactive" {
            argv[1] = "interactive".into();
        }
    }
    let cli = Cli::parse_from(argv);

    if cli.interactive {
        return cmd_interactive(None, None, None, cli.output_dir, 3);
    }

    match cli.command {
        None => {
            eprintln!("usage: lanzou -i | <parse|login|list|upload|download|...> ...");
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
        Some(Commands::Login {
            user,
            pass,
            cookie_str,
            cookie,
        }) => cmd_login(user, pass, cookie_str, cookie_or_default(cookie)),
        Some(Commands::Logout { cookie }) => {
            let cookie = cookie_or_default(cookie);
            let _ = std::fs::remove_file(&cookie);
            println!("[ok] cookie removed: {}", cookie.display());
            ExitCode::SUCCESS
        }
        Some(Commands::List {
            folder,
            user,
            pass,
            cookie,
        }) => cmd_list(folder, user, pass, cookie_or_default(cookie)),
        Some(Commands::Upload {
            file,
            folder,
            set_pwd,
            set_desc,
            user,
            pass,
            cookie,
        }) => cmd_upload(
            file,
            folder,
            set_pwd,
            set_desc,
            user,
            pass,
            cookie_or_default(cookie),
        ),
        Some(Commands::Mkdir {
            name,
            folder,
            desc,
            user,
            pass,
            cookie,
        }) => cmd_mkdir(name, folder, desc, user, pass, cookie_or_default(cookie)),
        Some(Commands::Rm {
            file,
            folder,
            user,
            pass,
            cookie,
        }) => cmd_rm(file, folder, user, pass, cookie_or_default(cookie)),
        Some(Commands::Info {
            file,
            folder,
            user,
            pass,
            cookie,
        }) => cmd_info(file, folder, user, pass, cookie_or_default(cookie)),
        Some(Commands::Passwd {
            file,
            folder,
            pwd,
            user,
            pass,
            cookie,
        }) => cmd_passwd(file, folder, pwd, user, pass, cookie_or_default(cookie)),
        Some(Commands::Download {
            target,
            folder,
            output_dir,
            jobs,
            user,
            pass,
            cookie,
        }) => cmd_download(
            target,
            folder,
            output_dir,
            jobs,
            user,
            pass,
            cookie_or_default(cookie),
        ),
        Some(Commands::Interactive {
            user,
            pass,
            cookie,
            output_dir,
            jobs,
        }) => cmd_interactive(user, pass, cookie, output_dir, jobs),
    }
}

fn open_account(
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> Result<Account, ExitCode> {
    ensure_cookie_dir(&cookie);
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
    ensure_cookie_dir(&cookie);
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
        println!(
            "[ok] cookie imported and verified, saved to {}",
            cookie.display()
        );
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

fn print_list(folder: &str, list: &[ListEntry]) {
    println!("folder={folder}  entries={}", list.len());
    for e in list {
        let kind = match e.kind {
            EntryKind::Folder => "DIR ",
            EntryKind::File => "FILE",
        };
        let extra = e.size.clone().or_else(|| e.url.clone()).unwrap_or_default();
        println!("  [{kind}] id={:<12}  {}  {}", e.id, e.name, extra);
    }
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
            print_list(&folder, &list);
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

// ---------- download ----------

#[derive(Clone)]
struct DlJob {
    name: String,
    dest_dir: PathBuf,
    share_url: String,
    pwd: String,
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn sanitize_name(name: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return "unnamed".into();
    }
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

fn resolve_entry<'a>(list: &'a [ListEntry], target: &str) -> Option<&'a ListEntry> {
    if let Some(e) = list.iter().find(|e| e.id == target) {
        return Some(e);
    }
    if let Some(e) = list.iter().find(|e| e.name == target) {
        return Some(e);
    }
    let lt = target.to_lowercase();
    list.iter().find(|e| e.name.to_lowercase() == lt)
}

fn download_share(share_url: &str, pwd: &str, out_dir: &Path, filename: &str) -> Result<(), String> {
    let mut client = Client::new().map_err(|e| e.to_string())?;
    let opts = ParseOptions {
        password: if pwd.is_empty() {
            None
        } else {
            Some(pwd.to_string())
        },
        resolve_direct: true,
    };
    let res = client.parse(share_url, opts).map_err(|e| e.to_string())?;
    let u = res.direct.as_deref().unwrap_or(res.telecom.as_str());
    let name = if filename.is_empty() {
        res.filename.as_deref()
    } else {
        Some(filename)
    };
    client
        .download(u, out_dir, name, None)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn download_file_by_id(acc: &Account, file_id: &str, name: &str, out_dir: &Path) -> Result<(), String> {
    let (share, pwd) = acc
        .get_file_download_info(file_id)
        .map_err(|e| e.to_string())?;
    if share.is_empty() {
        return Err(format!("empty share url for file {file_id}"));
    }
    println!("[download] file id={file_id} name={name}");
    download_share(&share, &pwd, out_dir, name)
}

fn collect_folder_files(
    acc: &Account,
    folder_id: &str,
    dest_dir: &Path,
) -> Result<Vec<DlJob>, String> {
    let mut files = Vec::new();
    fn walk(
        acc: &Account,
        folder_id: &str,
        dest_dir: &Path,
        files: &mut Vec<DlJob>,
    ) -> Result<(), String> {
        std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;
        let list = acc.list(folder_id).map_err(|e| e.to_string())?;
        for e in list {
            match e.kind {
                EntryKind::Folder => {
                    let sub = dest_dir.join(sanitize_name(&e.name));
                    walk(acc, &e.id, &sub, files)?;
                }
                EntryKind::File => match acc.get_file_download_info(&e.id) {
                    Ok((share, pwd)) => {
                        files.push(DlJob {
                            name: e.name,
                            dest_dir: dest_dir.to_path_buf(),
                            share_url: share,
                            pwd,
                        });
                    }
                    Err(err) => {
                        eprintln!("[warn] skip {}: {err}", e.name);
                    }
                },
            }
        }
        Ok(())
    }
    walk(acc, folder_id, dest_dir, &mut files)?;
    Ok(files)
}

fn download_jobs(jobs: Vec<DlJob>, concurrency: usize) -> Result<(), String> {
    if jobs.is_empty() {
        return Ok(());
    }
    let concurrency = concurrency.max(1);
    let total = jobs.len();
    let queue = Arc::new(Mutex::new(jobs.into_iter().collect::<std::collections::VecDeque<_>>()));
    let done = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for _ in 0..concurrency {
        let queue = Arc::clone(&queue);
        let done = Arc::clone(&done);
        let fail = Arc::clone(&fail);
        handles.push(thread::spawn(move || {
            loop {
                let job = {
                    let mut q = queue.lock().unwrap();
                    q.pop_front()
                };
                let Some(j) = job else { break };
                let res = download_share(&j.share_url, &j.pwd, &j.dest_dir, &j.name);
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                match res {
                    Ok(()) => println!("[ok {n}/{total}] {}", j.name),
                    Err(e) => {
                        fail.fetch_add(1, Ordering::SeqCst);
                        eprintln!("[fail {n}/{total}] {}: {e}", j.name);
                    }
                }
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    let f = fail.load(Ordering::SeqCst);
    if f > 0 {
        Err(format!("{f}/{total} downloads failed"))
    } else {
        Ok(())
    }
}

fn download_target(
    acc: &Account,
    cur_folder: &str,
    target: &str,
    out_dir: &Path,
    jobs: usize,
) -> Result<(), String> {
    let list = acc.list(cur_folder).map_err(|e| e.to_string())?;
    if let Some(e) = resolve_entry(&list, target) {
        match e.kind {
            EntryKind::File => {
                return download_file_by_id(acc, &e.id, &e.name, out_dir);
            }
            EntryKind::Folder => {
                let dest = out_dir.join(sanitize_name(&e.name));
                println!(
                    "[download] folder {} ({}) -> {}  jobs={jobs}",
                    e.name,
                    e.id,
                    dest.display()
                );
                let files = collect_folder_files(acc, &e.id, &dest)?;
                println!("[download] {} files queued", files.len());
                return download_jobs(files, jobs);
            }
        }
    }
    if is_digits(target) {
        return download_file_by_id(acc, target, "", out_dir);
    }
    Err(format!("not found in folder {cur_folder}: {target}"))
}

fn cmd_download(
    target: String,
    folder: String,
    output_dir: String,
    jobs: usize,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    match download_target(&acc, &folder, &target, Path::new(&output_dir), jobs) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[error] {e}");
            ExitCode::from(1)
        }
    }
}

// ---------- interactive ----------

struct Shell {
    acc: Account,
    folder: String,
    stack: Vec<String>,
    cookie: PathBuf,
    out_dir: String,
    jobs: usize,
    user: String,
    pass: String,
}

fn cmd_interactive(
    user: Option<String>,
    pass: Option<String>,
    cookie: Option<PathBuf>,
    output_dir: String,
    jobs: usize,
) -> ExitCode {
    let cookie = cookie_or_default(cookie);
    ensure_cookie_dir(&cookie);
    let u = user.unwrap_or_default();
    let p = pass.unwrap_or_default();
    let mut acc = match Account::new(&u, &p) {
        Ok(a) => a.with_cookie_file(&cookie),
        Err(e) => {
            eprintln!("[error] {e}");
            return ExitCode::from(1);
        }
    };
    if !u.is_empty() && !p.is_empty() {
        if let Err(e) = acc.ensure_login() {
            eprintln!("[warn] login: {e}");
        }
    } else if !acc.verification() {
        eprintln!("[warn] not logged in. Use: login --user U --pass P");
    }

    let mut sh = Shell {
        acc,
        folder: "-1".into(),
        stack: Vec::new(),
        cookie,
        out_dir: output_dir,
        jobs,
        user: u,
        pass: p,
    };

    println!("lanzou interactive shell. type 'help', 'exit' to quit.");
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    loop {
        print!("lanzou:{}> ", sh.folder);
        let _ = stdout.flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                println!();
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[error] {e}");
                break;
            }
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match sh.exec(line) {
            Ok(true) => break, // exit
            Ok(false) => {}
            Err(e) => eprintln!("[error] {e}"),
        }
    }
    ExitCode::SUCCESS
}

fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for c in line.chars() {
        match c {
            '"' => in_q = !in_q,
            ' ' | '\t' if !in_q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl Shell {
    fn exec(&mut self, line: &str) -> Result<bool, String> {
        let parts = split_args(line);
        if parts.is_empty() {
            return Ok(false);
        }
        let cmd = parts[0].as_str();
        let args = &parts[1..];
        match cmd {
            "help" | "?" => {
                println!("ls | cd <id|name|/|..> | pwd | download <id|name> [-j N] [-o DIR]");
                println!("info <id|name> | upload <path> | mkdir <name> | rm <id|name>");
                println!("login [--user U --pass P] | logout | exit");
                Ok(false)
            }
            "exit" | "quit" | "q" => Ok(true),
            "pwd" => {
                println!("{}", self.folder);
                Ok(false)
            }
            "ls" | "ll" | "list" => {
                let list = self.acc.list(&self.folder).map_err(|e| e.to_string())?;
                print_list(&self.folder, &list);
                Ok(false)
            }
            "cd" => {
                if args.is_empty() {
                    return Err("usage: cd <id|name|/|..>".into());
                }
                self.cd(&args[0])?;
                Ok(false)
            }
            "download" | "dl" | "get" => {
                self.cmd_download(args)?;
                Ok(false)
            }
            "info" => {
                if args.is_empty() {
                    return Err("usage: info <id|name>".into());
                }
                self.cmd_info(&args[0])?;
                Ok(false)
            }
            "upload" | "up" => {
                if args.is_empty() {
                    return Err("usage: upload <local-path>".into());
                }
                let res = self
                    .acc
                    .upload(Path::new(&args[0]), &self.folder)
                    .map_err(|e| e.to_string())?;
                println!("[ok] uploaded {} {}", res.file_id, res.name);
                Ok(false)
            }
            "mkdir" => {
                if args.is_empty() {
                    return Err("usage: mkdir <name>".into());
                }
                self.acc
                    .create_folder(&args[0], &self.folder, "")
                    .map_err(|e| e.to_string())?;
                println!("[ok] mkdir {}", args[0]);
                Ok(false)
            }
            "rm" | "delete" => {
                if args.is_empty() {
                    return Err("usage: rm <id|name>".into());
                }
                self.cmd_rm(&args[0])?;
                Ok(false)
            }
            "login" => {
                self.cmd_login(args)?;
                Ok(false)
            }
            "logout" => {
                let _ = std::fs::remove_file(&self.cookie);
                let _ = self.acc.set_cookie("");
                println!("[ok] logged out");
                Ok(false)
            }
            _ => Err(format!("unknown command: {cmd} (help for list)")),
        }
    }

    fn cd(&mut self, target: &str) -> Result<(), String> {
        if target == "/" || target == "~" || target == "root" {
            self.folder = "-1".into();
            self.stack.clear();
            return Ok(());
        }
        if target == ".." {
            if let Some(p) = self.stack.pop() {
                self.folder = p;
            } else {
                self.folder = "-1".into();
            }
            return Ok(());
        }
        let list = self.acc.list(&self.folder).map_err(|e| e.to_string())?;
        if let Some(e) = resolve_entry(&list, target) {
            if e.kind != EntryKind::Folder {
                return Err(format!("{} is a file, not a folder", e.name));
            }
            self.stack.push(self.folder.clone());
            self.folder = e.id.clone();
            println!("cd -> {} ({})", e.name, e.id);
            return Ok(());
        }
        if is_digits(target) {
            self.stack.push(self.folder.clone());
            self.folder = target.into();
            return Ok(());
        }
        Err(format!("folder not found: {target}"))
    }

    fn cmd_download(&self, args: &[String]) -> Result<(), String> {
        if args.is_empty() {
            return Err("usage: download <id|name> [-j N] [-o DIR]".into());
        }
        let target = &args[0];
        let mut jobs = self.jobs;
        let mut out_dir = self.out_dir.clone();
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "-j" | "--jobs" if i + 1 < args.len() => {
                    i += 1;
                    if let Ok(n) = args[i].parse::<usize>() {
                        if n > 0 {
                            jobs = n;
                        }
                    }
                }
                "-o" | "--output-dir" if i + 1 < args.len() => {
                    i += 1;
                    out_dir = args[i].clone();
                }
                _ => {}
            }
            i += 1;
        }
        download_target(&self.acc, &self.folder, target, Path::new(&out_dir), jobs)
    }

    fn cmd_info(&self, target: &str) -> Result<(), String> {
        let list = self.acc.list(&self.folder).map_err(|e| e.to_string())?;
        if let Some(e) = resolve_entry(&list, target) {
            match e.kind {
                EntryKind::File => {
                    let fi = self.acc.get_file_info(&e.id).map_err(|e| e.to_string())?;
                    println!("type:    FILE");
                    println!("id:      {}", e.id);
                    println!("name:    {}", e.name);
                    if let Some(s) = &e.size {
                        println!("size:    {s}");
                    }
                    println!("share:   {}", fi.share_url);
                    println!("password:{}", fi.password);
                }
                EntryKind::Folder => {
                    let info = self.acc.get_folder_info(&e.id).map_err(|e| e.to_string())?;
                    println!("type:    DIR");
                    println!("id:      {}", e.id);
                    println!("name:    {}", e.name);
                    println!("url:     {}", info.url);
                    println!("password:{}", info.password);
                    println!("files:   {}", info.file_count);
                    println!("size:    {}", info.file_size);
                }
            }
            return Ok(());
        }
        if is_digits(target) {
            let fi = self.acc.get_file_info(target).map_err(|e| e.to_string())?;
            println!("file_id: {}", fi.id);
            println!("share:   {}", fi.share_url);
            println!("password:{}", fi.password);
            return Ok(());
        }
        Err(format!("not found: {target}"))
    }

    fn cmd_rm(&self, target: &str) -> Result<(), String> {
        let list = self.acc.list(&self.folder).map_err(|e| e.to_string())?;
        let e = resolve_entry(&list, target).ok_or_else(|| format!("not found: {target}"))?;
        match e.kind {
            EntryKind::File => {
                self.acc.delete_file(&e.id).map_err(|e| e.to_string())?;
            }
            EntryKind::Folder => {
                self.acc.delete_folder(&e.id).map_err(|e| e.to_string())?;
            }
        }
        println!("[ok] deleted {} {}", e.name, e.id);
        Ok(())
    }

    fn cmd_login(&mut self, args: &[String]) -> Result<(), String> {
        let mut user = self.user.clone();
        let mut pass = self.pass.clone();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--user" | "-u" if i + 1 < args.len() => {
                    i += 1;
                    user = args[i].clone();
                }
                "--pass" if i + 1 < args.len() => {
                    i += 1;
                    pass = args[i].clone();
                }
                "--cookie-str" if i + 1 < args.len() => {
                    i += 1;
                    self.acc
                        .set_cookie(args[i].clone())
                        .map_err(|e| e.to_string())?;
                    if !self.acc.verification() {
                        return Err("cookie invalid".into());
                    }
                    println!("[ok] cookie imported");
                    return Ok(());
                }
                _ => {}
            }
            i += 1;
        }
        if user.is_empty() || pass.is_empty() {
            return Err("usage: login --user U --pass P".into());
        }
        self.user = user.clone();
        self.pass = pass.clone();
        self.acc = Account::new(&user, &pass)
            .map_err(|e| e.to_string())?
            .with_cookie_file(&self.cookie);
        self.acc.login().map_err(|e| e.to_string())?;
        println!("[ok] logged in");
        Ok(())
    }
}
