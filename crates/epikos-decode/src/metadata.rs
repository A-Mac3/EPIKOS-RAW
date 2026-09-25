use epikos_core::{CaptureMetadata, GpsInfo, Orientation, Ratio, SRatio};
use rawler::decoders::{RawDecodeParams, RawMetadata};
use rawler::formats::tiff::{Rational, SRational};
use rawler::rawsource::RawSource;

/// Orientation plus capture metadata. Missing or unreadable metadata is not an error:
/// the image still develops, it just exports without EXIF.
pub(crate) fn read(
    src: &RawSource,
    params: &RawDecodeParams,
    make: &str,
    model: &str,
) -> (Orientation, CaptureMetadata) {
    let Some(meta) = rawler::get_decoder(src)
        .and_then(|d| d.raw_metadata(src, params))
        .ok()
    else {
        return (
            Orientation::Normal,
            CaptureMetadata {
                make: make.into(),
                model: model.into(),
                ..Default::default()
            },
        );
    };
    let orientation = meta
        .exif
        .orientation
        .map(Orientation::from_exif)
        .unwrap_or_default();
    (orientation, convert(meta, make, model))
}

pub(crate) fn convert(meta: RawMetadata, make: &str, model: &str) -> CaptureMetadata {
    let e = meta.exif;
    let lens = meta.lens;
    let text = |s: Option<String>| {
        s.map(|s| s.trim().trim_end_matches('\0').to_string())
            .filter(|s| !s.is_empty())
    };
    CaptureMetadata {
        make: make.to_string(),
        model: model.to_string(),
        artist: text(e.artist),
        copyright: text(e.copyright),
        date_time_original: text(e.date_time_original),
        date_time_digitized: text(e.create_date),
        offset_time_original: text(e.offset_time_original),
        sub_sec_time_original: text(e.sub_sec_time_original),
        exposure_time: e.exposure_time.map(r),
        f_number: e.fnumber.map(r),
        focal_length: e.focal_length.map(r),
        max_aperture: e.max_aperture_value.map(r),
        exposure_bias: e.exposure_bias.map(sr),
        iso: e
            .iso_speed_ratings
            .map(u32::from)
            .or(e.iso_speed)
            .or(e.recommended_exposure_index)
            .filter(|v| *v > 0),
        exposure_program: e.exposure_program,
        metering_mode: e.metering_mode,
        flash: e.flash,
        exposure_mode: e.exposure_mode,
        white_balance: e.white_balance,
        serial_number: text(e.serial_number),
        lens_make: text(e.lens_make).or_else(|| {
            lens.as_ref()
                .map(|l| l.lens_make.clone())
                .filter(|s| !s.is_empty())
        }),
        lens_model: text(e.lens_model).or_else(|| {
            lens.as_ref()
                .map(|l| l.lens_model.clone())
                .filter(|s| !s.is_empty())
        }),
        lens_serial_number: text(e.lens_serial_number),
        lens_spec: e.lens_spec.map(|s| s.map(r)),
        gps: e.gps.map(|g| GpsInfo {
            version: g.gps_version_id,
            latitude_ref: text(g.gps_latitude_ref),
            latitude: g.gps_latitude.map(|v| v.map(r)),
            longitude_ref: text(g.gps_longitude_ref),
            longitude: g.gps_longitude.map(|v| v.map(r)),
            altitude_ref: g.gps_altitude_ref,
            altitude: g.gps_altitude.map(r),
            time_stamp: g.gps_timestamp.map(|v| v.map(r)),
            date_stamp: text(g.gps_date_stamp),
            img_direction_ref: text(g.gps_img_direction_ref),
            img_direction: g.gps_img_direction.map(r),
            satellites: text(g.gps_satellites),
            status: text(g.gps_status),
            measure_mode: text(g.gps_measure_mode),
            dop: g.gps_dop.map(r),
            map_datum: text(g.gps_map_datum),
            h_positioning_error: g.gps_h_positioning_error.map(r),
        }),
    }
}

fn r(v: Rational) -> Ratio {
    (v.n, v.d)
}

fn sr(v: SRational) -> SRatio {
    (v.n, v.d)
}
