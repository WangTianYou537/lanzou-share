# lanzou-share

Lanzou (蓝奏云) share-link resolver **and account manager** for Rust.

Repository: <https://github.com/WangTianYou537/lanzou-share>

## Features

- public / password share resolve
- Alibaba WAF / ESA `acw_sc__v2` cookie
- CDN pseudo-link + risk-page fallback
- **account login / folder / file management** (ported from `lanzou.class.php`)
- optional local download

## Install CLI

```bash
cargo install lanzou-share
```

## Library

```toml
[dependencies]
lanzou-share = { version = "0.1.3", default-features = false }
```

### Share resolve

```rust
use lanzou_share::{Client, ParseOptions};

let mut client = Client::new()?;
let r = client.parse(
    "https://hya.lanzouu.com/iUTg43ww9ich",
    ParseOptions::default(),
)?;
```

### Account manager

```rust
use lanzou_share::Account;

let mut acc = Account::new("user", "password")?
    .with_cookie_file("./cookie.txt");
acc.ensure_login()?;
let list = acc.list("-1")?; // root
acc.create_folder("demo", "-1", "desc")?;
acc.set_file_password(&file_id, "abcd")?;
let (share_url, pwd) = acc.get_file_download_info(&file_id)?;
```

## CLI

```bash
lanzou https://hya.lanzouu.com/iUTg43ww9ich
lanzou --pwd 5grc --down https://wwbss.lanzouu.com/ioHpR10k7d4b
```

## License

MIT
