// rustc test_latest.rs -o test_latest && ./test_latest

fn split_stem_ext(filename: &str) -> (&str, &str) {
    const KNOWN_EXTS: &[&str] = &[
        ".tar.gz.sha256",
        ".tar.xz.sha256",
        ".tar.bz2.sha256",
        ".tar.zst.sha256",
        ".zip.sha256",
        ".xz.sha256",
        ".gz.sha256",
        ".bz2.sha256",
        ".xz.asc",
        ".tar.gz",
        ".tar.xz",
        ".tar.bz2",
        ".tar.zst",
        ".xz",
        ".gz",
        ".bz2",
        ".zst",
        ".zip",
        ".sha256",
        ".sha512",
        ".exe",
        ".msi",
        ".deb",
        ".rpm",
    ];
    for ext in KNOWN_EXTS {
        if let Some(stem) = filename.strip_suffix(ext) { return (stem, ext); }
    }
    if let Some(pos) = filename.rfind('.') {
        let suffix = &filename[pos + 1..];
        if !suffix.is_empty() && !suffix.contains('-') && !suffix.chars().all(|c| c.is_ascii_digit()) {
            return (&filename[..pos], &filename[pos..]);
        }
    }
    (filename, "")
}
fn rename_to_latest(filename: &str) -> String {
    let (stem, ext) = split_stem_ext(filename);
    let sep = if stem.contains('-') { '-' } else { '_' };
    let parts: Vec<&str> = stem.split(sep).collect();
    let version_idx = parts.iter().position(|p| {
        let p = p.strip_prefix('v').unwrap_or(p);
        let mut iter = p.split('.');
        iter.next().map_or(false, |s| s.parse::<u64>().is_ok())
            && iter.next().map_or(false, |s| s.parse::<u64>().is_ok())
    });
    let suffix_parts = match version_idx {
        Some(idx) => &parts[idx + 1..],
        None => &parts[1..],
    };
    if suffix_parts.is_empty() { format!("latest{}", ext) }
    else { format!("latest{}{}{}", sep, suffix_parts.join(&sep.to_string()), ext) }
}
fn main() {
    let cases = [
        "forgejo-14.0.2-linux-amd64",
        "forgejo-14.0.2-linux-amd64.xz",
        "forgejo-14.0.2-linux-amd64.xz.sha256",
        "forgejo-14.0.2-linux-arm-6.xz.sha256",
        "forgejo-src-14.0.2.tar.gz.sha256",
        "bat-v0.26.1-x86_64-unknown-linux-gnu.tar.gz",
        "linux-6.19.2.tar.gz",
        "checkup-windows-x86_64.exe",
        "bat_0.26.1_amd64.deb",
        "bat_0.26.1_arm64.deb"
    ];
    for name in &cases { println!("{:<50} -> {}", name, rename_to_latest(name)); }
}
