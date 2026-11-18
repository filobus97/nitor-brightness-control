//! Update logic that needs no OS: version comparison, the GitHub release JSON
//! and the `SHA256SUMS.txt` format (spec §10 "Updates").

use crate::json::{Json, Parser, field, field_str};

/// Minimum time between automatic checks.
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `<owner>/<repo>` of the GitHub repository releases are published in.
pub fn repo_slug() -> &'static str {
    let url = REPOSITORY.trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    url.strip_prefix("https://github.com/").unwrap_or(url)
}

/// The edition this build is: `Nitor`, or `NitorPlus` with the `plus` feature.
/// Release assets are named after it, so the updater stays on the user's edition.
pub fn edition() -> &'static str {
    if cfg!(feature = "plus") {
        "NitorPlus"
    } else {
        "Nitor"
    }
}

/// Release asset holding the exe of this edition for the architecture this build targets.
pub fn asset_name() -> String {
    let arch = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };
    format!("{}-{arch}.exe", edition())
}

/// The version as shown in Settings: `0.2.0`, or `0.2.0 Plus`.
pub fn version_label() -> String {
    if cfg!(feature = "plus") {
        format!("{} Plus", current_version())
    } else {
        current_version().to_string()
    }
}

/// A window title starting with "Nitor", with the edition in it for Plus.
pub fn product_title(title: &str) -> String {
    if cfg!(feature = "plus") {
        title.replacen("Nitor", "Nitor Plus", 1)
    } else {
        title.to_string()
    }
}

/// `v1.2.3`, `1.2.3-beta.1` or `1.2.3+build` → `(1, 2, 3)`; a missing minor/patch is not accepted.
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// Whether `latest` is a higher version than `current` (unparsable versions are never newer).
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// Whether an automatic check is due (also when the clock moved back past the last check).
pub fn check_due(last_check: u64, now: u64) -> bool {
    last_check > now || now - last_check >= CHECK_INTERVAL_SECS
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

/// The parts of a GitHub release Nitor uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    /// The release page, for manual download.
    pub page: String,
    pub assets: Vec<Asset>,
}

impl Release {
    /// The tag without a leading `v`.
    pub fn version(&self) -> &str {
        self.tag.trim_start_matches(['v', 'V'])
    }

    pub fn asset_url(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.url.as_str())
    }
}

/// Parse a `releases/latest` response; `None` if `tag_name` or `html_url` is missing.
pub fn parse_release(json: &str) -> Option<Release> {
    let Json::Obj(top) = Parser::new(json).value()? else {
        return None;
    };
    let tag = field_str(&top, "tag_name")?;
    let page = field_str(&top, "html_url")?;
    let mut assets = Vec::new();
    if let Some(Json::Arr(items)) = field(&top, "assets") {
        for item in items {
            if let Json::Obj(o) = item
                && let (Some(name), Some(url)) =
                    (field_str(o, "name"), field_str(o, "browser_download_url"))
            {
                assets.push(Asset { name, url });
            }
        }
    }
    Some(Release { tag, page, assets })
}

/// The expected digest of `filename` from `sha256sum` output (`<hex>  <name>` lines).
pub fn parse_sha256sums(text: &str, filename: &str) -> Option<[u8; 32]> {
    text.lines().find_map(|line| {
        let (hex, name) = line.split_once(char::is_whitespace)?;
        // Binary mode prefixes the name with `*`.
        let name = name.trim_start().trim_start_matches('*');
        (name.trim_end() == filename)
            .then(|| parse_hex32(hex))
            .flatten()
    })
}

fn parse_hex32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  "url": "https://api.github.com/repos/filobus97/nitor-brightness-controll/releases/1",
  "assets_url": "https://api.github.com/repos/filobus97/nitor-brightness-controll/releases/1/assets",
  "html_url": "https:\/\/github.com\/filobus97\/nitor-brightness-controll\/releases\/tag\/v1.2.0",
  "id": 1,
  "author": {"login": "someone", "html_url": "https://github.com/someone", "site_admin": false},
  "tag_name": "v1.2.0",
  "draft": false,
  "prerelease": false,
  "name": "Nitor v1.2.0 \"fast\" è 😀",
  "assets": [
    {
      "name": "Nitor-arm64.exe",
      "uploader": {"login": "bot", "html_url": "https://github.com/bot"},
      "size": 1234,
      "browser_download_url": "https://github.com/filobus97/nitor-brightness-controll/releases/download/v1.2.0/Nitor-arm64.exe"
    },
    {
      "browser_download_url": "https://github.com/filobus97/nitor-brightness-controll/releases/download/v1.2.0/Nitor-x64.exe",
      "name": "Nitor-x64.exe"
    },
    {"name": "SHA256SUMS.txt", "browser_download_url": "https://example.com/SHA256SUMS.txt"}
  ],
  "body": "notes with \"quotes\", } and ] chars"
}"#;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.10.0-beta.1"), Some((0, 10, 0)));
        assert_eq!(parse_version("1.2.3+abc"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("latest"), None);
        assert!(is_newer("0.1.0", "v0.2.0"));
        assert!(is_newer("0.9.9", "0.10.0"));
        assert!(!is_newer("1.0.0", "v1.0.0"));
        assert!(!is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "garbage"));
    }

    #[test]
    fn due() {
        assert!(check_due(0, 1_700_000_000));
        assert!(!check_due(1000, 1000 + CHECK_INTERVAL_SECS - 1));
        assert!(check_due(1000, 1000 + CHECK_INTERVAL_SECS));
        assert!(check_due(5000, 100));
    }

    #[test]
    fn release_json() {
        let r = parse_release(SAMPLE).unwrap();
        assert_eq!(r.tag, "v1.2.0");
        assert_eq!(r.version(), "1.2.0");
        assert_eq!(
            r.page,
            "https://github.com/filobus97/nitor-brightness-controll/releases/tag/v1.2.0"
        );
        assert_eq!(r.assets.len(), 3);
        assert_eq!(
            r.asset_url("Nitor-x64.exe"),
            Some(
                "https://github.com/filobus97/nitor-brightness-controll/releases/download/v1.2.0/Nitor-x64.exe"
            )
        );
        assert_eq!(
            r.asset_url("SHA256SUMS.txt"),
            Some("https://example.com/SHA256SUMS.txt")
        );
        assert_eq!(r.asset_url("missing.exe"), None);
    }

    #[test]
    fn release_json_rejects_garbage() {
        assert!(parse_release("").is_none());
        assert!(parse_release("[]").is_none());
        assert!(parse_release(r#"{"tag_name": "v1"}"#).is_none());
        assert!(parse_release(r#"{"tag_name": "v1", "html_url": "x""#).is_none());
        // Rate-limit style error body
        assert!(parse_release(r#"{"message": "API rate limit exceeded"}"#).is_none());
    }

    #[test]
    fn sha256sums() {
        let a = "a".repeat(64);
        let b = "0123456789abcdef".repeat(4);
        let text = format!("{a}  Nitor-arm64.exe\n{b} *Nitor-x64.exe\r\nnot a line\n");
        assert_eq!(parse_sha256sums(&text, "Nitor-arm64.exe"), Some([0xaa; 32]));
        let x64 = parse_sha256sums(&text, "Nitor-x64.exe").unwrap();
        assert_eq!(x64[..4], [0x01, 0x23, 0x45, 0x67]);
        assert_eq!(parse_sha256sums(&text, "Nitor.exe"), None);
        assert_eq!(
            parse_sha256sums("abc  Nitor-x64.exe", "Nitor-x64.exe"),
            None
        );
    }

    #[test]
    fn repo_and_asset() {
        assert_eq!(repo_slug(), "filobus97/nitor-brightness-controll");
        let name = asset_name();
        assert!(name.starts_with(edition()) && name.ends_with(".exe"));
        if cfg!(feature = "plus") {
            assert_eq!(edition(), "NitorPlus");
            assert!(name.starts_with("NitorPlus-"));
            assert_eq!(version_label(), format!("{} Plus", current_version()));
            assert_eq!(product_title("Nitor — Settings"), "Nitor Plus — Settings");
        } else {
            assert_eq!(edition(), "Nitor");
            assert!(name.starts_with("Nitor-"));
            assert_eq!(version_label(), current_version());
            assert_eq!(product_title("Nitor — Settings"), "Nitor — Settings");
        }
    }
}
