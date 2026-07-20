# lanzou-share

Lanzou (蓝奏云) Rust library + CLI: share resolve **and** account ops.

Repository: <https://github.com/WangTianYou537/lanzou-share>

## Install CLI

```bash
cargo install lanzou-share
```

## CLI

```bash
# share
lanzou parse https://hya.lanzouu.com/xxx
lanzou https://hya.lanzouu.com/xxx   # legacy
lanzou parse --pwd 5grc --down https://wwbss.lanzouu.com/xxx

# account
lanzou login --user U --pass P
lanzou list
lanzou upload ./file.zip --folder -1
lanzou upload ./a.doc --set-pwd abcd --set-desc "说明"
lanzou mkdir demo
lanzou info --file 111
lanzou passwd --file 111 --pwd ab12
lanzou rm --file 111
lanzou logout
```

Env: `LANZOU_USER` / `LANZOU_PASS` / `LANZOU_COOKIE` (default `./lanzou.cookie`).

## Library

```toml
lanzou-share = "0.1.4"
```

```rust
use lanzou_share::{Account, Client, ParseOptions};

// share
let mut c = Client::new()?;
let r = c.parse(url, ParseOptions::default())?;

// account
let mut acc = Account::new("user", "pass")?.with_cookie_file("./lanzou.cookie");
acc.ensure_login()?;
acc.upload("./a.zip", "-1")?;
```

## License

MIT
