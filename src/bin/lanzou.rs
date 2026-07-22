use clap::{Parser, Subcommand};
use lanzou_share::{
    config_keys, config_path_used, get_config, get_config_value, is_upload_allowed_ext,
    parse_convert_note, parse_file_note, parse_part_note, save_config, set_config_value, unescape_list, Account,
    Client, EntryKind, Error, ListEntry, ParseOptions,
};
use std::collections::HashMap;
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
#[command(name = "lanzou", about = "Lanzou share resolve + account CLI", version = lanzou_share::VERSION)]
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
    #[command(visible_alias = "get", visible_alias = "p")]
    Parse {
        url: String,
        #[arg(short = 'p', long = "pwd")]
        password: Option<String>,
        #[arg(long = "down")]
        down: bool,
        #[arg(long = "no-down")]
        no_down: bool,
        #[arg(short = 'o', long = "output-dir", default_value = ".")]
        output_dir: String,
        #[arg(short = 'f', long = "filename")]
        filename: Option<String>,
        #[arg(long = "no-resolve")]
        no_resolve: bool,
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// Login and save cookie
    #[command(visible_alias = "signin", visible_alias = "auth")]
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
    #[command(visible_alias = "signout")]
    Logout {
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// List folder entries
    #[command(visible_alias = "ls", visible_alias = "ll", visible_alias = "dir")]
    List {
        #[arg(long = "folder", default_value = "-1")]
        folder: String,
        /// Disable list_unescape grouping
        #[arg(long = "raw")]
        raw: bool,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// Upload a local file
    #[command(visible_alias = "up", visible_alias = "put")]
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
    #[command(visible_alias = "md")]
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
    #[command(visible_alias = "delete", visible_alias = "del", visible_alias = "remove", visible_alias = "unlink")]
    Rm {
        /// id or original name (via notes); optional if --file/--folder given
        target: Option<String>,
        #[arg(long = "file")]
        file: Option<String>,
        #[arg(long = "folder")]
        folder: Option<String>,
        /// folder context for name resolution
        #[arg(long = "in", default_value = "-1")]
        in_folder: String,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// Move file into another folder (folders cannot be moved)
    #[command(visible_alias = "move")]
    Mv {
        /// source file id/name
        source: String,
        /// dest folder id/name or / | -1 | root
        dest: String,
        #[arg(long = "in", default_value = "-1")]
        in_folder: String,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// Rename file or folder
    #[command(visible_alias = "rn", visible_alias = "ren")]
    Rename {
        /// id or name
        target: String,
        /// new name
        new_name: String,
        #[arg(long = "in", default_value = "-1")]
        in_folder: String,
        /// only update JSON note display name (no VIP required)
        #[arg(long = "note")]
        note: bool,
        #[arg(short = 'u', long = "user", env = "LANZOU_USER")]
        user: Option<String>,
        #[arg(long = "pass", env = "LANZOU_PASS")]
        pass: Option<String>,
        #[arg(short = 'c', long = "cookie", env = "LANZOU_COOKIE")]
        cookie: Option<PathBuf>,
    },
    /// Show file/folder info
    #[command(visible_alias = "show", visible_alias = "stat")]
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
    #[command(visible_alias = "password", visible_alias = "pwdset")]
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
    #[command(visible_alias = "dl", visible_alias = "down", visible_alias = "fetch")]
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
    #[command(visible_alias = "i", visible_alias = "shell", visible_alias = "sh", visible_alias = "repl")]
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
    /// Get/set config (stored in ~/.lanzou/config.json)
    #[command(visible_alias = "conf", visible_alias = "cfg", visible_alias = "settings")]
    Config {
        /// list | get | set | path | reset
        action: Option<String>,
        /// key for get/set, or key for bare `config KEY VALUE`
        key: Option<String>,
        /// value for set
        value: Option<String>,
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
            no_down,
            output_dir,
            filename,
            no_resolve,
            cookie,
        }) => cmd_parse(url, password, down, no_down, output_dir, filename, no_resolve, cookie),
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
            raw,
            user,
            pass,
            cookie,
        }) => cmd_list(folder, raw, user, pass, cookie_or_default(cookie)),
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
            target,
            file,
            folder,
            in_folder,
            user,
            pass,
            cookie,
        }) => cmd_rm(target, file, folder, in_folder, user, pass, cookie_or_default(cookie)),
        Some(Commands::Mv {
            source,
            dest,
            in_folder,
            user,
            pass,
            cookie,
        }) => cmd_mv(source, dest, in_folder, user, pass, cookie_or_default(cookie)),
        Some(Commands::Rename {
            target,
            new_name,
            in_folder,
            note,
            user,
            pass,
            cookie,
        }) => cmd_rename(target, new_name, in_folder, note, user, pass, cookie_or_default(cookie)),
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
        Some(Commands::Config { action, key, value }) => cmd_config(action, key, value),
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
    no_down: bool,
    output_dir: String,
    filename: Option<String>,
    no_resolve: bool,
    _cookie: Option<PathBuf>,
) -> ExitCode {
    let mut client = match Client::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[error] {e}");
            return ExitCode::from(1);
        }
    };
    let pwd = password.clone();
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
    if !result.orig_name.is_empty() {
        println!("  orig:     {}", result.orig_name);
    }
    if !result.note_kind.is_empty() {
        println!("  note:     {}", result.note_kind);
    }
    if !result.description.is_empty() {
        let d = if result.description.len() > 200 {
            format!("{}...", &result.description[..200])
        } else {
            result.description.clone()
        };
        println!("  desc:     {d}");
    }
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

    let mut want_down = down;
    if !no_down && (result.note_kind == "convert" || result.note_kind == "part") {
        want_down = true;
    }
    if !want_down {
        return ExitCode::SUCCESS;
    }

    let out = PathBuf::from(&output_dir);
    let pwd_str = pwd.unwrap_or_default();

    if result.note_kind == "convert" || result.note_kind == "raw" {
        let remote = result
            .filename
            .clone()
            .unwrap_or_else(|| "download.bin".into());
        let orig = if result.orig_name.is_empty() {
            remote.clone()
        } else {
            result.orig_name.clone()
        };
        let tmp_name = format!(".dl-{remote}");
        if let Err(e) = download_share(&url, &pwd_str, &out, &tmp_name) {
            eprintln!("[error] download: {e}");
            return ExitCode::from(1);
        }
        let tmp = out.join(&tmp_name);
        let final_name = filename.unwrap_or(orig);
        let final_path = out.join(sanitize_name(&final_name));
        // unzip if convert zip
        let note = parse_file_note(&result.description);
        let mode = note.as_ref().map(|n| n.mode.as_str()).unwrap_or("");
        let kind = note.as_ref().map(|n| n.kind.as_str()).unwrap_or("");
        if kind == "convert" && (mode == "zip" || mode.is_empty()) {
            if unzip_single_to(&tmp, &final_path).is_ok() {
                let _ = std::fs::remove_file(&tmp);
                println!("[download] restored convert -> {}", final_path.display());
                println!("[done] saved: {}", final_path.display());
                return ExitCode::SUCCESS;
            }
        }
        let _ = std::fs::rename(&tmp, &final_path).or_else(|_| {
            std::fs::copy(&tmp, &final_path).map(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        });
        println!("[done] saved: {}", final_path.display());
        return ExitCode::SUCCESS;
    }

    if result.note_kind == "part" {
        // Prefer nextUrl; fall back to nextId via account cookie.
        let head_note = parse_file_note(&result.description);
        let mut jobs: Vec<(usize, String, String)> = vec![(
            head_note.as_ref().map(|n| n.index).unwrap_or(1),
            url.clone(),
            pwd_str.clone(),
        )];
        let mut total = head_note.as_ref().map(|n| n.total).unwrap_or(1);
        let mut next_url = head_note
            .as_ref()
            .map(|n| normalize_share_url(&n.next_url))
            .unwrap_or_default();
        let mut next_pwd = head_note.as_ref().map(|n| n.npwd.clone()).unwrap_or_default();
        let mut next_id = head_note.as_ref().map(|n| n.next_id.clone()).unwrap_or_default();
        // v1 next is always file id (normalize_part_note already maps it; keep defensive copy)
        if next_id.is_empty() {
            if let Some(n) = head_note.as_ref() {
                if !n.next.is_empty() {
                    next_id = n.next.clone();
                }
            }
        }
        let mut seen_url = std::collections::HashSet::new();
        seen_url.insert(url.clone());
        let mut seen_id = std::collections::HashSet::new();
        seen_id.insert(result.fid.clone());
        let cookie_path = default_cookie_path();
        let acc = Account::new("", "")
            .ok()
            .map(|a| a.with_cookie_file(&cookie_path))
            .filter(|a| a.verification());
        let mut guard = 0;
        while guard < 256 {
            guard += 1;
            if !next_url.is_empty() {
                if !seen_url.insert(next_url.clone()) {
                    break;
                }
                let mut nc = match Client::new() {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[error] {e}");
                        return ExitCode::from(1);
                    }
                };
                let opts = ParseOptions {
                    password: if next_pwd.is_empty() {
                        None
                    } else {
                        Some(next_pwd.clone())
                    },
                    resolve_direct: false,
                };
                match nc.parse(&next_url, opts) {
                    Ok(nres) => {
                        let mut idx = jobs.len() + 1;
                        let mut following = String::new();
                        let mut following_pwd = String::new();
                        let mut following_id = String::new();
                        if let Some(n) = parse_file_note(&nres.description) {
                            if n.kind == "part" {
                                idx = n.index;
                                if n.total > total {
                                    total = n.total;
                                }
                                following = n.next_url;
                                following_pwd = n.npwd;
                                following_id = n.next_id;
                                if following_id.is_empty() && !n.next.is_empty() {
                                    following_id = n.next;
                                }
                            }
                        }
                        jobs.push((idx, next_url.clone(), next_pwd.clone()));
                        next_url = normalize_share_url(&following);
                        next_pwd = following_pwd;
                        next_id = following_id;
                        continue;
                    }
                    Err(e) => {
                        if next_id.is_empty() || acc.is_none() {
                            eprintln!("[warn] next part share: {e}");
                            break;
                        }
                        next_url.clear();
                    }
                }
            }
            if next_id.is_empty() {
                break;
            }
            if !seen_id.insert(next_id.clone()) {
                break;
            }
            let Some(ref acc) = acc else {
                eprintln!(
                    "[warn] part note nextId={next_id} needs account cookie (no nextUrl); stopping chain"
                );
                break;
            };
            match acc.get_file_download_info(&next_id) {
                Ok((share, p)) => {
                    let desc = acc.get_file_describe(&next_id).unwrap_or_default();
                    let (idx, following_url, following_pwd, following_id, tot) =
                        if let Some(pm) = parse_part_note(&desc) {
                            (pm.index, pm.next_url, pm.npwd, pm.next_id, pm.total)
                        } else {
                            (jobs.len() + 1, String::new(), String::new(), String::new(), total)
                        };
                    if tot > total {
                        total = tot;
                    }
                    jobs.push((idx, share, p));
                    next_url = normalize_share_url(&following_url);
                    next_pwd = following_pwd;
                    next_id = following_id;
                }
                Err(e) => {
                    eprintln!("[warn] next part id {next_id}: {e}");
                    break;
                }
            }
        }
        jobs.sort_by_key(|(i, _, _)| *i);
        let orig = if result.orig_name.is_empty() {
            "merged.bin".into()
        } else {
            result.orig_name.clone()
        };
        let orig = filename.unwrap_or(orig);
        let orig_s = sanitize_name(&orig);
        println!(
            "[download] split {}  parts={}/{}  (serial, note chain)",
            orig_s,
            jobs.len(),
            total
        );
        let mut parts: Vec<(usize, PathBuf)> = Vec::new();
        for (n, (idx, share, p)) in jobs.iter().enumerate() {
            let n = n + 1;
            println!("[download] part {n}/{} index={idx}", jobs.len());
            let part_name = format!(".{orig_s}.s{idx:03}.download");
            if let Err(e) = download_share(share, p, &out, &part_name) {
                eprintln!("[error] part index={idx}: {e}");
                for (_, f) in &parts {
                    let _ = std::fs::remove_file(f);
                }
                return ExitCode::from(1);
            }
            let downloaded = out.join(&part_name);
            let prefer = out.join(format!(".{orig_s}.s{idx:03}.bin"));
            match extract_part_payload(&downloaded, &prefer) {
                Ok(raw) => {
                    let _ = std::fs::remove_file(&downloaded);
                    parts.push((*idx, raw));
                    println!("[ok {n}/{}] part index={idx}", jobs.len());
                }
                Err(e) => {
                    eprintln!("[error] part index={idx} extract: {e}");
                    let _ = std::fs::remove_file(&downloaded);
                    for (_, f) in &parts {
                        let _ = std::fs::remove_file(f);
                    }
                    return ExitCode::from(1);
                }
            }
            if n < jobs.len() {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        }
        parts.sort_by_key(|(i, _)| *i);
        let out_path = out.join(&orig_s);
        match std::fs::File::create(&out_path) {
            Ok(mut o) => {
                for (_, f) in parts {
                    match std::fs::File::open(&f) {
                        Ok(mut inp) => {
                            if let Err(e) = std::io::copy(&mut inp, &mut o) {
                                eprintln!("[error] merge: {e}");
                                return ExitCode::from(1);
                            }
                        }
                        Err(e) => {
                            eprintln!("[error] merge open: {e}");
                            return ExitCode::from(1);
                        }
                    }
                    let _ = std::fs::remove_file(f);
                }
            }
            Err(e) => {
                eprintln!("[error] create: {e}");
                return ExitCode::from(1);
            }
        }
        println!("[done] merged: {}", out_path.display());
        println!("[done] saved: {}", out_path.display());
        return ExitCode::SUCCESS;
    }

// plain download
    let u = result.direct.as_deref().unwrap_or(result.telecom.as_str());
    let name = filename.as_deref().or(result.filename.as_deref());
    match client.download(u, &output_dir, name, None) {
        Ok(p) => println!("[done] saved: {}", p.display()),
        Err(e) => {
            eprintln!("[error] download failed: {e}");
            return ExitCode::from(1);
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

fn print_list(acc: &Account, folder: &str, list: &[ListEntry], unescape: bool) {
    let cfg = get_config();
    let do_unescape = unescape && cfg.list_unescape;
    let notes = if do_unescape {
        acc.fetch_notes(list)
    } else {
        Default::default()
    };
    let rows = unescape_list(list, &notes, do_unescape);
    print!("folder={folder}  entries={}", list.len());
    if do_unescape && rows.len() != list.len() {
        print!("  display={}", rows.len());
    }
    println!();
    for e in rows {
        let kind = match e.kind.as_str() {
            "DIR" => "DIR ",
            "SPLIT" => "SPLIT",
            _ => "FILE",
        };
        let extra = if e.extra.is_empty() {
            e.size.clone()
        } else {
            e.extra.clone()
        };
        println!("  [{kind}] id={:<12}  {}  {}", e.id, e.name, extra);
        if e.kind == "SPLIT" {
            for p in e.parts {
                println!(
                    "           └─ id={:<12}  {}  {}",
                    p.id,
                    p.name,
                    p.size.unwrap_or_default()
                );
            }
        }
    }
}

fn cmd_list(
    folder: String,
    raw: bool,
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
            print_list(&acc, &folder, &list, !raw);
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
    let cfg = get_config();
    println!("[upload] {} -> folder {folder}", file.display());
    if let Some(ext) = file.extension().and_then(|s| s.to_str()) {
        if !is_upload_allowed_ext(ext) {
            if cfg.suffix_auto_convert {
                println!(
                    "[upload] suffix .{ext} not allowed; convert mode={} -> .{}",
                    cfg.suffix_mode, cfg.suffix_name
                );
            } else {
                println!("[upload] suffix .{ext} not allowed (suffix_auto_convert=false)");
            }
        }
    }
    if let Ok(meta) = std::fs::metadata(&file) {
        let limit = (cfg.split_size_mb as u64) * 1024 * 1024;
        if cfg.split_enable && meta.len() > limit {
            println!(
                "[upload] size {} > {}MB, will split (format={})",
                meta.len(),
                cfg.split_size_mb,
                cfg.split_name_format
            );
        }
    }
    match acc.upload(&file, &folder) {
        Ok(res) => {
            if !res.parts.is_empty() {
                println!(
                    "[ok] uploaded {} parts  group={}  orig={}",
                    res.parts.len(),
                    res.group_id.as_deref().unwrap_or(""),
                    res.orig_name.as_deref().unwrap_or("")
                );
                for p in &res.parts {
                    println!(
                        "  part {}/{} id={} name={} size={}",
                        p.index, p.total, p.file_id, p.name, p.size
                    );
                }
            } else {
                println!("[ok] uploaded");
                println!("  file_id: {}", res.file_id);
                println!("  name:    {}", res.name);
            }
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
                if !res.file_id.is_empty() && res.parts.is_empty() {
                    if let Err(e) = acc.set_file_describe(&res.file_id, &desc) {
                        eprintln!("[warn] set desc: {e}");
                    } else {
                        println!("  description set");
                    }
                }
            }
            if !res.file_id.is_empty() && res.parts.is_empty() {
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

fn cmd_config(action: Option<String>, key: Option<String>, value: Option<String>) -> ExitCode {
    let act = action.as_deref().unwrap_or("list");
    match act {
        "list" | "show" | "ls" => {
            let cfg = get_config();
            println!("config: {}", config_path_used().display());
            for (k, desc) in config_keys() {
                let v = get_config_value(&cfg, k).unwrap_or_default();
                println!("  {k:<20} = {v:<10}  # {desc}");
            }
            ExitCode::SUCCESS
        }
        "get" => {
            let Some(k) = key else {
                eprintln!("usage: lanzou config get <key>");
                return ExitCode::from(1);
            };
            match get_config_value(&get_config(), &k) {
                Ok(v) => {
                    println!("{v}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("[error] {e}");
                    ExitCode::from(1)
                }
            }
        }
        "set" => {
            let (Some(k), Some(v)) = (key, value) else {
                eprintln!("usage: lanzou config set <key> <value>");
                return ExitCode::from(1);
            };
            match set_config_value(get_config(), &k, &v) {
                Ok(cfg) => {
                    if let Err(e) = save_config(config_path_used(), cfg.clone()) {
                        eprintln!("[error] save: {e}");
                        return ExitCode::from(1);
                    }
                    let nv = get_config_value(&cfg, &k).unwrap_or_default();
                    println!("[ok] {k} = {nv}");
                    println!("  saved: {}", config_path_used().display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("[error] {e}");
                    ExitCode::from(1)
                }
            }
        }
        "path" => {
            println!("{}", config_path_used().display());
            ExitCode::SUCCESS
        }
        "reset" => {
            let cfg = lanzou_share::Config::default();
            if let Err(e) = save_config(config_path_used(), cfg) {
                eprintln!("[error] {e}");
                return ExitCode::from(1);
            }
            println!("[ok] reset to defaults -> {}", config_path_used().display());
            ExitCode::SUCCESS
        }
        "help" | "-h" | "--help" => {
            println!("lanzou config list");
            println!("lanzou config get <key>");
            println!("lanzou config set <key> <value>");
            println!("lanzou config path");
            println!("lanzou config reset");
            for (k, d) in config_keys() {
                println!("  {k:<20}  {d}");
            }
            ExitCode::SUCCESS
        }
        other => {
            // bare: config KEY VALUE
            if let Some(v) = key {
                match set_config_value(get_config(), other, &v) {
                    Ok(cfg) => {
                        if let Err(e) = save_config(config_path_used(), cfg.clone()) {
                            eprintln!("[error] save: {e}");
                            return ExitCode::from(1);
                        }
                        let nv = get_config_value(&cfg, other).unwrap_or_default();
                        println!("[ok] {other} = {nv}");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("[error] {e}");
                        ExitCode::from(1)
                    }
                }
            } else {
                eprintln!("unknown config subcommand: {other}");
                ExitCode::from(1)
            }
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

fn delete_target(acc: &Account, cur_folder: &str, target: &str) -> Result<(), String> {
    let list = acc.list(cur_folder).map_err(|e| e.to_string())?;
    let notes = acc.fetch_notes(&list);
    if let Some(r) = resolve_by_notes(&list, &notes, target) {
        if r.kind == "split" {
            println!("[rm] split {}  parts={}", r.orig_name, r.parts.len());
            let mut failed = 0usize;
            for p in &r.parts {
                if let Err(e) = acc.delete_file(&p.file_id) {
                    failed += 1;
                    eprintln!(
                        "[warn] delete part {} id={}: {e}",
                        p.index, p.file_id
                    );
                } else {
                    println!(
                        "[ok] deleted part {}/{} id={} {}",
                        p.index, p.total, p.file_id, p.name
                    );
                }
            }
            if failed > 0 {
                return Err(format!("{failed}/{} parts failed to delete", r.parts.len()));
            }
            return Ok(());
        }
        acc.delete_file(&r.file_id).map_err(|e| e.to_string())?;
        println!(
            "[ok] deleted {} (id={} remote via convert note)",
            r.orig_name, r.file_id
        );
        return Ok(());
    }
    if let Some(e) = resolve_entry(&list, target) {
        match e.kind {
            EntryKind::File => {
                if let Some(note) = notes.get(&e.id) {
                    if let Some(pm) = parse_part_note(note) {
                        let name = if pm.name.is_empty() {
                            pm.group_id
                        } else {
                            pm.name
                        };
                        return delete_target(acc, cur_folder, &name);
                    }
                }
                acc.delete_file(&e.id).map_err(|e| e.to_string())?;
                println!("[ok] deleted file {} ({})", e.name, e.id);
                return Ok(());
            }
            EntryKind::Folder => {
                acc.delete_folder(&e.id).map_err(|e| e.to_string())?;
                println!("[ok] deleted folder {} ({})", e.name, e.id);
                return Ok(());
            }
        }
    }
    if is_digits(target) {
        if acc.delete_file(target).is_ok() {
            println!("[ok] deleted file {target}");
            return Ok(());
        }
        if acc.delete_folder(target).is_ok() {
            println!("[ok] deleted folder {target}");
            return Ok(());
        }
    }
    Err(format!("not found in folder {cur_folder}: {target}"))
}

fn cmd_rm(
    target: Option<String>,
    file: Option<String>,
    folder: Option<String>,
    in_folder: String,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    if let Some(t) = target {
        return match delete_target(&acc, &in_folder, &t) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("[error] {e}");
                ExitCode::from(1)
            }
        };
    }
    let res = if let Some(id) = file {
        acc.delete_file(&id)
    } else if let Some(id) = folder {
        acc.delete_folder(&id)
    } else {
        eprintln!("usage: lanzou rm <id|name> [--in folderID]");
        eprintln!("   or: lanzou rm --file ID | --folder ID");
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

fn cmd_mv(
    source: String,
    dest: String,
    in_folder: String,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    match move_target(&acc, &in_folder, &source, &dest) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[error] {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_rename(
    target: String,
    new_name: String,
    in_folder: String,
    note_only: bool,
    user: Option<String>,
    pass: Option<String>,
    cookie: PathBuf,
) -> ExitCode {
    let acc = match open_account(user, pass, cookie) {
        Ok(a) => a,
        Err(c) => return c,
    };
    match rename_target(&acc, &in_folder, &target, &new_name, note_only) {
        Ok(()) => ExitCode::SUCCESS,
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

fn normalize_share_url(u: &str) -> String {
    let u = u.trim();
    if u.is_empty() {
        return String::new();
    }
    if let Some(rest) = u.strip_prefix("//") {
        return format!("https://{rest}");
    }
    if u.starts_with("http://") || u.starts_with("https://") {
        return u.to_string();
    }
    if u.contains('.') && u.contains('/') {
        return format!("https://{u}");
    }
    u.to_string()
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

fn resolve_folder_id(acc: &Account, cur_folder: &str, dest: &str) -> Result<String, String> {
    let dest = dest.trim();
    if dest.is_empty() || dest == "/" || dest == "root" || dest == "~" || dest == "-1" {
        return Ok("-1".into());
    }
    if is_digits(dest) || dest == "-1" {
        return Ok(dest.to_string());
    }
    let list = acc.list(cur_folder).map_err(|e| e.to_string())?;
    if let Some(e) = resolve_entry(&list, dest) {
        if e.kind != EntryKind::Folder {
            return Err(format!("destination is a file, not folder: {dest}"));
        }
        return Ok(e.id.clone());
    }
    if cur_folder != "-1" {
        if let Ok(root_list) = acc.list("-1") {
            if let Some(e) = resolve_entry(&root_list, dest) {
                if e.kind == EntryKind::Folder {
                    return Ok(e.id.clone());
                }
            }
        }
    }
    Err(format!("destination folder not found: {dest}"))
}

fn move_target(acc: &Account, cur_folder: &str, src: &str, dest: &str) -> Result<(), String> {
    let list = acc.list(cur_folder).map_err(|e| e.to_string())?;
    let notes = acc.fetch_notes(&list);

    let (file_ids, label): (Vec<String>, String) =
        if let Some(r) = resolve_by_notes(&list, &notes, src) {
            if r.kind == "split" {
                let ids = r.parts.iter().map(|p| p.file_id.clone()).collect();
                (ids, format!("{} (split)", r.orig_name))
            } else {
                (vec![r.file_id], r.orig_name)
            }
        } else if let Some(e) = resolve_entry(&list, src) {
            if e.kind == EntryKind::Folder {
                return Err(format!(
                    "cannot move folder {} (Lanzou has no folder-move API)",
                    e.name
                ));
            }
            if let Some(note) = notes.get(&e.id) {
                if let Some(pm) = parse_part_note(note) {
                    let name = if pm.name.is_empty() {
                        pm.group_id.clone()
                    } else {
                        pm.name.clone()
                    };
                    if let Some(r) = resolve_by_notes(&list, &notes, &name) {
                        if r.kind == "split" {
                            return move_target(acc, cur_folder, &name, dest);
                        }
                    }
                }
            }
            (vec![e.id.clone()], e.name.clone())
        } else if is_digits(src) {
            (vec![src.to_string()], src.to_string())
        } else {
            return Err(format!("source not found in folder {cur_folder}: {src}"));
        };

    let dest_id = resolve_folder_id(acc, cur_folder, dest)?;
    println!(
        "[mv] {label} -> folder {dest_id}  files={}",
        file_ids.len()
    );
    let mut failed = 0;
    for id in &file_ids {
        match acc.move_file(id, &dest_id) {
            Ok(raw) => println!("[ok] moved id={id}  {}", raw.trim()),
            Err(e) => {
                failed += 1;
                eprintln!("[warn] move id={id}: {e}");
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed}/{} moves failed", file_ids.len()));
    }
    Ok(())
}

fn rename_target(
    acc: &Account,
    cur_folder: &str,
    target: &str,
    new_name: &str,
    note_only: bool,
) -> Result<(), String> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("new name is empty".into());
    }
    let list = acc.list(cur_folder).map_err(|e| e.to_string())?;
    let notes = acc.fetch_notes(&list);

    if let Some(e) = resolve_entry(&list, target) {
        if e.kind == EntryKind::Folder {
            let raw = acc
                .rename_folder(&e.id, new_name, "")
                .map_err(|e| e.to_string())?;
            println!("[ok] renamed folder {} -> {new_name}  id={}", e.name, e.id);
            println!("{raw}");
            return Ok(());
        }
    }

    if let Some(r) = resolve_by_notes(&list, &notes, target) {
        if r.kind == "split" {
            if !note_only {
                eprintln!("[info] split group: updating note display name on all parts (logical)");
            }
            let mut failed = 0;
            for p in &r.parts {
                match acc.rename_note(&p.file_id, new_name) {
                    Ok(_) => println!("[ok] note name={new_name}  part id={}", p.file_id),
                    Err(e) => {
                        failed += 1;
                        eprintln!("[warn] note rename part id={}: {e}", p.file_id);
                    }
                }
            }
            if failed > 0 {
                return Err(format!("{failed}/{} part notes failed", r.parts.len()));
            }
            return Ok(());
        }
        if note_only {
            let raw = acc
                .rename_note(&r.file_id, new_name)
                .map_err(|e| e.to_string())?;
            println!(
                "[ok] note rename {} -> {new_name}  id={}",
                r.orig_name, r.file_id
            );
            println!("{raw}");
            return Ok(());
        }
        match acc.rename_file(&r.file_id, new_name) {
            Ok(raw) => {
                if let Err(e) = acc.rename_note(&r.file_id, new_name) {
                    eprintln!("[warn] server renamed but note update failed: {e}");
                }
                println!(
                    "[ok] renamed file {} -> {new_name}  id={}",
                    r.orig_name, r.file_id
                );
                println!("{raw}");
                return Ok(());
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("会员") {
                    eprintln!("[warn] server rename requires VIP ({msg}); falling back to --note");
                    let raw2 = acc
                        .rename_note(&r.file_id, new_name)
                        .map_err(|e2| format!("rename VIP-only and note fallback failed: {msg} / {e2}"))?;
                    println!(
                        "[ok] note rename {} -> {new_name}  id={} (VIP rename unavailable)",
                        r.orig_name, r.file_id
                    );
                    println!("{raw2}");
                    return Ok(());
                }
                return Err(msg);
            }
        }
    }

    if let Some(e) = resolve_entry(&list, target) {
        if e.kind == EntryKind::Folder {
            let raw = acc
                .rename_folder(&e.id, new_name, "")
                .map_err(|e| e.to_string())?;
            println!("[ok] renamed folder {} -> {new_name}  id={}", e.name, e.id);
            println!("{raw}");
            return Ok(());
        }
        if let Some(note) = notes.get(&e.id) {
            if let Some(pm) = parse_part_note(note) {
                let name = if pm.name.is_empty() {
                    pm.group_id.clone()
                } else {
                    pm.name.clone()
                };
                return rename_target(acc, cur_folder, &name, new_name, true);
            }
        }
        if note_only {
            match acc.rename_note(&e.id, new_name) {
                Ok(raw) => {
                    println!("[ok] note rename {} -> {new_name}  id={}", e.name, e.id);
                    println!("{raw}");
                    return Ok(());
                }
                Err(_) => {
                    let body = lanzou_share::format_raw_note(new_name, &e.name, 0);
                    let raw2 = acc
                        .set_file_describe(&e.id, &body)
                        .map_err(|e| e.to_string())?;
                    println!(
                        "[ok] wrote raw note name={new_name} as={}  id={}",
                        e.name, e.id
                    );
                    println!("{raw2}");
                    return Ok(());
                }
            }
        }
        match acc.rename_file(&e.id, new_name) {
            Ok(raw) => {
                println!("[ok] renamed file {} -> {new_name}  id={}", e.name, e.id);
                println!("{raw}");
                return Ok(());
            }
            Err(e2) => {
                let msg = e2.to_string();
                if msg.contains("会员") {
                    eprintln!("[warn] server rename requires VIP ({msg}); falling back to --note");
                    return rename_target(acc, cur_folder, target, new_name, true);
                }
                return Err(msg);
            }
        }
    }

    if is_digits(target) {
        if let Ok(info) = acc.get_folder_info(target) {
            if !info.name.is_empty() {
                let raw = acc
                    .rename_folder(target, new_name, &info.description)
                    .map_err(|e| e.to_string())?;
                println!(
                    "[ok] renamed folder {} -> {new_name}  id={target}",
                    info.name
                );
                println!("{raw}");
                return Ok(());
            }
        }
        if note_only {
            let raw = acc
                .rename_note(target, new_name)
                .map_err(|e| e.to_string())?;
            println!("[ok] note rename id={target} -> {new_name}");
            println!("{raw}");
            return Ok(());
        }
        match acc.rename_file(target, new_name) {
            Ok(raw) => {
                println!("[ok] renamed file id={target} -> {new_name}");
                println!("{raw}");
                return Ok(());
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("会员") {
                    eprintln!("[warn] server rename requires VIP ({msg}); falling back to --note");
                    return rename_target(acc, cur_folder, target, new_name, true);
                }
                return Err(msg);
            }
        }
    }
    Err(format!("not found in folder {cur_folder}: {target}"))
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
    let pwd = if pwd.is_empty() { None } else { Some(pwd) };
    let name = if filename.is_empty() { None } else { Some(filename) };
    client
        .download_share(share_url, pwd, out_dir, name)
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
    let mut save_name = name.to_string();
    if let Ok(desc) = acc.get_file_describe(file_id) {
        if let Some(cm) = parse_convert_note(&desc) {
            if !cm.name.is_empty() {
                save_name = cm.name;
            }
        }
    }
    println!("[download] file id={file_id} name={save_name}");
    download_share(&share, &pwd, out_dir, &save_name)?;
    let saved = out_dir.join(&save_name);
    if needs_unzip_convert(&saved, &save_name) {
        let raw = out_dir.join(format!("{save_name}.raw"));
        if unzip_single_to(&saved, &raw).is_ok() {
            let _ = std::fs::remove_file(&saved);
            let _ = std::fs::rename(&raw, &saved);
            println!("[download] extracted original payload -> {}", saved.display());
        }
    }
    Ok(())
}

#[derive(Clone)]
struct ResolvedDownload {
    kind: String, // file | split
    orig_name: String,
    file_id: String,
    file_name: String,
    parts: Vec<ResolvedPart>,
}

#[derive(Clone)]
struct ResolvedPart {
    file_id: String,
    name: String,
    index: usize,
    total: usize,
    size: u64,
}

fn resolve_by_notes(
    list: &[ListEntry],
    notes: &HashMap<String, String>,
    target: &str,
) -> Option<ResolvedDownload> {
    let lt = target.trim().to_lowercase();
    if lt.is_empty() {
        return None;
    }
    // convert notes
    for e in list {
        if e.kind != EntryKind::File {
            continue;
        }
        let note = notes
            .get(&e.id)
            .cloned()
            .or_else(|| e.description.clone())
            .unwrap_or_default();
        if let Some(cm) = parse_convert_note(&note) {
            if cm.name.eq_ignore_ascii_case(target) {
                let save = if cm.name.is_empty() {
                    e.name.clone()
                } else {
                    cm.name.clone()
                };
                return Some(ResolvedDownload {
                    kind: "file".into(),
                    orig_name: cm.name,
                    file_id: e.id.clone(),
                    file_name: save,
                    parts: Vec::new(),
                });
            }
        }
    }
    // split notes: group by group_id (same orig name may have multiple uploads)
    let mut groups: HashMap<String, (String, String, Vec<ResolvedPart>)> = HashMap::new();
    // key=group_id -> (group_id, orig_name, parts)
    for e in list {
        if e.kind != EntryKind::File {
            continue;
        }
        let note = notes
            .get(&e.id)
            .cloned()
            .or_else(|| e.description.clone())
            .unwrap_or_default();
        if let Some(pm) = parse_part_note(&note) {
            let key = if pm.group_id.is_empty() {
                format!("{}#{}", pm.name.to_lowercase(), e.id)
            } else {
                pm.group_id.clone()
            };
            let ent = groups.entry(key).or_insert_with(|| {
                (pm.group_id.clone(), pm.name.clone(), Vec::new())
            });
            if ent.1.is_empty() && !pm.name.is_empty() {
                ent.1 = pm.name.clone();
            }
            ent.2.push(ResolvedPart {
                file_id: e.id.clone(),
                name: e.name.clone(),
                index: pm.index,
                total: pm.total,
                size: pm.size,
            });
        }
    }
    // exact group id
    if let Some((gid, name, mut parts)) = groups.remove(target) {
        if !parts.is_empty() {
            parts.sort_by_key(|p| p.index);
            let orig = if name.is_empty() {
                target.to_string()
            } else {
                name
            };
            let _ = gid;
            return Some(ResolvedDownload {
                kind: "split".into(),
                orig_name: orig,
                file_id: String::new(),
                file_name: String::new(),
                parts,
            });
        }
    }
    // by original name: prefer complete + newest (max file id)
    let mut candidates: Vec<(String, String, Vec<ResolvedPart>)> = groups
        .into_values()
        .filter(|(_, name, parts)| name.eq_ignore_ascii_case(target) && !parts.is_empty())
        .collect();
    if !candidates.is_empty() {
        candidates.sort_by(|a, b| {
            let score = |parts: &Vec<ResolvedPart>, total_hint: usize| {
                let mut seen = std::collections::HashSet::new();
                let mut max_id: u64 = 0;
                for p in parts {
                    seen.insert(p.index);
                    if let Ok(id) = p.file_id.parse::<u64>() {
                        if id > max_id {
                            max_id = id;
                        }
                    }
                }
                let n = seen.len();
                let want = if total_hint > 0 { total_hint } else { n };
                let complete = if n >= want && want > 0 { 1 } else { 0 };
                (complete, max_id, n)
            };
            let ta = a.2.first().map(|p| p.total).unwrap_or(0);
            let tb = b.2.first().map(|p| p.total).unwrap_or(0);
            score(&b.2, tb).cmp(&score(&a.2, ta))
        });
        let (_gid, name, parts_in) = candidates.remove(0);
        // dedupe index keep highest file id
        let mut best: HashMap<usize, ResolvedPart> = HashMap::new();
        for p in parts_in {
            best.entry(p.index)
                .and_modify(|cur| {
                    let a = cur.file_id.parse::<u64>().unwrap_or(0);
                    let b = p.file_id.parse::<u64>().unwrap_or(0);
                    if b > a {
                        *cur = p.clone();
                    }
                })
                .or_insert(p);
        }
        let mut parts: Vec<ResolvedPart> = best.into_values().collect();
        parts.sort_by_key(|p| p.index);
        let orig = if name.is_empty() {
            target.to_string()
        } else {
            name
        };
        return Some(ResolvedDownload {
            kind: "split".into(),
            orig_name: orig,
            file_id: String::new(),
            file_name: String::new(),
            parts,
        });
    }
    None
}

fn needs_unzip_convert(path: &Path, want_name: &str) -> bool {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return false,
    };
    if archive.len() != 1 {
        return false;
    }
    let name = archive.by_index(0).map(|f| f.name().to_string()).unwrap_or_default();
    name == want_name || Path::new(&name).file_name().and_then(|s| s.to_str()) == Some(want_name)
}

fn unzip_single_to(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    if archive.is_empty() {
        return Err("empty zip".into());
    }
    let mut entry = archive.by_index(0).map_err(|e| e.to_string())?;
    let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    Ok(())
}

fn extract_part_payload(downloaded: &Path, prefer: &Path) -> Result<PathBuf, String> {
    if let Ok(file) = std::fs::File::open(downloaded) {
        if let Ok(mut archive) = zip::ZipArchive::new(file) {
            if !archive.is_empty() {
                let mut entry = archive.by_index(0).map_err(|e| e.to_string())?;
                let mut out = std::fs::File::create(prefer).map_err(|e| e.to_string())?;
                std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
                return Ok(prefer.to_path_buf());
            }
        }
    }
    std::fs::rename(downloaded, prefer)
        .or_else(|_| {
            std::fs::copy(downloaded, prefer).map(|_| {
                let _ = std::fs::remove_file(downloaded);
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(prefer.to_path_buf())
}

fn download_split_group(
    acc: &Account,
    r: &ResolvedDownload,
    out_dir: &Path,
    _jobs: usize,
) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let total = r.parts.len();
    println!("[download] split {}  parts={total}  (serial)", r.orig_name);
    let mut files: Vec<(usize, PathBuf)> = Vec::new();
    let mut fail = 0usize;
    for (i, p) in r.parts.iter().enumerate() {
        let n = i + 1;
        let part_name = format!(".{}.part{:03}.download", sanitize_name(&r.orig_name), p.index);
        let (share, pwd) = match acc.get_file_download_info(&p.file_id) {
            Ok(v) => v,
            Err(e) => {
                fail += 1;
                eprintln!("[fail {n}/{total}] part {}: {e}", p.index);
                continue;
            }
        };
        if let Err(e) = download_share(&share, &pwd, out_dir, &part_name) {
            fail += 1;
            let _ = std::fs::remove_file(out_dir.join(&part_name));
            eprintln!("[fail {n}/{total}] part {}: {e}", p.index);
            continue;
        }
        let downloaded = out_dir.join(&part_name);
        let prefer = out_dir.join(format!(".part-{:03}.bin", p.index));
        match extract_part_payload(&downloaded, &prefer) {
            Ok(raw) => {
                let _ = std::fs::remove_file(&downloaded);
                files.push((p.index, raw));
                println!("[ok {n}/{total}] part {}", p.index);
                if n < total {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
            Err(e) => {
                fail += 1;
                let _ = std::fs::remove_file(&downloaded);
                eprintln!("[fail {n}/{total}] part {} extract: {e}", p.index);
            }
        }
    }
    if fail > 0 {
        for (_, p) in &files {
            let _ = std::fs::remove_file(p);
        }
        return Err(format!("{fail}/{total} split parts failed"));
    }
    files.sort_by_key(|(i, _)| *i);
    let out_path = out_dir.join(sanitize_name(&r.orig_name));
    let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
    for (_, p) in files {
        let mut inp = std::fs::File::open(&p).map_err(|e| e.to_string())?;
        std::io::copy(&mut inp, &mut out).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(p);
    }
    println!("[done] merged: {}", out_path.display());
    Ok(())
}

fn download_resolved(
    acc: &Account,
    r: ResolvedDownload,
    out_dir: &Path,
    jobs: usize,
) -> Result<(), String> {
    if r.kind == "split" {
        download_split_group(acc, &r, out_dir, jobs)
    } else {
        download_file_by_id(acc, &r.file_id, &r.file_name, out_dir)
    }
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
    let notes = acc.fetch_notes(&list);
    if let Some(r) = resolve_by_notes(&list, &notes, target) {
        return download_resolved(acc, r, out_dir, jobs);
    }
    if let Some(e) = resolve_entry(&list, target) {
        match e.kind {
            EntryKind::File => {
                let mut name = e.name.clone();
                if let Some(note) = notes.get(&e.id) {
                    if let Some(cm) = parse_convert_note(note) {
                        if !cm.name.is_empty() {
                            name = cm.name;
                        }
                    }
                }
                return download_file_by_id(acc, &e.id, &name, out_dir);
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

struct PathSeg {
    id: String,
    name: String,
}

struct Shell {
    acc: Account,
    path: Vec<PathSeg>,
    cookie: PathBuf,
    out_dir: String,
    jobs: usize,
    user: String,
    pass: String,
}

impl Shell {
    fn folder_id(&self) -> &str {
        self.path.last().map(|s| s.id.as_str()).unwrap_or("-1")
    }
    fn path_string(&self) -> String {
        if self.path.is_empty() {
            return "/".into();
        }
        let mut s = String::new();
        for seg in &self.path {
            s.push('/');
            s.push_str(&seg.name);
        }
        s
    }
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
        path: Vec::new(),
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
        print!("lanzou:{}> ", sh.path_string());
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
                println!("ls|ll|list|dir | cd <name|id|/abs/path|..|.> | pwd");
                println!("download|dl|down|fetch <id|name> [-j N] [-o DIR]");
                println!("info|show|stat <id|name> | upload|up|put <path>");
                println!("mkdir|md <name> | rm|delete|del|remove <id|name>");
                println!("mv|move <file> <dest-folder|/|-1> | rename|rn|ren <id|name> <new-name> [--note]");
                println!("login|signin [--user U --pass P] | logout|signout");
                println!("config|conf|cfg [list|get|set ...] | exit|quit|q");
                Ok(false)
            }
            "exit" | "quit" | "q" => Ok(true),
            "pwd" => {
                println!("{}", self.path_string());
                Ok(false)
            }
            "ls" | "ll" | "list" | "dir" => {
                let list = self.acc.list(self.folder_id()).map_err(|e| e.to_string())?;
                print_list(&self.acc, &self.path_string(), &list, true);
                Ok(false)
            }
            "cd" => {
                if args.is_empty() {
                    return Err("usage: cd <name|id|/abs/path|..|.>".into());
                }
                self.cd(&args[0])?;
                Ok(false)
            }
            "download" | "dl" | "down" | "fetch" | "get" => {
                self.cmd_download(args)?;
                Ok(false)
            }
            "info" | "show" | "stat" => {
                if args.is_empty() {
                    return Err("usage: info <id|name>".into());
                }
                self.cmd_info(&args[0])?;
                Ok(false)
            }
            "upload" | "up" | "put" => {
                if args.is_empty() {
                    return Err("usage: upload <local-path>".into());
                }
                let res = self
                    .acc
                    .upload(Path::new(&args[0]), self.folder_id())
                    .map_err(|e| e.to_string())?;
                println!("[ok] uploaded {} {}", res.file_id, res.name);
                Ok(false)
            }
            "mkdir" | "md" => {
                if args.is_empty() {
                    return Err("usage: mkdir <name>".into());
                }
                self.acc
                    .create_folder(&args[0], self.folder_id(), "")
                    .map_err(|e| e.to_string())?;
                println!("[ok] mkdir {}", args[0]);
                Ok(false)
            }
            "rm" | "delete" | "del" | "remove" | "unlink" => {
                if args.is_empty() {
                    return Err("usage: rm <id|name>".into());
                }
                self.cmd_rm(&args[0])?;
                Ok(false)
            }
            "mv" | "move" => {
                if args.len() < 2 {
                    return Err("usage: mv|move <file> <dest-folder|/|-1>".into());
                }
                move_target(&self.acc, self.folder_id(), &args[0], &args[1])?;
                Ok(false)
            }
            "rename" | "rn" | "ren" => {
                if args.len() < 2 {
                    return Err("usage: rename|rn|ren <id|name> <new-name> [--note]".into());
                }
                let note_only = args.iter().any(|a| a == "--note");
                let new_name = args
                    .iter()
                    .skip(1)
                    .rev()
                    .find(|a| a.as_str() != "--note")
                    .cloned()
                    .unwrap_or_else(|| args[1].clone());
                rename_target(&self.acc, self.folder_id(), &args[0], &new_name, note_only)?;
                Ok(false)
            }
            "login" | "signin" | "auth" => {
                self.cmd_login(args)?;
                Ok(false)
            }
            "logout" | "signout" => {
                let _ = std::fs::remove_file(&self.cookie);
                let _ = self.acc.set_cookie("");
                println!("[ok] logged out");
                Ok(false)
            }
            "config" | "conf" | "cfg" | "settings" => {
                let action = args.first().cloned();
                let key = args.get(1).cloned();
                let value = args.get(2).cloned();
                let _ = cmd_config(action, key, value);
                Ok(false)
            }
            _ => Err(format!("unknown command: {cmd} (help for list)")),
        }
    }

    fn cd(&mut self, target: &str) -> Result<(), String> {
        let target = target.trim();
        if target.is_empty() || target == "." {
            return Ok(());
        }
        if target.starts_with('/') || target == "~" || target == "root" {
            self.path.clear();
            let rest = if target == "~" || target == "root" {
                ""
            } else {
                target.trim_start_matches('/')
            };
            if rest.is_empty() {
                return Ok(());
            }
            return self.cd_relative(rest);
        }
        self.cd_relative(target)
    }

    fn cd_relative(&mut self, rel: &str) -> Result<(), String> {
        let rel = rel.replace('\\', "/");
        for p in rel.split('/') {
            let p = p.trim();
            if p.is_empty() || p == "." {
                continue;
            }
            if p == ".." {
                self.path.pop();
                continue;
            }
            let cur = self.folder_id().to_string();
            let list = self.acc.list(&cur).map_err(|e| e.to_string())?;
            if let Some(e) = resolve_entry(&list, p) {
                if e.kind != EntryKind::Folder {
                    return Err(format!("{} is a file, not a folder", e.name));
                }
                self.path.push(PathSeg {
                    id: e.id.clone(),
                    name: e.name.clone(),
                });
                continue;
            }
            if is_digits(p) {
                let mut name = p.to_string();
                if let Ok(info) = self.acc.get_folder_info(p) {
                    if !info.name.is_empty() {
                        name = info.name;
                    }
                }
                self.path.push(PathSeg {
                    id: p.into(),
                    name,
                });
                continue;
            }
            return Err(format!(
                "folder not found: {p} (at {})",
                self.path_string()
            ));
        }
        Ok(())
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
        download_target(&self.acc, self.folder_id(), target, Path::new(&out_dir), jobs)
    }

    fn cmd_info(&self, target: &str) -> Result<(), String> {
        let list = self.acc.list(self.folder_id()).map_err(|e| e.to_string())?;
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
        delete_target(&self.acc, self.folder_id(), target)
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
