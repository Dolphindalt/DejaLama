//! Produce the editor's bitmaps, which the repository does not ship: they belong to the original
//! plugin. Take `Delay Lama.dll` from `DEJA_LAMA_DLL`, from `assets/`, or from the original
//! package in `assets/`, downloaded from the Internet Archive when missing; check both hashes;
//! read the eight bitmap resources and write them to `assets/*.png` for the editor to embed. Do
//! nothing while the eight files exist.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const PACKAGE_URL: &str = "https://archive.org/download/delay-lama-vst/Delay%20Lama.zip";
const PACKAGE_NAME: &str = "Delay Lama.zip";
const PACKAGE_SHA256: &str = "469ce87eb4ee80736919dd09aaa068d801de8fca4e57d6d2198c7b315ac09a40";
const DLL_NAME: &str = "Delay Lama.dll";
const DLL_SHA256: &str = "abf4d545935b664727a698124d4a2c3ad365e1949e2124460791635dacb5ac04";

/// The bitmap resources: id, file name, and whether white pixels become transparent (the three
/// handle images, which VSTGUI drew with white as the key colour).
const BITMAPS: [(u32, &str, bool); 8] = [
    (130, "background", false),
    (131, "monk_atlas", false),
    (141, "tri_vowel", true),
    (142, "tri_pitch", true),
    (151, "fader_handle", true),
    (152, "knob_glide", false),
    (153, "knob_voice", false),
    (160, "help", false),
];

const RT_BITMAP: u32 = 2;

fn main() -> ExitCode {
    let manifest_dir =
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let assets = Path::new(&manifest_dir).join("assets");
    println!("cargo::rerun-if-env-changed=DEJA_LAMA_DLL");
    for (_, name, _) in BITMAPS {
        println!(
            "cargo::rerun-if-changed={}",
            png_path(&assets, name).display()
        );
    }
    if BITMAPS
        .iter()
        .all(|(_, name, _)| png_path(&assets, name).exists())
    {
        return ExitCode::SUCCESS;
    }
    match generate(&assets) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            eprintln!(
                "The editor's bitmaps come from the original plugin. Put `{DLL_NAME}` or \
                 `{PACKAGE_NAME}` ({PACKAGE_URL}) into `assets/`, or point DEJA_LAMA_DLL at the \
                 DLL, and build again; delete a file that fails its hash check. The download \
                 needs `curl`."
            );
            ExitCode::FAILURE
        }
    }
}

fn png_path(assets: &Path, name: &str) -> PathBuf {
    assets.join(format!("{name}.png"))
}

fn generate(assets: &Path) -> Result<(), String> {
    let dll = load_dll(assets)?;
    for (id, name, key_white) in BITMAPS {
        let dib = bitmap_resource(&dll, id)
            .ok_or_else(|| format!("{DLL_NAME}: bitmap resource {id} not found"))?;
        let (width, height, rgba) = decode_dib(dib, key_white)?;
        write_png(&png_path(assets, name), width, height, &rgba)?;
    }
    Ok(())
}

/// Find the DLL, downloading the package when nothing local is present, and check its hash.
fn load_dll(assets: &Path) -> Result<Vec<u8>, String> {
    let from_env = std::env::var_os("DEJA_LAMA_DLL").map(PathBuf::from);
    let dll_path = from_env.clone().unwrap_or_else(|| assets.join(DLL_NAME));
    let bytes = if from_env.is_some() || dll_path.exists() {
        read(&dll_path)?
    } else {
        let package = assets.join(PACKAGE_NAME);
        if !package.exists() {
            download(&package)?;
        }
        let zip = read(&package)?;
        check_hash(&package, &zip, PACKAGE_SHA256)?;
        let bytes = extract(&package, &zip)?;
        check_hash(&package, &bytes, DLL_SHA256)?;
        return Ok(bytes);
    };
    check_hash(&dll_path, &bytes, DLL_SHA256)?;
    Ok(bytes)
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))
}

fn download(package: &Path) -> Result<(), String> {
    println!("cargo::warning=downloading {PACKAGE_URL} into assets/");
    std::fs::create_dir_all(package.parent().unwrap_or(Path::new(".")))
        .map_err(|err| format!("{}: {err}", package.display()))?;
    let status = Command::new("curl")
        .args([
            "--location",
            "--fail",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(package)
        .arg(PACKAGE_URL)
        .status()
        .map_err(|err| format!("curl: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        let _ = std::fs::remove_file(package);
        Err(format!("curl failed to download {PACKAGE_URL} ({status})"))
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

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let describe = |err: &dyn std::fmt::Display| format!("{}: {err}", path.display());
    let file = std::fs::File::create(path).map_err(|err| describe(&err))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|err| describe(&err))?;
    writer
        .write_image_data(rgba)
        .map_err(|err| describe(&err))?;
    writer.finish().map_err(|err| describe(&err))
}
