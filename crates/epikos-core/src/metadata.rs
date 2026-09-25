use serde::{Deserialize, Serialize};

/// EXIF RATIONAL (numerator, denominator). Kept exact so exports copy values verbatim.
pub type Ratio = (u32, u32);
/// EXIF SRATIONAL.
pub type SRatio = (i32, i32);

/// Capture metadata read from the RAW, carried through to exports.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CaptureMetadata {
    /// EXIF `Make` / `Model` exactly as the camera wrote them.
    pub make: String,
    pub model: String,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    /// `YYYY:MM:DD HH:MM:SS`, camera local time.
    pub date_time_original: Option<String>,
    pub date_time_digitized: Option<String>,
    pub offset_time_original: Option<String>,
    pub sub_sec_time_original: Option<String>,
    pub exposure_time: Option<Ratio>,
    pub f_number: Option<Ratio>,
    pub focal_length: Option<Ratio>,
    pub max_aperture: Option<Ratio>,
    pub exposure_bias: Option<SRatio>,
    pub iso: Option<u32>,
    pub exposure_program: Option<u16>,
    pub metering_mode: Option<u16>,
    pub flash: Option<u16>,
    pub exposure_mode: Option<u16>,
    pub white_balance: Option<u16>,
    pub serial_number: Option<String>,
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub lens_serial_number: Option<String>,
    /// Min/max focal length and min/max f-number.
    pub lens_spec: Option<[Ratio; 4]>,
    pub gps: Option<GpsInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GpsInfo {
    pub version: Option<[u8; 4]>,
    pub latitude_ref: Option<String>,
    /// Degrees, minutes, seconds.
    pub latitude: Option<[Ratio; 3]>,
    pub longitude_ref: Option<String>,
    pub longitude: Option<[Ratio; 3]>,
    /// 0 = above sea level, 1 = below.
    pub altitude_ref: Option<u8>,
    pub altitude: Option<Ratio>,
    /// UTC hours, minutes, seconds.
    pub time_stamp: Option<[Ratio; 3]>,
    /// `YYYY:MM:DD`, UTC.
    pub date_stamp: Option<String>,
    pub img_direction_ref: Option<String>,
    pub img_direction: Option<Ratio>,
    pub satellites: Option<String>,
    /// `A` = measurement in progress, `V` = interrupted.
    pub status: Option<String>,
    /// `2` = 2D, `3` = 3D fix.
    pub measure_mode: Option<String>,
    pub dop: Option<Ratio>,
    /// Geodetic datum, e.g. `WGS-84`.
    pub map_datum: Option<String>,
    /// Horizontal positioning error in metres.
    pub h_positioning_error: Option<Ratio>,
}

impl GpsInfo {
    pub fn has_position(&self) -> bool {
        self.latitude.is_some() && self.longitude.is_some()
    }
}
