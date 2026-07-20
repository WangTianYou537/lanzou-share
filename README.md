# lanzou-share

Lanzou (蓝奏云) share-link resolver library and CLI for Rust.

Repository: <https://github.com/WangTianYou537/lanzou-share>

Supports:

- public shares and password-protected shares
- Alibaba WAF / ESA `acw_sc__v2` challenge cookie
- CDN pseudo-link resolution (`developer*.lanrar.com`)
- optional risk-page fallback via `/file/ajax.php`
- optional local download

## Install CLI

```bash
cargo install lanzou-share
```

## Library

```toml
[dependencies]
lanzou-share = { version = "0.1", default-features = false }
```

```rust
use lanzou_share::{Client, ParseOptions};

fn main() -> lanzou_share::Result<()> {
    let mut client = Client::new()?;
    let r = client.parse(
        "https://hya.lanzouu.com/iUTg43ww9ich",
        ParseOptions::default(),
    )?;
    println!(
        "{} -> {}",
        r.filename.as_deref().unwrap_or("?"),
        r.direct.as_deref().unwrap_or(&r.telecom)
    );
    Ok(())
}
```

Password share:

```rust
let r = client.parse(
    "https://wwbss.lanzouu.com/ioHpR10k7d4b",
    ParseOptions {
        password: Some("5grc".into()),
        resolve_direct: true,
    },
)?;
```

## CLI

```bash
# public
lanzou https://hya.lanzouu.com/iUTg43ww9ich

# password + download
lanzou --pwd 5grc --down https://wwbss.lanzouu.com/ioHpR10k7d4b
```

## Notes

- Missing `Accept-Language` on CDN often triggers a risk HTML page.
- Password-required links raise `Error::PasswordRequired` when `--pwd` is omitted.

## License

MIT
