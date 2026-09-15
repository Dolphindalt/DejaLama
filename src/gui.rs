//! The original Delay Lama editor, re-created pixel for pixel with egui.
//!
//! Keep everything here a mirror of the DLL's editor and of the VSTGUI 2.2 controls it creates:
//!
//! | control         | class (VSTGUI 2.2) | rect (l,t,r,b)  | notes                                  |
//! |-----------------|--------------------|-----------------|----------------------------------------|
//! | background      | CFrame background  | 0,0,360,510     |                                        |
//! | monk animation  | CMovieBitmap       | 22,5,336,316    | frame = round(29·mouth), 5×6 atlas     |
//! | Glide knob      | CAnimKnob          | 21,448,71,498   | 60 frames of 50×50, frame = ⌊v·59⌋     |
//! | Voice knob      | CAnimKnob          | 293,447,343,497 |                                        |
//! | Delay fader     | CHorizontalSlider  | 104,479,256,504 | handle x = 104 + ⌊v·131⌋, y = 479      |
//! | XY pad          | CHorizontalSlider  | 96,362,259,440  | invisible; x/163, y/78                 |
//! | vowel indicator | CVerticalSlider    | 86,358,96,446   | mouse disabled; y = 358 + ⌊(1 - v)·79⌋ |
//! | pitch indicator | CHorizontalSlider  | 93,352,265,362  | mouse disabled; x = 93 + ⌊v·161⌋       |
//! | "?" help        | CSplashScreen      | 284,300,327,335 | shows 253×275 image at 57,13 (modal)   |

use crate::DejaLamaParams;
use crate::original;
use crate::shared::GuiShared;
use egui::{Color32, ColorImage, Context, Pos2, Rect, TextureHandle, TextureOptions, Vec2};
use nice_plug::context::gui::GuiContext;
use nice_plug::editor::dpi::LogicalSize;
use nice_plug::prelude::*;
use nice_plug_egui::baseview;
use nice_plug_egui::{EguiEditor, EguiEditorState, EguiNiceSettings, NiceEguiApp, RepaintNotifier};

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

pub const WIDTH: u16 = 360;
pub const HEIGHT: u16 = 510;

// ---- geometry (see the table above) ----
const MONK_POS: Vec2 = Vec2::new(22.0, 5.0);
const FRAME_W: usize = 314;
const FRAME_H: usize = 311;
const FRAME_SIZE: Vec2 = Vec2::new(314.0, 311.0);
const MONK_FRAMES: usize = 30;
const ATLAS_W: usize = 1570;
const ATLAS_H: usize = 1866;
const KNOB_GLIDE: Rect = Rect::from_min_max(Pos2::new(21.0, 448.0), Pos2::new(71.0, 498.0));
const KNOB_VOICE: Rect = Rect::from_min_max(Pos2::new(293.0, 447.0), Pos2::new(343.0, 497.0));
const KNOB_FRAMES: usize = 60;
const FADER: Rect = Rect::from_min_max(Pos2::new(104.0, 479.0), Pos2::new(256.0, 504.0));
const FADER_MIN_X: f32 = 104.0;
const FADER_MAX_X: f32 = 235.0; // 255 - handle width
const FADER_HANDLE_W: f32 = 20.0;
const PAD: Rect = Rect::from_min_max(Pos2::new(96.0, 362.0), Pos2::new(259.0, 440.0));
const PAD_X_RANGE: f32 = 163.0;
const PAD_Y_RANGE: f32 = 78.0;
const TRI_VOWEL_X: f32 = 86.0;
const TRI_VOWEL_MIN_Y: f32 = 358.0;
const TRI_VOWEL_RANGE: f32 = 79.0;
const TRI_PITCH_Y: f32 = 352.0;
const TRI_PITCH_MIN_X: f32 = 93.0;
const TRI_PITCH_RANGE: f32 = 161.0;
const HELP_BUTTON: Rect = Rect::from_min_max(Pos2::new(284.0, 300.0), Pos2::new(327.0, 335.0));
const HELP_POS: Vec2 = Vec2::new(57.0, 13.0);

// CControl / CKnob / CSlider constants from the DLL
const KNOB_DEFAULT: f32 = 0.5; // CControl::defaultValue
const FADER_DEFAULT: f32 = 0.75; // the fader's ctrl-click value in the original
const KNOB_RANGE_PX: f32 = 200.0; // linear mode: full range over 200 px
const KNOB_ZOOM: f32 = 1.5; // shift in linear mode
const SLIDER_ZOOM: f32 = 10.0; // shift on the fader
const KNOB_START_ANGLE: f32 = 3.926_990_8; // 5π/4 as float (setStartAngle)
const KNOB_RANGE_ANGLE: f32 = -4.712_389; // -3π/2 as float (setRangeAngle, clockwise)
const TWO_PI: f32 = 6.283_185_5; // VSTGUI's k2PI as float
// The double 2π that `valueFromPoint` compares with.
const TWO_PI_D: f64 = std::f64::consts::TAU;
// `CKnob::compute`: aCoef = (vmax - vmin) / rangeAngle and halfAngle = (k2PI - |rangeAngle|) / 2,
// computed on the FPU and stored as floats; the casts are those float stores.
#[allow(clippy::cast_possible_truncation)]
const KNOB_A_COEF: f32 = (1.0 / KNOB_RANGE_ANGLE as f64) as f32;
#[allow(clippy::manual_midpoint)]
const KNOB_HALF_ANGLE: f32 = (TWO_PI + KNOB_RANGE_ANGLE) * 0.5;

/// Knob drag mode. Circular is VSTGUI's default; the original let VST2 hosts switch it, and no
/// host switches it here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnobMode {
    Circular,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Knob {
    Glide,
    Voice,
}

/// `CKnob::mouse` in linear mode: value = entry + (Δx - Δy)·coef.
#[derive(Clone, Copy, Debug)]
struct LinearDrag {
    which: Knob,
    first: Pos2,
    entry: f32,
    coef: f32,
    shift: bool,
}

impl LinearDrag {
    /// Start at the pointer with the knob's current value; shift zooms the range.
    fn new(which: Knob, first: Pos2, entry: f32, shift: bool) -> Self {
        Self {
            which,
            first,
            entry,
            coef: 1.0 / Self::range(shift),
            shift,
        }
    }

    fn range(shift: bool) -> f32 {
        if shift {
            KNOB_RANGE_PX * KNOB_ZOOM
        } else {
            KNOB_RANGE_PX
        }
    }

    /// The value for the pointer position, re-anchored when shift toggles so it does not jump.
    fn update(&mut self, pos: Pos2, shift_now: bool) -> f32 {
        let diff = (pos.x - pos.y) - (self.first.x - self.first.y);
        if shift_now != self.shift {
            let new_coef = 1.0 / Self::range(shift_now);
            self.entry += (self.coef - new_coef) * diff;
            self.coef = new_coef;
            self.shift = shift_now;
        }
        (diff * self.coef + self.entry).clamp(0.0, 1.0)
    }
}

/// `CHorizontalSlider::mouse` with free click: the handle centres on the pointer.
#[derive(Clone, Copy, Debug)]
struct FaderDrag {
    /// bFreeClick: delta = iMinPos + widthOfSlider/2 - 1
    delta: f32,
    /// the anchor of the shift fine adjustment
    old_val: f32,
    shift: bool,
}

impl FaderDrag {
    fn new(current: f32, shift: bool) -> Self {
        Self {
            delta: FADER_MIN_X + FADER_HANDLE_W / 2.0 - 1.0,
            old_val: current,
            shift,
        }
    }

    /// The value for the pointer position. Take the fine-adjust anchor when shift goes down and
    /// track the value while shift is up, like `CSlider::mouse`.
    fn update(&mut self, pos: Pos2, current: f32, shift_now: bool) -> f32 {
        if !shift_now || shift_now != self.shift {
            self.old_val = current;
        }
        let mut v = (pos.x - self.delta) / (FADER_MAX_X - FADER_MIN_X);
        if shift_now {
            v = self.old_val + (v - self.old_val) / SLIDER_ZOOM;
        }
        self.shift = shift_now;
        v.clamp(0.0, 1.0)
    }
}

/// The control under the held button, with the state its VSTGUI mouse loop kept on the stack.
#[derive(Clone, Copy, Debug)]
enum Drag {
    None,
    /// `CKnob::mouse` in circular mode: the value follows the angle around the knob centre.
    KnobCircular {
        which: Knob,
        old_value: f32,
    },
    KnobLinear(LinearDrag),
    Fader(FaderDrag),
    /// The XY pad: pressed = note on, released = note off.
    Pad,
}

/// Repack the 60-frame 50×3000 film strips into a 6×10 grid (300×500) and split the monk atlas
/// into one texture per frame: hosts report a maximum texture side of 2048, which the original
/// 50×3000 strips exceed, and egui panics on oversize textures.
const KNOB_GRID_COLS: usize = 6;
const KNOB_GRID_ROWS: usize = 10;

struct Textures {
    background: TextureHandle,
    /// 30 monk frames, index = frame number (column-major order of the original atlas)
    monk: Vec<TextureHandle>,
    tri_vowel: TextureHandle,
    tri_pitch: TextureHandle,
    fader_handle: TextureHandle,
    knob_glide: TextureHandle,
    knob_voice: TextureHandle,
    help: TextureHandle,
}

impl Textures {
    /// Upload the eight decoded bitmaps (in `original::BITMAPS` order, sizes checked by
    /// `decode_bitmaps`) as textures.
    fn new(ctx: &Context, images: [ColorImage; 8]) -> Self {
        let [
            background,
            atlas,
            tri_vowel,
            tri_pitch,
            fader_handle,
            glide,
            voice,
            help,
        ] = images;
        let load =
            |name: &str, image: ColorImage| ctx.load_texture(name, image, TextureOptions::NEAREST);
        Self {
            background: load("background", background),
            monk: monk_frames(&atlas)
                .into_iter()
                .enumerate()
                .map(|(frame, image)| load(&format!("monk_{frame}"), image))
                .collect(),
            tri_vowel: load("tri_vowel", tri_vowel),
            tri_pitch: load("tri_pitch", tri_pitch),
            fader_handle: load("fader_handle", fader_handle),
            knob_glide: load("knob_glide", knob_grid(&glide)),
            knob_voice: load("knob_voice", knob_grid(&voice)),
            help: load("help", help),
        }
    }
}

/// The eight bitmaps' sizes, in `original::BITMAPS` order.
const BITMAP_SIZES: [[usize; 2]; 8] = [
    [360, 510],
    [ATLAS_W, ATLAS_H],
    [10, 10],
    [10, 10],
    [20, 17],
    [50, 50 * KNOB_FRAMES],
    [50, 50 * KNOB_FRAMES],
    [253, 275],
];

/// Decode the eight PNGs and check their sizes, so that nothing after this can fail.
fn decode_bitmaps(bitmaps: [&[u8]; 8]) -> Result<[ColorImage; 8], String> {
    let mut images = Vec::with_capacity(8);
    for ((bytes, size), (_, name, _)) in bitmaps.iter().zip(BITMAP_SIZES).zip(original::BITMAPS) {
        let image = decode_png(bytes).map_err(|err| format!("{name}.png: {err}"))?;
        if image.size != size {
            return Err(format!(
                "{name}.png: {}x{} instead of {}x{}",
                image.width(),
                image.height(),
                size[0],
                size[1]
            ));
        }
        images.push(image);
    }
    images.try_into().map_err(|_| "eight bitmaps".to_owned())
}

/// Read and decode the PNG files in `dir`; a missing, truncated or foreign file is an error.
#[cfg(not(feature = "embed-assets"))]
fn read_images(dir: &std::path::Path) -> Result<[ColorImage; 8], String> {
    let bitmaps = original::read_bitmaps(dir)?;
    decode_bitmaps(bitmaps.each_ref().map(Vec::as_slice))
}

/// The loader thread's work: make sure the files exist, then decode them; extract them again
/// once when they exist but fail to decode.
#[cfg(not(feature = "embed-assets"))]
fn obtain_images(dir: &std::path::Path) -> Result<[ColorImage; 8], String> {
    original::ensure_bitmaps(dir)?;
    if let Ok(images) = read_images(dir) {
        return Ok(images);
    }
    original::remove_bitmaps(dir)?;
    original::ensure_bitmaps(dir)?;
    read_images(dir).map_err(|err| format!("{err}. Delete the PNG files in the folder and retry."))
}

/// The bitmaps baked into the bundle by `build.rs`.
#[cfg(feature = "embed-assets")]
fn embedded_bitmaps() -> [&'static [u8]; 8] {
    [
        include_bytes!("../assets/background.png"),
        include_bytes!("../assets/monk_atlas.png"),
        include_bytes!("../assets/tri_vowel.png"),
        include_bytes!("../assets/tri_pitch.png"),
        include_bytes!("../assets/fader_handle.png"),
        include_bytes!("../assets/knob_glide.png"),
        include_bytes!("../assets/knob_voice.png"),
        include_bytes!("../assets/help.png"),
    ]
}

/// Where the bitmaps come from at run time, and how far the loader got.
enum Assets {
    /// The loader thread runs: it fetches the original package into the data folder when the
    /// bitmaps are missing, then decodes them.
    #[cfg_attr(feature = "embed-assets", allow(dead_code))]
    Loading {
        receiver: Receiver<Result<[ColorImage; 8], String>>,
        thread: Option<JoinHandle<()>>,
    },
    Ready(Textures),
    Failed(String),
}

pub struct DejaLamaGui {
    params: Arc<DejaLamaParams>,
    shared: Arc<GuiShared>,
    gui_ctx: Option<GuiContext>,
    assets: Option<Assets>,
    drag: Drag,
    help_shown: bool,
    knob_mode: KnobMode,
}

impl DejaLamaGui {
    pub fn new(params: Arc<DejaLamaParams>, shared: Arc<GuiShared>) -> Self {
        Self {
            params,
            shared,
            gui_ctx: None,
            assets: None,
            drag: Drag::None,
            help_shown: false,
            knob_mode: KnobMode::Circular,
        }
    }

    fn textures(&self) -> Option<&Textures> {
        match &self.assets {
            Some(Assets::Ready(textures)) => Some(textures),
            _ => None,
        }
    }

    /// Take the cached bitmaps straight to textures when they are all there and decode, or
    /// start the loader thread, which fetches and extracts them; the window shows a message
    /// meanwhile instead of blocking. Keep a loader that already runs.
    #[cfg(not(feature = "embed-assets"))]
    fn load_or_start(&mut self, ctx: &Context) {
        if let Some(Assets::Loading { .. }) = &self.assets {
            return;
        }
        if let Some(dir) = original::data_dir()
            && original::bitmaps_present(&dir)
            && let Ok(images) = read_images(&dir)
        {
            self.assets = Some(Assets::Ready(Textures::new(ctx, images)));
            return;
        }
        self.start_loading();
    }

    #[cfg(not(feature = "embed-assets"))]
    fn start_loading(&mut self) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let result = original::data_dir()
                .ok_or_else(|| "no data folder: set DEJA_LAMA_ASSETS".to_owned())
                .and_then(|dir| obtain_images(&dir));
            let _ = sender.send(result);
        });
        self.assets = Some(Assets::Loading {
            receiver,
            thread: Some(thread),
        });
    }

    /// Turn a finished loader into textures, or into the error to show.
    fn poll_loader(&mut self, ctx: &Context) {
        let Some(Assets::Loading { receiver, thread }) = &mut self.assets else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the loader thread stopped; retry".to_owned()),
        };
        if let Some(thread) = thread.take() {
            let _ = thread.join();
        }
        self.assets = Some(match result {
            Ok(images) => Assets::Ready(Textures::new(ctx, images)),
            Err(message) => Assets::Failed(message),
        });
    }

    /// The window while the bitmaps are not there yet: a message, and a way to retry.
    fn draw_status(&mut self, ui: &mut egui::Ui, origin: Pos2) {
        let window = Rect::from_min_size(origin, Vec2::new(f32::from(WIDTH), f32::from(HEIGHT)));
        ui.painter()
            .rect_filled(window, 0.0, Color32::from_rgb(28, 24, 20));
        let folder =
            original::data_dir().map_or_else(|| "?".to_owned(), |dir| dir.display().to_string());
        let mut retry = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(window.shrink(24.0)), |ui| {
            ui.style_mut().visuals.override_text_color = Some(Color32::from_rgb(232, 220, 200));
            ui.add_space(40.0);
            ui.heading("Deja Lama");
            ui.add_space(16.0);
            if let Some(Assets::Failed(message)) = &self.assets {
                ui.label("The original's bitmaps could not be fetched.");
                ui.add_space(8.0);
                ui.label(message.as_str());
                ui.add_space(8.0);
                ui.label(format!("Folder: {folder}"));
                ui.add_space(8.0);
                ui.label(original::HINT);
                ui.add_space(16.0);
                retry = ui.button("Retry").clicked();
            } else {
                ui.label(
                    "Fetching the original Delay Lama package (1.3 MB) from the Internet \
                         Archive and extracting its bitmaps.",
                );
                ui.add_space(8.0);
                ui.label(format!("Folder: {folder}"));
            }
        });
        #[cfg(not(feature = "embed-assets"))]
        if retry {
            self.start_loading();
        }
        #[cfg(feature = "embed-assets")]
        let _ = retry;
    }
}

/// Build the editor with the original's fixed 360×510 size.
pub fn create_editor(
    params: Arc<DejaLamaParams>,
    shared: Arc<GuiShared>,
) -> Option<EguiEditor<DejaLamaGui>> {
    nice_plug_egui::create_egui_editor(
        EguiEditorState::from_size(LogicalSize::new(f64::from(WIDTH), f64::from(HEIGHT)), 1.0),
        RepaintNotifier::new(),
        EguiNiceSettings::new().with_tile("Deja Lama"),
        DejaLamaGui::new(params, shared),
    )
}

fn decode_png(bytes: &[u8]) -> Result<ColorImage, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|err| err.to_string())?;
    let mut buf = vec![0u8; reader.output_buffer_size().ok_or("size overflow")?];
    let info = reader.next_frame(&mut buf).map_err(|err| err.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3]
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => return Err(format!("unexpected PNG colour type {other:?}")),
    };
    Ok(ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}

/// Split the monk atlas (5 columns × 6 rows, column-major) into its 30 frames.
fn monk_frames(atlas: &ColorImage) -> Vec<ColorImage> {
    (0..MONK_FRAMES)
        .map(|frame| {
            let (col, row) = (frame / 6, frame % 6);
            atlas.region_by_pixels([col * FRAME_W, row * FRAME_H], [FRAME_W, FRAME_H])
        })
        .collect()
}

/// Repack a vertical film strip of square frames into a `KNOB_GRID_COLS`-wide grid.
fn knob_grid(strip: &ColorImage) -> ColorImage {
    let size = strip.width();
    let mut grid = ColorImage::filled(
        [size * KNOB_GRID_COLS, size * KNOB_GRID_ROWS],
        Color32::TRANSPARENT,
    );
    for frame in 0..KNOB_FRAMES {
        let (col, row) = (frame % KNOB_GRID_COLS, frame / KNOB_GRID_COLS);
        for y in 0..size {
            let s0 = (frame * size + y) * size;
            let d0 = (row * size + y) * grid.width() + col * size;
            grid.pixels[d0..d0 + size].copy_from_slice(&strip.pixels[s0..s0 + size]);
        }
    }
    grid
}

/// Texture coordinates of the `CAnimKnob` frame for a knob value: frame = (int)(v·59), stored
/// column-major in the repacked `KNOB_GRID_COLS` × `KNOB_GRID_ROWS` grid.
fn knob_frame_uv(v: f32) -> Rect {
    // multiply in wide precision and truncate like the DLL's x87 code; the caller clamps v to
    // 0..=1
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let frame = ((f64::from(v) * (KNOB_FRAMES - 1) as f64) as usize).min(KNOB_FRAMES - 1);
    let (col, row) = (frame % KNOB_GRID_COLS, frame / KNOB_GRID_COLS);
    // grid coordinates are tiny integers, exact in f32
    #[allow(clippy::cast_precision_loss)]
    let (cols, rows, col, row) = (
        KNOB_GRID_COLS as f32,
        KNOB_GRID_ROWS as f32,
        col as f32,
        row as f32,
    );
    Rect::from_min_max(
        Pos2::new(col / cols, row / rows),
        Pos2::new((col + 1.0) / cols, (row + 1.0) / rows),
    )
}

/// `FSTP float`: round a wide value to float, like the original's stores.
#[allow(clippy::cast_possible_truncation)]
fn store(v: f64) -> f32 {
    v as f32
}

/// `(int)(v · range)` for a slider position: multiply in wide precision and truncate toward
/// zero, like the original's x87 code and `_ftol`.
fn scaled(v: f32, range: f32) -> f32 {
    store((f64::from(v) * f64::from(range)).trunc())
}

/// `CKnob::valueFromPoint` for a knob with rangeAngle < 0 (the VSTGUI 2.2 defaults). Mirror the
/// original's x87 code: wide arithmetic (f64 here) on float constants, a float store only for
/// `limit`, and a wide result that the caller compares before it stores it as float.
fn knob_value_from_point(p: Vec2, radius: f32) -> f64 {
    let (vmin, vmax) = (0.0f64, 1.0f64);
    let range_angle = f64::from(KNOB_RANGE_ANGLE);
    let half_angle = f64::from(KNOB_HALF_ANGLE);
    let a_coef = f64::from(KNOB_A_COEF);
    let two_pi = f64::from(TWO_PI);
    let radius = f64::from(radius);
    let mut alpha = (radius - f64::from(p.y)).atan2(f64::from(p.x) - radius);
    if alpha < 0.0 {
        alpha += two_pi;
    }
    let alpha3 = alpha - f64::from(KNOB_START_ANGLE) - range_angle;
    let alpha2 = if alpha3 < 0.0 {
        alpha3 + two_pi
    } else if alpha3 > TWO_PI_D {
        alpha3 - two_pi
    } else {
        alpha3
    };
    let limit = f64::from(store(half_angle - range_angle));
    if alpha2 > limit {
        return vmax;
    }
    if alpha2 > -range_angle {
        return vmin;
    }
    if alpha3 > limit {
        return (alpha3 - two_pi) * a_coef + vmax;
    }
    let alpha3 = if alpha3 < -half_angle {
        alpha3 + two_pi
    } else {
        alpha3
    };
    alpha3 * a_coef + vmax
}

impl DejaLamaGui {
    fn knob_param(&self, which: Knob) -> &FloatParam {
        match which {
            Knob::Glide => &self.params.port_time,
            Knob::Voice => &self.params.head_size,
        }
    }

    fn knob_rect(which: Knob) -> Rect {
        match which {
            Knob::Glide => KNOB_GLIDE,
            Knob::Voice => KNOB_VOICE,
        }
    }

    fn set(&self, param: &FloatParam, value: f32) {
        if let Some(ctx) = &self.gui_ctx {
            ctx.param_setter()
                .set_parameter(param, value.clamp(0.0, 1.0));
        }
    }

    fn begin(&self, param: &FloatParam) {
        if let Some(ctx) = &self.gui_ctx {
            ctx.param_setter().begin_set_parameter(param);
        }
    }

    fn end(&self, param: &FloatParam) {
        if let Some(ctx) = &self.gui_ctx {
            ctx.param_setter().end_set_parameter(param);
        }
    }

    /// One-shot set with begin/end (ctrl-click "default value", pad gate).
    fn set_once(&self, param: &FloatParam, value: f32) {
        self.begin(param);
        self.set(param, value);
        self.end(param);
    }

    fn set_gate(&self, down: bool) {
        if let Some(ctx) = &self.gui_ctx {
            let s = ctx.param_setter();
            s.begin_set_parameter(&self.params.pad_gate);
            s.set_parameter(&self.params.pad_gate, down);
            s.end_set_parameter(&self.params.pad_gate);
        }
    }

    /// The XY pad's mouse loop body: X sets the pitch (param 11), 1 - Y sets the vowel (param 10).
    fn pad_send(&self, p: Pos2) {
        let x = ((p.x - PAD.min.x) / PAD_X_RANGE).clamp(0.0, 1.0);
        let y = ((p.y - PAD.min.y) / PAD_Y_RANGE).clamp(0.0, 1.0);
        self.set(&self.params.pad_x, x);
        self.set(&self.params.pad_y, 1.0 - y);
    }

    /// Drive the controls like their VSTGUI mouse loops. Replay the frame's button events in
    /// order, so that a tap (press and release in one frame) ends its drag and a fast second
    /// click ends the old drag first; then poll the held button, because the pointer position
    /// and the modifiers count every frame, moved or not.
    fn handle_input(&mut self, ui: &egui::Ui, origin: Pos2) {
        let local = |p: Pos2| p - origin.to_vec2();
        let (buttons, down, pos, mods) = ui.input(|i| {
            let buttons: Vec<(bool, Pos2, egui::Modifiers)> = i
                .events
                .iter()
                .filter_map(|event| match *event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    } => Some((pressed, local(pos), modifiers)),
                    _ => None,
                })
                .collect();
            (
                buttons,
                i.pointer.primary_down(),
                i.pointer.latest_pos(),
                i.modifiers,
            )
        });
        let mut pressed = false;
        for (is_press, pos, modifiers) in buttons {
            if is_press {
                pressed = true;
                self.press(pos, modifiers);
            } else {
                self.release();
            }
        }
        // the press itself set the value for its frame
        if down
            && !pressed
            && let Some(latest) = pos
        {
            self.drag_to(local(latest), mods);
        }
    }

    /// The `mouse()` entry of the control under the pointer.
    fn press(&mut self, pos: Pos2, mods: egui::Modifiers) {
        // end a drag whose release never arrived (window closed, pointer gone) before a new one
        if !matches!(self.drag, Drag::None) {
            self.release();
        }
        // treat CSplashScreen as modal: the next click anywhere closes it
        if self.help_shown {
            self.help_shown = false;
            return;
        }
        if HELP_BUTTON.contains(pos) {
            self.help_shown = true;
            return;
        }
        if PAD.contains(pos) {
            self.set_gate(true); // the original's setParameter(9, 1)
            self.begin(&self.params.pad_x);
            self.begin(&self.params.pad_y);
            self.pad_send(pos);
            self.drag = Drag::Pad;
            return;
        }
        for which in [Knob::Glide, Knob::Voice] {
            let rect = Self::knob_rect(which);
            if rect.contains(pos) {
                self.press_knob(which, rect, pos, mods);
                return;
            }
        }
        if FADER.contains(pos) {
            if mods.ctrl {
                self.set_once(&self.params.delay, FADER_DEFAULT);
                return;
            }
            self.begin(&self.params.delay);
            let current = self.params.delay.value();
            let mut fader = FaderDrag::new(current, mods.shift);
            let v = fader.update(pos, current, mods.shift);
            self.drag = Drag::Fader(fader);
            self.set_fader(v);
        }
    }

    /// The `CKnob::mouse` entry: ctrl-click resets the knob; otherwise start a circular or a
    /// linear drag.
    fn press_knob(&mut self, which: Knob, rect: Rect, pos: Pos2, mods: egui::Modifiers) {
        let param = self.knob_param(which);
        if mods.ctrl {
            self.set_once(param, KNOB_DEFAULT);
            return;
        }
        self.begin(param);
        let linear = match self.knob_mode {
            KnobMode::Linear => !mods.alt,
            KnobMode::Circular => mods.alt,
        };
        if linear {
            self.drag = Drag::KnobLinear(LinearDrag::new(which, pos, param.value(), mods.shift));
        } else {
            let radius = rect.width() / 2.0;
            let old_value = store(knob_value_from_point(pos - rect.min, radius));
            self.drag = Drag::KnobCircular { which, old_value };
            // the first pass of the original's loop runs at the press point
            self.drag_to(pos, mods);
        }
    }

    /// The body of the control's mouse loop while the button stays down.
    fn drag_to(&mut self, pos: Pos2, mods: egui::Modifiers) {
        let mut drag = self.drag;
        match &mut drag {
            Drag::None => {}
            Drag::Pad => self.pad_send(pos),
            Drag::KnobCircular { which, old_value } => {
                let rect = Self::knob_rect(*which);
                let mut v = knob_value_from_point(pos - rect.min, rect.width() / 2.0);
                let middle = 0.5;
                // don't jump across the gap at the bottom of the knob
                if f64::from(*old_value) - v > middle {
                    v = 1.0;
                } else if v - f64::from(*old_value) > middle {
                    v = 0.0;
                } else {
                    *old_value = store(v);
                }
                self.set_knob(*which, store(v));
            }
            Drag::KnobLinear(linear) => {
                let v = linear.update(pos, mods.shift);
                self.set_knob(linear.which, v);
            }
            Drag::Fader(fader) => {
                let v = fader.update(pos, self.params.delay.value(), mods.shift);
                self.set_fader(v);
            }
        }
        self.drag = drag;
    }

    // exact comparisons on purpose: send a parameter only when the pointer changed its value
    #[allow(clippy::float_cmp)]
    fn set_knob(&self, which: Knob, v: f32) {
        let param = self.knob_param(which);
        if v != param.value() {
            self.set(param, v);
        }
    }

    #[allow(clippy::float_cmp)]
    fn set_fader(&self, v: f32) {
        if v != self.params.delay.value() {
            self.set(&self.params.delay, v);
        }
    }

    /// The end of the mouse loop: close the host's automation gesture and release the pad.
    fn release(&mut self) {
        match self.drag {
            Drag::None => {}
            Drag::Pad => {
                self.end(&self.params.pad_x);
                self.end(&self.params.pad_y);
                self.set_gate(false); // the original's setParameter(9, 0)
            }
            Drag::KnobCircular { which, .. } | Drag::KnobLinear(LinearDrag { which, .. }) => {
                self.end(self.knob_param(which));
            }
            Drag::Fader(_) => self.end(&self.params.delay),
        }
        self.drag = Drag::None;
    }

    fn draw(&self, ui: &egui::Ui, origin: Pos2) {
        let Some(t) = self.textures() else {
            return;
        };
        let painter = ui.painter();
        let at = |x: f32, y: f32, w: f32, h: f32| {
            Rect::from_min_size(origin + Vec2::new(x, y), Vec2::new(w, h))
        };
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

        painter.image(
            t.background.id(),
            at(0.0, 0.0, f32::from(WIDTH), f32::from(HEIGHT)),
            full,
            Color32::WHITE,
        );

        // monk: frame = ftol(29·mouth + 0.5) (column-major in the original 5×6 atlas)
        let mouth = self.shared.mouth.load(Ordering::Relaxed).clamp(0.0, 1.0);
        // truncate like the DLL's ftol; the clamp keeps the value in 0..=29.5
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frame = ((29.0 * f64::from(mouth) + 0.5) as usize).min(MONK_FRAMES - 1);
        painter.image(
            t.monk[frame].id(),
            Rect::from_min_size(origin + MONK_POS, FRAME_SIZE),
            full,
            Color32::WHITE,
        );

        // vowel indicator (CVerticalSlider, kBottom): top = 358 + (int)((1 - v)·79)
        let vowel = self.shared.vowel.load(Ordering::Relaxed).clamp(0.0, 1.0);
        let ty = TRI_VOWEL_MIN_Y + scaled(1.0 - vowel, TRI_VOWEL_RANGE);
        painter.image(
            t.tri_vowel.id(),
            at(TRI_VOWEL_X, ty, 10.0, 10.0),
            full,
            Color32::WHITE,
        );

        // pitch indicator (CHorizontalSlider, kLeft): left = 93 + (int)(v·161)
        let expr = self
            .shared
            .expression
            .load(Ordering::Relaxed)
            .clamp(0.0, 1.0);
        let tx = TRI_PITCH_MIN_X + scaled(expr, TRI_PITCH_RANGE);
        painter.image(
            t.tri_pitch.id(),
            at(tx, TRI_PITCH_Y, 10.0, 10.0),
            full,
            Color32::WHITE,
        );

        // delay fader handle: left = 104 + (int)(v·131), top = 479
        let delay = self.params.delay.value().clamp(0.0, 1.0);
        let fx = FADER_MIN_X + scaled(delay, FADER_MAX_X - FADER_MIN_X);
        painter.image(
            t.fader_handle.id(),
            at(fx, FADER.min.y, FADER_HANDLE_W, 17.0),
            full,
            Color32::WHITE,
        );

        // knobs: CAnimKnob frame = (int)(v·59); look it up in the repacked 6×10 grid
        for (which, tex) in [(Knob::Glide, &t.knob_glide), (Knob::Voice, &t.knob_voice)] {
            let v = self.knob_param(which).value().clamp(0.0, 1.0);
            let uv = knob_frame_uv(v);
            let rect = Self::knob_rect(which);
            painter.image(
                tex.id(),
                rect.translate(origin.to_vec2()),
                uv,
                Color32::WHITE,
            );
        }

        if self.help_shown {
            painter.image(
                t.help.id(),
                at(HELP_POS.x, HELP_POS.y, 253.0, 275.0),
                full,
                Color32::WHITE,
            );
        }
    }
}

/// Wait for a running loader: hosts unload the module after the last instance, and a thread
/// still inside it would crash on return.
impl Drop for DejaLamaGui {
    fn drop(&mut self) {
        if let Some(Assets::Loading { thread, .. }) = &mut self.assets
            && let Some(thread) = thread.take()
        {
            let _ = thread.join();
        }
    }
}

impl NiceEguiApp for DejaLamaGui {
    fn build(
        &mut self,
        egui_ctx: Context,
        nice_gui_ctx: GuiContext,
        _frame: &mut nice_plug_egui::Frame,
    ) -> Result<(), baseview::HandlerError> {
        self.release(); // a drag the previous window took with it
        self.gui_ctx = Some(nice_gui_ctx);
        #[cfg(feature = "embed-assets")]
        {
            let images = decode_bitmaps(embedded_bitmaps()).expect("build.rs checked the bitmaps");
            self.assets = Some(Assets::Ready(Textures::new(&egui_ctx, images)));
        }
        #[cfg(not(feature = "embed-assets"))]
        self.load_or_start(&egui_ctx);
        self.drag = Drag::None;
        self.help_shown = false;
        Ok(())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut nice_plug_egui::Frame) {
        let origin = ui.max_rect().min;
        self.poll_loader(ui.ctx());
        if self.textures().is_none() {
            self.draw_status(ui, origin);
            ui.ctx().request_repaint_after(Duration::from_millis(100));
            return;
        }
        // reserve the whole window so that egui adds no layout of its own
        ui.allocate_rect(
            Rect::from_min_size(origin, Vec2::new(f32::from(WIDTH), f32::from(HEIGHT))),
            egui::Sense::hover(),
        );
        self.handle_input(ui, origin);
        self.draw(ui, origin);
        // keep the mouth moving while idle: the original animates from its idle timer
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }

    fn editor_closed(&mut self) {
        self.release();
        self.assets = None;
        self.gui_ctx = None;
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact values on purpose
mod tests {
    use super::*;

    /// The knob's value at the points the pixel comparison against the original used: the
    /// bottom-right corner reads exactly 1 once stored as float (the original shows frame 59
    /// there; f32 arithmetic lands one ulp short, frame 58), the top centre 0.5, the
    /// bottom-left 0.
    #[test]
    fn knob_value_from_point_matches_vstgui() {
        let radius = KNOB_GLIDE.width() / 2.0;
        let at = |x: f32, y: f32| store(knob_value_from_point(Vec2::new(x, y), radius));
        assert_eq!(at(45.0, 45.0), 1.0);
        assert_eq!(at(44.0, 44.0), 1.0);
        assert!((at(25.0, 2.0) - 0.5).abs() < 1e-6);
        assert!(at(5.0, 45.0).abs() < 1e-6);
        // the dead zone below the knob: the nearer end wins on either side of straight down
        assert_eq!(at(22.0, 48.0), 0.0);
        assert_eq!(at(28.0, 48.0), 1.0);
        // the derived constants come out as the original's float stores
        assert_eq!(KNOB_A_COEF.to_bits(), 0xbe59_4caf);
        assert_eq!(KNOB_HALF_ANGLE.to_bits(), 0x3f49_0fdc);
    }

    /// The linear knob keeps its value when shift toggles mid-drag and zooms from there.
    #[test]
    fn linear_drag_re_anchors_when_shift_toggles() {
        let mut drag = LinearDrag::new(Knob::Glide, Pos2::new(100.0, 100.0), 0.5, false);
        assert_eq!(drag.update(Pos2::new(140.0, 100.0), false), 0.7); // 40 px / 200
        assert_eq!(drag.update(Pos2::new(140.0, 100.0), true), 0.7); // shift: no jump
        assert_eq!(drag.update(Pos2::new(170.0, 100.0), true), 0.8); // 30 px / 300
        assert_eq!(drag.update(Pos2::new(170.0, 100.0), false), 0.8); // release shift: no jump
        assert_eq!(drag.update(Pos2::new(170.0, 140.0), false), 0.6); // up is down: -40 px / 200
        assert_eq!(drag.update(Pos2::new(400.0, 100.0), false), 1.0); // clamped
    }

    /// The fader tracks the pointer freely, then fine-adjusts from the shift-down anchor.
    #[test]
    fn fader_drag_fine_adjusts_with_shift() {
        let mut drag = FaderDrag::new(0.8, false);
        let at = |x: f32| Pos2::new(x, 490.0);
        let v = drag.update(at(178.0), 0.8, false);
        assert!((v - 65.0 / 131.0).abs() < 1e-6, "{v}"); // (178 - 113) / 131
        // shift down: the anchor is the current value, and a full-range move counts a tenth
        let fine = drag.update(at(178.0 + 131.0), v, true);
        assert!((fine - (v + 0.1)).abs() < 1e-6, "{fine}");
        // shift up: the pointer position counts again
        assert_eq!(drag.update(at(113.0), fine, false), 0.0);
        assert_eq!(drag.update(at(300.0), 0.0, false), 1.0);
    }

    #[test]
    fn knob_frames_sit_in_the_repacked_grid() {
        let first = knob_frame_uv(0.0);
        assert_eq!(first.min, Pos2::ZERO);
        assert_eq!(first.max, Pos2::new(1.0 / 6.0, 0.1));
        let last = knob_frame_uv(1.0);
        assert_eq!(last.min, Pos2::new(5.0 / 6.0, 0.9));
        assert_eq!(knob_frame_uv(0.5).min, Pos2::new(5.0 / 6.0, 0.4)); // frame 29
    }

    #[cfg(feature = "embed-assets")]
    #[test]
    fn embedded_bitmaps_decode_at_the_measured_sizes() {
        decode_bitmaps(embedded_bitmaps()).unwrap();
    }

    /// A truncated or foreign file is an error, not a panic.
    #[test]
    fn decode_rejects_bad_files() {
        assert!(decode_png(b"not a png").is_err());
        let tiny = {
            let mut out = Vec::new();
            let mut encoder = png::Encoder::new(&mut out, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 0, 0, 255]).unwrap();
            writer.finish().unwrap();
            out
        };
        let err = decode_bitmaps([tiny.as_slice(); 8]).unwrap_err();
        assert!(
            err.starts_with("background.png: 1x1 instead of 360x510"),
            "{err}"
        );
    }

    #[test]
    fn the_atlas_holds_the_monk_frames() {
        assert_eq!(ATLAS_W, 5 * FRAME_W);
        assert_eq!(ATLAS_H, 6 * FRAME_H);
    }
}
