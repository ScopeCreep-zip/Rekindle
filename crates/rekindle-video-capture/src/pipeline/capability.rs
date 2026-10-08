//! Choosing the camera mode to capture in.
//!
//! The source caps used to pin nothing, leaving each platform's source
//! element to fixate a mode on its own. `avfvideosrc` truncates to the first
//! structure of its device caps, which it lists from the largest format down
//! (`gst_av_capture_device_get_caps` walks `device.formats` in reverse;
//! `fixate` in `avfvideosrc.m`), so a 4K webcam captured 4K frames that the
//! pipeline converted in software and shrank to 854×480 and 320×180.
//! `v4l2src` picks differently, so the two platforms diverged.
//!
//! The mode is now chosen from the device's own mode list the way
//! production capturers choose it, then pinned in the source caps (so
//! `avfvideosrc` sets it as the device's `activeFormat`, as Chromium and
//! Firefox set `activeFormat` before starting):
//! - the request's aspect ratio first, Chromium's fitness-distance term
//!   (`media_stream_constraints_util_video_device.cc`);
//! - then libwebrtc's rule (`modules/video_capture/device_info_impl.cc`
//!   `DeviceInfoImpl::GetBestMatchedCapability`): the smallest height at
//!   or above the requested one (the largest when none reaches it), then
//!   the same for width, then for frame rate, then a raw format over a
//!   compressed one.

use gstreamer as gst;

/// One camera mode from the device caps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CameraMode {
    /// `video/x-raw` or `image/jpeg`.
    media: String,
    /// The raw pixel format, for `video/x-raw`.
    format: Option<String>,
    width: i32,
    height: i32,
    /// Highest frame rate the mode offers, and the lowest.
    max_fps: gst::Fraction,
    min_fps: gst::Fraction,
}

impl CameraMode {
    /// The frame rate to ask for: `fps` when the mode offers it, else its
    /// closest bound.
    fn rate_for(&self, fps: i32) -> gst::Fraction {
        let want = gst::Fraction::new(fps, 1);
        if want > self.max_fps {
            self.max_fps
        } else if want < self.min_fps {
            self.min_fps
        } else {
            want
        }
    }

    /// Caps pinning exactly this mode (system memory: no caps features).
    pub(crate) fn caps(&self, fps: i32) -> gst::Caps {
        let mut builder = gst::Structure::builder(self.media.as_str())
            .field("width", self.width)
            .field("height", self.height)
            .field("framerate", self.rate_for(fps));
        if let Some(format) = &self.format {
            builder = builder.field("format", format.as_str());
        }
        gst::Caps::builder_full().structure(builder.build()).build()
    }

    pub(crate) fn describe(&self, fps: i32) -> String {
        let rate = self.rate_for(fps);
        format!(
            "{} {} {}x{} @ {}/{}",
            self.media,
            self.format.as_deref().unwrap_or("-"),
            self.width,
            self.height,
            rate.numer(),
            rate.denom()
        )
    }
}

/// The frame-rate bounds of a caps field: a fixed rate, a list, or a range.
fn fps_bounds(s: &gst::StructureRef) -> Option<(gst::Fraction, gst::Fraction)> {
    if let Ok(f) = s.get::<gst::Fraction>("framerate") {
        return Some((f, f));
    }
    if let Ok(r) = s.get::<gst::FractionRange>("framerate") {
        return Some((r.min(), r.max()));
    }
    if let Ok(list) = s.get::<gst::List>("framerate") {
        let rates: Vec<gst::Fraction> = list
            .iter()
            .filter_map(|v| v.get::<gst::Fraction>().ok())
            .collect();
        let min = rates.iter().min().copied()?;
        let max = rates.iter().max().copied()?;
        return Some((min, max));
    }
    None
}

/// Every fixed-size mode in `caps` the pipeline can decode (raw or MJPEG).
pub(crate) fn modes(caps: &gst::Caps) -> Vec<CameraMode> {
    caps.iter()
        .filter_map(|s| {
            let media = s.name().to_string();
            if media != "video/x-raw" && media != "image/jpeg" {
                return None;
            }
            let width = s.get::<i32>("width").ok()?;
            let height = s.get::<i32>("height").ok()?;
            let (min_fps, max_fps) = fps_bounds(s)?;
            let format = (media == "video/x-raw")
                .then(|| s.get::<String>("format").ok())
                .flatten();
            if media == "video/x-raw" && format.is_none() {
                return None;
            }
            Some(CameraMode {
                media,
                format,
                width,
                height,
                max_fps,
                min_fps,
            })
        })
        .collect()
}

/// libwebrtc's ordering for one dimension: a value at or above the target
/// beats one below it; among values at or above, the closest wins; among
/// values below, the largest wins.
fn closer(candidate: i64, best: i64, target: i64) -> std::cmp::Ordering {
    use std::cmp::Ordering::{Equal, Greater, Less};
    let (c_ok, b_ok) = (candidate >= target, best >= target);
    match (c_ok, b_ok) {
        (true, false) => Less,
        (false, true) => Greater,
        (true, true) => (candidate - target).cmp(&(best - target)),
        (false, false) => best.cmp(&candidate),
    }
    .then(Equal)
}

/// A raw format libwebrtc prefers (`I420`, `YUY2`, `YV12`, `NV12`) ranks
/// before other raw formats, which rank before MJPEG.
fn format_rank(mode: &CameraMode) -> u8 {
    match mode.format.as_deref() {
        Some("I420" | "YUY2" | "YV12" | "NV12") => 0,
        Some(_) => 1,
        None => 2,
    }
}

/// Frame rate in millihertz, for comparing modes.
fn mhz(f: gst::Fraction) -> i64 {
    i64::from(f.numer()) * 1000 / i64::from(f.denom().max(1))
}

/// Whether `mode` has the request's aspect ratio (within 1 %).
fn same_aspect(mode: &CameraMode, width: i32, height: i32) -> bool {
    let want = i64::from(width) * i64::from(mode.height);
    let have = i64::from(mode.width) * i64::from(height);
    (want - have).abs() * 100 <= want
}

/// The mode to capture in for `width`×`height` at `fps`.
pub(crate) fn best_matched(
    modes: &[CameraMode],
    width: i32,
    height: i32,
    fps: i32,
) -> Option<CameraMode> {
    modes
        .iter()
        .min_by(|a, b| {
            // A mode with the request's shape needs no cropping or bars.
            same_aspect(b, width, height)
                .cmp(&same_aspect(a, width, height))
                .then_with(|| closer(i64::from(a.height), i64::from(b.height), i64::from(height)))
                .then_with(|| closer(i64::from(a.width), i64::from(b.width), i64::from(width)))
                .then_with(|| closer(mhz(a.max_fps), mhz(b.max_fps), i64::from(fps) * 1000))
                .then_with(|| format_rank(a).cmp(&format_rank(b)))
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(media: &str, format: Option<&str>, w: i32, h: i32, fps: i32) -> CameraMode {
        CameraMode {
            media: media.into(),
            format: format.map(str::to_string),
            width: w,
            height: h,
            max_fps: gst::Fraction::new(fps, 1),
            min_fps: gst::Fraction::new(1, 1),
        }
    }

    /// A 4K webcam's modes, listed largest first as avfvideosrc lists them.
    fn webcam() -> Vec<CameraMode> {
        vec![
            mode("video/x-raw", Some("UYVY"), 3840, 2160, 30),
            mode("video/x-raw", Some("NV12"), 3840, 2160, 30),
            mode("video/x-raw", Some("NV12"), 1920, 1080, 60),
            mode("video/x-raw", Some("NV12"), 1280, 720, 60),
            mode("video/x-raw", Some("UYVY"), 960, 540, 30),
            mode("video/x-raw", Some("NV12"), 960, 540, 30),
            mode("video/x-raw", Some("NV12"), 640, 360, 30),
        ]
    }

    #[test]
    fn picks_the_smallest_mode_covering_the_request_not_the_largest() {
        let m = best_matched(&webcam(), 854, 480, 15).unwrap();
        assert_eq!((m.width, m.height), (960, 540));
        assert_eq!(m.format.as_deref(), Some("NV12"), "NV12 ranks before UYVY");
    }

    #[test]
    fn prefers_the_requested_aspect_ratio() {
        let modes = vec![
            mode("video/x-raw", Some("NV12"), 1024, 768, 30),
            mode("video/x-raw", Some("NV12"), 1280, 720, 30),
        ];
        let m = best_matched(&modes, 854, 480, 15).unwrap();
        assert_eq!((m.width, m.height), (1280, 720), "16:9 over a closer 4:3");
    }

    #[test]
    fn takes_the_largest_when_none_reaches_the_request() {
        let small = vec![
            mode("video/x-raw", Some("YUY2"), 320, 240, 30),
            mode("video/x-raw", Some("YUY2"), 640, 360, 30),
        ];
        let m = best_matched(&small, 854, 480, 15).unwrap();
        assert_eq!((m.width, m.height), (640, 360));
    }

    #[test]
    fn prefers_raw_over_mjpeg_at_the_same_size() {
        let modes = vec![
            mode("image/jpeg", None, 1280, 720, 30),
            mode("video/x-raw", Some("YUY2"), 1280, 720, 30),
        ];
        let m = best_matched(&modes, 1280, 720, 15).unwrap();
        assert_eq!(m.media, "video/x-raw");
    }

    #[test]
    fn asks_for_the_requested_rate_within_a_mode_range() {
        let m = mode("video/x-raw", Some("NV12"), 960, 540, 30);
        assert_eq!(m.rate_for(15), gst::Fraction::new(15, 1));
        assert_eq!(m.rate_for(60), gst::Fraction::new(30, 1));
    }

    #[test]
    fn reads_modes_from_device_caps() {
        gst::init().unwrap();
        let caps = gst::Caps::builder_full()
            .structure(
                gst::Structure::builder("video/x-raw")
                    .field("format", "NV12")
                    .field("width", 960i32)
                    .field("height", 540i32)
                    .field(
                        "framerate",
                        gst::FractionRange::new(
                            gst::Fraction::new(1, 1),
                            gst::Fraction::new(30, 1),
                        ),
                    )
                    .build(),
            )
            .structure(
                gst::Structure::builder("image/jpeg")
                    .field("width", 1920i32)
                    .field("height", 1080i32)
                    .field("framerate", gst::Fraction::new(30, 1))
                    .build(),
            )
            .build();
        let modes = modes(&caps);
        assert_eq!(modes.len(), 2);
        let pinned = modes[0].caps(15);
        let s = pinned.structure(0).unwrap();
        assert_eq!(s.get::<i32>("width").unwrap(), 960);
        assert_eq!(
            s.get::<gst::Fraction>("framerate").unwrap(),
            gst::Fraction::new(15, 1)
        );
    }
}
