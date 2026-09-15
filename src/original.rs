//! Obtain the original plugin's bitmaps, which the repository does not ship: they belong to the
//! original. Take `Delay Lama.dll` from `DEJA_LAMA_DLL`, from the given folder, or from the
//! original package in that folder, downloaded from the Internet Archive when missing; check
//! both hashes; read the eight bitmap resources and write them as PNG files into the folder.
//! `build.rs` runs this at build time to embed the files; the plugin runs it at first launch.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const PACKAGE_URL: &str = "https://archive.org/download/delay-lama-vst/Delay%20Lama.zip";
pub const PACKAGE_NAME: &str = "Delay Lama.zip";
pub const PACKAGE_SHA256: &str = "469ce87eb4ee80736919dd09aaa068d801de8fca4e57d6d2198c7b315ac09a40";
pub const DLL_NAME: &str = "Delay Lama.dll";
pub const DLL_SHA256: &str = "abf4d545935b664727a698124d4a2c3ad365e1949e2124460791635dacb5ac04";

/// The bitmap resources: id, file name, and whether white pixels become transparent (the three
/// handle images, which VSTGUI drew with white as the key colour).
pub const BITMAPS: [(u32, &str, bool); 8] = [
    (130, "background", false),
    (131, "monk_atlas", false),
    (141, "tri_vowel", true),
    (142, "tri_pitch", true),
    (151, "fader_handle", true),
    (152, "knob_glide", false),
    (153, "knob_voice", false),
    (160, "help", false),
];

/// What to do when nothing worked.
pub const HINT: &str = "Put `Delay Lama.dll` or `Delay Lama.zip` (https://archive.org/download/delay-lama-vst/Delay%20Lama.zip) into the folder, or point DEJA_LAMA_DLL at the DLL; delete a file that fails its hash check. The download needs `curl`.";

const RT_BITMAP: u32 = 2;

/// The PNG file of a bitmap in `dir`.
pub fn bitmap_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.png"))
}

/// Whether all eight PNG files exist in `dir`.
pub fn bitmaps_present(dir: &Path) -> bool {
    BITMAPS
        .iter()
        .all(|(_, name, _)| bitmap_path(dir, name).exists())
}

/// Write the eight PNG files into `dir` unless they exist, obtaining the DLL as needed.
pub fn ensure_bitmaps(dir: &Path) -> Result<(), String> {
    if bitmaps_present(dir) {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let dll = obtain_dll(dir)?;
    for (id, name, key_white) in BITMAPS {
        let dib = bitmap_resource(&dll, id)
            .ok_or_else(|| format!("{DLL_NAME}: bitmap resource {id} not found"))?;
        let (width, height, rgba) = decode_dib(dib, key_white)?;
        write_png(&bitmap_path(dir, name), width, height, &rgba)?;
    }
    Ok(())
}

/// The eight PNG files of `dir`, in `BITMAPS` order.
pub fn read_bitmaps(dir: &Path) -> Result<[Vec<u8>; 8], String> {
    let mut files = Vec::with_capacity(BITMAPS.len());
    for (_, name, _) in BITMAPS {
        files.push(read(&bitmap_path(dir, name))?);
    }
    files
        .try_into()
        .map_err(|_| format!("{}: expected eight bitmaps", dir.display()))
}

/// Delete the eight PNG files of `dir`, so that `ensure_bitmaps` extracts them again.
pub fn remove_bitmaps(dir: &Path) -> Result<(), String> {
    for (_, name, _) in BITMAPS {
        let path = bitmap_path(dir, name);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        }
    }
    Ok(())
}

/// The folder the plugin keeps the bitmaps in: `DEJA_LAMA_ASSETS`, or the user's data
/// directory for this platform.
pub fn data_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    data_dir_from(
        var("DEJA_LAMA_ASSETS"),
        var("HOME"),
        var("XDG_DATA_HOME"),
        var("LOCALAPPDATA"),
    )
}

/// `data_dir` on the given environment: the override wins, then the platform's convention.
fn data_dir_from(
    assets: Option<PathBuf>,
    home: Option<PathBuf>,
    xdg_data_home: Option<PathBuf>,
    local_appdata: Option<PathBuf>,
) -> Option<PathBuf> {
    if assets.is_some() {
        return assets;
    }
    if cfg!(target_os = "windows") {
        local_appdata.map(|dir| dir.join("Deja Lama"))
    } else if cfg!(target_os = "macos") {
        home.map(|home| home.join("Library/Application Support/Deja Lama"))
    } else {
        xdg_data_home
            .or_else(|| home.map(|home| home.join(".local/share")))
            .map(|data| data.join("deja-lama"))
    }
}

/// Find the DLL, downloading the package when nothing local is present, and check its hash.
fn obtain_dll(dir: &Path) -> Result<Vec<u8>, String> {
    let from_env = std::env::var_os("DEJA_LAMA_DLL").map(PathBuf::from);
    let dll_path = from_env.clone().unwrap_or_else(|| dir.join(DLL_NAME));
    if from_env.is_some() || dll_path.exists() {
        let bytes = read(&dll_path)?;
        check_hash(&dll_path, &bytes, DLL_SHA256)?;
        return Ok(bytes);
    }
    let package = dir.join(PACKAGE_NAME);
    let zip = if package.exists() {
        let zip = read(&package)?;
        check_hash(&package, &zip, PACKAGE_SHA256)?;
        zip
    } else {
        download(&package)?
    };
    let bytes = extract(&package, &zip)?;
    check_hash(&package, &bytes, DLL_SHA256)?;
    Ok(bytes)
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))
}

/// A private name next to `path` for a file under construction, so that another instance
/// never sees a partial file at the final name.
fn partial(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".part-{}", std::process::id()));
    PathBuf::from(name)
}

/// Move a finished file to its final name.
fn finish(part: &Path, path: &Path) -> Result<(), String> {
    std::fs::rename(part, path).map_err(|err| format!("{}: {err}", path.display()))
}

/// Download the package into a partial file, keep it only when its hash matches, and return
/// its bytes.
fn download(package: &Path) -> Result<Vec<u8>, String> {
    let part = partial(package);
    let mut curl = Command::new("curl");
    curl.args([
        "--location",
        "--fail",
        "--silent",
        "--show-error",
        "--connect-timeout",
        "30",
        "--max-time",
        "300",
        "--retry",
        "2",
        "--output",
    ])
    .arg(&part)
    .arg(PACKAGE_URL)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        curl.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = curl.output().map_err(|err| format!("curl: {err}"))?;
    let result = if output.status.success() {
        read(&part).and_then(|zip| check_hash(&part, &zip, PACKAGE_SHA256).map(|()| zip))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "curl failed to download {PACKAGE_URL}: {}",
            stderr.trim()
        ))
    };
    match result {
        Ok(zip) => {
            finish(&part, package)?;
            Ok(zip)
        }
        Err(message) => {
            let _ = std::fs::remove_file(&part);
            Err(message)
        }
    }
}

fn extract(package: &Path, zip: &[u8]) -> Result<Vec<u8>, String> {
    let describe = |err: &dyn std::fmt::Display| format!("{}: {err}", package.display());
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).map_err(|err| describe(&err))?;
    let mut entry = archive.by_name(DLL_NAME).map_err(|err| describe(&err))?;
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .map_err(|err| describe(&err))?;
    Ok(bytes)
}

fn check_hash(path: &Path, bytes: &[u8], expected: &str) -> Result<(), String> {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(hex, "{byte:02x}").expect("write to a String");
    }
    if hex == expected {
        Ok(())
    } else {
        Err(format!(
            "{}: sha256 {hex} does not match the June 2002 release ({expected})",
            path.display()
        ))
    }
}

// ------------------------------------------------------------------ PE resources

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The raw DIB of bitmap resource `id` in a PE32 image, found through the resource directory:
/// type, then id, then the first language.
fn bitmap_resource(dll: &[u8], id: u32) -> Option<&[u8]> {
    let pe = u32_at(dll, 0x3c)? as usize;
    if dll.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let section_count = u16_at(dll, pe + 6)? as usize;
    let optional = pe + 24;
    let optional_size = u16_at(dll, pe + 20)? as usize;
    if u16_at(dll, optional)? != 0x10b {
        return None; // PE32 only
    }
    let resource_rva = u32_at(dll, optional + 96 + 2 * 8)? as usize;
    let sections: Vec<(usize, usize, usize)> = (0..section_count)
        .map(|i| {
            let s = optional + optional_size + 40 * i;
            Some((
                u32_at(dll, s + 12)? as usize,
                u32_at(dll, s + 8)?.max(u32_at(dll, s + 16)?) as usize,
                u32_at(dll, s + 20)? as usize,
            ))
        })
        .collect::<Option<_>>()?;
    let to_offset = |rva: usize| {
        sections
            .iter()
            .find(|&&(va, size, _)| va <= rva && rva < va + size)
            .map(|&(va, _, raw)| rva - va + raw)
    };
    let base = to_offset(resource_rva)?;

    // walk one directory level: the entry with `wanted`, or the first entry when `wanted` is None
    let walk = |dir: usize, wanted: Option<u32>| -> Option<u32> {
        let count = u16_at(dll, dir + 12)? as usize + u16_at(dll, dir + 14)? as usize;
        (0..count).map(|i| dir + 16 + 8 * i).find_map(|entry| {
            let name = u32_at(dll, entry)?;
            let data = u32_at(dll, entry + 4)?;
            (wanted.is_none_or(|w| w == name)).then_some(data)
        })
    };
    let subdir = |data: u32| base + (data & 0x7fff_ffff) as usize;
    let types = walk(base, Some(RT_BITMAP))?;
    let ids = walk(subdir(types), Some(id))?;
    let entry = walk(subdir(ids), None)?;
    let data_entry = base + entry as usize;
    let data_rva = u32_at(dll, data_entry)? as usize;
    let size = u32_at(dll, data_entry + 4)? as usize;
    let offset = to_offset(data_rva)?;
    dll.get(offset..offset + size)
}

// ------------------------------------------------------------------ DIB decoding

/// Decode an uncompressed 8-bit palette or 24-bit bottom-up DIB into RGBA rows, top-down.
fn decode_dib(dib: &[u8], key_white: bool) -> Result<(u32, u32, Vec<u8>), String> {
    let bad = |what: &str| format!("bitmap resource: {what}");
    let header = u32_at(dib, 0).ok_or_else(|| bad("truncated"))? as usize;
    let width = u32_at(dib, 4).ok_or_else(|| bad("truncated"))?;
    let height = u32_at(dib, 8)
        .ok_or_else(|| bad("truncated"))?
        .cast_signed();
    let bpp = u16_at(dib, 14).ok_or_else(|| bad("truncated"))?;
    let compression = u32_at(dib, 16).ok_or_else(|| bad("truncated"))?;
    let colours_used = u32_at(dib, 32).ok_or_else(|| bad("truncated"))? as usize;
    if compression != 0 || height <= 0 || !(bpp == 8 || bpp == 24) {
        return Err(bad("unsupported format"));
    }
    let height = height.cast_unsigned();
    let (columns, rows) = (width as usize, height as usize);
    let palette_len = if bpp == 8 {
        if colours_used == 0 { 256 } else { colours_used }
    } else {
        0
    };
    let palette = dib
        .get(header..header + 4 * palette_len)
        .ok_or_else(|| bad("truncated palette"))?;
    let pixels = dib
        .get(header + 4 * palette_len..)
        .ok_or_else(|| bad("truncated pixel data"))?;
    let stride = (columns * bpp as usize).div_ceil(32) * 4;
    if pixels.len() < stride * rows {
        return Err(bad("truncated pixel data"));
    }
    let mut rgba = Vec::with_capacity(columns * rows * 4);
    for row in (0..rows).rev() {
        let line = &pixels[row * stride..row * stride + stride];
        for x in 0..columns {
            let bgr: [u8; 3] = if bpp == 8 {
                let p = 4 * line[x] as usize;
                [palette[p], palette[p + 1], palette[p + 2]]
            } else {
                [line[3 * x], line[3 * x + 1], line[3 * x + 2]]
            };
            let [blue, green, red] = bgr;
            let alpha = if key_white && bgr == [255, 255, 255] {
                0
            } else {
                255
            };
            rgba.extend_from_slice(&[red, green, blue, alpha]);
        }
    }
    Ok((width, height, rgba))
}

/// Write a PNG through a partial file, so that the final name only ever holds a whole file.
fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let part = partial(path);
    let describe = |err: &dyn std::fmt::Display| format!("{}: {err}", path.display());
    let result = (|| {
        let file = std::fs::File::create(&part).map_err(|err| describe(&err))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|err| describe(&err))?;
        writer
            .write_image_data(rgba)
            .map_err(|err| describe(&err))?;
        writer.finish().map_err(|err| describe(&err))
    })();
    match result {
        Ok(()) => finish(&part, path),
        Err(message) => {
            let _ = std::fs::remove_file(&part);
            Err(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extract the eight bitmaps from the package in `assets/` (CI keeps it there) into a
    /// scratch folder and check their sizes; skip with a note when the package is absent.
    #[test]
    fn extracts_the_eight_bitmaps() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        if !assets.join(PACKAGE_NAME).exists() && !assets.join(DLL_NAME).exists() {
            eprintln!("no original package in assets/: skipping the extraction test");
            return;
        }
        let dir = std::env::temp_dir().join(format!("deja-lama-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in [PACKAGE_NAME, DLL_NAME] {
            if assets.join(name).exists() {
                std::fs::copy(assets.join(name), dir.join(name)).unwrap();
            }
        }
        ensure_bitmaps(&dir).unwrap();
        let bitmaps = read_bitmaps(&dir).unwrap();
        let sizes = [
            [360, 510],
            [1570, 1866],
            [10, 10],
            [10, 10],
            [20, 17],
            [50, 3000],
            [50, 3000],
            [253, 275],
        ];
        for ((bytes, size), (_, name, _)) in bitmaps.iter().zip(sizes).zip(BITMAPS) {
            let decoder = png::Decoder::new(Cursor::new(bytes));
            let info = decoder.read_info().unwrap().info().clone();
            assert_eq!([info.width, info.height], size, "{name}");
        }
        // present now: a second call does nothing
        assert!(bitmaps_present(&dir));
        ensure_bitmaps(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_dir_prefers_the_override_then_the_platform_folder() {
        let over = Some(PathBuf::from("/x/assets"));
        assert_eq!(data_dir_from(over.clone(), None, None, None), over);
        let home = Some(PathBuf::from("/home/u"));
        let dir = data_dir_from(None, home.clone(), None, Some(PathBuf::from("C:/a")));
        if cfg!(target_os = "windows") {
            assert_eq!(dir, Some(PathBuf::from("C:/a/Deja Lama")));
        } else if cfg!(target_os = "macos") {
            assert_eq!(
                dir,
                Some(PathBuf::from(
                    "/home/u/Library/Application Support/Deja Lama"
                ))
            );
        } else {
            assert_eq!(dir, Some(PathBuf::from("/home/u/.local/share/deja-lama")));
            let xdg = Some(PathBuf::from("/data"));
            assert_eq!(
                data_dir_from(None, home, xdg, None),
                Some(PathBuf::from("/data/deja-lama"))
            );
        }
    }
}
