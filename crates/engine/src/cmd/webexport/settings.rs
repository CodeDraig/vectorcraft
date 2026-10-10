//! Save for Web options, presets and serialized defaults.

use super::super::*;
use super::{AntiAlias, ColorSort, Deserialize, Deserializer, Dither, Reduction, Serialize, Serializer, Value, json};
use serde::de::Error as _;

// ---------- settings ----------

/// Enums of the render crate written in settings by id.
trait Keyed: Sized + Copy {
    fn key(self) -> &'static str;
    fn parse(s: &str) -> Option<Self>;
    const KEYS: &'static str;
}

impl Keyed for Reduction {
    fn key(self) -> &'static str {
        self.id()
    }
    fn parse(s: &str) -> Option<Self> {
        Self::from_id(s)
    }
    const KEYS: &'static str = "perceptual, selective, adaptive, web, blackWhite or gray";
}

impl Keyed for Dither {
    fn key(self) -> &'static str {
        self.id()
    }
    fn parse(s: &str) -> Option<Self> {
        Self::from_id(s)
    }
    const KEYS: &'static str = "none, diffusion, pattern or noise";
}

impl Keyed for AntiAlias {
    fn key(self) -> &'static str {
        self.id()
    }
    fn parse(s: &str) -> Option<Self> {
        Self::from_id(s)
    }
    const KEYS: &'static str = "none, art or type";
}

impl Keyed for ColorSort {
    fn key(self) -> &'static str {
        self.id()
    }
    fn parse(s: &str) -> Option<Self> {
        Self::from_id(s)
    }
    const KEYS: &'static str = "none, hue, luminance or popularity";
}

fn ser_key<T: Keyed, S: Serializer>(v: &T, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(v.key())
}

fn de_key<'de, T: Keyed, D: Deserializer<'de>>(d: D) -> std::result::Result<T, D::Error> {
    let s = String::deserialize(d)?;
    T::parse(&s).ok_or_else(|| D::Error::custom(format!("`{s}`: {}", T::KEYS)))
}

/// The file format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebFormat {
    #[default]
    Gif,
    #[serde(alias = "jpeg")]
    Jpg,
    Png8,
    Png24,
}

impl WebFormat {
    pub const ALL: [WebFormat; 4] = [WebFormat::Gif, WebFormat::Jpg, WebFormat::Png8, WebFormat::Png24];

    pub fn id(self) -> &'static str {
        match self {
            WebFormat::Gif => "gif",
            WebFormat::Jpg => "jpg",
            WebFormat::Png8 => "png8",
            WebFormat::Png24 => "png24",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WebFormat::Gif => "GIF",
            WebFormat::Jpg => "JPEG",
            WebFormat::Png8 => "PNG-8",
            WebFormat::Png24 => "PNG-24",
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            WebFormat::Gif => "gif",
            WebFormat::Jpg => "jpg",
            WebFormat::Png8 | WebFormat::Png24 => "png",
        }
    }

    /// A palette format (colour reduction, colour table).
    pub fn palette(self) -> bool {
        matches!(self, WebFormat::Gif | WebFormat::Png8)
    }
}

/// Which File Info goes into the files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebMetadata {
    None,
    #[default]
    Copyright,
    /// The copyright and the author.
    Contact,
    All,
}

impl WebMetadata {
    pub const ALL: [WebMetadata; 4] = [WebMetadata::None, WebMetadata::Copyright, WebMetadata::Contact, WebMetadata::All];

    pub fn key(self) -> &'static str {
        match self {
            WebMetadata::None => "none",
            WebMetadata::Copyright => "copyright",
            WebMetadata::Contact => "contact",
            WebMetadata::All => "all",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WebMetadata::None => "None",
            WebMetadata::Copyright => "Copyright",
            WebMetadata::Contact => "Copyright and Contact Info",
            WebMetadata::All => "All",
        }
    }

    /// Whether File Info entry `keyword` (as [`vectorcraft_doc::Metadata::png_text`] names it) goes in.
    pub(super) fn keeps(self, keyword: &str) -> bool {
        match self {
            WebMetadata::None => false,
            WebMetadata::Copyright => keyword.starts_with("Copyright"),
            WebMetadata::Contact => keyword.starts_with("Copyright") || keyword == "Author",
            WebMetadata::All => true,
        }
    }
}

/// Which slices get images.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SliceScope {
    #[default]
    All,
    Selected,
    /// One image, slices or not.
    None,
}

impl SliceScope {
    pub const ALL: [SliceScope; 3] = [SliceScope::All, SliceScope::Selected, SliceScope::None];

    pub fn key(self) -> &'static str {
        match self {
            SliceScope::All => "all",
            SliceScope::Selected => "selected",
            SliceScope::None => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SliceScope::All => "All Slices",
            SliceScope::Selected => "Selected Slices",
            SliceScope::None => "No Slices",
        }
    }
}

/// What Save writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebOutput {
    #[default]
    Images,
    /// An HTML page placing the images too.
    Html,
}

impl WebOutput {
    pub const ALL: [WebOutput; 2] = [WebOutput::Images, WebOutput::Html];

    pub fn key(self) -> &'static str {
        match self {
            WebOutput::Images => "images",
            WebOutput::Html => "html",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            WebOutput::Images => "Images Only",
            WebOutput::Html => "HTML and Images",
        }
    }
}

/// The colour table's edits of a palette format, by the colours the reduction makes (the
/// preview's `colors[].source`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColorTable {
    /// Kept when the colours are reduced further.
    pub locked: Vec<String>,
    /// Become transparent.
    pub transparent: Vec<String>,
    /// Become their nearest web-safe colour.
    pub web_shift: Vec<String>,
    #[serde(serialize_with = "ser_key", deserialize_with = "de_key")]
    pub sort: ColorSort,
}

/// The most colours a colour table list holds.
const MAX_TABLE: usize = 256;

impl ColorTable {
    pub(super) fn colors(list: &[String], what: &str) -> std::result::Result<Vec<[u8; 3]>, String> {
        if list.len() > MAX_TABLE {
            return Err(format!("colorTable.{what}: at most {MAX_TABLE} colours"));
        }
        list.iter()
            .map(|h| {
                vectorcraft_color::Color::from_hex(h)
                    .map(|c| {
                        let [r, g, b, _] = c.to_rgba8(1.0);
                        [r, g, b]
                    })
                    .ok_or_else(|| format!("colorTable.{what}: `{h}` is not a colour such as \"#ff8800\""))
            })
            .collect()
    }
}

/// Save for Web's settings: the params of `document.exportForWeb`, a preset's content and what the
/// dialog opens on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebSettings {
    pub format: WebFormat,
    #[serde(serialize_with = "ser_key", deserialize_with = "de_key")]
    pub reduction: Reduction,
    /// Most palette entries, 2–256.
    pub colors: u16,
    #[serde(serialize_with = "ser_key", deserialize_with = "de_key")]
    pub dither: Dither,
    /// 0–100.
    pub dither_amount: u8,
    /// Keep transparent pixels transparent (GIF, PNG-8, PNG-24).
    pub transparency: bool,
    /// What partly transparent edges blend into (JPEG and opaque PNG-24: the background): `white`,
    /// `black`, `none` or `#rrggbb`.
    pub matte: String,
    pub interlaced: bool,
    /// 0–100 %: palette colours that close to a web-safe colour become it.
    pub web_snap: u8,
    /// 0–100: lossy GIF compression.
    pub lossy: u8,
    pub color_table: ColorTable,
    /// JPEG quality 0–100.
    pub quality: u8,
    pub progressive: bool,
    /// Optimised Huffman tables (smaller JPEG files).
    pub optimized: bool,
    /// JPEG: embed the sRGB profile.
    pub embed_profile: bool,
    /// The image width in pixels (the height follows).
    pub width: Option<u32>,
    /// The image height in pixels (the width follows).
    pub height: Option<u32>,
    /// The image size in percent of the art's size in points.
    pub percent: Option<f64>,
    #[serde(serialize_with = "ser_key", deserialize_with = "de_key")]
    pub anti_alias: AntiAlias,
    /// The artboard (else the visible art and the slices).
    pub clip_to_artboard: bool,
    /// 0-based (default the first).
    pub artboard: Option<usize>,
    /// PNG files say their colours are sRGB (what the renderer draws).
    pub convert_to_srgb: bool,
    pub metadata: WebMetadata,
    pub slices: SliceScope,
    pub output: WebOutput,
}

impl Default for WebSettings {
    fn default() -> Self {
        Self {
            format: WebFormat::Gif,
            reduction: Reduction::Selective,
            colors: 128,
            dither: Dither::Diffusion,
            dither_amount: 100,
            transparency: true,
            matte: "white".into(),
            interlaced: false,
            web_snap: 0,
            lossy: 0,
            color_table: ColorTable::default(),
            quality: 60,
            progressive: false,
            optimized: true,
            embed_profile: false,
            width: None,
            height: None,
            percent: None,
            anti_alias: AntiAlias::Art,
            clip_to_artboard: true,
            artboard: None,
            convert_to_srgb: true,
            metadata: WebMetadata::Copyright,
            slices: SliceScope::All,
            output: WebOutput::Images,
        }
    }
}

impl WebSettings {
    /// `self` with the settings keys of `p` (other keys are ignored: params carry `path`, `kbps`…).
    pub fn merged(&self, p: &Value) -> std::result::Result<Self, String> {
        let mut v = serde_json::to_value(self).map_err(|e| e.to_string())?;
        if let (Some(obj), Some(over)) = (v.as_object_mut(), p.as_object()) {
            for (k, val) in over {
                if let Some(slot) = obj.get_mut(k) {
                    *slot = val.clone();
                }
            }
        }
        let s: WebSettings = serde_json::from_value(v).map_err(|e| format!("Save for Web settings: {e}"))?;
        s.checked()
    }

    /// Reject values no image can have.
    fn checked(mut self) -> std::result::Result<Self, String> {
        self.colors = self.colors.clamp(2, 256);
        self.dither_amount = self.dither_amount.min(100);
        self.web_snap = self.web_snap.min(100);
        self.lossy = self.lossy.min(100);
        self.quality = self.quality.min(100);
        if let Some(p) = self.percent.filter(|p| !(p.is_finite() && *p > 0.0 && *p <= 6400.0)) {
            return Err(format!("percent must be above 0 and at most 6400, not {p}"));
        }
        if self.width == Some(0) || self.height == Some(0) {
            return Err("width and height are at least 1 pixel".into());
        }
        self.matte_rgb()?;
        for (list, what) in
            [(&self.color_table.locked, "locked"), (&self.color_table.transparent, "transparent"), (&self.color_table.web_shift, "webShift")]
        {
            ColorTable::colors(list, what)?;
        }
        Ok(self)
    }

    /// The matte as RGB (`None`: none).
    pub fn matte_rgb(&self) -> std::result::Result<Option<[u8; 3]>, String> {
        super::super::fileio::background(&json!(self.matte)).map_err(|e| format!("matte: {e}"))
    }

    /// The JSON the commands return (and presets store).
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// A named set of settings: built in, or saved in the preferences ([`crate::Prefs::web_export_presets`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebPreset {
    pub name: String,
    #[serde(default)]
    pub settings: WebSettings,
}

/// The built-in presets (made here, named by what they do).
pub fn builtin_presets() -> Vec<WebPreset> {
    let d = WebSettings::default;
    let preset = |name: &str, settings: WebSettings| WebPreset { name: name.into(), settings };
    vec![
        preset("GIF, 128 colours", d()),
        preset("GIF, 32 colours, no dither", WebSettings { colors: 32, dither: Dither::None, ..d() }),
        preset("GIF, web-safe colours", WebSettings { reduction: Reduction::Web, colors: 256, dither: Dither::None, ..d() }),
        preset("JPEG, quality 80", WebSettings { format: WebFormat::Jpg, quality: 80, ..d() }),
        preset("JPEG, quality 50", WebSettings { format: WebFormat::Jpg, quality: 50, ..d() }),
        preset("JPEG, quality 25", WebSettings { format: WebFormat::Jpg, quality: 25, ..d() }),
        preset("PNG-8, 64 colours", WebSettings { format: WebFormat::Png8, colors: 64, ..d() }),
        preset("PNG-24", WebSettings { format: WebFormat::Png24, ..d() }),
    ]
}

pub(super) fn saved_index(saved: &[WebPreset], name: &str) -> Option<usize> {
    saved.iter().position(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

pub(super) fn is_builtin(name: &str) -> bool {
    builtin_presets().iter().any(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

impl Session {
    /// Every Save for Web preset: the built-in ones, then the saved ones.
    pub fn web_presets(&self) -> Vec<WebPreset> {
        let mut v = builtin_presets();
        v.extend(self.prefs.web_export_presets.iter().cloned());
        v
    }

    /// The settings `p` asks for: its `preset` (else `base`) with its settings keys.
    pub fn web_settings_over(&self, base: &WebSettings, p: &Value) -> std::result::Result<WebSettings, String> {
        let start = match p.get("preset").and_then(Value::as_str) {
            Some(name) => {
                let key = name.trim();
                self.web_presets()
                    .into_iter()
                    .find(|q| q.name.eq_ignore_ascii_case(key))
                    .ok_or_else(|| format!("unknown preset `{name}` (see webExport.presets.list)"))?
                    .settings
            }
            None => base.clone(),
        };
        start.merged(p)
    }

    /// The settings of a `document.exportForWeb` call: the defaults (or its preset) with its keys.
    pub fn web_settings(&self, p: &Value) -> std::result::Result<WebSettings, String> {
        self.web_settings_over(&WebSettings::default(), p)
    }

    /// What the Save for Web dialog opens on: the remembered settings, else the defaults.
    pub fn last_web_settings(&self) -> WebSettings {
        self.prefs.web_export_settings.clone().unwrap_or_default()
    }
}
