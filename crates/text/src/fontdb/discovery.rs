//! Platform font-file discovery, enumeration and directory policy.

use super::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use super::{FaceStyle, Path, face_styles};

/// The font files held by the file at `path`: its contents, or for a suitcase font the fonts in
/// its resource fork ([`crate::suitcase`]). None when it can't be read.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn font_files(path: &Path) -> Vec<Vec<u8>> {
    if crate::suitcase::is_suitcase(path) {
        return crate::suitcase::read_fonts(path);
    }
    std::fs::read(path).into_iter().collect()
}

/// The tables holding outlines the font engine draws: TrueType, CFF, CFF2 and VARC. A face with
/// none of them (outlines in Apple's `hvgl` table, bitmaps only) draws nothing.
#[cfg(not(target_arch = "wasm32"))]
const OUTLINE_TABLES: [&[u8; 4]; 4] = [b"glyf", b"CFF ", b"CFF2", b"VARC"];

/// The styles of every face in the font file at `path` ([`face_styles`]), reading only its table
/// directories and `name` and `fvar` tables: a scan opens hundreds of font files, many of them
/// megabytes long. Faces without outlines the font engine draws (no [`OUTLINE_TABLES`] table) are
/// left out.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn file_face_names(path: &Path) -> Vec<FaceStyle> {
    if crate::suitcase::is_suitcase(path) {
        return crate::suitcase::read_fonts(path).into_iter().flat_map(|font| reader_face_names(std::io::Cursor::new(font))).collect();
    }
    match std::fs::File::open(path) {
        Ok(file) => reader_face_names(file),
        Err(_) => vec![],
    }
}

/// [`file_face_names`] for the font file `file` reads.
#[cfg(not(target_arch = "wasm32"))]
fn reader_face_names(mut file: impl std::io::Read + std::io::Seek) -> Vec<FaceStyle> {
    use std::io::SeekFrom;
    /// Caps on what a (possibly damaged) file can make the scan read.
    const MAX_FACES: u32 = 256;
    const MAX_NAME_TABLE: u32 = 1 << 20;
    let mut read_at = |offset: u64, len: usize| -> Option<Vec<u8>> {
        let mut buf = vec![0; len];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let be32 = |b: &[u8], at: usize| b.get(at..at + 4).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes);
    let Some(head) = read_at(0, 12) else { return vec![] };
    // A collection lists where each face's table directory starts.
    let starts: Vec<u32> = if head.starts_with(b"ttcf") {
        let n = be32(&head, 8).unwrap_or(0).min(MAX_FACES) as usize;
        read_at(12, n * 4).map(|b| b.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect()).unwrap_or_default()
    } else {
        vec![0]
    };
    starts
        .into_iter()
        .filter_map(|start| {
            let dir = read_at(start.into(), 12)?;
            let tables = u16::from_be_bytes(dir.get(4..6)?.try_into().ok()?) as usize;
            let records = read_at(u64::from(start) + 12, tables * 16)?;
            if !records.as_chunks::<16>().0.iter().any(|r| OUTLINE_TABLES.iter().any(|t| r.starts_with(*t))) {
                return None;
            }
            let mut table = |tag: &[u8; 4]| -> Option<Vec<u8>> {
                let rec: &[u8] = records.as_chunks::<16>().0.iter().find(|r| r.starts_with(tag))?;
                let (offset, len) = (be32(rec, 8)?, be32(rec, 12)?);
                if len > MAX_NAME_TABLE {
                    return None;
                }
                read_at(offset.into(), len as usize)
            };
            let name = table(b"name")?;
            // A variable font's named instances are styles too.
            let fvar = table(b"fvar");
            // Its OS/2 table (the font's classification, for the menus' filters), when it has one.
            let os2 = table(b"OS/2");
            // A font holding just these tables, to read them as the font itself would.
            let mut tables: Vec<(&[u8; 4], &[u8])> = vec![(b"name", &name)];
            if let Some(fvar) = &fvar {
                tables.push((b"fvar", fvar));
            }
            if let Some(os2) = &os2 {
                tables.push((b"OS/2", os2));
            }
            let font = sfnt_of(&tables)?;
            Some(face_styles(&skrifa::FontRef::new(&font).ok()?))
        })
        .flatten()
        .collect()
}

/// A font file holding `tables` (tag, data), sorted by tag as table directories are.
#[cfg(any(test, not(target_arch = "wasm32")))]
pub(crate) fn sfnt_of(tables: &[(&[u8; 4], &[u8])]) -> Option<Vec<u8>> {
    let mut tables = tables.to_vec();
    tables.sort_by_key(|t| *t.0);
    let n = u16::try_from(tables.len()).ok()?;
    let mut font = Vec::new();
    font.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    for v in [n, 0, 0, 0] {
        font.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend_from_slice(*tag);
        for v in [0, u32::try_from(offset).ok()?, u32::try_from(data.len()).ok()?] {
            font.extend_from_slice(&v.to_be_bytes());
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    Some(font)
}

/// Lists the font files the platform's font service knows, for [`set_platform_font_files`].
pub type PlatformFontFiles = fn() -> Vec<String>;

/// What lists the platform's font files, set by the app ([`set_platform_font_files`]).
#[cfg(not(target_arch = "wasm32"))]
static PLATFORM_FONT_FILES: std::sync::OnceLock<PlatformFontFiles> = std::sync::OnceLock::new();

/// Have [`system_font_dirs`] add the font files `list` gives (full paths) outside the font
/// folders, asked again by each scan and each check for installed fonts
/// ([`super::FontDb::installed_fonts_changed`]). The desktop app lists DirectWrite's system font
/// collection on Windows: fonts a font service such as Adobe Fonts loads in place, from files
/// outside the font folders and unknown to the registry (#579). On macOS it lists the fonts
/// CoreText's font manager has available, which apps and font managers can register from any
/// folder. Only the first call counts.
pub fn set_platform_font_files(list: PlatformFontFiles) {
    #[cfg(not(target_arch = "wasm32"))]
    // A second call keeps the first lister, as documented.
    let _ = PLATFORM_FONT_FILES.set(list);
    #[cfg(target_arch = "wasm32")]
    let _ = list;
}

/// Folders the user adds to the font scan (Preferences › Type › Additional Fonts Folder, #683),
/// read with their subfolders after the platform's.
#[cfg(not(target_arch = "wasm32"))]
static USER_FONT_DIRS: std::sync::RwLock<Vec<PathBuf>> = std::sync::RwLock::new(Vec::new());

/// Have [`system_font_dirs`] also read `dirs` (#683), so the fonts in them are listed and used as
/// if installed. The next check for installed fonts ([`super::FontDb::installed_fonts_changed`]) then
/// reports the change, and a scan catalogs them.
pub fn set_user_font_dirs(dirs: Vec<PathBuf>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        *USER_FONT_DIRS.write().unwrap_or_else(std::sync::PoisonError::into_inner) = dirs;
    }
    #[cfg(target_arch = "wasm32")]
    let _ = dirs;
}

/// The folders [`set_user_font_dirs`] added (none on wasm).
pub fn user_font_dirs() -> Vec<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    return USER_FONT_DIRS.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    #[cfg(target_arch = "wasm32")]
    Vec::new()
}

/// The platform's font folders (the system's and the user's) and the font files known to it
/// outside them (on Windows those registered with it, and those [`set_platform_font_files`]
/// lists), scanned by [`super::FontDb::global`].
pub fn system_font_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_arch = "wasm32") {
        return dirs;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts", "/Network/Library/Fonts"].map(Into::into));
        if let Some(h) = &home {
            dirs.push(h.join("Library/Fonts"));
        }
        // Fonts macOS downloads on demand (Yu Gothic, Yu Mincho, Tsukushi Maru Gothic…): one
        // asset folder per font, under a catalog folder whose number changes with macOS.
        for e in std::fs::read_dir("/System/Library/AssetsV2").into_iter().flatten().flatten() {
            if e.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font") {
                dirs.push(e.path());
            }
        }
    } else if cfg!(windows) {
        let root = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        dirs.push(root.join("Fonts"));
        // Fonts installed for the current user only (what Install does without administrator rights).
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(l).join("Microsoft\\Windows\\Fonts"));
        }
        #[cfg(windows)]
        dirs.extend(registered_font_files(&dirs));
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(Into::into));
        // The XDG data folders' `fonts` (fontconfig's user folder; Flatpak, Snap, NixOS and Guix
        // list their own data folders), relative paths ignored as the XDG spec says.
        let xdg = |var| std::env::var_os(var).map(|v| std::env::split_paths(&v).filter(|p| p.is_absolute()).collect::<Vec<_>>()).unwrap_or_default();
        let data_home = xdg("XDG_DATA_HOME").into_iter().next().or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        dirs.extend(data_home.into_iter().chain(xdg("XDG_DATA_DIRS")).map(|d| d.join("fonts")));
        if let Some(h) = &home {
            dirs.push(h.join(".fonts"));
        }
        // In a Flatpak sandbox, the host's fonts (system, local and the user's).
        dirs.extend(["/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts"].map(Into::into));
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(list) = PLATFORM_FONT_FILES.get() {
        let files = fonts_outside(list(), &dirs);
        dirs.extend(files);
    }
    dirs.extend(user_font_dirs());
    dirs
}

/// The registry key listing the fonts installed on Windows (for all users under the local
/// machine's key, for the current user under theirs): one value per face, holding its file's
/// name in the Windows font folder or its full path.
#[cfg(windows)]
const REGISTERED_FONTS: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";

/// The font files registered with Windows outside `font_dirs` (fonts installed as shortcuts to
/// where they are, or by programs into their own folders), which apps find by the registry.
#[cfg(windows)]
fn registered_font_files(font_dirs: &[PathBuf]) -> Vec<PathBuf> {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
    let values = [LOCAL_MACHINE, CURRENT_USER].into_iter().filter_map(|root| root.open(REGISTERED_FONTS).ok()).flat_map(|key| {
        // A value that isn't a string isn't a font file.
        key.values().map(|vs| vs.filter_map(|(_, v)| String::try_from(v).ok()).collect::<Vec<_>>()).unwrap_or_default()
    });
    fonts_outside(values, font_dirs)
}

/// The full paths among `registered` (font registrations' data) that aren't in `font_dirs`
/// (ignoring case, as Windows paths do), sorted and deduplicated. File names alone are files in
/// the Windows font folder, which is scanned anyway. The count is capped.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn fonts_outside(registered: impl IntoIterator<Item = String>, font_dirs: &[PathBuf]) -> Vec<PathBuf> {
    const MAX_REGISTERED: usize = 1 << 16;
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let dirs: Vec<PathBuf> = font_dirs.iter().map(|d| lower(d)).collect();
    let mut files: Vec<PathBuf> = registered
        .into_iter()
        .take(MAX_REGISTERED)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && !dirs.iter().any(|d| lower(p).starts_with(d)))
        .collect();
    files.sort();
    files.dedup();
    files
}
