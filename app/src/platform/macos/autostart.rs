use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Error;

const PLIST_NAME: &str = "com.tilex.app.plist";

fn plist_path() -> Result<PathBuf, Error> {
    let home = std::env::var_os("HOME").ok_or_else(|| Error::Platform("HOME is not set".into()))?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(PLIST_NAME))
}

fn path_to_utf8(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .ok_or_else(|| Error::Platform("path is not valid UTF-8".into()))
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn generate_plist(exe_path: &str) -> String {
    let escaped = escape_xml(exe_path);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.tilex.app</string>
    <key>ProgramArguments</key>
    <array>
        <string>{escaped}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#
    )
}

fn parse_plist_exe(plist: &str) -> Option<String> {
    let source = plist
        .split_once("<key>ProgramArguments</key>")
        .map_or(plist, |(_, rest)| rest);
    let after_string = source.split_once("<string>")?.1;
    let (raw_val, _) = after_string.split_once("</string>")?;
    Some(unescape_xml(raw_val.trim()))
}

fn autostart_enabled_at(plist_file: &Path, current_exe: &Path) -> Result<bool, Error> {
    if !plist_file.exists() {
        return Ok(false);
    }
    let content = match fs::read_to_string(plist_file) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let Some(parsed_exe) = parse_plist_exe(&content) else {
        return Ok(false);
    };
    let current_str = path_to_utf8(current_exe)?;
    Ok(parsed_exe == current_str)
}

fn set_autostart_at(plist_file: &Path, current_exe: &Path, on: bool) -> Result<(), Error> {
    if on {
        if let Some(parent) = plist_file.parent() {
            fs::create_dir_all(parent)?;
        }
        let current_str = path_to_utf8(current_exe)?;
        let content = generate_plist(current_str);
        fs::write(plist_file, content)?;
        Ok(())
    } else {
        match fs::remove_file(plist_file) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

pub fn autostart_enabled() -> Result<bool, Error> {
    let path = plist_path()?;
    let exe = std::env::current_exe()?;
    autostart_enabled_at(&path, &exe)
}

pub fn set_autostart(on: bool) -> Result<(), Error> {
    let path = plist_path()?;
    let exe = std::env::current_exe()?;
    set_autostart_at(&path, &exe, on)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_parse_plist_roundtrip() {
        let fake_path = "/Applications/Ti&Lex 词典/TiLex";
        let plist = generate_plist(fake_path);
        let parsed = parse_plist_exe(&plist);
        assert_eq!(parsed.as_deref(), Some("/Applications/Ti&Lex 词典/TiLex"));
    }

    #[test]
    fn autostart_enabled_points_to_different_exe() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir =
            std::env::temp_dir().join(format!("tilex-autostart-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir); // ignore: 准备测试目录
        let plist_path = temp_dir.join(PLIST_NAME);

        let plist_content = generate_plist("/Applications/OtherApp/Other");
        fs::write(&plist_path, plist_content)?;

        let enabled = autostart_enabled_at(&plist_path, Path::new("/Applications/TiLex/TiLex"))?;
        assert!(!enabled);

        let _ = fs::remove_dir_all(&temp_dir); // ignore: 清理测试目录
        Ok(())
    }

    #[test]
    fn autostart_enabled_points_to_same_exe() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir =
            std::env::temp_dir().join(format!("tilex-autostart-same-{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir); // ignore: 准备测试目录
        let plist_path = temp_dir.join(PLIST_NAME);

        let exe_path = "/Applications/TiLex/TiLex";
        let plist_content = generate_plist(exe_path);
        fs::write(&plist_path, plist_content)?;

        let enabled = autostart_enabled_at(&plist_path, Path::new(exe_path))?;
        assert!(enabled);

        let _ = fs::remove_dir_all(&temp_dir); // ignore: 清理测试目录
        Ok(())
    }

    #[test]
    fn autostart_file_missing_returns_false() -> Result<(), Box<dyn std::error::Error>> {
        let missing = Path::new("/non/existent/com.tilex.app.plist");
        let enabled = autostart_enabled_at(missing, Path::new("/Applications/TiLex/TiLex"))?;
        assert!(!enabled);
        Ok(())
    }

    #[test]
    fn set_autostart_disable_when_missing_is_ok() -> Result<(), Box<dyn std::error::Error>> {
        let missing = Path::new("/non/existent/com.tilex.app.plist");
        let res = set_autostart_at(missing, Path::new("/Applications/TiLex/TiLex"), false);
        assert!(res.is_ok(), "{res:?}");
        Ok(())
    }
}
