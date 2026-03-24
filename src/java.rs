/// Returns the Java major version required by a given Paper server version string.
///
/// Mapping (source: https://docs.papermc.io/paper/getting-started):
/// - 1.17.x and earlier : Java 16 (handled as 17 — minimum JBR version we support)
/// - 1.18.x – 1.20.4    : Java 17
/// - 1.20.6 – 1.25.x    : Java 21  (1.20.5 doesn't exist; 1.20.6 was the first requiring 21)
/// - 1.26+               : Java 25
pub fn java_version_for_paper(paper_version: &str) -> u32 {
    let parts: Vec<u32> = paper_version
        .splitn(3, '.')
        .filter_map(|s| s.parse().ok())
        .collect();

    let minor = parts.get(1).copied().unwrap_or(0);
    let patch = parts.get(2).copied().unwrap_or(0);

    if minor >= 26 {
        return 25;
    }
    if minor >= 21 {
        return 21;
    }
    if minor == 20 && patch >= 5 {
        return 21;
    }
    17
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_java_version_mapping() {
        assert_eq!(java_version_for_paper("1.18.2"), 17);
        assert_eq!(java_version_for_paper("1.19.4"), 17);
        assert_eq!(java_version_for_paper("1.20.4"), 17);
        assert_eq!(java_version_for_paper("1.20.6"), 21);
        assert_eq!(java_version_for_paper("1.21.4"), 21);
        assert_eq!(java_version_for_paper("1.25.1"), 21);
        assert_eq!(java_version_for_paper("1.26.0"), 25);
    }
}
