//! System fonts for scripts the bundled DejaVu faces do not cover.
//!
//! Nothing is loaded up front: when text in one of these scripts first
//! shows up, a font for it is looked for on the computer (fontconfig on
//! Linux, then the Windows fonts folder, which WSL can also see) and read
//! on a background thread. Someone who never sees Chinese text never pays
//! the 15-20 MB such a font takes.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    /// Chinese characters (also used in Japanese and, rarely, Korean).
    Han,
    /// Japanese hiragana and katakana.
    Kana,
    Hangul,
    Thai,
    // The Indian scripts each need a font of their own: one with Hindi
    // letters may have no Tamil ones.
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Oriya,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Sinhala,
}

impl Script {
    /// The script's name in English; `tr` gives it in the user's language.
    pub fn name(self) -> &'static str {
        match self {
            Self::Han => crate::i18n::tr_key("Chinese characters"),
            Self::Kana => crate::i18n::tr_key("Japanese kana"),
            Self::Hangul => crate::i18n::tr_key("Korean"),
            Self::Thai => crate::i18n::tr_key("Thai"),
            Self::Devanagari => crate::i18n::tr_key("Devanagari"),
            Self::Bengali => crate::i18n::tr_key("Bengali"),
            Self::Gurmukhi => crate::i18n::tr_key("Gurmukhi"),
            Self::Gujarati => crate::i18n::tr_key("Gujarati"),
            Self::Oriya => crate::i18n::tr_key("Odia"),
            Self::Tamil => crate::i18n::tr_key("Tamil"),
            Self::Telugu => crate::i18n::tr_key("Telugu"),
            Self::Kannada => crate::i18n::tr_key("Kannada"),
            Self::Malayalam => crate::i18n::tr_key("Malayalam"),
            Self::Sinhala => crate::i18n::tr_key("Sinhala"),
        }
    }

    /// The fontconfig language that needs this script.
    #[cfg(target_os = "linux")]
    fn language(self) -> &'static str {
        match self {
            Self::Han => "zh-cn",
            Self::Kana => "ja",
            Self::Hangul => "ko",
            Self::Thai => "th",
            Self::Devanagari => "hi",
            Self::Bengali => "bn",
            Self::Gurmukhi => "pa",
            Self::Gujarati => "gu",
            Self::Oriya => "or",
            Self::Tamil => "ta",
            Self::Telugu => "te",
            Self::Kannada => "kn",
            Self::Malayalam => "ml",
            Self::Sinhala => "si",
        }
    }

    /// Font files that cover the script, best first, with the face index
    /// inside a collection (`.ttc`).
    fn known_files(self) -> &'static [(&'static str, u32)] {
        #[cfg(target_os = "macos")]
        {
            match self {
                Self::Han => &[
                    ("/System/Library/Fonts/PingFang.ttc", 0),
                    ("/System/Library/Fonts/STHeiti Light.ttc", 0),
                ],
                Self::Kana => &[("/System/Library/Fonts/Hiragino Sans GB.ttc", 0)],
                Self::Hangul => &[("/System/Library/Fonts/AppleSDGothicNeo.ttc", 0)],
                Self::Thai => &[("/System/Library/Fonts/Thonburi.ttc", 0)],
                Self::Devanagari => &[("/System/Library/Fonts/Kohinoor.ttc", 0)],
                Self::Bengali => &[("/System/Library/Fonts/KohinoorBangla.ttc", 0)],
                Self::Gujarati => &[("/System/Library/Fonts/KohinoorGujarati.ttc", 0)],
                Self::Telugu => &[("/System/Library/Fonts/KohinoorTelugu.ttc", 0)],
                Self::Tamil => &[("/System/Library/Fonts/Supplemental/Tamil MN.ttc", 0)],
                Self::Gurmukhi => &[("/System/Library/Fonts/Supplemental/Gurmukhi MN.ttc", 0)],
                Self::Oriya => &[("/System/Library/Fonts/Supplemental/Oriya MN.ttc", 0)],
                Self::Kannada => &[("/System/Library/Fonts/Supplemental/Kannada MN.ttc", 0)],
                Self::Malayalam => &[("/System/Library/Fonts/Supplemental/Malayalam MN.ttc", 0)],
                Self::Sinhala => &[("/System/Library/Fonts/Supplemental/Sinhala MN.ttc", 0)],
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            match self {
                Self::Han => &[
                    ("msyh.ttc", 0),
                    ("YuGothR.ttc", 0),
                    ("msjh.ttc", 0),
                    ("simsun.ttc", 0),
                ],
                Self::Kana => &[
                    ("YuGothR.ttc", 0),
                    ("meiryo.ttc", 0),
                    ("msgothic.ttc", 0),
                    ("msyh.ttc", 0),
                ],
                Self::Hangul => &[("malgun.ttf", 0), ("gulim.ttc", 0)],
                Self::Thai => &[("LeelawUI.ttf", 0), ("tahoma.ttf", 0)],
                // Nirmala UI covers every Indian script; the older fonts one each.
                Self::Devanagari => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("mangal.ttf", 0)],
                Self::Bengali => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("vrinda.ttf", 0)],
                Self::Gurmukhi => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("raavi.ttf", 0)],
                Self::Gujarati => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("shruti.ttf", 0)],
                Self::Oriya => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("kalinga.ttf", 0)],
                Self::Tamil => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("latha.ttf", 0)],
                Self::Telugu => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("gautami.ttf", 0)],
                Self::Kannada => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("tunga.ttf", 0)],
                Self::Malayalam => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("kartika.ttf", 0)],
                Self::Sinhala => &[("Nirmala.ttc", 0), ("Nirmala.ttf", 0), ("iskpota.ttf", 0)],
            }
        }
    }
}

/// The script of a character, if the bundled fonts lack it.
pub fn script_of(ch: char) -> Option<Script> {
    match ch as u32 {
        0x0900..=0x097F | 0xA8E0..=0xA8FF => Some(Script::Devanagari),
        0x0980..=0x09FF => Some(Script::Bengali),
        0x0A00..=0x0A7F => Some(Script::Gurmukhi),
        0x0A80..=0x0AFF => Some(Script::Gujarati),
        0x0B00..=0x0B7F => Some(Script::Oriya),
        0x0B80..=0x0BFF => Some(Script::Tamil),
        0x0C00..=0x0C7F => Some(Script::Telugu),
        0x0C80..=0x0CFF => Some(Script::Kannada),
        0x0D00..=0x0D7F => Some(Script::Malayalam),
        0x0D80..=0x0DFF => Some(Script::Sinhala),
        0x0E00..=0x0E7F => Some(Script::Thai),
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7AF => {
            Some(Script::Hangul)
        }
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => Some(Script::Kana),
        0x2E80..=0x2FDF
        | 0x3000..=0x303F
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFF00..=0xFF65
        | 0x20000..=0x2FFFF => Some(Script::Han),
        _ => None,
    }
}

/// The scripts in `text` that need a system font. Plain ASCII and the
/// alphabets DejaVu covers are passed over quickly.
pub fn scripts_in(text: &str, found: &mut Vec<Script>) {
    if text.is_ascii() {
        return;
    }
    for ch in text.chars() {
        if (ch as u32) < 0x0900 {
            continue;
        }
        if let Some(script) = script_of(ch)
            && !found.contains(&script)
        {
            found.push(script);
        }
    }
}

/// A font file and the face to use in it.
pub struct Found {
    pub path: PathBuf,
    pub index: u32,
}

/// Look for a font covering `script` on this computer.
pub fn find(script: Script) -> Option<Found> {
    #[cfg(target_os = "linux")]
    if let Some(found) = fontconfig(script) {
        return Some(found);
    }
    let folders = font_folders();
    for (file, index) in script.known_files() {
        let file = std::path::Path::new(file);
        if file.is_absolute() {
            if file.is_file() {
                return Some(Found {
                    path: file.to_path_buf(),
                    index: *index,
                });
            }
            continue;
        }
        for folder in &folders {
            let path = folder.join(file);
            if path.is_file() {
                return Some(Found {
                    path,
                    index: *index,
                });
            }
        }
    }
    None
}

/// Where Windows keeps fonts: the system folder and the per-user one. On
/// Linux this is the Windows folder as WSL sees it, if there is one.
fn font_folders() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let windows = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        let mut folders = vec![PathBuf::from(windows).join("Fonts")];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            folders.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
        }
        folders
    }
    #[cfg(target_os = "linux")]
    {
        vec![PathBuf::from("/mnt/c/Windows/Fonts")]
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Vec::new()
    }
}

/// Ask fontconfig for an installed font that supports the script's
/// language, preferring a regular sans face.
#[cfg(target_os = "linux")]
fn fontconfig(script: Script) -> Option<Found> {
    let output = std::process::Command::new("fc-list")
        .arg(format!(":lang={}", script.language()))
        .args(["-f", "%{file}\t%{index}\t%{style}\n"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    best_listed(&String::from_utf8_lossy(&output.stdout))
}

/// The best regular sans face in `fc-list` output of `file<TAB>index<TAB>style`
/// lines.
#[cfg(target_os = "linux")]
fn best_listed(text: &str) -> Option<Found> {
    let mut best: Option<(u32, Found)> = None;
    for line in text.lines() {
        let mut fields = line.split('\t');
        let (Some(file), Some(index)) = (fields.next(), fields.next()) else {
            continue;
        };
        let style = fields.next().unwrap_or_default().to_ascii_lowercase();
        let lower = file.to_ascii_lowercase();
        // Bitmap and bold faces make poor chat text.
        if !(lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".ttc")) {
            continue;
        }
        let mut score: u32 = 0;
        if style.contains("regular") || style.contains("normal") || style.contains("book") {
            score += 4;
        }
        if lower.contains("sans") {
            score += 2;
        }
        if lower.contains("noto") {
            score += 1;
        }
        if style.contains("bold") || style.contains("italic") || lower.contains("serif") {
            score = score.saturating_sub(3);
        }
        if best.as_ref().is_none_or(|(top, _)| score > *top) {
            best = Some((
                score,
                Found {
                    path: PathBuf::from(file),
                    index: index.trim().parse().unwrap_or(0),
                },
            ));
        }
    }
    best.map(|(_, found)| found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_told_apart() {
        let mut found = Vec::new();
        scripts_in("hello Привет ελληνικά עברית", &mut found);
        assert!(found.is_empty(), "DejaVu covers these");
        scripts_in("你好 こんにちは 안녕 สวัสดี नमस्ते 你", &mut found);
        assert_eq!(
            found,
            vec![
                Script::Han,
                Script::Kana,
                Script::Hangul,
                Script::Thai,
                Script::Devanagari
            ]
        );
        let mut found = Vec::new();
        scripts_in("தமிழ் বাংলা नमस्ते", &mut found);
        assert_eq!(
            found,
            vec![Script::Tamil, Script::Bengali, Script::Devanagari]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn each_indian_script_asks_for_its_own_language() {
        let all = [
            Script::Han,
            Script::Kana,
            Script::Hangul,
            Script::Thai,
            Script::Devanagari,
            Script::Bengali,
            Script::Gurmukhi,
            Script::Gujarati,
            Script::Oriya,
            Script::Tamil,
            Script::Telugu,
            Script::Kannada,
            Script::Malayalam,
            Script::Sinhala,
        ];
        let languages: std::collections::HashSet<&str> =
            all.iter().map(|script| script.language()).collect();
        assert_eq!(languages.len(), all.len(), "one language each");
        assert_eq!(Script::Tamil.language(), "ta");
        assert_eq!(Script::Devanagari.language(), "hi");
        assert_eq!(Script::Malayalam.language(), "ml");
        // Tamil text leads to a Tamil query, not a Hindi one.
        let mut found = Vec::new();
        scripts_in("வணக்கம்", &mut found);
        assert_eq!(found, vec![Script::Tamil]);
        assert_eq!(found[0].language(), "ta");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_regular_sans_face_is_picked_from_the_list() {
        let listed = "/f/NotoSerifTamil-Bold.ttf\t0\tBold\n\
                      /f/Lohit-Tamil.ttf\t0\tRegular\n\
                      /f/NotoSansTamil-Regular.ttf\t0\tRegular\n\
                      /f/tamil.pcf.gz\t0\tRegular\n\
                      /f/Collection.ttc\t2\tBook\n";
        let best = best_listed(listed).unwrap();
        assert_eq!(best.path, PathBuf::from("/f/NotoSansTamil-Regular.ttf"));
        let only_ttc = best_listed("/f/Collection.ttc\t2\tBook\n").unwrap();
        assert_eq!(only_ttc.index, 2, "the face inside a collection is kept");
        assert!(best_listed("/f/bitmap.pcf\t0\tRegular\n").is_none());
    }

    #[test]
    fn looking_for_a_font_is_safe_without_one() {
        // Whether or not this machine has one, the search must not panic,
        // and anything it returns must exist.
        for script in [Script::Han, Script::Thai] {
            if let Some(found) = find(script) {
                assert!(found.path.is_file());
            }
        }
    }
}
