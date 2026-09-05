//! Video file type plugin: core and presentation halves.
//!
//! One plugin covers the common video containers (MP4, Matroska, `WebM`,
//! AVI), matching how the existing `audio` plugin covers multiple audio
//! codecs with one crate. The view is metadata only (format, duration,
//! resolution, codecs), not a decoded thumbnail or playback frames: per
//! `plugin-api`'s presentation half, a front end gets lines of text, not
//! pixels.

use matroska::Matroska;
use mp4::{Mp4Reader, TrackType};
use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::path::Path;

/// MP4 major/compatible brands that identify a video container, distinct
/// from the audio-only `M4A `/`M4B `/`M4P ` brands the `audio` plugin's own
/// (brand-agnostic) `ftyp` check already claims.
const VIDEO_MP4_BRANDS: &[&[u8; 4]] = &[
    b"isom", b"iso2", b"iso3", b"iso4", b"iso5", b"iso6", b"mp41", b"mp42", b"avc1", b"M4V ",
    b"qt  ", b"3gp4", b"3gp5", b"3g2a", b"mmp4", b"dash",
];

/// Whether `prefix` is an MPEG-4 container (`.mp4`, `.mov`, ...) carrying a
/// video-specific brand: an `ftyp` box whose major brand is one of
/// [`VIDEO_MP4_BRANDS`], checked ahead of `audio`'s own looser `ftyp` check
/// in `CORE_PLUGINS` so a real video file is claimed here first.
fn is_mp4(prefix: &[u8]) -> bool {
    prefix.len() >= 12
        && &prefix[4..8] == b"ftyp"
        && VIDEO_MP4_BRANDS
            .iter()
            .any(|brand| &prefix[8..12] == brand.as_slice())
}

/// Whether `prefix` opens with the EBML header ID every Matroska (`.mkv`)
/// and `WebM` (`.webm`) file shares, `WebM` being a restricted Matroska
/// profile.
fn is_matroska(prefix: &[u8]) -> bool {
    prefix.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
}

/// Whether `prefix` is an AVI file: a RIFF container declaring an AVI
/// form, distinct from `audio`'s own RIFF/WAVE check.
fn is_avi(prefix: &[u8]) -> bool {
    prefix.len() >= 12 && prefix.starts_with(b"RIFF") && &prefix[8..12] == b"AVI "
}

/// View data produced by [`VideoCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoView {
    /// The detected container format, e.g. `"MP4"`, `"Matroska"`, `"WebM"`, `"AVI"`.
    pub format: String,
    /// Duration in seconds.
    pub duration_secs: f64,
    /// Pixel width of the video track, if known.
    pub width: Option<u32>,
    /// Pixel height of the video track, if known.
    pub height: Option<u32>,
    /// Video codec identifier, if known.
    pub video_codec: Option<String>,
    /// Audio codec identifier, if an audio track is present.
    pub audio_codec: Option<String>,
    /// Size of the file on disk, in bytes.
    pub file_size: u64,
}

/// Reads `path` as an MP4 container.
fn view_mp4(path: &Path, file_size: u64) -> io::Result<VideoView> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let reader = Mp4Reader::read_header(BufReader::new(file), size)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let mut width = None;
    let mut height = None;
    let mut video_codec = None;
    let mut audio_codec = None;
    for track in reader.tracks().values() {
        match track.track_type() {
            Ok(TrackType::Video) => {
                width = Some(u32::from(track.width()));
                height = Some(u32::from(track.height()));
                video_codec = track.media_type().ok().map(|media| media.to_string());
            }
            Ok(TrackType::Audio) => {
                audio_codec = track.media_type().ok().map(|media| media.to_string());
            }
            _ => {}
        }
    }
    Ok(VideoView {
        format: "MP4".to_owned(),
        duration_secs: reader.duration().as_secs_f64(),
        width,
        height,
        video_codec,
        audio_codec,
        file_size,
    })
}

/// Reads `path` as a Matroska or `WebM` container. The `matroska` crate
/// exposes no `DocType` accessor, so `WebM` is distinguished from plain
/// Matroska by scanning `prefix` for the literal `webm` `DocType` string
/// every `WebM` file's EBML header carries near its start (well within the
/// bounded sniff prefix) - a marker-in-raw-prefix approach matching this
/// project's other structurally-sniffed formats.
fn view_matroska(path: &Path, prefix: &[u8], file_size: u64) -> io::Result<VideoView> {
    let file = File::open(path)?;
    let mkv =
        Matroska::open(file).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let format = if prefix.windows(4).any(|window| window == b"webm") {
        "WebM"
    } else {
        "Matroska"
    };
    let video_track = mkv.tracks.iter().find(|track| track.is_video());
    let audio_track = mkv.tracks.iter().find(|track| track.is_audio());
    let (width, height) = match video_track.map(|track| &track.settings) {
        Some(matroska::Settings::Video(video)) => (
            u32::try_from(video.pixel_width).ok(),
            u32::try_from(video.pixel_height).ok(),
        ),
        _ => (None, None),
    };
    Ok(VideoView {
        format: format.to_owned(),
        duration_secs: mkv
            .info
            .duration
            .map_or(0.0, |duration| duration.as_secs_f64()),
        width,
        height,
        video_codec: video_track.map(|track| track.codec_id.clone()),
        audio_codec: audio_track.map(|track| track.codec_id.clone()),
        file_size,
    })
}

/// Recursively walks RIFF chunks in `bytes` looking for the `avih` main AVI
/// header chunk, descending into `LIST` chunks since `avih` lives nested
/// inside the `hdrl` list rather than at the top level.
fn find_avih_chunk(bytes: &[u8]) -> Option<&[u8]> {
    let mut offset = 0;
    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size =
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let body_start = offset + 8;
        let body_end = (body_start + chunk_size).min(bytes.len());
        if chunk_id == b"avih" {
            return Some(&bytes[body_start..body_end]);
        }
        if chunk_id == b"LIST"
            && body_start + 4 <= body_end
            && let Some(found) = find_avih_chunk(&bytes[body_start + 4..body_end])
        {
            return Some(found);
        }
        offset = body_start + chunk_size + (chunk_size % 2);
    }
    None
}

/// Reads `path` as an AVI file, hand-parsing the RIFF `avih` main header
/// chunk for resolution and duration: no well-established minimal-dependency
/// AVI-reading crate matched this project's pattern, so this follows
/// `word-document`'s precedent of a hand-rolled reader for a
/// less-common container.
fn view_avi(path: &Path, file_size: u64) -> io::Result<VideoView> {
    let bytes = std::fs::read(path)?;
    let mut width = None;
    let mut height = None;
    let mut duration_secs = 0.0;
    // The `avih` chunk is an `AVIMAINHEADER` structure per the OpenDML
    // spec: a sequence of 4-byte fields - microseconds-per-frame, max
    // bytes/sec, padding granularity, flags, total frame count, initial
    // frames, stream count, suggested buffer size, then width and height.
    if let Some(avih) = find_avih_chunk(bytes.get(12..).unwrap_or(&[]))
        && avih.len() >= 40
    {
        let field = |n: usize| -> u32 {
            let start = n * 4;
            u32::from_le_bytes(avih[start..start + 4].try_into().unwrap())
        };
        let microsec_per_frame = field(0);
        let total_frames = field(4);
        width = Some(field(8));
        height = Some(field(9));
        if microsec_per_frame > 0 {
            duration_secs = f64::from(total_frames) * f64::from(microsec_per_frame) / 1_000_000.0;
        }
    }
    Ok(VideoView {
        format: "AVI".to_owned(),
        duration_secs,
        width,
        height,
        video_codec: None,
        audio_codec: None,
        file_size,
    })
}

/// The video plugin's core half.
#[derive(Debug, Default)]
pub struct VideoCore;

impl PluginCore for VideoCore {
    fn name(&self) -> &'static str {
        "video"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        is_mp4(prefix) || is_matroska(prefix) || is_avi(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let prefix = {
            let mut buf = [0u8; 512];
            let file = File::open(path)?;
            let mut reader = BufReader::new(file);
            let n = io::Read::read(&mut reader, &mut buf)?;
            buf[..n].to_vec()
        };
        let file_size = std::fs::metadata(path)?.len();
        let view = if is_mp4(&prefix) {
            view_mp4(path, file_size)?
        } else if is_matroska(&prefix) {
            view_matroska(path, &prefix, file_size)?
        } else {
            view_avi(path, file_size)?
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The video plugin's presentation half.
#[derive(Debug, Default)]
pub struct VideoPresentation;

impl PluginPresentation for VideoPresentation {
    fn name(&self) -> &'static str {
        "video"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: VideoView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![
            format!("{} video", view.format),
            format!("{:.1} seconds", view.duration_secs),
        ];
        if let (Some(width), Some(height)) = (view.width, view.height) {
            lines.push(format!("{width}x{height}"));
        }
        if let Some(video_codec) = &view.video_codec {
            lines.push(format!("Video codec: {video_codec}"));
        }
        if let Some(audio_codec) = &view.audio_codec {
            lines.push(format!("Audio codec: {audio_codec}"));
        }
        lines.push(format!("{} bytes on disk", view.file_size));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{VideoCore, VideoPresentation, VideoView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-video-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn sniffs_an_mp4_video_brand() {
        let mut prefix = vec![0, 0, 0, 0x18];
        prefix.extend_from_slice(b"ftypisom");
        assert!(VideoCore.sniff(&prefix));
    }

    #[test]
    fn does_not_sniff_an_m4a_audio_brand() {
        let mut prefix = vec![0, 0, 0, 0x18];
        prefix.extend_from_slice(b"ftypM4A ");
        assert!(!VideoCore.sniff(&prefix));
    }

    #[test]
    fn sniffs_a_matroska_magic() {
        assert!(VideoCore.sniff(&[0x1A, 0x45, 0xDF, 0xA3, 0x00]));
    }

    #[test]
    fn sniffs_an_avi_header() {
        let mut prefix = Vec::from(*b"RIFF");
        prefix.extend_from_slice(&[0, 0, 0, 0]);
        prefix.extend_from_slice(b"AVI ");
        assert!(VideoCore.sniff(&prefix));
    }

    #[test]
    fn does_not_sniff_a_plain_riff_that_is_not_avi() {
        let mut prefix = Vec::from(*b"RIFF");
        prefix.extend_from_slice(&[0, 0, 0, 0]);
        prefix.extend_from_slice(b"WAVE");
        assert!(!VideoCore.sniff(&prefix));
    }

    #[test]
    fn does_not_sniff_unrelated_bytes() {
        assert!(!VideoCore.sniff(b"not a video"));
    }

    /// A minimal, valid AVI file: RIFF/AVI header, a `hdrl` LIST containing
    /// an `avih` main header with a fixed frame rate/count/resolution -
    /// enough for the hand-rolled reader to compute duration and
    /// resolution.
    fn write_test_avi(path: &std::path::Path) {
        let mut avih_body = vec![0u8; 56];
        avih_body[0..4].copy_from_slice(&40_000u32.to_le_bytes()); // microsec per frame
        avih_body[16..20].copy_from_slice(&10u32.to_le_bytes()); // total frames
        avih_body[32..36].copy_from_slice(&320u32.to_le_bytes()); // width
        avih_body[36..40].copy_from_slice(&240u32.to_le_bytes()); // height

        let mut avih_chunk = Vec::new();
        avih_chunk.extend_from_slice(b"avih");
        avih_chunk.extend_from_slice(&u32::try_from(avih_body.len()).unwrap().to_le_bytes());
        avih_chunk.extend_from_slice(&avih_body);

        let mut hdrl_list = Vec::new();
        hdrl_list.extend_from_slice(b"LIST");
        hdrl_list.extend_from_slice(&u32::try_from(4 + avih_chunk.len()).unwrap().to_le_bytes());
        hdrl_list.extend_from_slice(b"hdrl");
        hdrl_list.extend_from_slice(&avih_chunk);

        let mut riff_body = Vec::new();
        riff_body.extend_from_slice(b"AVI ");
        riff_body.extend_from_slice(&hdrl_list);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&u32::try_from(riff_body.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&riff_body);

        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn views_a_real_avi_file_for_resolution_and_duration() {
        let path = unique_temp_file("test.avi");
        write_test_avi(&path);

        let data = VideoCore.view(&path).unwrap();
        let view: VideoView = serde_json::from_value(data).unwrap();

        assert_eq!(view.format, "AVI");
        assert_eq!(view.width, Some(320));
        assert_eq!(view.height, Some(240));
        assert!((view.duration_secs - 0.4).abs() < 1e-9);
        assert!(view.file_size > 0);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_format_and_playback_metadata() {
        let data = serde_json::to_value(VideoView {
            format: "MP4".to_owned(),
            duration_secs: 12.5,
            width: Some(1920),
            height: Some(1080),
            video_codec: Some("H264".to_owned()),
            audio_codec: Some("AAC".to_owned()),
            file_size: 4096,
        })
        .unwrap();

        let lines = VideoPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "MP4 video",
                "12.5 seconds",
                "1920x1080",
                "Video codec: H264",
                "Audio codec: AAC",
                "4096 bytes on disk",
            ]
        );
    }
}
