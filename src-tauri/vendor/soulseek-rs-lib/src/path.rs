use std::fmt;

/// A Soulseek protocol path.
///
/// Wire format: `<virtual_share_name>\<subdirs>\<filename>`
///
/// The virtual share name is the first `\`-separated component and is a
/// user-chosen alias for the shared folder (e.g. `@@rldqn`). It is not
/// meaningful to the downloader and is excluded from [`components`].
///
/// Paths are always stored in normalized backslash form; forward slashes
/// received from the wire are converted on construction via [`from_wire`].
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SoulseekPath(String);

impl SoulseekPath {
    /// Construct from a string that came off the wire.
    ///
    /// Normalizes any forward slashes to backslashes so the rest of the API
    /// can assume `\` is the sole separator.
    pub fn from_wire(s: String) -> Self {
        Self(s.replace('/', "\\"))
    }

    /// The raw protocol string as sent/received on the wire.
    ///
    /// Use this for wire writes and equality matching against other
    /// `SoulseekPath` values.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The filename — the last `\`-separated component.
    ///
    /// ```
    /// # use soulseek_rs::SoulseekPath;
    /// let p = SoulseekPath::from_wire("@@rldqn\\complete\\Canciones\\file.flac".into());
    /// assert_eq!(p.filename(), "file.flac");
    /// ```
    pub fn filename(&self) -> &str {
        self.0.split('\\').next_back().unwrap_or(&self.0)
    }

    /// All path components after the virtual share name, in order.
    ///
    /// The first `\`-separated segment is the virtual share name (the `@...`
    /// prefix) and is skipped. The returned slice contains every subdirectory
    /// component followed by the filename as the last element.
    ///
    /// Returns an empty slice if the path has no components beyond the share
    /// name, or if the path is empty.
    ///
    /// ```
    /// # use soulseek_rs::SoulseekPath;
    /// let p = SoulseekPath::from_wire("@@rldqn\\complete\\Canciones\\file.flac".into());
    /// assert_eq!(p.components(), vec!["complete", "Canciones", "file.flac"]);
    /// ```
    pub fn components(&self) -> Vec<&str> {
        let mut parts = self.0.split('\\');
        parts.next(); // skip virtual share name
        parts.filter(|s| !s.is_empty()).collect()
    }
}

impl From<String> for SoulseekPath {
    fn from(s: String) -> Self {
        Self::from_wire(s)
    }
}

impl From<&str> for SoulseekPath {
    fn from(s: &str) -> Self {
        Self::from_wire(s.to_string())
    }
}

impl AsRef<str> for SoulseekPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SoulseekPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for SoulseekPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SoulseekPath({:?})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::SoulseekPath;

    #[test]
    fn from_wire_normalizes_forward_slashes() {
        let p = SoulseekPath::from_wire("share/subdir/file.mp3".into());
        assert_eq!(p.as_str(), "share\\subdir\\file.mp3");
    }

    #[test]
    fn from_wire_noop_when_no_forward_slashes() {
        let p = SoulseekPath::from_wire("@@x\\dir\\file.mp3".into());
        assert_eq!(p.as_str(), "@@x\\dir\\file.mp3");
    }

    #[test]
    fn filename_typical_path() {
        let p = SoulseekPath::from_wire("@@rldqn\\complete\\Canciones\\file.flac".into());
        assert_eq!(p.filename(), "file.flac");
    }

    #[test]
    fn filename_bare_name() {
        let p = SoulseekPath::from_wire("file.mp3".into());
        assert_eq!(p.filename(), "file.mp3");
    }

    #[test]
    fn filename_windows_style() {
        let p = SoulseekPath::from_wire("C:\\path\\to\\file.mp3".into());
        assert_eq!(p.filename(), "file.mp3");
    }

    #[test]
    fn filename_real_protocol_path() {
        let p = SoulseekPath::from_wire(
            "@@bhfrv\\Soulseek Downloads\\complete\\Beatport Top Deep House (2021)\\michel test file.mp3".into(),
        );
        assert_eq!(p.filename(), "michel test file.mp3");
    }

    #[test]
    fn filename_forward_slash_path() {
        let p = SoulseekPath::from_wire("/path/to/file.mp3".into());
        assert_eq!(p.filename(), "file.mp3");
    }

    #[test]
    fn components_typical() {
        let p = SoulseekPath::from_wire("@@rldqn\\complete\\Canciones\\file.flac".into());
        assert_eq!(p.components(), vec!["complete", "Canciones", "file.flac"]);
    }

    #[test]
    fn components_single_level() {
        let p = SoulseekPath::from_wire("@@rldqn\\file.mp3".into());
        assert_eq!(p.components(), vec!["file.mp3"]);
    }

    #[test]
    fn components_bare_no_share() {
        let p = SoulseekPath::from_wire("file.mp3".into());
        // share name consumed ("file.mp3"), nothing left
        assert_eq!(p.components(), Vec::<&str>::new());
    }
}
