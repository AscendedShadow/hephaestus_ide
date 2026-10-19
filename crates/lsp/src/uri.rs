use std::{
    path::{Path, PathBuf},
    str::FromStr as _,
};

use lsp_types::Uri;

const UNRESERVED: &str = "-._~:@$&()*+,;=!'";

pub fn path_to_uri(path: &Path) -> Option<Uri> {
    let path = path.to_str()?;
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let mut text = String::from("file://");
    if path.as_bytes().get(1) == Some(&b':') {
        text.push('/');
    }
    for character in path.chars() {
        match character {
            '\\' | '/' => text.push('/'),
            _ if character.is_ascii_alphanumeric() || UNRESERVED.contains(character) => {
                text.push(character)
            }
            _ => {
                let mut buffer = [0; 4];
                for byte in character.encode_utf8(&mut buffer).as_bytes() {
                    text.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    Uri::from_str(&text).ok()
}

pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let path = uri.path().as_str();
    let mut decoded = String::with_capacity(path.len());
    let mut bytes = Vec::new();
    let mut characters = path.chars();
    while let Some(character) = characters.next() {
        if character == '%' {
            let digits: String = characters.by_ref().take(2).collect();
            match u8::from_str_radix(&digits, 16) {
                Ok(byte) => bytes.push(byte),
                Err(_) => return None,
            }
            continue;
        }
        if !bytes.is_empty() {
            decoded.push_str(&String::from_utf8(std::mem::take(&mut bytes)).ok()?);
        }
        decoded.push(character);
    }
    if !bytes.is_empty() {
        decoded.push_str(&String::from_utf8(bytes).ok()?);
    }
    let path = match decoded.strip_prefix('/') {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest,
        _ => &decoded,
    };
    let path = PathBuf::from(path);
    Some(path.canonicalize().unwrap_or(path))
}
