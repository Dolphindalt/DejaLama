//! Write the five factory programs as VST3 preset files, so that hosts list them like the
//! original's program menu: `cargo run --example write_presets [dir]` (default `presets/`).
//! README.md says where hosts look for them.
//!
//! Each preset carries the program's three values (`PortTime`, `Delay`, `HeadSize`) and nothing
//! else, like the original's `setProgram`: loading one leaves the vowel and the pad alone.

use deja_lama::DejaLama;
use deja_lama::engine::{PROGRAMS, Program};
use nice_plug::plugin::{ParamValue, PluginState};
use nice_plug::prelude::{Plugin, Vst3Plugin};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

/// The plugin state a preset loads: nice-plug's JSON, with the program's parameters only.
fn component_state(program: Program) -> Vec<u8> {
    let params: BTreeMap<String, ParamValue> = [
        ("porttime", program.port_time),
        ("delay", program.delay),
        ("headsize", program.head_size),
    ]
    .into_iter()
    .map(|(id, value)| (id.to_owned(), ParamValue::F32(value)))
    .collect();
    let state = PluginState {
        version: DejaLama::VERSION.to_owned(),
        params,
        fields: BTreeMap::new(),
    };
    serde_json::to_vec(&state).expect("serialize the preset state")
}

/// Wrap a component state in the VST3 preset container: a header with the class ID and the
/// offset of the chunk list, the chunk data, then the list with one `Comp` entry.
fn vstpreset(class_id: [u8; 16], component_state: &[u8]) -> Vec<u8> {
    const HEADER_LEN: usize = 4 + 4 + 32 + 8;
    let mut out = Vec::with_capacity(HEADER_LEN + component_state.len() + 28);
    out.extend_from_slice(b"VST3");
    out.extend_from_slice(&1i32.to_le_bytes());
    for byte in class_id {
        out.extend_from_slice(format!("{byte:02X}").as_bytes());
    }
    let int64 = |n: usize| i64::try_from(n).expect("preset size").to_le_bytes();
    out.extend_from_slice(&int64(HEADER_LEN + component_state.len()));
    out.extend_from_slice(component_state);
    out.extend_from_slice(b"List");
    out.extend_from_slice(&1i32.to_le_bytes());
    out.extend_from_slice(b"Comp");
    out.extend_from_slice(&int64(HEADER_LEN));
    out.extend_from_slice(&int64(component_state.len()));
    out
}

fn main() -> ExitCode {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "presets".to_owned());
    let dir = Path::new(&dir);
    if let Err(err) = std::fs::create_dir_all(dir) {
        eprintln!("{}: {err}", dir.display());
        return ExitCode::FAILURE;
    }
    for program in PROGRAMS {
        let path = dir.join(format!("{}.vstpreset", program.name));
        let bytes = vstpreset(DejaLama::VST3_CLASS_ID, &component_state(program));
        if let Err(err) = std::fs::write(&path, bytes) {
            eprintln!("{}: {err}", path.display());
            return ExitCode::FAILURE;
        }
        println!("{}", path.display());
    }
    println!(
        "copy them to the VST3 preset directory for \"{}\" / \"{}\" (Linux: ~/.vst3/presets/{}/{}/)",
        DejaLama::VENDOR,
        DejaLama::NAME,
        DejaLama::VENDOR,
        DejaLama::NAME
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read the container back the way a host does and check the layout.
    #[test]
    fn container_holds_one_comp_chunk() {
        let state = component_state(PROGRAMS[0]);
        let file = vstpreset(DejaLama::VST3_CLASS_ID, &state);
        let int32 = |at: usize| i32::from_le_bytes(file[at..at + 4].try_into().unwrap());
        let int64 = |at: usize| {
            usize::try_from(i64::from_le_bytes(file[at..at + 8].try_into().unwrap())).unwrap()
        };
        assert_eq!(&file[..4], b"VST3");
        assert_eq!(int32(4), 1);
        assert_eq!(&file[8..40], b"44656A614C616D612D646F6C7068696E");
        let list = int64(40);
        assert_eq!(&file[list..list + 4], b"List");
        assert_eq!(int32(list + 4), 1);
        let entry = list + 8;
        assert_eq!(&file[entry..entry + 4], b"Comp");
        let (offset, size) = (int64(entry + 4), int64(entry + 12));
        assert_eq!(&file[offset..offset + size], &state[..]);
        assert_eq!(file.len(), entry + 20);
    }

    /// Parse the state back as nice-plug's `PluginState` and check the program's three values.
    #[test]
    fn state_carries_the_program_values() {
        let state: PluginState = serde_json::from_slice(&component_state(PROGRAMS[4])).unwrap();
        assert_eq!(state.version, DejaLama::VERSION);
        assert!(state.fields.is_empty());
        let value = |id: &str| match state.params[id] {
            ParamValue::F32(v) => v,
            ref other => panic!("{id}: {other:?}"),
        };
        assert_eq!(
            (value("porttime"), value("delay"), value("headsize")),
            (1.0, 0.9, 1.0)
        );
        assert_eq!(state.params.len(), 3);
    }
}
